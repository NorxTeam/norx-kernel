use crate::drivers::block;

const SECTOR_SIZE: usize = 512;
const MAX_BLOCK_SIZE: usize = 4096;
const MAX_COMPONENTS: usize = 16;
const MAX_DIRECTORY_BLOCKS: u64 = 64;
const EXT4_MAGIC: u16 = 0xef53;
const EXT4_EXTENTS_FL: u32 = 0x0008_0000;
const INCOMPAT_FILETYPE: u32 = 0x0002;
const INCOMPAT_RECOVER: u32 = 0x0004;
const INCOMPAT_EXTENTS: u32 = 0x0040;
const INCOMPAT_64BIT: u32 = 0x0080;
const COMPAT_HAS_JOURNAL: u32 = 0x0004;

pub type ReadSector = fn(u64, &mut [u8; SECTOR_SIZE]) -> bool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Io,
    InvalidSuperblock,
    UnsupportedFeature,
    JournalRecoveryRequired,
    InvalidPath,
    NotFound,
    NotDirectory,
    IsDirectory,
    BadExtent,
    BadDirectory,
    BufferTooSmall,
    PermissionDenied,
    ReadOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub block_size: u32,
    pub blocks: u64,
    pub groups: u32,
    pub inode_size: u16,
    pub has_journal: bool,
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub mode: u16,
    pub size: u64,
    pub directory: bool,
}

#[derive(Clone, Copy)]
pub struct Mount {
    reader: ReadSector,
    base_lba: u64,
    block_size: u32,
    blocks: u64,
    inodes_per_group: u32,
    inode_size: u16,
    groups: u32,
    descriptor_size: u16,
    has_journal: bool,
}

#[derive(Clone, Copy)]
struct Inode {
    mode: u16,
    flags: u32,
    size: u64,
    extents: [u8; 60],
}

impl Mount {
    pub fn open(reader: ReadSector) -> Result<Self, Error> {
        Self::open_at(reader, 0, u64::MAX)
    }

    pub fn open_at(
        reader: ReadSector,
        base_lba: u64,
        available_sectors: u64,
    ) -> Result<Self, Error> {
        let mut superblock = [0u8; 1024];
        let mut sector = [0u8; SECTOR_SIZE];
        let sector_lba = |relative: u64| base_lba.checked_add(relative);
        if !sector_lba(2).is_some_and(|lba| reader(lba, &mut sector)) {
            return Err(Error::Io);
        }
        superblock[..SECTOR_SIZE].copy_from_slice(&sector);
        if !sector_lba(3).is_some_and(|lba| reader(lba, &mut sector)) {
            return Err(Error::Io);
        }
        superblock[SECTOR_SIZE..].copy_from_slice(&sector);
        if le_u16(&superblock, 0x38) != EXT4_MAGIC {
            return Err(Error::InvalidSuperblock);
        }
        let log_block_size = le_u32(&superblock, 0x18);
        if log_block_size > 2 {
            return Err(Error::UnsupportedFeature);
        }
        let block_size = 1024u32 << log_block_size;
        let blocks = le_u32(&superblock, 0x04) as u64 | (le_u32(&superblock, 0x150) as u64) << 32;
        let first_data_block = le_u32(&superblock, 0x14);
        let blocks_per_group = le_u32(&superblock, 0x20);
        let inodes_per_group = le_u32(&superblock, 0x28);
        let inode_size = le_u16(&superblock, 0x58);
        let descriptor_size = match le_u16(&superblock, 0xfe) {
            0 => 32,
            value => value,
        };
        let compat = le_u32(&superblock, 0x5c);
        let incompat = le_u32(&superblock, 0x60);
        if blocks <= first_data_block as u64
            || blocks_per_group == 0
            || inodes_per_group == 0
            || !matches!(inode_size, 128 | 256)
            || descriptor_size < 32
            || descriptor_size as u32 > block_size
            || incompat & INCOMPAT_RECOVER != 0
            || incompat & INCOMPAT_64BIT != 0
            || incompat & !(INCOMPAT_FILETYPE | INCOMPAT_EXTENTS) != 0
            || incompat & INCOMPAT_EXTENTS == 0
            || blocks.checked_mul(block_size as u64).is_none()
        {
            return if incompat & INCOMPAT_RECOVER != 0 {
                Err(Error::JournalRecoveryRequired)
            } else {
                Err(Error::InvalidSuperblock)
            };
        }
        let groups = (blocks - first_data_block as u64)
            .div_ceil(blocks_per_group as u64)
            .try_into()
            .map_err(|_| Error::InvalidSuperblock)?;
        let inode_count = (groups as u64)
            .checked_mul(inodes_per_group as u64)
            .ok_or(Error::InvalidSuperblock)?;
        if inode_count < 2 {
            return Err(Error::InvalidSuperblock);
        }
        if blocks
            .checked_mul(block_size as u64)
            .is_none_or(|bytes| bytes.div_ceil(SECTOR_SIZE as u64) > available_sectors)
            || base_lba
                .checked_add(blocks * block_size as u64 / SECTOR_SIZE as u64)
                .is_none()
        {
            return Err(Error::InvalidSuperblock);
        }
        Ok(Self {
            reader,
            base_lba,
            block_size,
            blocks,
            inodes_per_group,
            inode_size,
            groups,
            descriptor_size,
            has_journal: compat & COMPAT_HAS_JOURNAL != 0,
        })
    }

