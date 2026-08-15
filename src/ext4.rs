use crate::drivers::block;

const SECTOR_SIZE: usize = 512;
const MAX_BLOCK_SIZE: usize = 4096;
const MAX_COMPONENTS: usize = 16;
const MAX_DIRECTORY_BLOCKS: u64 = 64;
pub const MAX_NAME_LENGTH: usize = 255;
const EXT4_MAGIC: u16 = 0xef53;
const EXT4_EXTENTS_FL: u32 = 0x0008_0000;
const INCOMPAT_FILETYPE: u32 = 0x0002;
const INCOMPAT_RECOVER: u32 = 0x0004;
const INCOMPAT_EXTENTS: u32 = 0x0040;
const INCOMPAT_64BIT: u32 = 0x0080;
const COMPAT_HAS_JOURNAL: u32 = 0x0004;
const RO_COMPAT_GDT_CSUM: u32 = 0x0010;
const RO_COMPAT_METADATA_CSUM: u32 = 0x0400;
const EXT4_VALID_FS: u16 = 0x0001;
const EXT4_ERROR_FS: u16 = 0x0002;
const EXT4_ORPHAN_FS: u16 = 0x0004;
const JBD2_MAGIC: u32 = 0xc03b_3998;
const JBD2_DESCRIPTOR_BLOCK: u32 = 1;
const JBD2_COMMIT_BLOCK: u32 = 2;
const JBD2_SUPERBLOCK_V2: u32 = 4;
const JBD2_FLAG_LAST_TAG: u32 = 0x0000_0008;
const MAX_REWRITE_BLOCKS: usize = 8;
const MAX_TRANSACTION_BLOCKS: usize = MAX_REWRITE_BLOCKS + 1;

pub type ReadSector = fn(u64, &mut [u8; SECTOR_SIZE]) -> bool;
/// A synchronous, write-through sector boundary. A false result means the
/// sector was not committed, and callers may retry or roll back it.
pub type WriteSector = fn(u64, &[u8; SECTOR_SIZE]) -> bool;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub inode: u32,
    pub mode: u16,
    pub size: u64,
    pub directory: bool,
    pub name: [u8; MAX_NAME_LENGTH],
    pub name_length: usize,
}

impl DirectoryEntry {
    pub const EMPTY: Self = Self {
        inode: 0,
        mode: 0,
        size: 0,
        directory: false,
        name: [0; MAX_NAME_LENGTH],
        name_length: 0,
    };
}

#[derive(Clone, Copy)]
pub struct Mount {
    reader: ReadSector,
    writer: Option<WriteSector>,
    base_lba: u64,
    block_size: u32,
    blocks: u64,
    inodes_per_group: u32,
    inode_size: u16,
    groups: u32,
    descriptor_size: u16,
    has_journal: bool,
    journal_inode: u32,
    journal: Option<Journal>,
}

#[derive(Clone, Copy)]
struct Journal {
    inode: u32,
    max_length: u32,
    first: u32,
    sequence: u32,
}

#[derive(Clone, Copy)]
struct TransactionBlock {
    block: u64,
    old: [u8; MAX_BLOCK_SIZE],
    new: [u8; MAX_BLOCK_SIZE],
}

impl TransactionBlock {
    const EMPTY: Self = Self {
        block: 0,
        old: [0; MAX_BLOCK_SIZE],
        new: [0; MAX_BLOCK_SIZE],
    };
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

    pub fn open_rw(reader: ReadSector, writer: WriteSector) -> Result<Self, Error> {
        Self::open_at_rw(reader, writer, 0, u64::MAX)
    }

    pub fn open_at(
        reader: ReadSector,
        base_lba: u64,
        available_sectors: u64,
    ) -> Result<Self, Error> {
        Self::open_at_with_writer(reader, None, base_lba, available_sectors)
    }

    pub fn open_at_rw(
        reader: ReadSector,
        writer: WriteSector,
        base_lba: u64,
        available_sectors: u64,
    ) -> Result<Self, Error> {
        Self::open_at_with_writer(reader, Some(writer), base_lba, available_sectors)
    }

