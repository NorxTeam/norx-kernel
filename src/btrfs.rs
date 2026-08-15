use crate::drivers::block;

const SECTOR_SIZE: usize = 512;
const SUPERBLOCK_OFFSET: u64 = 0x10_000;
const SUPERBLOCK_SIZE: usize = 4096;
const SYS_CHUNK_ARRAY_OFFSET: usize = 0x32b;
const SYSTEM_CHUNK_ARRAY_SIZE: usize = 2048;
const MAX_NODE_SIZE: usize = 4096;
const TREE_HEADER_SIZE: usize = 101;
const LEAF_ITEM_SIZE: usize = 25;
const NODE_PTR_SIZE: usize = 33;
const MAX_CHUNKS: usize = 16;
const MAX_PENDING_NODES: usize = 512;
const MAX_TREE_VISITS: usize = 4096;
const MAX_COMPONENTS: usize = 16;
const MAX_TRANSACTION_SECTORS: usize = 32;
pub const MAX_NAME_LENGTH: usize = 255;

const MAGIC: &[u8; 8] = b"_BHRfS_M";
const CSUM_CRC32C: u16 = 0;
const BTRFS_ROOT_TREE_OBJECTID: u64 = 1;
const BTRFS_CHUNK_TREE_OBJECTID: u64 = 3;
const BTRFS_FS_TREE_OBJECTID: u64 = 5;
const BTRFS_FIRST_FREE_OBJECTID: u64 = 256;
const ROOT_ITEM_KEY: u8 = 132;
const INODE_ITEM_KEY: u8 = 1;
const DIR_ITEM_KEY: u8 = 84;
const EXTENT_DATA_KEY: u8 = 108;
const CHUNK_ITEM_KEY: u8 = 228;
const CHUNK_PROFILE_MASK: u64 = 0x7f8;
const ROOT_SUBVOL_RDONLY: u64 = 1;
const INODE_DIRECTORY: u32 = 0x4000;
const INODE_REGULAR: u32 = 0x8000;