    pub const fn geometry(self) -> Geometry {
        Geometry {
            block_size: self.block_size,
            blocks: self.blocks,
            groups: self.groups,
            inode_size: self.inode_size,
            has_journal: self.has_journal,
            read_only: true,
        }
    }

    pub fn stat(self, path: &str) -> Result<FileInfo, Error> {
        let inode = self.read_inode(self.resolve(path)?)?;
        Ok(FileInfo {
            mode: inode.mode,
            size: inode.size,
            directory: is_directory(inode.mode),
        })
    }

    pub fn read_file(self, path: &str, output: &mut [u8]) -> Result<usize, Error> {
        let inode = self.read_inode(self.resolve(path)?)?;
        if is_directory(inode.mode) {
            return Err(Error::IsDirectory);
        }
        if inode.mode & 0o444 == 0 {
            return Err(Error::PermissionDenied);
        }
        let size = usize::try_from(inode.size).map_err(|_| Error::BufferTooSmall)?;
        if size > output.len() {
            return Err(Error::BufferTooSmall);
        }
        let mut block = [0u8; MAX_BLOCK_SIZE];
        let mut copied = 0;
        while copied < size {
            let logical = (copied / self.block_size as usize) as u32;
            let physical = self.extent_block(&inode, logical)?;
            self.read_block(physical, &mut block)?;
            let offset = copied % self.block_size as usize;
            let length = (size - copied).min(self.block_size as usize - offset);
            output[copied..copied + length].copy_from_slice(&block[offset..offset + length]);
            copied += length;
        }
        Ok(size)
    }

    pub fn write_file(self, _path: &str, _input: &[u8]) -> Result<usize, Error> {
        Err(Error::ReadOnly)
    }

    fn resolve(self, path: &str) -> Result<u32, Error> {
        let (components, count) = components(path)?;
        let mut inode = 2;
        for component in components.iter().take(count) {
            inode = self.find_in_directory(inode, component)?;
        }
        Ok(inode)
    }

    fn find_in_directory(self, inode_number: u32, target: &str) -> Result<u32, Error> {
        let inode = self.read_inode(inode_number)?;
        if !is_directory(inode.mode) {
            return Err(Error::NotDirectory);
        }
        let block_count = inode.size.div_ceil(self.block_size as u64);
        if block_count > MAX_DIRECTORY_BLOCKS {
            return Err(Error::UnsupportedFeature);
        }
        let mut block = [0u8; MAX_BLOCK_SIZE];
        for logical in 0..block_count as u32 {
            let physical = self.extent_block(&inode, logical)?;
            self.read_block(physical, &mut block)?;
            let mut offset = 0usize;
            let block_size = self.block_size as usize;
            while offset < block_size {
                if offset + 8 > block_size {
                    return Err(Error::BadDirectory);
                }
                let child = le_u32(&block, offset);
                let record_length = le_u16(&block, offset + 4) as usize;
                let name_length = block[offset + 6] as usize;
                if record_length < 8
                    || !record_length.is_multiple_of(4)
                    || offset + record_length > block_size
                    || name_length > record_length - 8
                {
                    return Err(Error::BadDirectory);
                }
                if child != 0
                    && name_length == target.len()
                    && &block[offset + 8..offset + 8 + name_length] == target.as_bytes()
                {
                    return Ok(child);
                }
                offset += record_length;
            }
        }
        Err(Error::NotFound)
    }