    fn open_at_with_writer(
        reader: ReadSector,
        writer: Option<WriteSector>,
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
        let ro_compat = le_u32(&superblock, 0x64);
        let incompat = le_u32(&superblock, 0x60);
        let state = le_u16(&superblock, 0x3a);
        let journal_inode = le_u32(&superblock, 0xe0);
        let journal_device = le_u32(&superblock, 0xe4);
        if ro_compat & (RO_COMPAT_GDT_CSUM | RO_COMPAT_METADATA_CSUM) != 0 {
            return Err(Error::UnsupportedFeature);
        }
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
        if writer.is_some()
            && (state & EXT4_VALID_FS == 0 || state & (EXT4_ERROR_FS | EXT4_ORPHAN_FS) != 0)
        {
            return Err(Error::JournalRecoveryRequired);
        }
        if writer.is_some() && (!has_journal(compat) || journal_inode == 0 || journal_device != 0) {
            return Err(Error::ReadOnly);
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
        let mut mount = Self {
            reader,
            writer,
            base_lba,
            block_size,
            blocks,
            inodes_per_group,
            inode_size,
            groups,
            descriptor_size,
            has_journal: compat & COMPAT_HAS_JOURNAL != 0,
            journal_inode,
            journal: None,
        };
        if mount.writer.is_some() {
            mount.journal = Some(mount.read_journal()?);
        }
        Ok(mount)
    }

    pub const fn geometry(self) -> Geometry {
        Geometry {
            block_size: self.block_size,
            blocks: self.blocks,
            groups: self.groups,
            inode_size: self.inode_size,
            has_journal: self.has_journal,
            read_only: self.writer.is_none() || self.journal.is_none(),
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

    pub fn read_dir(self, path: &str, output: &mut [DirectoryEntry]) -> Result<usize, Error> {
        let inode_number = self.resolve(path)?;
        let mut count = 0;
        self.walk_directory(inode_number, |child, name| {
            if name == b"." || name == b".." {
                return Ok(false);
            }
            if count == output.len() {
                return Ok(false);
            }
            let inode = self.read_inode(child)?;
            let name_length = name.len();
            let mut entry_name = [0u8; MAX_NAME_LENGTH];
            entry_name[..name_length].copy_from_slice(name);
            output[count] = DirectoryEntry {
                inode: child,
                mode: inode.mode,
                size: inode.size,
                directory: is_directory(inode.mode),
                name: entry_name,
                name_length,
            };
            count += 1;
            Ok(false)
        })?;
        Ok(count)
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

    pub fn write_file(self, path: &str, input: &[u8]) -> Result<usize, Error> {
        if self.writer.is_none() {
            return Err(Error::ReadOnly);
        }
        let journal = self.read_journal()?;
        let inode_number = self.resolve(path)?;
        let inode = self.read_inode(inode_number)?;
        if is_directory(inode.mode) {
            return Err(Error::IsDirectory);
        }
        if inode.mode & 0o222 == 0 {
            return Err(Error::PermissionDenied);
        }
        let capacity = self.extent_capacity(&inode)?;
        let block_size = self.block_size as usize;
        let max_bytes = capacity
            .checked_mul(self.block_size as u64)
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(Error::UnsupportedFeature)?;
        if input.len() > max_bytes {
            return Err(Error::UnsupportedFeature);
        }
        let old_size = usize::try_from(inode.size).map_err(|_| Error::UnsupportedFeature)?;
        let touched_bytes = old_size.max(input.len());
        let touched_blocks = touched_bytes.div_ceil(block_size);
        if touched_blocks > MAX_REWRITE_BLOCKS {
            return Err(Error::UnsupportedFeature);
        }

        let mut changes = [TransactionBlock::EMPTY; MAX_TRANSACTION_BLOCKS];
        let mut change_count = 0;
        for logical in 0..touched_blocks as u32 {
            let physical = self.extent_block(&inode, logical)?;
            let mut new_block = [0u8; MAX_BLOCK_SIZE];
            self.read_block(physical, &mut new_block)?;
            let old_block = new_block;
            new_block[..block_size].fill(0);
            let start = logical as usize * block_size;
            if start < input.len() {
                let length = (input.len() - start).min(block_size);
                new_block[..length].copy_from_slice(&input[start..start + length]);
            }
            if old_block[..block_size] != new_block[..block_size] {
                changes[change_count] = TransactionBlock {
                    block: physical,
                    old: old_block,
                    new: new_block,
                };
                change_count += 1;
            }
        }

        if input.len() != old_size {
            let (inode_block, inode_offset) = self.inode_location(inode_number)?;
            let index = if let Some(index) = find_change(&changes, change_count, inode_block) {
                index
            } else {
                let index = change_count;
                if index == changes.len() {
                    return Err(Error::UnsupportedFeature);
                }
                self.read_block(inode_block, &mut changes[index].old)?;
                changes[index].new = changes[index].old;
                changes[index].block = inode_block;
                change_count += 1;
                index
            };
            let size = u64::try_from(input.len()).map_err(|_| Error::UnsupportedFeature)?;
            write_u32(&mut changes[index].new, inode_offset + 4, size as u32);
            write_u32(
                &mut changes[index].new,
                inode_offset + 108,
                (size >> 32) as u32,
            );
        }

        if change_count == 0 {
            return Ok(input.len());
        }
        self.commit_transaction(journal, &mut changes[..change_count])?;
        Ok(input.len())
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
        let mut result = None;
        self.walk_directory(inode_number, |child, name| {
            if name == target.as_bytes() {
                result = Some(child);
                Ok(true)
            } else {
                Ok(false)
            }
        })?;
        result.ok_or(Error::NotFound)
    }

    fn walk_directory<F>(self, inode_number: u32, mut visitor: F) -> Result<(), Error>
    where
        F: FnMut(u32, &[u8]) -> Result<bool, Error>,
    {
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
            let block_start = logical as u64 * self.block_size as u64;
            let block_length = (inode.size - block_start).min(self.block_size as u64) as usize;
            let mut offset = 0usize;
            while offset < block_length {
                if offset + 8 > block_length {
                    return Err(Error::BadDirectory);
                }
                let child = le_u32(&block, offset);
                let record_length = le_u16(&block, offset + 4) as usize;
                let name_length = block[offset + 6] as usize;
                let file_type = block[offset + 7];
                if record_length < 8
                    || !record_length.is_multiple_of(4)
                    || offset + record_length > block_length
                    || name_length > record_length - 8
                    || file_type > 7
                    || (child != 0 && name_length == 0)
                {
                    return Err(Error::BadDirectory);
                }
                if child != 0 {
                    let name = &block[offset + 8..offset + 8 + name_length];
                    if visitor(child, name)? {
                        return Ok(());
                    }
                }
                offset += record_length;
            }
        }
        Ok(())
    }

    fn read_inode(self, inode_number: u32) -> Result<Inode, Error> {
        let (inode_block, inode_offset) = self.inode_location(inode_number)?;
        let mut block = [0u8; MAX_BLOCK_SIZE];
        self.read_block(inode_block, &mut block)?;
        let bytes = &block[inode_offset..inode_offset + self.inode_size as usize];
        let mut extents = [0u8; 60];
        extents.copy_from_slice(&bytes[40..100]);
        Ok(Inode {
            mode: le_u16(bytes, 0),
            flags: le_u32(bytes, 32),
            size: le_u32(bytes, 4) as u64 | (le_u32(bytes, 108) as u64) << 32,
            extents,
        })
    }

    fn inode_location(self, inode_number: u32) -> Result<(u64, usize), Error> {
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
        let block = inode_offset / self.block_size as u64;
        let offset = usize::try_from(inode_offset % self.block_size as u64)
            .map_err(|_| Error::InvalidSuperblock)?;
        if block >= self.blocks || offset + self.inode_size as usize > self.block_size as usize {
            return Err(Error::InvalidSuperblock);
        }
        Ok((block, offset))
    }

    fn extent_capacity(self, inode: &Inode) -> Result<u64, Error> {
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
        let mut next_logical = 0u32;
        let mut capacity = 0u64;
        for index in 0..entries {
            let offset = 12 + index * 12;
            let first = le_u32(&inode.extents, offset);
            let raw_length = le_u16(&inode.extents, offset + 4);
            if raw_length & 0x8000 != 0 {
                return Err(Error::UnsupportedFeature);
            }
            let length = raw_length as u32;
            if length == 0 || first != next_logical {
                return Err(Error::BadExtent);
            }
            let physical = le_u32(&inode.extents, offset + 8) as u64
                | (le_u16(&inode.extents, offset + 6) as u64) << 32;
            let end = physical
                .checked_add(length as u64)
                .ok_or(Error::BadExtent)?;
            if end > self.blocks {
                return Err(Error::BadExtent);
            }
            next_logical = next_logical.checked_add(length).ok_or(Error::BadExtent)?;
            capacity = capacity
                .checked_add(length as u64)
                .ok_or(Error::BadExtent)?;
        }
        (capacity != 0).then_some(capacity).ok_or(Error::BadExtent)
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

    fn read_journal(self) -> Result<Journal, Error> {
        let inode = self.read_inode(self.journal_inode)?;
        if is_directory(inode.mode) {
            return Err(Error::UnsupportedFeature);
        }
        let capacity = self.extent_capacity(&inode)?;
        if capacity < 3 || inode.size < u64::from(self.block_size) {
            return Err(Error::UnsupportedFeature);
        }
        let mut superblock = [0u8; MAX_BLOCK_SIZE];
        self.read_block(self.extent_block(&inode, 0)?, &mut superblock)?;
        if be_u32(&superblock, 0) != JBD2_MAGIC
            || be_u32(&superblock, 4) != JBD2_SUPERBLOCK_V2
            || be_u32(&superblock, 12) != self.block_size
        {
            return Err(Error::UnsupportedFeature);
        }
        let max_length = be_u32(&superblock, 16);
        let first = be_u32(&superblock, 20);
        let sequence = be_u32(&superblock, 24);
        let start = be_u32(&superblock, 28);
        if max_length == 0
            || u64::from(max_length) > capacity
            || first == 0
            || first >= max_length
            || max_length > u32::MAX / 2
        {
            return Err(Error::UnsupportedFeature);
        }
        if be_u32(&superblock, 36) != 0
            || be_u32(&superblock, 40) != 0
            || be_u32(&superblock, 44) != 0
        {
            return Err(Error::UnsupportedFeature);
        }
        if start != 0 {
            return Err(Error::JournalRecoveryRequired);
        }
        Ok(Journal {
            inode: self.journal_inode,
            max_length,
            first,
            sequence,
        })
    }

    fn commit_transaction(
        self,
        journal: Journal,
        changes: &mut [TransactionBlock],
    ) -> Result<(), Error> {
        let journal_inode = self.read_inode(journal.inode)?;
        let journal_start = journal.first as u64;
        let needed = changes.len() as u64 + 2;
        if journal_start
            .checked_add(needed)
            .is_none_or(|end| end > u64::from(journal.max_length))
        {
            return Err(Error::UnsupportedFeature);
        }
        for change in changes.iter() {
            if change.block > u64::from(u32::MAX) {
                return Err(Error::UnsupportedFeature);
            }
        }

        let mut journal_superblock = [0u8; MAX_BLOCK_SIZE];
        self.read_block(
            self.extent_block(&journal_inode, 0)?,
            &mut journal_superblock,
        )?;
        let mut dirty_superblock = journal_superblock;
        let sequence = journal.sequence;
        write_be_u32(&mut dirty_superblock, 28, journal.first);
        write_be_u32(&mut dirty_superblock, 24, sequence);

        let mut descriptor = [0u8; MAX_BLOCK_SIZE];
        write_be_u32(&mut descriptor, 0, JBD2_MAGIC);
        write_be_u32(&mut descriptor, 4, JBD2_DESCRIPTOR_BLOCK);
        write_be_u32(&mut descriptor, 8, sequence);
        for (index, change) in changes.iter().enumerate() {
            let offset = 12 + index * 8;
            write_be_u32(&mut descriptor, offset, change.block as u32);
            write_be_u32(
                &mut descriptor,
                offset + 4,
                if index + 1 == changes.len() {
                    JBD2_FLAG_LAST_TAG
                } else {
                    0
                },
            );
        }
        let mut commit = [0u8; MAX_BLOCK_SIZE];
        write_be_u32(&mut commit, 0, JBD2_MAGIC);
        write_be_u32(&mut commit, 4, JBD2_COMMIT_BLOCK);
        write_be_u32(&mut commit, 8, sequence);

        let descriptor_block = self.extent_block(&journal_inode, journal.first)?;
        if self.write_block(descriptor_block, &descriptor).is_err() {
            return Err(Error::Io);
        }
        for (index, change) in changes.iter().enumerate() {
            let block = self.extent_block(&journal_inode, journal.first + 1 + index as u32)?;
            if self.write_block(block, &change.new).is_err() {
                return Err(Error::Io);
            }
        }
        let commit_block =
            self.extent_block(&journal_inode, journal.first + 1 + changes.len() as u32)?;
        if self.write_block(commit_block, &commit).is_err() {
            return Err(Error::Io);
        }
        let journal_superblock_block = self.extent_block(&journal_inode, 0)?;
        if self
            .write_block(journal_superblock_block, &dirty_superblock)
            .is_err()
        {
            let _ = self.write_block(journal_superblock_block, &journal_superblock);
            return Err(Error::Io);
        }

        for index in 0..changes.len() {
            if self
                .write_block(changes[index].block, &changes[index].new)
                .is_err()
            {
                let rolled_back = self.rollback_changes(changes, index + 1);
                if rolled_back {
                    let _ = self.write_block(journal_superblock_block, &journal_superblock);
                }
                return Err(Error::Io);
            }
        }
        let mut clean_superblock = dirty_superblock;
        write_be_u32(&mut clean_superblock, 28, 0);
        write_be_u32(&mut clean_superblock, 24, sequence.wrapping_add(1));
        if self
            .write_block(journal_superblock_block, &clean_superblock)
            .is_err()
        {
            return Err(Error::Io);
        }
        Ok(())
    }

    fn rollback_changes(self, changes: &[TransactionBlock], written: usize) -> bool {
        let mut success = true;
        for change in changes[..written.min(changes.len())].iter().rev() {
            if self.write_block(change.block, &change.old).is_err() {
                success = false;
            }
        }
        success
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

    fn write_block(self, block: u64, input: &[u8; MAX_BLOCK_SIZE]) -> Result<(), Error> {
        let writer = self.writer.ok_or(Error::ReadOnly)?;
        if block >= self.blocks {
            return Err(Error::BadExtent);
        }
        let offset = block
            .checked_mul(self.block_size as u64)
            .ok_or(Error::BadExtent)?;
        let sectors = self.block_size as usize / SECTOR_SIZE;
        for index in 0..sectors {
            let mut sector = [0u8; SECTOR_SIZE];
            let start = index * SECTOR_SIZE;
            sector.copy_from_slice(&input[start..start + SECTOR_SIZE]);
            let lba = offset / SECTOR_SIZE as u64 + index as u64;
            let absolute = self.base_lba.checked_add(lba).ok_or(Error::Io)?;
            if !writer(absolute, &sector) {
                return Err(Error::Io);
            }
        }
        Ok(())
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

pub fn probe_block_rw() -> Result<Mount, Error> {
    let partition = block::partition(0).ok_or(Error::InvalidSuperblock)?;
    Mount::open_at_rw(
        ramdisk_read_sector,
        ramdisk_write_sector,
        partition.start_lba,
        partition.sectors,
    )
}

pub fn contract_self_check() {
    assert!(fixture_check());
}

fn ramdisk_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    usize::try_from(lba)
        .ok()
        .is_some_and(|lba| block::read_sector(lba, output))
}

fn ramdisk_write_sector(lba: u64, input: &[u8; SECTOR_SIZE]) -> bool {
    usize::try_from(lba)
        .ok()
        .is_some_and(|lba| block::write_sector(lba, input))
}

const FIXTURE_SECTORS: usize = 32;
static mut FIXTURE_MEDIA: [u8; SECTOR_SIZE * FIXTURE_SECTORS] = [0; SECTOR_SIZE * FIXTURE_SECTORS];
static mut FIXTURE_MEDIA_READY: bool = false;
static mut FIXTURE_WRITE_COUNT: usize = 0;
static mut FIXTURE_FAIL_AFTER: usize = usize::MAX;

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
    let mut entries = [DirectoryEntry::EMPTY; 2];
    let Ok(count) = volume.read_dir("/", &mut entries) else {
        return false;
    };
    if count != 1
        || entries[0].inode != 3
        || entries[0].mode & 0o777 != 0o644
        || entries[0].size != 13
        || entries[0].directory
        || entries[0].name_length != 9
        || &entries[0].name[..entries[0].name_length] != b"hello.txt"
    {
        return false;
    }
    if volume.read_dir("/", &mut []).is_err()
        || volume
            .read_dir("/hello.txt", &mut entries)
            .is_err_and(|error| error != Error::NotDirectory)
    {
        return false;
    }
    if !Mount::open(bad_directory_read_sector)
        .and_then(|volume| volume.read_dir("/", &mut entries).map(|_| volume))
        .is_err_and(|error| error == Error::BadDirectory)
    {
        return false;
    }
    if !Mount::open(checksum_feature_read_sector)
        .is_err_and(|error| error == Error::UnsupportedFeature)
    {
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
    if !volume
        .write_file("/hello.txt", b"rejected")
        .is_err_and(|error| error == Error::ReadOnly)
    {
        return false;
    }

    reset_fixture_media();
    let Ok(writable) = Mount::open_rw(fixture_rw_read_sector, fixture_rw_write_sector) else {
        return false;
    };
    if writable.geometry().read_only {
        return false;
    }
    if writable.write_file("/hello.txt", b"ext4 updated\n") != Ok(13) {
        return false;
    }
    let Ok(reopened) = Mount::open(fixture_rw_read_sector) else {
        return false;
    };
    let mut updated = [0u8; 32];
    if reopened.read_file("/hello.txt", &mut updated) != Ok(13)
        || &updated[..13] != b"ext4 updated\n"
    {
        return false;
    }

    reset_fixture_media();
    set_fixture_failure(11);
    if writable.write_file("/hello.txt", b"ext4 failed\n") != Err(Error::Io) {
        return false;
    }
    set_fixture_failure(usize::MAX);
    let Ok(rolled_back) = Mount::open(fixture_rw_read_sector) else {
        return false;
    };
    let mut original = [0u8; 32];
    if rolled_back.read_file("/hello.txt", &mut original) != Ok(13)
        || &original[..13] != b"ext4 fixture\n"
    {
        return false;
    }
    if !Mount::open_rw(fixture_recovery_read_sector, fixture_rw_write_sector)
        .is_err_and(|error| error == Error::JournalRecoveryRequired)
    {
        return false;
    }
    if !Mount::open_rw(indirect_extent_read_sector, fixture_rw_write_sector)
        .and_then(|volume| volume.write_file("/hello.txt", b"rejected"))
        .is_err_and(|error| error == Error::UnsupportedFeature)
    {
        return false;
    }
    true
}

fn fixture_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    output.fill(0);
    match lba {
        2 => fixture_superblock(output),
        4 => write_u32(output, 8, 4),
        8 => fixture_inode_table(output),
        9 => fixture_inode_table_tail(output),
        10 => fixture_directory(output),
        12 => output[..13].copy_from_slice(b"ext4 fixture\n"),
        14 => fixture_journal_superblock(output),
        _ => {}
    }
    true
}

fn fixture_rw_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    let Ok(lba) = usize::try_from(lba) else {
        return false;
    };
    if lba >= FIXTURE_SECTORS {
        return false;
    }
    unsafe {
        if !FIXTURE_MEDIA_READY {
            return false;
        }
        let media = &*core::ptr::addr_of!(FIXTURE_MEDIA);
        let start = lba * SECTOR_SIZE;
        output.copy_from_slice(&media[start..start + SECTOR_SIZE]);
    }
    true
}

fn fixture_rw_write_sector(lba: u64, input: &[u8; SECTOR_SIZE]) -> bool {
    let Ok(lba) = usize::try_from(lba) else {
        return false;
    };
    if lba >= FIXTURE_SECTORS {
        return false;
    }
    unsafe {
        if !FIXTURE_MEDIA_READY {
            return false;
        }
        let failed = FIXTURE_WRITE_COUNT == FIXTURE_FAIL_AFTER;
        FIXTURE_WRITE_COUNT += 1;
        if failed {
            return false;
        }
        let media = &mut *core::ptr::addr_of_mut!(FIXTURE_MEDIA);
        let start = lba * SECTOR_SIZE;
        media[start..start + SECTOR_SIZE].copy_from_slice(input);
    }
    true
}

fn reset_fixture_media() {
    unsafe {
        FIXTURE_MEDIA_READY = false;
        FIXTURE_WRITE_COUNT = 0;
        FIXTURE_FAIL_AFTER = usize::MAX;
    }
    for lba in 0..FIXTURE_SECTORS {
        let mut sector = [0u8; SECTOR_SIZE];
        fixture_read_sector(lba as u64, &mut sector);
        unsafe {
            let media = &mut *core::ptr::addr_of_mut!(FIXTURE_MEDIA);
            let start = lba * SECTOR_SIZE;
            media[start..start + SECTOR_SIZE].copy_from_slice(&sector);
        }
    }
    unsafe {
        FIXTURE_MEDIA_READY = true;
    }
}

fn set_fixture_failure(after: usize) {
    unsafe {
        FIXTURE_WRITE_COUNT = 0;
        FIXTURE_FAIL_AFTER = after;
    }
}

fn fixture_partition_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    lba.checked_sub(100)
        .is_some_and(|relative| fixture_read_sector(relative, output))
}

fn bad_directory_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    fixture_read_sector(lba, output);
    if lba == 10 {
        write_u16(output, 4, 2);
    }
    true
}

fn checksum_feature_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    fixture_read_sector(lba, output);
    if lba == 2 {
        write_u32(output, 0x64, RO_COMPAT_METADATA_CSUM);
    }
    true
}

fn fixture_recovery_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    fixture_read_sector(lba, output);
    if lba == 14 {
        write_be_u32(output, 28, 1);
    }
    true
}