pub type ReadSector = fn(u64, &mut [u8; SECTOR_SIZE]) -> bool;
pub type WriteSector = fn(u64, &[u8; SECTOR_SIZE]) -> bool;
pub type FlushCache = fn() -> bool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Io,
    InvalidSuperblock,
    UnsupportedChecksum,
    ChecksumMismatch,
    UnsupportedFeature,
    UnmappedLogical,
    TreeCorrupt,
    InvalidPath,
    NotFound,
    NotDirectory,
    IsDirectory,
    BufferTooSmall,
    PermissionDenied,
    ReadOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub sectorsize: u32,
    pub nodesize: u32,
    pub total_bytes: u64,
    pub generation: u64,
    pub chunks: usize,
    pub checksum: bool,
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subvolume {
    pub id: u64,
    pub root_dirid: u64,
    pub bytenr: u64,
    pub read_only: bool,
    pub snapshot: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub mode: u32,
    pub size: u64,
    pub directory: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub inode: u64,
    pub mode: u32,
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
struct Chunk {
    logical: u64,
    length: u64,
    physical: u64,
}

impl Chunk {
    const EMPTY: Self = Self {
        logical: 0,
        length: 0,
        physical: 0,
    };
}

#[derive(Clone, Copy)]
struct Key {
    objectid: u64,
    kind: u8,
    offset: u64,
}

#[derive(Clone, Copy)]
struct Inode {
    mode: u32,
    size: u64,
}

#[derive(Clone, Copy)]
struct RootItem {
    id: u64,
    root_dirid: u64,
    bytenr: u64,
    flags: u64,
    snapshot: bool,
}

#[derive(Clone, Copy)]
pub struct Mount {
    reader: ReadSector,
    writer: Option<WriteSector>,
    flush: Option<FlushCache>,
    base_lba: u64,
    fsid: [u8; 16],
    sectorsize: u32,
    nodesize: u32,
    total_bytes: u64,
    generation: u64,
    root: u64,
    chunk_root: u64,
    csum_type: u16,
    chunks: [Chunk; MAX_CHUNKS],
    chunk_count: usize,
}

impl Mount {
    pub fn open(reader: ReadSector) -> Result<Self, Error> {
        Self::open_at(reader, 0, u64::MAX)
    }

    pub fn open_with_writer(
        reader: ReadSector,
        writer: WriteSector,
        flush: FlushCache,
    ) -> Result<Self, Error> {
        Self::open_at_with_writer(reader, writer, flush, 0, u64::MAX)
    }

    pub fn open_at(
        reader: ReadSector,
        base_lba: u64,
        available_sectors: u64,
    ) -> Result<Self, Error> {
        Self::open_at_with_io(reader, None, None, base_lba, available_sectors)
    }

    pub fn open_at_with_writer(
        reader: ReadSector,
        writer: WriteSector,
        flush: FlushCache,
        base_lba: u64,
        available_sectors: u64,
    ) -> Result<Self, Error> {
        Self::open_at_with_io(
            reader,
            Some(writer),
            Some(flush),
            base_lba,
            available_sectors,
        )
    }

    fn open_at_with_io(
        reader: ReadSector,
        writer: Option<WriteSector>,
        flush: Option<FlushCache>,
        base_lba: u64,
        available_sectors: u64,
    ) -> Result<Self, Error> {
        let mut superblock = [0u8; SUPERBLOCK_SIZE];
        read_sectors(reader, base_lba, SUPERBLOCK_OFFSET, &mut superblock)?;
        if &superblock[0x40..0x48] != MAGIC {
            return Err(Error::InvalidSuperblock);
        }
        let csum_type = le_u16(&superblock, 0xc4);
        if csum_type != CSUM_CRC32C {
            return Err(Error::UnsupportedChecksum);
        }
        if le_u32(&superblock, 0) != crc32c(&superblock[32..]) {
            return Err(Error::ChecksumMismatch);
        }
        let sectorsize = le_u32(&superblock, 0x90);
        let nodesize = le_u32(&superblock, 0x94);
        let total_bytes = le_u64(&superblock, 0x70);
        let generation = le_u64(&superblock, 0x48);
        let incompat = le_u64(&superblock, 0xbc);
        if !matches!(sectorsize, 512 | 1024 | 2048 | 4096)
            || nodesize != MAX_NODE_SIZE as u32
            || sectorsize > nodesize
            || total_bytes < SUPERBLOCK_OFFSET + SUPERBLOCK_SIZE as u64
            || total_bytes.div_ceil(SECTOR_SIZE as u64) > available_sectors
            || base_lba
                .checked_add(total_bytes.div_ceil(SECTOR_SIZE as u64))
                .is_none()
            || incompat != 0
        {
            return Err(Error::UnsupportedFeature);
        }
        let root = le_u64(&superblock, 0x50);
        let chunk_root = le_u64(&superblock, 0x58);
        if root == 0 || chunk_root == 0 {
            return Err(Error::InvalidSuperblock);
        }
        let mut chunks = [Chunk::EMPTY; MAX_CHUNKS];
        let mut chunk_count = 0;
        parse_chunk_array(
            &superblock,
            &mut chunks,
            &mut chunk_count,
            le_u32(&superblock, 0xa0) as usize,
        )?;
        let mut mount = Self {
            reader,
            writer,
            flush,
            base_lba,
            fsid: copy_array(&superblock[0x20..0x30]),
            sectorsize,
            nodesize,
            total_bytes,
            generation,
            root,
            chunk_root,
            csum_type,
            chunks,
            chunk_count,
        };
        let (chunks, chunk_count) = mount.collect_chunk_tree()?;
        mount.chunks = chunks;
        mount.chunk_count = chunk_count;
        let _ = mount.find_root_item(BTRFS_FS_TREE_OBJECTID)?;
        Ok(mount)
    }

    pub const fn geometry(self) -> Geometry {
        Geometry {
            sectorsize: self.sectorsize,
            nodesize: self.nodesize,
            total_bytes: self.total_bytes,
            generation: self.generation,
            chunks: self.chunk_count,
            checksum: self.csum_type == CSUM_CRC32C,
            read_only: true,
        }
    }

    pub fn begin_transaction(self) -> Result<Transaction, Error> {
        let writer = self.writer.ok_or(Error::ReadOnly)?;
        let flush = self.flush.ok_or(Error::ReadOnly)?;
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(Error::UnsupportedFeature)?;
        Ok(Transaction {
            reader: self.reader,
            writer,
            flush,
            base_lba: self.base_lba,
            total_bytes: self.total_bytes,
            fsid: self.fsid,
            nodesize: self.nodesize,
            generation,
            pending: [PendingSector::EMPTY; MAX_TRANSACTION_SECTORS],
            pending_count: 0,
        })
    }

    pub fn subvolumes(self, output: &mut [Subvolume]) -> Result<usize, Error> {
        let mut count = 0;
        self.walk_leaves(self.root, |leaf| {
            self.each_item(leaf, |key, data| {
                if key.kind != ROOT_ITEM_KEY || key.objectid < BTRFS_FIRST_FREE_OBJECTID {
                    return Ok(false);
                }
                if count == output.len() {
                    return Ok(true);
                }
                let root = parse_root_item(key.objectid, data)?;
                output[count] = Subvolume {
                    id: root.id,
                    root_dirid: root.root_dirid,
                    bytenr: root.bytenr,
                    read_only: root.flags & ROOT_SUBVOL_RDONLY != 0,
                    snapshot: root.snapshot,
                };
                count += 1;
                Ok(false)
            })
        })?;
        Ok(count)
    }

    pub fn stat(self, path: &str) -> Result<FileInfo, Error> {
        let (root, inode) = self.resolve(path)?;
        let info = self.find_inode(root, inode)?;
        Ok(FileInfo {
            mode: info.mode,
            size: info.size,
            directory: info.mode & 0xf000 == INODE_DIRECTORY,
        })
    }

    pub fn read_dir(self, path: &str, output: &mut [DirectoryEntry]) -> Result<usize, Error> {
        let (root, parent) = self.resolve(path)?;
        let mut count = 0;
        self.walk_directory(root, parent, |child, name| {
            if name == b"." || name == b".." {
                return Ok(false);
            }
            if count == output.len() {
                return Ok(false);
            }
            let inode = self.find_inode(root, child)?;
            let name_length = name.len();
            let mut entry_name = [0u8; MAX_NAME_LENGTH];
            entry_name[..name_length].copy_from_slice(name);
            output[count] = DirectoryEntry {
                inode: child,
                mode: inode.mode,
                size: inode.size,
                directory: inode.mode & 0xf000 == INODE_DIRECTORY,
                name: entry_name,
                name_length,
            };
            count += 1;
            Ok(false)
        })?;
        Ok(count)
    }

    pub fn read_file(self, path: &str, output: &mut [u8]) -> Result<usize, Error> {
        let (root, inode_number) = self.resolve(path)?;
        let inode = self.find_inode(root, inode_number)?;
        if inode.mode & 0xf000 == INODE_DIRECTORY {
            return Err(Error::IsDirectory);
        }
        if inode.mode & 0o444 == 0 {
            return Err(Error::PermissionDenied);
        }
        let size = usize::try_from(inode.size).map_err(|_| Error::BufferTooSmall)?;
        if size > output.len() {
            return Err(Error::BufferTooSmall);
        }
        let mut copied = 0;
        let mut found = false;
        self.walk_leaves(root, |leaf| {
            self.each_item(leaf, |key, data| {
                if key.objectid != inode_number || key.kind != EXTENT_DATA_KEY || key.offset != 0 {
                    return Ok(false);
                }
                if data.len() < 53 || data[16] != 0 || data[17] != 0 || data[18] != 0 {
                    return Err(Error::UnsupportedFeature);
                }
                if data[19] != 0 {
                    return Err(Error::UnsupportedFeature);
                }
                let inline = &data[53..];
                if inline.len() != size {
                    return Err(Error::TreeCorrupt);
                }
                output[..size].copy_from_slice(inline);
                copied = size;
                found = true;
                Ok(true)
            })
        })?;
        if size == 0 {
            return Ok(0);
        }
        if !found {
            return Err(Error::UnsupportedFeature);
        }
        Ok(copied)
    }

    pub fn write_file(self, path: &str, input: &[u8]) -> Result<usize, Error> {
        if self.writer.is_none() || self.flush.is_none() {
            return Err(Error::ReadOnly);
        }
        self.validate_inline_rewrite(path, input)?;
        // ponytail: COW allocation and root publication are deliberately not
        // faked; add them before exposing a persistent btrfs mutation.
        let _transaction = self.begin_transaction()?;
        Err(Error::UnsupportedFeature)
    }

    fn validate_inline_rewrite(self, path: &str, input: &[u8]) -> Result<(), Error> {
        let (root, inode_number) = self.resolve(path)?;
        let inode = self.find_inode(root, inode_number)?;
        if inode.mode & 0xf000 != INODE_REGULAR || input.is_empty() {
            return Err(Error::UnsupportedFeature);
        }
        let size = usize::try_from(inode.size).map_err(|_| Error::BufferTooSmall)?;
        if size != input.len() {
            return Err(Error::UnsupportedFeature);
        }
        let mut current = [0u8; MAX_NODE_SIZE];
        self.read_file(path, &mut current[..size])?;
        Ok(())
    }

    fn resolve(self, path: &str) -> Result<(u64, u64), Error> {
        let (components, count) = components(path)?;
        let root = self.find_root_item(BTRFS_FS_TREE_OBJECTID)?;
        let mut inode = root.root_dirid;
        for component in components.iter().take(count) {
            inode = self.find_dir_entry(root.bytenr, inode, component)?;
        }
        Ok((root.bytenr, inode))
    }

    fn find_root_item(self, id: u64) -> Result<RootItem, Error> {
        let mut result = None;
        self.walk_leaves(self.root, |leaf| {
            self.each_item(leaf, |key, data| {
                if key.kind == ROOT_ITEM_KEY && key.objectid == id {
                    result = Some(parse_root_item(id, data)?);
                    Ok(true)
                } else {
                    Ok(false)
                }
            })
        })?;
        result.ok_or(Error::NotFound)
    }

    fn find_inode(self, root: u64, inode_number: u64) -> Result<Inode, Error> {
        let mut result = None;
        self.walk_leaves(root, |leaf| {
            self.each_item(leaf, |key, data| {
                if key.objectid == inode_number && key.kind == INODE_ITEM_KEY {
                    if data.len() < 64 {
                        return Err(Error::TreeCorrupt);
                    }
                    result = Some(Inode {
                        mode: le_u32(data, 52),
                        size: le_u64(data, 16),
                    });
                    Ok(true)
                } else {
                    Ok(false)
                }
            })
        })?;
        result.ok_or(Error::NotFound)
    }

    fn find_dir_entry(self, root: u64, parent: u64, target: &str) -> Result<u64, Error> {
        let mut result = None;
        self.walk_directory(root, parent, |child, name| {
            if name == target.as_bytes() {
                result = Some(child);
                Ok(true)
            } else {
                Ok(false)
            }
        })?;
        result.ok_or(Error::NotFound)
    }

    fn walk_directory<F>(self, root: u64, parent: u64, mut visitor: F) -> Result<(), Error>
    where
        F: FnMut(u64, &[u8]) -> Result<bool, Error>,
    {
        if self.find_inode(root, parent)?.mode & 0xf000 != INODE_DIRECTORY {
            return Err(Error::NotDirectory);
        }
        self.walk_leaves(root, |leaf| {
            self.each_item(leaf, |key, data| {
                if key.objectid != parent || key.kind != DIR_ITEM_KEY {
                    return Ok(false);
                }
                let mut offset = 0usize;
                while offset < data.len() {
                    if data.len() - offset < 30 {
                        return Err(Error::TreeCorrupt);
                    }
                    let data_length = le_u16(data, offset + 25) as usize;
                    let name_length = le_u16(data, offset + 27) as usize;
                    let record_length = 30usize
                        .checked_add(data_length)
                        .and_then(|length| length.checked_add(name_length))
                        .ok_or(Error::TreeCorrupt)?;
                    let name_end = offset
                        .checked_add(30)
                        .and_then(|start| start.checked_add(name_length))
                        .ok_or(Error::TreeCorrupt)?;
                    if record_length <= 30
                        || name_length > MAX_NAME_LENGTH
                        || name_end > data.len()
                        || offset + record_length > data.len()
                        || le_u64(data, offset) == 0
                        || data[offset + 8] != INODE_ITEM_KEY
                        || data[offset + 29] > 7
                    {
                        return Err(Error::TreeCorrupt);
                    }
                    if visitor(le_u64(data, offset), &data[offset + 30..name_end])? {
                        return Ok(true);
                    }
                    offset += record_length;
                }
                Ok(false)
            })
        })
    }

    fn collect_chunk_tree(self) -> Result<([Chunk; MAX_CHUNKS], usize), Error> {
        let mut chunks = self.chunks;
        let mut count = self.chunk_count;
        self.walk_leaves(self.chunk_root, |leaf| {
            self.each_item(leaf, |key, data| {
                if key.kind == CHUNK_ITEM_KEY {
                    add_chunk(&mut chunks, &mut count, parse_chunk(key.offset, data)?)?;
                }
                Ok(false)
            })
        })?;
        Ok((chunks, count))
    }

    fn walk_leaves<F>(self, root: u64, mut visitor: F) -> Result<(), Error>
    where
        F: FnMut(&[u8]) -> Result<bool, Error>,
    {
        let mut pending = [0u64; MAX_PENDING_NODES];
        let mut pending_count = 1;
        pending[0] = root;
        let mut visits = 0;
        while pending_count != 0 {
            pending_count -= 1;
            let logical = pending[pending_count];
            visits += 1;
            if visits > MAX_TREE_VISITS {
                return Err(Error::TreeCorrupt);
            }
            let mut block = [0u8; MAX_NODE_SIZE];
            self.read_tree_block(logical, &mut block)?;
            let nritems = le_u32(&block, 0x60) as usize;
            let level = block[0x64];
            let capacity = if level == 0 {
                (self.nodesize as usize - TREE_HEADER_SIZE) / LEAF_ITEM_SIZE
            } else {
                (self.nodesize as usize - TREE_HEADER_SIZE) / NODE_PTR_SIZE
            };
            if nritems > capacity {
                return Err(Error::TreeCorrupt);
            }
            if level == 0 {
                if visitor(&block[..self.nodesize as usize])? {
                    return Ok(());
                }
                continue;
            }
            if level > 8 || pending_count + nritems > MAX_PENDING_NODES {
                return Err(Error::UnsupportedFeature);
            }
            for index in (0..nritems).rev() {
                let offset = TREE_HEADER_SIZE + index * NODE_PTR_SIZE + 17;
                pending[pending_count] = le_u64(&block, offset);
                pending_count += 1;
            }
        }
        Ok(())
    }

    fn each_item<F>(self, leaf: &[u8], mut visitor: F) -> Result<bool, Error>
    where
        F: FnMut(Key, &[u8]) -> Result<bool, Error>,
    {
        let nritems = le_u32(leaf, 0x60) as usize;
        let item_end = TREE_HEADER_SIZE
            .checked_add(nritems * LEAF_ITEM_SIZE)
            .ok_or(Error::TreeCorrupt)?;
        for index in 0..nritems {
            let offset = TREE_HEADER_SIZE + index * LEAF_ITEM_SIZE;
            let data_offset = le_u32(leaf, offset + 17) as usize;
            let data_size = le_u32(leaf, offset + 21) as usize;
            if data_offset < item_end
                || data_offset > leaf.len()
                || data_size > leaf.len() - data_offset
            {
                return Err(Error::TreeCorrupt);
            }
            let key = Key {
                objectid: le_u64(leaf, offset),
                kind: leaf[offset + 8],
                offset: le_u64(leaf, offset + 9),
            };
            if visitor(key, &leaf[data_offset..data_offset + data_size])? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn read_tree_block(self, logical: u64, output: &mut [u8; MAX_NODE_SIZE]) -> Result<(), Error> {
        let physical = self.map_logical(logical)?;
        let end = logical
            .checked_add(self.nodesize as u64)
            .ok_or(Error::TreeCorrupt)?;
        if self.map_logical(end - 1)? != physical + self.nodesize as u64 - 1 {
            return Err(Error::UnsupportedFeature);
        }
        self.read_physical(physical, &mut output[..self.nodesize as usize])?;
        if le_u32(output, 0) != crc32c(&output[32..self.nodesize as usize]) {
            return Err(Error::ChecksumMismatch);
        }
        if output[0x20..0x30] != self.fsid || le_u64(output, 0x30) != logical || output[0x64] > 8 {
            return Err(Error::TreeCorrupt);
        }
        Ok(())
    }

    fn map_logical(self, logical: u64) -> Result<u64, Error> {
        for chunk in self.chunks.iter().take(self.chunk_count) {
            let end = chunk
                .logical
                .checked_add(chunk.length)
                .ok_or(Error::UnmappedLogical)?;
            if logical >= chunk.logical && logical < end {
                return chunk
                    .physical
                    .checked_add(logical - chunk.logical)
                    .ok_or(Error::UnmappedLogical);
            }
        }
        Err(Error::UnmappedLogical)
    }

    fn read_physical(self, offset: u64, output: &mut [u8]) -> Result<(), Error> {
        let end = offset.checked_add(output.len() as u64).ok_or(Error::Io)?;
        if !offset.is_multiple_of(SECTOR_SIZE as u64)
            || end > self.total_bytes
            || !output.len().is_multiple_of(SECTOR_SIZE)
        {
            return Err(Error::Io);
        }
        for index in 0..output.len() / SECTOR_SIZE {
            let mut sector = [0u8; SECTOR_SIZE];
            if !self.read_sector(offset / SECTOR_SIZE as u64 + index as u64, &mut sector) {
                return Err(Error::Io);
            }
            let start = index * SECTOR_SIZE;
            output[start..start + SECTOR_SIZE].copy_from_slice(&sector);
        }
        Ok(())
    }

    fn read_sector(self, lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
        self.base_lba
            .checked_add(lba)
            .is_some_and(|absolute| (self.reader)(absolute, output))
    }
}

const TRANSACTION_DATA_PHASE: u8 = 0;
const TRANSACTION_POINTER_PHASE: u8 = 1;
const TRANSACTION_SUPERBLOCK_PHASE: u8 = 2;
const MAX_FIXTURE_WRITES: usize = 64;

#[derive(Clone, Copy)]
struct PendingSector {
    lba: u64,
    phase: u8,
    before: [u8; SECTOR_SIZE],
    after: [u8; SECTOR_SIZE],
}

impl PendingSector {
    const EMPTY: Self = Self {
        lba: 0,
        phase: 0,
        before: [0; SECTOR_SIZE],
        after: [0; SECTOR_SIZE],
    };
}

pub struct Transaction {
    reader: ReadSector,
    writer: WriteSector,
    flush: FlushCache,
    base_lba: u64,
    total_bytes: u64,
    fsid: [u8; 16],
    nodesize: u32,
    generation: u64,
    pending: [PendingSector; MAX_TRANSACTION_SECTORS],
    pending_count: usize,
}

impl Transaction {
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    // The caller must supply a block obtained from the future COW allocator.
    // There is intentionally no allocator here, so write_file never calls it.
    fn stage_tree_block(
        &mut self,
        physical_offset: u64,
        logical: u64,
        phase: u8,
        mut block: [u8; MAX_NODE_SIZE],
    ) -> Result<(), Error> {
        if phase > TRANSACTION_POINTER_PHASE
            || physical_offset % self.nodesize as u64 != 0
            || logical % self.nodesize as u64 != 0
            || block[0x20..0x30] != self.fsid
            || le_u64(&block, 0x30) != logical
            || block[0x64] > 8
            || le_u32(&block, 0) != crc32c(&block[32..])
        {
            return Err(Error::TreeCorrupt);
        }
        write_u64(&mut block, 0x50, self.generation);
        let checksum = crc32c(&block[32..]);
        write_u32(&mut block, 0, checksum);
        self.stage_block(physical_offset, &block, phase)
    }

    // Superblock publication is always the final phase. Mirrors are outside
    // this bounded groundwork and therefore cannot be staged accidentally.
    fn stage_superblock(
        &mut self,
        physical_offset: u64,
        mut superblock: [u8; SUPERBLOCK_SIZE],
    ) -> Result<(), Error> {
        if physical_offset != SUPERBLOCK_OFFSET
            || &superblock[0x40..0x48] != MAGIC
            || superblock[0x20..0x30] != self.fsid
            || le_u16(&superblock, 0xc4) != CSUM_CRC32C
            || le_u32(&superblock, 0) != crc32c(&superblock[32..])
        {
            return Err(Error::InvalidSuperblock);
        }
        write_u64(&mut superblock, 0x48, self.generation);
        let checksum = crc32c(&superblock[32..]);
        write_u32(&mut superblock, 0, checksum);
        self.stage_block(physical_offset, &superblock, TRANSACTION_SUPERBLOCK_PHASE)
    }

    fn stage_block(&mut self, physical_offset: u64, bytes: &[u8], phase: u8) -> Result<(), Error> {
        if phase > TRANSACTION_SUPERBLOCK_PHASE
            || !physical_offset.is_multiple_of(SECTOR_SIZE as u64)
            || !bytes.len().is_multiple_of(SECTOR_SIZE)
            || physical_offset
                .checked_add(bytes.len() as u64)
                .is_none_or(|end| end > self.total_bytes)
        {
            return Err(Error::Io);
        }
        for index in 0..bytes.len() / SECTOR_SIZE {
            let lba = self
                .base_lba
                .checked_add(physical_offset / SECTOR_SIZE as u64 + index as u64)
                .ok_or(Error::Io)?;
            if self.pending[..self.pending_count]
                .iter()
                .any(|pending| pending.lba == lba)
            {
                return Err(Error::UnsupportedFeature);
            }
            if self.pending_count == self.pending.len() {
                return Err(Error::UnsupportedFeature);
            }
            let mut before = [0u8; SECTOR_SIZE];
            if !(self.reader)(lba, &mut before) {
                return Err(Error::Io);
            }
            let start = index * SECTOR_SIZE;
            let mut after = [0u8; SECTOR_SIZE];
            after.copy_from_slice(&bytes[start..start + SECTOR_SIZE]);
            self.pending[self.pending_count] = PendingSector {
                lba,
                phase,
                before,
                after,
            };
            self.pending_count += 1;
        }
        Ok(())
    }

    pub fn commit(self) -> Result<(), Error> {
        let mut written = [false; MAX_TRANSACTION_SECTORS];
        for phase in 0..=TRANSACTION_SUPERBLOCK_PHASE {
            for (index, pending) in self.pending[..self.pending_count].iter().enumerate() {
                if pending.phase != phase {
                    continue;
                }
                if !(self.writer)(pending.lba, &pending.after) {
                    self.rollback(&written);
                    return Err(Error::Io);
                }
                written[index] = true;
            }
        }
        if !(self.flush)() {
            self.rollback(&written);
            return Err(Error::Io);
        }
        Ok(())
    }

    fn rollback(&self, written: &[bool; MAX_TRANSACTION_SECTORS]) {
        for phase in (0..=TRANSACTION_SUPERBLOCK_PHASE).rev() {
            for (index, pending) in self.pending[..self.pending_count].iter().enumerate().rev() {
                if pending.phase == phase && written[index] {
                    let _ = (self.writer)(pending.lba, &pending.before);
                }
            }
        }
        let _ = (self.flush)();
    }
}

pub fn probe_block() -> Result<Mount, Error> {
    let partition = block::partition(0).ok_or(Error::InvalidSuperblock)?;
    Mount::open_at(ramdisk_read_sector, partition.start_lba, partition.sectors)
}

pub fn probe_block_rw() -> Result<Mount, Error> {
    let partition = block::partition(0).ok_or(Error::InvalidSuperblock)?;
    Mount::open_at_with_writer(
        ramdisk_read_sector,
        ramdisk_write_sector,
        ramdisk_flush_cache,
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

fn ramdisk_flush_cache() -> bool {
    block::flush_cache().is_ok()
}

fn fixture_check() -> bool {
    let Ok(volume) = Mount::open(fixture_read_sector) else {
        return false;
    };
    let geometry = volume.geometry();
    if geometry.nodesize != 4096 || geometry.chunks < 1 || !geometry.checksum || !geometry.read_only
    {
        return false;
    }
    let mut subvolumes = [Subvolume {
        id: 0,
        root_dirid: 0,
        bytenr: 0,
        read_only: false,
        snapshot: false,
    }; 4];
    let Ok(count) = volume.subvolumes(&mut subvolumes) else {
        return false;
    };
    if count != 1 || subvolumes[0].id != BTRFS_FIRST_FREE_OBJECTID || !subvolumes[0].snapshot {
        return false;
    }
    let Ok(info) = volume.stat("/hello.txt") else {
        return false;
    };
    if info.directory || info.mode & 0o777 != 0o644 || info.size != 14 {
        return false;
    }
    let mut output = [0u8; 32];
    let Ok(length) = volume.read_file("/hello.txt", &mut output) else {
        return false;
    };
    if length != 14 || &output[..length] != b"btrfs fixture\n" {
        return false;
    }
    let mut entries = [DirectoryEntry::EMPTY; 2];
    let Ok(count) = volume.read_dir("/", &mut entries) else {
        return false;
    };
    if count != 2
        || entries[0].inode != 257
        || entries[0].mode & 0o777 != 0o644
        || entries[0].size != 14
        || entries[0].directory
        || entries[0].name_length != 9
        || &entries[0].name[..entries[0].name_length] != b"hello.txt"
        || entries[1].name_length != 8
        || &entries[1].name[..entries[1].name_length] != b"copy.txt"
    {
        return false;
    }
    let mut bounded = [DirectoryEntry::EMPTY; 1];
    if volume.read_dir("/", &mut bounded) != Ok(1)
        || &bounded[0].name[..bounded[0].name_length] != b"hello.txt"
        || volume.read_dir("/", &mut []).is_err()
        || volume
            .read_dir("/hello.txt", &mut entries)
            .is_err_and(|error| error != Error::NotDirectory)
    {
        return false;
    }
    if !Mount::open(malformed_directory_read_sector)
        .and_then(|volume| volume.read_dir("/", &mut entries).map(|_| volume))
        .is_err_and(|error| error == Error::TreeCorrupt)
    {
        return false;
    }
    if !volume
        .read_file("/", &mut [0u8; 1])
        .is_err_and(|error| error == Error::IsDirectory)
    {
        return false;
    }
    if !volume
        .read_file("/hello.txt", &mut [0u8; 4])
        .is_err_and(|error| error == Error::BufferTooSmall)
    {
        return false;
    }
    let Ok(partitioned) = Mount::open_at(fixture_partition_read_sector, 100, 256) else {
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
    let Ok(writable) = Mount::open_at_with_writer(
        fixture_transaction_read_sector,
        fixture_transaction_write_sector,
        fixture_transaction_flush_cache,
        0,
        u64::MAX,
    ) else {
        return false;
    };
    if !writable.geometry().read_only
        || writable
            .write_file("/hello.txt", b"btrfs fixture\n")
            .is_err_and(|error| error != Error::UnsupportedFeature)
        || unsafe { *core::ptr::addr_of!(FIXTURE_TX_WRITE_COUNT) } != 0
    {
        return false;
    }
    if !fixture_transaction_check() {
        return false;
    }
    if !Mount::open(corrupt_fixture_read_sector)
        .is_err_and(|error| error == Error::ChecksumMismatch)
    {
        return false;
    }
    Mount::open(unsupported_checksum_read_sector)
        .is_err_and(|error| error == Error::UnsupportedChecksum)
}

#[derive(Clone, Copy)]
struct FixtureSector {
    lba: u64,
    data: [u8; SECTOR_SIZE],
    valid: bool,
}

impl FixtureSector {
    const EMPTY: Self = Self {
        lba: 0,
        data: [0; SECTOR_SIZE],
        valid: false,
    };
}

static mut FIXTURE_TX_MEDIA: [FixtureSector; MAX_TRANSACTION_SECTORS] =
    [FixtureSector::EMPTY; MAX_TRANSACTION_SECTORS];
static mut FIXTURE_TX_WRITE_LOG: [u64; MAX_FIXTURE_WRITES] = [0; MAX_FIXTURE_WRITES];
static mut FIXTURE_TX_WRITE_COUNT: usize = 0;
static mut FIXTURE_TX_FAIL_ON: usize = usize::MAX;

fn fixture_transaction_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    let media = unsafe { &*core::ptr::addr_of!(FIXTURE_TX_MEDIA) };
    if let Some(sector) = media
        .iter()
        .find(|sector| sector.valid && sector.lba == lba)
    {
        output.copy_from_slice(&sector.data);
        true
    } else {
        fixture_read_sector(lba, output)
    }
}

fn fixture_transaction_write_sector(lba: u64, input: &[u8; SECTOR_SIZE]) -> bool {
    let media = unsafe { &mut *core::ptr::addr_of_mut!(FIXTURE_TX_MEDIA) };
    let write_count = unsafe { &mut *core::ptr::addr_of_mut!(FIXTURE_TX_WRITE_COUNT) };
    let write_log = unsafe { &mut *core::ptr::addr_of_mut!(FIXTURE_TX_WRITE_LOG) };
    let fail_on = unsafe { *core::ptr::addr_of!(FIXTURE_TX_FAIL_ON) };
    let call = *write_count;
    *write_count += 1;
    if call == fail_on {
        return false;
    }
    if call < write_log.len() {
        write_log[call] = lba;
    }
    let mut slot_index = None;
    for (index, sector) in media.iter().enumerate() {
        if sector.valid && sector.lba == lba {
            slot_index = Some(index);
            break;
        }
    }
    if slot_index.is_none() {
        slot_index = media.iter().position(|sector| !sector.valid);
    }
    let Some(slot_index) = slot_index else {
        return false;
    };
    let sector = &mut media[slot_index];
    sector.lba = lba;
    sector.data.copy_from_slice(input);
    sector.valid = true;
    true
}

fn fixture_transaction_flush_cache() -> bool {
    true
}

fn reset_fixture_transaction() {
    unsafe {
        *core::ptr::addr_of_mut!(FIXTURE_TX_MEDIA) =
            [FixtureSector::EMPTY; MAX_TRANSACTION_SECTORS];
        *core::ptr::addr_of_mut!(FIXTURE_TX_WRITE_LOG) = [0; MAX_FIXTURE_WRITES];
        *core::ptr::addr_of_mut!(FIXTURE_TX_WRITE_COUNT) = 0;
        *core::ptr::addr_of_mut!(FIXTURE_TX_FAIL_ON) = usize::MAX;
    }
}

fn fixture_transaction_block(lba: u64, output: &mut [u8; MAX_NODE_SIZE]) -> bool {
    for index in 0..output.len() / SECTOR_SIZE {
        let mut sector = [0u8; SECTOR_SIZE];
        if !fixture_transaction_read_sector(lba + index as u64, &mut sector) {
            return false;
        }
        let start = index * SECTOR_SIZE;
        output[start..start + SECTOR_SIZE].copy_from_slice(&sector);
    }
    true
}

fn fixture_transaction_check() -> bool {
    reset_fixture_transaction();
    let Ok(volume) = Mount::open_at_with_writer(
        fixture_transaction_read_sector,
        fixture_transaction_write_sector,
        fixture_transaction_flush_cache,
        0,
        u64::MAX,
    ) else {
        return false;
    };
    let Ok(mut transaction) = volume.begin_transaction() else {
        return false;
    };
    if transaction.generation() != 2 {
        return false;
    }

    let mut data_block = [0u8; MAX_NODE_SIZE];
    fixture_tree(0x4000, &mut data_block);
    write_u64(&mut data_block, 0x30, 0x5000);
    let checksum = crc32c(&data_block[32..]);
    write_u32(&mut data_block, 0, checksum);
    if transaction
        .stage_tree_block(0x5000, 0x5000, TRANSACTION_DATA_PHASE, data_block)
        .is_err()
    {
        return false;
    }

    let mut pointer_block = [0u8; MAX_NODE_SIZE];
    fixture_tree(0x4000, &mut pointer_block);
    write_u64(&mut pointer_block, 0x30, 0x6000);
    let checksum = crc32c(&pointer_block[32..]);
    write_u32(&mut pointer_block, 0, checksum);
    if transaction
        .stage_tree_block(0x6000, 0x6000, TRANSACTION_POINTER_PHASE, pointer_block)
        .is_err()
    {
        return false;
    }

    let mut superblock = [0u8; SUPERBLOCK_SIZE];
    fixture_superblock(&mut superblock);
    if transaction
        .stage_superblock(SUPERBLOCK_OFFSET, superblock)
        .is_err()
        || transaction.commit().is_err()
    {
        return false;
    }
    let write_log = unsafe { &*core::ptr::addr_of!(FIXTURE_TX_WRITE_LOG) };
    let write_count = unsafe { *core::ptr::addr_of!(FIXTURE_TX_WRITE_COUNT) };
    if write_count != 24
        || write_log[0] != 0x5000 / SECTOR_SIZE as u64
        || write_log[8] != 0x6000 / SECTOR_SIZE as u64
        || write_log[16] != SUPERBLOCK_OFFSET / SECTOR_SIZE as u64
    {
        return false;
    }
    let mut committed_data = [0u8; MAX_NODE_SIZE];
    let mut committed_pointer = [0u8; MAX_NODE_SIZE];
    if !fixture_transaction_block(0x5000 / SECTOR_SIZE as u64, &mut committed_data)
        || !fixture_transaction_block(0x6000 / SECTOR_SIZE as u64, &mut committed_pointer)
        || le_u64(&committed_data, 0x50) != 2
        || le_u64(&committed_pointer, 0x50) != 2
        || le_u32(&committed_data, 0) != crc32c(&committed_data[32..])
        || le_u32(&committed_pointer, 0) != crc32c(&committed_pointer[32..])
    {
        return false;
    }
    let mut committed_superblock = [0u8; SUPERBLOCK_SIZE];
    if !fixture_transaction_block(
        SUPERBLOCK_OFFSET / SECTOR_SIZE as u64,
        &mut committed_superblock,
    ) || le_u64(&committed_superblock, 0x48) != 2
        || le_u32(&committed_superblock, 0) != crc32c(&committed_superblock[32..])
    {
        return false;
    }

    reset_fixture_transaction();
    unsafe { *core::ptr::addr_of_mut!(FIXTURE_TX_FAIL_ON) = 10 };
    let Ok(volume) = Mount::open_at_with_writer(
        fixture_transaction_read_sector,
        fixture_transaction_write_sector,
        fixture_transaction_flush_cache,
        0,
        u64::MAX,
    ) else {
        return false;
    };
    let Ok(mut transaction) = volume.begin_transaction() else {
        return false;
    };
    let mut data_block = [0u8; MAX_NODE_SIZE];
    fixture_tree(0x4000, &mut data_block);
    write_u64(&mut data_block, 0x30, 0x5000);
    let checksum = crc32c(&data_block[32..]);
    write_u32(&mut data_block, 0, checksum);
    let mut pointer_block = [0u8; MAX_NODE_SIZE];
    fixture_tree(0x4000, &mut pointer_block);
    write_u64(&mut pointer_block, 0x30, 0x6000);
    let checksum = crc32c(&pointer_block[32..]);
    write_u32(&mut pointer_block, 0, checksum);
    let mut superblock = [0u8; SUPERBLOCK_SIZE];
    fixture_superblock(&mut superblock);
    if transaction
        .stage_tree_block(0x5000, 0x5000, TRANSACTION_DATA_PHASE, data_block)
        .is_err()
        || transaction
            .stage_tree_block(0x6000, 0x6000, TRANSACTION_POINTER_PHASE, pointer_block)
            .is_err()
        || transaction
            .stage_superblock(SUPERBLOCK_OFFSET, superblock)
            .is_err()
        || transaction.commit() != Err(Error::Io)
    {
        return false;
    }
    let mut restored = [0u8; MAX_NODE_SIZE];
    if !fixture_transaction_block(0x5000 / SECTOR_SIZE as u64, &mut restored)
        || restored != [0; MAX_NODE_SIZE]
        || !fixture_transaction_block(0x6000 / SECTOR_SIZE as u64, &mut restored)
        || restored != [0; MAX_NODE_SIZE]
    {
        return false;
    }
    let mut restored_superblock = [0u8; SUPERBLOCK_SIZE];
    fixture_superblock(&mut restored_superblock);
    let mut observed_superblock = [0u8; SUPERBLOCK_SIZE];
    if !fixture_transaction_block(
        SUPERBLOCK_OFFSET / SECTOR_SIZE as u64,
        &mut observed_superblock,
    ) || observed_superblock != restored_superblock
    {
        return false;
    }
    true
}

fn fixture_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    let offset = lba * SECTOR_SIZE as u64;
    output.fill(0);
    if (SUPERBLOCK_OFFSET..SUPERBLOCK_OFFSET + SUPERBLOCK_SIZE as u64).contains(&offset) {
        let mut block = [0u8; SUPERBLOCK_SIZE];
        fixture_superblock(&mut block);
        let start = (offset - SUPERBLOCK_OFFSET) as usize;
        output.copy_from_slice(&block[start..start + SECTOR_SIZE]);
        return true;
    }
    for logical in [0x1000u64, 0x2000, 0x3000, 0x4000] {
        if (logical..logical + MAX_NODE_SIZE as u64).contains(&offset) {
            let mut block = [0u8; MAX_NODE_SIZE];
            fixture_tree(logical, &mut block);
            let start = (offset - logical) as usize;
            output.copy_from_slice(&block[start..start + SECTOR_SIZE]);
            return true;
        }
    }
    true
}

fn fixture_partition_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    lba.checked_sub(100)
        .is_some_and(|relative| fixture_read_sector(relative, output))
}

fn corrupt_fixture_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    fixture_read_sector(lba, output);
    if lba == SUPERBLOCK_OFFSET / SECTOR_SIZE as u64 {
        output[0] ^= 1;
    }
    true
}

fn unsupported_checksum_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    fixture_read_sector(lba, output);
    if lba == SUPERBLOCK_OFFSET / SECTOR_SIZE as u64 {
        output[0xc4] = 1;
    }
    true
}

fn malformed_directory_read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> bool {
    let tree_lba = 0x4000 / SECTOR_SIZE as u64;
    if lba == tree_lba || lba == tree_lba + 3820 / SECTOR_SIZE as u64 {
        let mut block = [0u8; MAX_NODE_SIZE];
        fixture_tree(0x4000, &mut block);
        block[3820 + 27] = 0xff;
        block[3820 + 28] = 0xff;
        let checksum = crc32c(&block[32..]);
        write_u32(&mut block, 0, checksum);
        let start = (lba - tree_lba) as usize * SECTOR_SIZE;
        output.copy_from_slice(&block[start..start + SECTOR_SIZE]);
        return true;
    }
    fixture_read_sector(lba, output)
}

fn fixture_superblock(output: &mut [u8; SUPERBLOCK_SIZE]) {
    output[0x20..0x30].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    write_u64(output, 0x30, SUPERBLOCK_OFFSET);
    output[0x40..0x48].copy_from_slice(MAGIC);
    write_u64(output, 0x48, 1);
    write_u64(output, 0x50, 0x2000);
    write_u64(output, 0x58, 0x1000);
    write_u64(output, 0x70, 0x20_000);
    write_u64(output, 0x78, 0x10_000);
    write_u64(output, 0x80, 6);
    write_u64(output, 0x88, 1);
    write_u32(output, 0x90, 4096);
    write_u32(output, 0x94, 4096);
    write_u32(output, 0x98, 4096);
    write_u32(output, 0x9c, 4096);
    write_u32(output, 0xa0, 113);
    write_u16(output, 0xc4, CSUM_CRC32C);
    output[0xc6] = 1;
    output[0xc7] = 0;
    write_key(
        &mut output[SYS_CHUNK_ARRAY_OFFSET..],
        0,
        256,
        CHUNK_ITEM_KEY,
        0,
    );
    write_chunk(&mut output[SYS_CHUNK_ARRAY_OFFSET + 17..], 0x10_000, 6, 0);
    let checksum = crc32c(&output[32..]);
    write_u32(output, 0, checksum);
}

fn fixture_tree(logical: u64, output: &mut [u8; MAX_NODE_SIZE]) {
    init_tree_header(
        output,
        logical,
        match logical {
            0x1000 => (1, 0, BTRFS_CHUNK_TREE_OBJECTID),
            0x2000 => (1, 1, BTRFS_ROOT_TREE_OBJECTID),
            0x3000 => (2, 0, BTRFS_ROOT_TREE_OBJECTID),
            0x4000 => (4, 0, BTRFS_FS_TREE_OBJECTID),
            _ => return,
        },
    );
    match logical {
        0x1000 => fixture_chunk_leaf(output),
        0x2000 => {
            write_key(output, TREE_HEADER_SIZE, 5, ROOT_ITEM_KEY, 0);
            write_u64(output, TREE_HEADER_SIZE + 17, 0x3000);
            write_u64(output, TREE_HEADER_SIZE + 25, 1);
        }
        0x3000 => fixture_root_leaf(output),
        0x4000 => fixture_fs_leaf(output),
        _ => {}
    }
    let checksum = crc32c(&output[32..]);
    write_u32(output, 0, checksum);
}

fn init_tree_header(
    output: &mut [u8; MAX_NODE_SIZE],
    logical: u64,
    level_and_owner: (u32, u8, u64),
) {
    output[0x20..0x30].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    write_u64(output, 0x30, logical);
    write_u64(output, 0x50, 1);
    write_u64(output, 0x58, level_and_owner.2);
    write_u32(output, 0x60, level_and_owner.0);
    output[0x64] = level_and_owner.1;
}

fn fixture_chunk_leaf(output: &mut [u8; MAX_NODE_SIZE]) {
    let data_offset: usize = 4000;
    write_key(output, TREE_HEADER_SIZE, 256, CHUNK_ITEM_KEY, 0);
    write_u32(output, TREE_HEADER_SIZE + 17, data_offset as u32);
    write_u32(output, TREE_HEADER_SIZE + 21, 96);
    write_chunk(&mut output[data_offset..], 0x10_000, 6, 0);
}

fn fixture_root_leaf(output: &mut [u8; MAX_NODE_SIZE]) {
    let mut first = [0u8; 239];
    write_root_item(&mut first, 0x4000, false, false);
    write_key(
        output,
        TREE_HEADER_SIZE,
        BTRFS_FS_TREE_OBJECTID,
        ROOT_ITEM_KEY,
        0,
    );
    write_u32(output, TREE_HEADER_SIZE + 17, 3857);
    write_u32(output, TREE_HEADER_SIZE + 21, first.len() as u32);
    output[3857..3857 + first.len()].copy_from_slice(&first);

    let mut snapshot = [0u8; 279];
    write_root_item(&mut snapshot, 0x5000, true, true);
    write_key(
        output,
        TREE_HEADER_SIZE + LEAF_ITEM_SIZE,
        256,
        ROOT_ITEM_KEY,
        0,
    );
    write_u32(output, TREE_HEADER_SIZE + LEAF_ITEM_SIZE + 17, 3578);
    write_u32(
        output,
        TREE_HEADER_SIZE + LEAF_ITEM_SIZE + 21,
        snapshot.len() as u32,
    );
    output[3578..3578 + snapshot.len()].copy_from_slice(&snapshot);
}

fn fixture_fs_leaf(output: &mut [u8; MAX_NODE_SIZE]) {
    let inode_offset: usize = 3936;
    write_key(output, TREE_HEADER_SIZE, 256, INODE_ITEM_KEY, 0);
    write_u32(output, TREE_HEADER_SIZE + 17, inode_offset as u32);
    write_u32(output, TREE_HEADER_SIZE + 21, 160);
    write_inode(output, inode_offset, INODE_DIRECTORY | 0o755, 1024);

    let dir_offset: usize = 3820;
    write_key(
        output,
        TREE_HEADER_SIZE + LEAF_ITEM_SIZE,
        256,
        DIR_ITEM_KEY,
        0,
    );
    write_u32(
        output,
        TREE_HEADER_SIZE + LEAF_ITEM_SIZE + 17,
        dir_offset as u32,
    );
    write_u32(output, TREE_HEADER_SIZE + LEAF_ITEM_SIZE + 21, 77);
    write_dir_item(&mut output[dir_offset..], 257, b"hello.txt");
    write_dir_item(&mut output[dir_offset + 39..], 257, b"copy.txt");

    let file_inode_offset: usize = 3737;
    write_key(
        output,
        TREE_HEADER_SIZE + LEAF_ITEM_SIZE * 2,
        257,
        INODE_ITEM_KEY,
        0,
    );
    write_u32(
        output,
        TREE_HEADER_SIZE + LEAF_ITEM_SIZE * 2 + 17,
        file_inode_offset as u32,
    );
    write_u32(output, TREE_HEADER_SIZE + LEAF_ITEM_SIZE * 2 + 21, 160);
    write_inode(output, file_inode_offset, INODE_REGULAR | 0o644, 14);

    let extent_offset: usize = 3671;
    write_key(
        output,
        TREE_HEADER_SIZE + LEAF_ITEM_SIZE * 3,
        257,
        EXTENT_DATA_KEY,
        0,
    );
    write_u32(
        output,
        TREE_HEADER_SIZE + LEAF_ITEM_SIZE * 3 + 17,
        extent_offset as u32,
    );
    write_u32(output, TREE_HEADER_SIZE + LEAF_ITEM_SIZE * 3 + 21, 67);
    write_u64(output, extent_offset + 8, 14);
    output[extent_offset + 19] = 0;
    output[extent_offset + 53..extent_offset + 67].copy_from_slice(b"btrfs fixture\n");
}

fn write_root_item(output: &mut [u8], bytenr: u64, read_only: bool, snapshot: bool) {
    write_u64(output, 160, 1);
    write_u64(output, 168, 256);
    write_u64(output, 176, bytenr);
    write_u64(output, 208, if read_only { ROOT_SUBVOL_RDONLY } else { 0 });
    write_u32(output, 216, 1);
    if snapshot {
        output[263..279].fill(1);
    }
}

fn write_inode(output: &mut [u8], offset: usize, mode: u32, size: u64) {
    write_u64(output, offset + 16, size);
    write_u32(output, offset + 40, 1);
    write_u32(output, offset + 52, mode);
}

fn write_dir_item(output: &mut [u8], child: u64, name: &[u8]) {
    write_u64(output, 0, child);
    output[8] = INODE_ITEM_KEY;
    write_u64(output, 17, 1);
    write_u16(output, 25, 0);
    write_u16(output, 27, name.len() as u16);
    output[29] = 1;
    output[30..30 + name.len()].copy_from_slice(name);
}

fn parse_chunk_array(
    superblock: &[u8],
    chunks: &mut [Chunk; MAX_CHUNKS],
    count: &mut usize,
    length: usize,
) -> Result<(), Error> {
    if length > SYSTEM_CHUNK_ARRAY_SIZE || SYS_CHUNK_ARRAY_OFFSET + length > superblock.len() {
        return Err(Error::InvalidSuperblock);
    }
    let mut offset = 0;
    while offset < length {
        if length - offset < 17 {
            return Err(Error::InvalidSuperblock);
        }
        let key_offset = le_u64(superblock, SYS_CHUNK_ARRAY_OFFSET + offset + 9);
        let data_offset = SYS_CHUNK_ARRAY_OFFSET + offset + 17;
        let data_length = length - offset - 17;
        let chunk = parse_chunk(
            key_offset,
            &superblock[data_offset..data_offset + data_length],
        )?;
        let consumed = 17 + 48 + 48;
        if consumed > length - offset {
            return Err(Error::InvalidSuperblock);
        }
        add_chunk(chunks, count, chunk)?;
        offset += consumed;
    }
    Ok(())
}

fn parse_chunk(logical: u64, data: &[u8]) -> Result<Chunk, Error> {
    if data.len() < 96 {
        return Err(Error::TreeCorrupt);
    }
    let length = le_u64(data, 0);
    let profile = le_u64(data, 24);
    if length == 0 || le_u16(data, 44) != 1 || profile & CHUNK_PROFILE_MASK != 0 {
        return Err(Error::UnsupportedFeature);
    }
    Ok(Chunk {
        logical,
        length,
        physical: le_u64(data, 56),
    })
}

fn add_chunk(
    chunks: &mut [Chunk; MAX_CHUNKS],
    count: &mut usize,
    chunk: Chunk,
) -> Result<(), Error> {
    for existing in chunks.iter_mut().take(*count) {
        if existing.logical == chunk.logical {
            *existing = chunk;
            return Ok(());
        }
    }
    if *count == chunks.len() {
        return Err(Error::UnsupportedFeature);
    }
    chunks[*count] = chunk;
    *count += 1;
    Ok(())
}

fn parse_root_item(id: u64, data: &[u8]) -> Result<RootItem, Error> {
    if data.len() < 239 {
        return Err(Error::TreeCorrupt);
    }
    let snapshot = data.len() >= 279 && data[263..279].iter().any(|byte| *byte != 0);
    Ok(RootItem {
        id,
        root_dirid: le_u64(data, 168),
        bytenr: le_u64(data, 176),
        flags: le_u64(data, 208),
        snapshot,
    })
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

fn read_sectors(
    reader: ReadSector,
    base_lba: u64,
    offset: u64,
    output: &mut [u8],
) -> Result<(), Error> {
    if !offset.is_multiple_of(SECTOR_SIZE as u64) || !output.len().is_multiple_of(SECTOR_SIZE) {
        return Err(Error::Io);
    }
    for index in 0..output.len() / SECTOR_SIZE {
        let mut sector = [0u8; SECTOR_SIZE];
        let lba = base_lba
            .checked_add(offset / SECTOR_SIZE as u64 + index as u64)
            .ok_or(Error::Io)?;
        if !reader(lba, &mut sector) {
            return Err(Error::Io);
        }
        let start = index * SECTOR_SIZE;
        output[start..start + SECTOR_SIZE].copy_from_slice(&sector);
    }
    Ok(())
}

fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0x82f6_3b78 & mask);
        }
    }
    crc
}

fn copy_array(bytes: &[u8]) -> [u8; 16] {
    let mut output = [0u8; 16];
    output.copy_from_slice(bytes);
    output
}

fn write_chunk(output: &mut [u8], length: u64, kind: u64, physical: u64) {
    write_u64(output, 0, length);
    write_u64(output, 8, BTRFS_CHUNK_TREE_OBJECTID);
    write_u64(output, 16, 4096);
    write_u64(output, 24, kind);
    write_u32(output, 32, 4096);
    write_u32(output, 36, 4096);
    write_u32(output, 40, 4096);
    write_u16(output, 44, 1);
    write_u16(output, 46, 0);
    write_u64(output, 48, 1);
    write_u64(output, 56, physical);
    output[64..80].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
}

fn write_key(output: &mut [u8], offset: usize, objectid: u64, kind: u8, key_offset: u64) {
    write_u64(output, offset, objectid);
    output[offset + 8] = kind;
    write_u64(output, offset + 9, key_offset);
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
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

fn le_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}