    fn read_inode(self, inode_number: u32) -> Result<Inode, Error> {
        if inode_number < 1 {
            return Err(Error::NotFound);
        }
        let index = inode_number as u64 - 1;
        let group = index / self.inodes_per_group as u64;
        if group >= self.groups as u64 {
            return Err(Error::NotFound);
        }
        let descriptor_block = if self.block_size == 1024 { 2 } else { 1 };
        let descriptor_offset =
            descriptor_block as u64 * self.block_size as u64 + group * self.descriptor_size as u64;
        let mut descriptor = [0u8; 64];
        self.read_at(descriptor_offset, &mut descriptor)?;
        let inode_table = le_u32(&descriptor, 8) as u64;
        if inode_table >= self.blocks {
            return Err(Error::InvalidSuperblock);
        }
        let inode_offset = inode_table
            .checked_mul(self.block_size as u64)
            .and_then(|offset| {
                offset.checked_add((index % self.inodes_per_group as u64) * self.inode_size as u64)
            })
            .ok_or(Error::InvalidSuperblock)?;
        let mut bytes = [0u8; 256];
        self.read_at(inode_offset, &mut bytes[..self.inode_size as usize])?;
        let mut extents = [0u8; 60];
        extents.copy_from_slice(&bytes[40..100]);
        Ok(Inode {
            mode: le_u16(&bytes, 0),
            flags: le_u32(&bytes, 32),
            size: le_u32(&bytes, 4) as u64 | (le_u32(&bytes, 108) as u64) << 32,
            extents,
        })
    }

    fn extent_block(self, inode: &Inode, logical: u32) -> Result<u64, Error> {
        if inode.flags & EXT4_EXTENTS_FL == 0 {
            return Err(Error::UnsupportedFeature);
        }
        if le_u16(&inode.extents, 0) != 0xf30a {
            return Err(Error::BadExtent);
        }
        let entries = le_u16(&inode.extents, 2) as usize;
        if le_u16(&inode.extents, 6) != 0 || entries > 4 {
            return Err(Error::UnsupportedFeature);
        }
        for index in 0..entries {
            let offset = 12 + index * 12;
            let first = le_u32(&inode.extents, offset);
            let raw_length = le_u16(&inode.extents, offset + 4);
            if raw_length & 0x8000 != 0 {
                return Err(Error::UnsupportedFeature);
            }
            let length = raw_length as u32;
            if length == 0 || logical < first || logical - first >= length {
                continue;
            }
            let physical = le_u32(&inode.extents, offset + 8) as u64
                | (le_u16(&inode.extents, offset + 6) as u64) << 32;
            let block = physical + (logical - first) as u64;
            return (block < self.blocks)
                .then_some(block)
                .ok_or(Error::BadExtent);
        }
        Err(Error::BadExtent)
    }

    fn read_block(self, block: u64, output: &mut [u8; MAX_BLOCK_SIZE]) -> Result<(), Error> {
        if block >= self.blocks {
            return Err(Error::BadExtent);
        }
        let offset = block
            .checked_mul(self.block_size as u64)
            .ok_or(Error::BadExtent)?;
        self.read_at(offset, &mut output[..self.block_size as usize])
    }

    fn read_at(self, offset: u64, output: &mut [u8]) -> Result<(), Error> {
        let end = offset.checked_add(output.len() as u64).ok_or(Error::Io)?;
        if end > self.blocks * self.block_size as u64 {
            return Err(Error::Io);
        }
        let mut copied = 0usize;
        while copied < output.len() {
            let position = offset.checked_add(copied as u64).ok_or(Error::Io)?;
            let sector_offset = position as usize % SECTOR_SIZE;
            let mut sector = [0u8; SECTOR_SIZE];
            if !self.read_sector(position / SECTOR_SIZE as u64, &mut sector) {
                return Err(Error::Io);
            }
            let length = (output.len() - copied).min(SECTOR_SIZE - sector_offset);
            output[copied..copied + length]
                .copy_from_slice(&sector[sector_offset..sector_offset + length]);
            copied += length;
        }
        Ok(())
    }

    fn read_sector(self, lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
        self.base_lba
            .checked_add(lba)
            .is_some_and(|absolute| (self.reader)(absolute, output))
    }
}

pub fn probe_block() -> Result<Mount, Error> {
    let partition = block::partition(0).ok_or(Error::InvalidSuperblock)?;
    Mount::open_at(ramdisk_read_sector, partition.start_lba, partition.sectors)
}

pub fn contract_self_check() {
    assert!(fixture_check());
}

fn ramdisk_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    usize::try_from(lba)
        .ok()
        .is_some_and(|lba| block::read_sector(lba, output))
}