fn indirect_extent_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    fixture_read_sector(lba, output);
    if lba == 8 {
        write_u32(output, 256 + 32, 0);
    }
    true
}

fn fixture_superblock(output: &mut [u8; SECTOR_SIZE]) {
    write_u32(output, 4, 8192);
    write_u32(output, 20, 1);
    write_u32(output, 24, 0);
    write_u32(output, 32, 8192);
    write_u32(output, 40, 16);
    write_u16(output, 56, 1);
    write_u16(output, 0x3a, EXT4_VALID_FS);
    write_u16(output, 88, 128);
    write_u32(output, 92, COMPAT_HAS_JOURNAL);
    write_u32(output, 96, INCOMPAT_EXTENTS | INCOMPAT_FILETYPE);
    write_u32(output, 100, 0);
    write_u32(output, 0xe0, 8);
    write_u16(output, 0x38, EXT4_MAGIC);
    write_u16(output, 0xfe, 32);
}

fn fixture_inode_table(output: &mut [u8; SECTOR_SIZE]) {
    write_inode(output, 128, 0x41ed, 1024, 5, 1);
    write_inode(output, 256, 0x81a4, 13, 6, 1);
}

fn fixture_inode_table_tail(output: &mut [u8; SECTOR_SIZE]) {
    write_inode(output, 384, 0x81a4, 8192, 7, 8);
}

fn write_inode(
    output: &mut [u8; SECTOR_SIZE],
    offset: usize,
    mode: u16,
    size: u64,
    block: u32,
    extent_length: u16,
) {
    write_u16(output, offset, mode);
    write_u32(output, offset + 4, size as u32);
    write_u32(output, offset + 108, (size >> 32) as u32);
    write_u32(output, offset + 32, EXT4_EXTENTS_FL);
    write_u16(output, offset + 40, 0xf30a);
    write_u16(output, offset + 42, 1);
    write_u16(output, offset + 44, 4);
    write_u32(output, offset + 60, block);
    write_u16(output, offset + 56, extent_length);
}

fn fixture_journal_superblock(output: &mut [u8; SECTOR_SIZE]) {
    write_be_u32(output, 0, JBD2_MAGIC);
    write_be_u32(output, 4, JBD2_SUPERBLOCK_V2);
    write_be_u32(output, 12, 1024);
    write_be_u32(output, 16, 8);
    write_be_u32(output, 20, 1);
    write_be_u32(output, 24, 1);
    write_be_u32(output, 28, 0);
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

fn has_journal(compat: u32) -> bool {
    compat & COMPAT_HAS_JOURNAL != 0
}

fn find_change(changes: &[TransactionBlock], count: usize, block: u64) -> Option<usize> {
    changes[..count]
        .iter()
        .position(|change| change.block == block)
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

fn be_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
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

fn write_be_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