fn fixture_check() -> bool {
    let Ok(volume) = Mount::open(fixture_read_sector) else {
        return false;
    };
    let geometry = volume.geometry();
    if geometry.block_size != 1024 || !geometry.has_journal || !geometry.read_only {
        return false;
    }
    let Ok(info) = volume.stat("/hello.txt") else {
        return false;
    };
    if info.directory || info.mode & 0o777 != 0o644 || info.size != 13 {
        return false;
    }
    if !volume
        .read_file("/", &mut [0u8; 1])
        .is_err_and(|error| error == Error::IsDirectory)
    {
        return false;
    }
    let mut output = [0u8; 32];
    let Ok(length) = volume.read_file("/hello.txt", &mut output) else {
        return false;
    };
    if length != 13 || &output[..length] != b"ext4 fixture\n" {
        return false;
    }
    if !volume
        .read_file("/hello.txt", &mut [0u8; 4])
        .is_err_and(|error| error == Error::BufferTooSmall)
    {
        return false;
    }
    let Ok(partitioned) = Mount::open_at(fixture_partition_read_sector, 100, 16_384) else {
        return false;
    };
    let mut partitioned_output = [0u8; 32];
    if partitioned
        .read_file("/hello.txt", &mut partitioned_output)
        .is_err()
    {
        return false;
    }
    volume
        .write_file("/hello.txt", b"rejected")
        .is_err_and(|error| error == Error::ReadOnly)
}

fn fixture_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    output.fill(0);
    match lba {
        2 => fixture_superblock(output),
        4 => write_u32(output, 8, 4),
        8 => fixture_inode_table(output),
        10 => fixture_directory(output),
        12 => output[..13].copy_from_slice(b"ext4 fixture\n"),
        _ => {}
    }
    true
}

fn fixture_partition_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    lba.checked_sub(100)
        .is_some_and(|relative| fixture_read_sector(relative, output))
}

fn fixture_superblock(output: &mut [u8; SECTOR_SIZE]) {
    write_u32(output, 4, 8192);
    write_u32(output, 20, 1);
    write_u32(output, 24, 0);
    write_u32(output, 32, 8192);
    write_u32(output, 40, 16);
    write_u16(output, 56, 1);
    write_u16(output, 88, 128);
    write_u32(output, 92, COMPAT_HAS_JOURNAL);
    write_u32(output, 96, INCOMPAT_EXTENTS | INCOMPAT_FILETYPE);
    write_u32(output, 100, 0);
    write_u16(output, 0x38, EXT4_MAGIC);
    write_u16(output, 0xfe, 32);
}

fn fixture_inode_table(output: &mut [u8; SECTOR_SIZE]) {
    write_inode(output, 128, 0x41ed, 1024, 5);
    write_inode(output, 256, 0x81a4, 13, 6);
}

fn write_inode(output: &mut [u8; SECTOR_SIZE], offset: usize, mode: u16, size: u32, block: u32) {
    write_u16(output, offset, mode);
    write_u32(output, offset + 4, size);
    write_u32(output, offset + 32, EXT4_EXTENTS_FL);
    write_u16(output, offset + 40, 0xf30a);
    write_u16(output, offset + 42, 1);
    write_u16(output, offset + 44, 4);
    write_u32(output, offset + 60, block);
    write_u16(output, offset + 56, 1);
}

fn fixture_directory(output: &mut [u8; SECTOR_SIZE]) {
    write_directory_entry(output, 0, 2, 12, 1, 2, b".");
    write_directory_entry(output, 12, 2, 12, 2, 2, b"..");
    write_directory_entry(output, 24, 3, 1000, 9, 1, b"hello.txt");
}

fn write_directory_entry(
    output: &mut [u8; SECTOR_SIZE],
    offset: usize,
    inode: u32,
    record_length: u16,
    name_length: u8,
    file_type: u8,
    name: &[u8],
) {
    write_u32(output, offset, inode);
    write_u16(output, offset + 4, record_length);
    output[offset + 6] = name_length;
    output[offset + 7] = file_type;
    output[offset + 8..offset + 8 + name.len()].copy_from_slice(name);
}

fn components(path: &str) -> Result<([&str; MAX_COMPONENTS], usize), Error> {
    let bytes = path.as_bytes();
    if !bytes.starts_with(b"/") {
        return Err(Error::InvalidPath);
    }
    let mut result = [""; MAX_COMPONENTS];
    let mut count = 0;
    let mut start = 1;
    for (index, byte) in bytes.iter().enumerate().skip(1) {
        if *byte != b'/' {
            continue;
        }
        if index > start {
            if count == MAX_COMPONENTS || &path[start..index] == "." || &path[start..index] == ".."
            {
                return Err(Error::InvalidPath);
            }
            result[count] = &path[start..index];
            count += 1;
        }
        start = index + 1;
    }
    if start < bytes.len() {
        if count == MAX_COMPONENTS || &path[start..] == "." || &path[start..] == ".." {
            return Err(Error::InvalidPath);
        }
        result[count] = &path[start..];
        count += 1;
    }
    Ok((result, count))
}

fn is_directory(mode: u16) -> bool {
    mode & 0xf000 == 0x4000
}

fn le_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn le_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
