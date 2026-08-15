use crate::drivers::block;

const BYTES_PER_SECTOR: usize = 512;
const MAX_LFN_CHARS: usize = 260;
const MAX_COMPONENTS: usize = 16;
const MAX_PATH_BYTES: usize = 4096;
const MAX_COMPONENT_BYTES: usize = MAX_LFN_CHARS * 4;
const EOC_MIN: u32 = 0x0fff_fff8;
const BAD_CLUSTER: u32 = 0x0fff_fff7;
const FAT_VALUE_MASK: u32 = 0x0fff_ffff;
const FAT_HIGH_BITS: u32 = 0xf000_0000;
const MAX_DATA_CLUSTER: u32 = 0x0fff_ffef;

pub type ReadSector = fn(u64, &mut [u8; BYTES_PER_SECTOR]) -> bool;
pub type WriteSector = fn(u64, &[u8; BYTES_PER_SECTOR]) -> bool;

pub const MAX_NAME_BYTES: usize = MAX_COMPONENT_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Io,
    InvalidBpb,
    InvalidPath,
    NotFound,
    NotDirectory,
    IsDirectory,
    BadDirectory,
    BadClusterChain,
    BufferTooSmall,
    NoSpace,
    FileTooLarge,
    FatMismatch,
    ReadOnly,
    InvalidPersistenceRecord,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub total_sectors: u64,
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub cluster_count: u32,
    pub root_cluster: u32,
    pub read_only: bool,
}

#[derive(Clone, Copy)]
pub struct Mount {
    reader: ReadSector,
    writer: Option<WriteSector>,
    base_lba: u64,
    reserved_sectors: u16,
    fat_count: u8,
    fat_size: u32,
    total_sectors: u64,
    first_data_sector: u64,
    sectors_per_cluster: u8,
    cluster_count: u32,
    root_cluster: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub directory: bool,
    pub cluster: u32,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub name: [u8; MAX_NAME_BYTES],
    pub name_len: u16,
    pub directory: bool,
    pub cluster: u32,
    pub size: u64,
}

impl DirectoryEntry {
    pub const EMPTY: Self = Self {
        name: [0; MAX_NAME_BYTES],
        name_len: 0,
        directory: false,
        cluster: 0,
        size: 0,
    };

    pub fn name_bytes(&self) -> &[u8] {
        &self.name[..(self.name_len as usize).min(self.name.len())]
    }
}

#[derive(Clone, Copy)]
struct EntryMetadata {
    directory: bool,
    cluster: u32,
    size: u32,
}

#[derive(Clone, Copy)]
struct EntryLocation {
    entry: EntryMetadata,
    sector: u64,
    offset: usize,
}

#[derive(Clone, Copy)]
struct LongName {
    chars: [u16; MAX_LFN_CHARS],
    checksum: u8,
    max_sequence: u8,
    next_sequence: u8,
    seen: u32,
    valid: bool,
}

impl LongName {
    const EMPTY: Self = Self {
        chars: [0; MAX_LFN_CHARS],
        checksum: 0,
        max_sequence: 0,
        next_sequence: 0,
        seen: 0,
        valid: false,
    };

    fn reset(&mut self) {
        *self = Self::EMPTY;
    }

    fn accept(&mut self, entry: &[u8; 32]) {
        let sequence = entry[0] & 0x1f;
        if sequence == 0
            || sequence as usize > MAX_LFN_CHARS / 13
            || entry[12] != 0
            || le_u16(entry, 26) != 0
        {
            self.reset();
            return;
        }
        if entry[0] & 0x40 != 0 {
            if self.valid {
                self.reset();
            }
            self.max_sequence = sequence;
            self.next_sequence = sequence;
            self.checksum = entry[13];
            self.seen = 0;
            self.valid = true;
        }
        if !self.valid
            || self.checksum != entry[13]
            || sequence != self.next_sequence
            || self.seen & (1u32 << (sequence - 1)) != 0
        {
            self.reset();
            return;
        }
        let start = (sequence as usize - 1) * 13;
        for (index, offset) in [1usize, 14, 28].iter().enumerate() {
            let length = [5usize, 6, 2][index];
            let character_offset = [0usize, 5, 11][index];
            for character in 0..length {
                let position = *offset + character * 2;
                self.chars[start + character_offset + character] =
                    u16::from_le_bytes([entry[position], entry[position + 1]]);
            }
        }
        self.seen |= 1 << (sequence - 1);
        self.next_sequence = sequence.saturating_sub(1);
    }

    fn matches(&self, target: &str, short_checksum: u8) -> bool {
        if !self.valid
            || self.max_sequence == 0
            || self.checksum != short_checksum
            || self.next_sequence != 0
            || self.seen & ((1u32 << self.max_sequence) - 1) != (1u32 << self.max_sequence) - 1
        {
            return false;
        }
        let mut encoded = [0u8; MAX_NAME_BYTES];
        let Some(length) = self.encode(&mut encoded) else {
            return false;
        };
        encoded[..length] == target.as_bytes()[..]
    }

    fn encode(self, output: &mut [u8; MAX_NAME_BYTES]) -> Option<usize> {
        if !self.valid || self.max_sequence == 0 || self.next_sequence != 0 {
            return None;
        }
        let length = self.max_sequence as usize * 13;
        let mut offset = 0;
        let mut index = 0;
        while index < length {
            let character = self.chars[index];
            if character == 0 {
                break;
            }
            if character == 0xffff {
                return None;
            }
            let code_point = if (0xd800..=0xdbff).contains(&character) {
                index += 1;
                let low = *self.chars.get(index)?;
                if !(0xdc00..=0xdfff).contains(&low) {
                    return None;
                }
                0x1_0000 + (((character as u32 - 0xd800) << 10) | (low as u32 - 0xdc00))
            } else if (0xdc00..=0xdfff).contains(&character) {
                return None;
            } else {
                character as u32
            };
            offset = encode_utf8(code_point, output, offset)?;
            index += 1;
        }
        Some(offset)
    }
}

impl Mount {
    pub fn open(reader: ReadSector) -> Result<Self, Error> {
        Self::open_at(reader, None, 0, u64::MAX)
    }

    pub fn open_rw(reader: ReadSector, writer: WriteSector) -> Result<Self, Error> {
        Self::open_at(reader, Some(writer), 0, u64::MAX)
    }

    pub fn open_at(
        reader: ReadSector,
        writer: Option<WriteSector>,
        base_lba: u64,
        available_sectors: u64,
    ) -> Result<Self, Error> {
        let mut bpb = [0u8; BYTES_PER_SECTOR];
        if !reader(base_lba, &mut bpb) {
            return Err(Error::Io);
        }
        if bpb[510] != 0x55 || bpb[511] != 0xaa {
            return Err(Error::InvalidBpb);
        }
        let bytes_per_sector = le_u16(&bpb, 11);
        let sectors_per_cluster = bpb[13];
        let reserved_sectors = le_u16(&bpb, 14);
        let fat_count = bpb[16];
        let root_entries = le_u16(&bpb, 17);
        let total_sectors = if le_u16(&bpb, 19) != 0 {
            le_u16(&bpb, 19) as u64
        } else {
            le_u32(&bpb, 32) as u64
        };
        let fat_size = if le_u16(&bpb, 22) == 0 {
            le_u32(&bpb, 36)
        } else {
            return Err(Error::InvalidBpb);
        };
        let root_cluster = le_u32(&bpb, 44);
        if bytes_per_sector != BYTES_PER_SECTOR as u16
            || sectors_per_cluster == 0
            || !sectors_per_cluster.is_power_of_two()
            || sectors_per_cluster > 128
            || reserved_sectors == 0
            || fat_count == 0
            || root_entries != 0
            || fat_size == 0
            || root_cluster < 2
            || total_sectors <= reserved_sectors as u64
        {
            return Err(Error::InvalidBpb);
        }
        let first_data_sector = (reserved_sectors as u64)
            .checked_add(
                (fat_count as u64)
                    .checked_mul(fat_size as u64)
                    .ok_or(Error::InvalidBpb)?,
            )
            .ok_or(Error::InvalidBpb)?;
        if total_sectors <= first_data_sector
            || total_sectors > available_sectors
            || base_lba.checked_add(total_sectors).is_none()
        {
            return Err(Error::InvalidBpb);
        }
        let cluster_count = ((total_sectors - first_data_sector) / sectors_per_cluster as u64)
            .try_into()
            .map_err(|_| Error::InvalidBpb)?;
        if cluster_count < 65_525
            || cluster_count > MAX_DATA_CLUSTER - 1
            || (cluster_count as u64 + 2) * 4 > fat_size as u64 * BYTES_PER_SECTOR as u64
            || root_cluster > cluster_count + 1
        {
            return Err(Error::InvalidBpb);
        }
        Ok(Self {
            reader,
            writer,
            base_lba,
            reserved_sectors,
            fat_count,
            fat_size,
            total_sectors,
            first_data_sector,
            sectors_per_cluster,
            cluster_count,
            root_cluster,
        })
    }

    pub const fn geometry(self) -> Geometry {
        Geometry {
            total_sectors: self.total_sectors,
            bytes_per_sector: BYTES_PER_SECTOR as u16,
            sectors_per_cluster: self.sectors_per_cluster,
            cluster_count: self.cluster_count,
            root_cluster: self.root_cluster,
            read_only: self.writer.is_none(),
        }
    }

    pub fn read_file(self, path: &str, output: &mut [u8]) -> Result<usize, Error> {
        let (components, count) = components(path)?;
        let mut directory_cluster = self.root_cluster;
        for (index, component) in components.iter().take(count).enumerate() {
            let entry = self.find_entry(directory_cluster, component)?;
            if index + 1 != count {
                if !entry.directory {
                    return Err(Error::NotDirectory);
                }
                directory_cluster = entry.cluster;
            } else {
                if entry.directory {
                    return Err(Error::IsDirectory);
                }
                return self.read_chain(entry.cluster, entry.size as usize, output);
            }
        }
        Err(Error::InvalidPath)
    }

    pub fn stat(self, path: &str) -> Result<FileInfo, Error> {
        let (components, count) = components_allow_root(path)?;
        if count == 0 {
            return Ok(FileInfo {
                directory: true,
                cluster: self.root_cluster,
                size: 0,
            });
        }
        let mut directory_cluster = self.root_cluster;
        for (index, component) in components.iter().take(count).enumerate() {
            let entry = self.find_entry(directory_cluster, component)?;
            if index + 1 == count {
                return Ok(FileInfo {
                    directory: entry.directory,
                    cluster: entry.cluster,
                    size: entry.size as u64,
                });
            }
            if !entry.directory {
                return Err(Error::NotDirectory);
            }
            directory_cluster = entry.cluster;
        }
        Err(Error::InvalidPath)
    }

    pub fn read_dir(self, path: &str, output: &mut [DirectoryEntry]) -> Result<usize, Error> {
        let (components, count) = components_allow_root(path)?;
        let mut directory_cluster = self.root_cluster;
        for component in components.iter().take(count) {
            let entry = self.find_entry(directory_cluster, component)?;
            if !entry.directory {
                return Err(Error::NotDirectory);
            }
            directory_cluster = entry.cluster;
        }

        let mut count = 0;
        self.scan_directory(directory_cluster, |sector, offset, raw, long_name| {
            if count == output.len() {
                return Ok(true);
            }
            let entry = self.entry_metadata(raw)?;
            let mut name = [0u8; MAX_NAME_BYTES];
            let checksum = short_checksum(&raw[..11]);
            let name_len = if long_name.checksum == checksum {
                long_name.encode(&mut name)
            } else {
                None
            }
            .or_else(|| encode_short_name(raw, &mut name))
            .ok_or(Error::BadDirectory)?;
            if (name_len == 1 && name[0] == b'.') || (name_len == 2 && name[..2] == *b"..") {
                return Ok(false);
            }
            output[count] = DirectoryEntry {
                name,
                name_len: name_len as u16,
                directory: entry.directory,
                cluster: entry.cluster,
                size: entry.size as u64,
            };
            let _ = (sector, offset);
            count += 1;
            Ok(false)
        })?;
        Ok(count)
    }

    pub fn write_file(self, path: &str, input: &[u8]) -> Result<usize, Error> {
        let Some(_writer) = self.writer else {
            return Err(Error::ReadOnly);
        };
        // ponytail: Ceiling: this bounded slice resizes only an existing directory entry;
        // directory-slot allocation, create, and rename remain out of scope.
        if input.len() > u32::MAX as usize {
            return Err(Error::FileTooLarge);
        }
        let (components, count) = components(path)?;
        let mut directory_cluster = self.root_cluster;
        for component in components.iter().take(count - 1) {
            let entry = self.find_entry(directory_cluster, component)?;
            if !entry.directory {
                return Err(Error::NotDirectory);
            }
            directory_cluster = entry.cluster;
        }
        let location = self.find_entry_location(directory_cluster, components[count - 1])?;
        if location.entry.directory {
            return Err(Error::IsDirectory);
        }
        let cluster_bytes = self.sectors_per_cluster as usize * BYTES_PER_SECTOR;
        let required_clusters = input.len() / cluster_bytes
            + if input.len() % cluster_bytes == 0 {
                0
            } else {
                1
            };
        if required_clusters > self.cluster_count as usize {
            return Err(Error::NoSpace);
        }
        let old_start = location.entry.cluster;
        let (old_count, old_last) = self.chain_length_and_last(old_start)?;
        if old_start == 0 && location.entry.size != 0 {
            return Err(Error::BadClusterChain);
        }
        if old_start != 0 {
            let capacity = old_count
                .checked_mul(cluster_bytes)
                .ok_or(Error::BadClusterChain)?;
            if location.entry.size as usize > capacity {
                return Err(Error::BadClusterChain);
            }
        }

        if required_clusters < old_count {
            if required_clusters == 0 {
                // Publish an empty file before releasing its old chain. If a later
                // FAT write fails, the result is a valid file with leaked clusters.
                self.update_directory_entry(location, 0, 0)?;
                self.free_chain(old_start)?;
                return Ok(0);
            }
            self.write_chain(old_start, input)?;
            self.update_directory_entry(location, old_start, input.len() as u32)?;
            let (keep_last, tail_start) = self.chain_split(old_start, required_clusters)?;
            self.write_fat_entry(keep_last, EOC_MIN)?;
            if let Some(tail) = tail_start {
                self.free_chain(tail)?;
            }
            return Ok(input.len());
        }

        if required_clusters > old_count {
            let (new_first, _) = self.allocate_chain(required_clusters - old_count)?;
            if let Some(last) = old_last {
                if let Err(error) = self.write_fat_entry(last, new_first) {
                    let _ = self.free_chain(new_first);
                    return Err(error);
                }
            }
            let new_start = old_last.map_or(new_first, |_| old_start);
            if let Err(error) = self.write_chain(new_start, input) {
                self.rollback_extension(old_last, new_first);
                return Err(error);
            }
            if let Err(error) = self.update_directory_entry(location, new_start, input.len() as u32)
            {
                self.rollback_extension(old_last, new_first);
                return Err(error);
            }
            return Ok(input.len());
        }

        if !input.is_empty() {
            self.write_chain(old_start, input)?;
        }
        self.update_directory_entry(location, old_start, input.len() as u32)?;
        Ok(input.len())
    }

    fn update_directory_entry(
        self,
        location: EntryLocation,
        cluster: u32,
        size: u32,
    ) -> Result<(), Error> {
        if cluster != 0 && !self.valid_cluster(cluster) {
            return Err(Error::BadClusterChain);
        }
        let Some(_writer) = self.writer else {
            return Err(Error::ReadOnly);
        };
        let Some(end) = location.offset.checked_add(32) else {
            return Err(Error::Io);
        };
        if end > BYTES_PER_SECTOR {
            return Err(Error::Io);
        }
        let mut data = [0u8; BYTES_PER_SECTOR];
        if !self.read_sector(location.sector, &mut data) {
            return Err(Error::Io);
        }
        let low = (cluster as u16).to_le_bytes();
        let high = ((cluster >> 16) as u16).to_le_bytes();
        data[location.offset + 20..location.offset + 22].copy_from_slice(&high);
        data[location.offset + 26..location.offset + 28].copy_from_slice(&low);
        data[location.offset + 28..location.offset + 32].copy_from_slice(&size.to_le_bytes());
        if !self.write_sector(location.sector, &data) {
            return Err(Error::Io);
        }
        let mut verify = [0u8; BYTES_PER_SECTOR];
        if !self.read_sector(location.sector, &mut verify)
            || le_u16(&verify, location.offset + 20) != (cluster >> 16) as u16
            || le_u16(&verify, location.offset + 26) != cluster as u16
            || le_u32(&verify, location.offset + 28) != size
        {
            return Err(Error::Io);
        }
        Ok(())
    }

    fn find_entry(self, directory_cluster: u32, target: &str) -> Result<EntryMetadata, Error> {
        Ok(self.find_entry_location(directory_cluster, target)?.entry)
    }

    fn find_entry_location(
        self,
        directory_cluster: u32,
        target: &str,
    ) -> Result<EntryLocation, Error> {
        let mut found = None;
        self.scan_directory(directory_cluster, |sector, offset, entry, long_name| {
            let checksum = short_checksum(&entry[..11]);
            if long_name.matches(target, checksum) || short_name_matches(entry, target) {
                found = Some(EntryLocation {
                    entry: self.entry_metadata(entry)?,
                    sector,
                    offset,
                });
                return Ok(true);
            }
            Ok(false)
        })?;
        found.ok_or(Error::NotFound)
    }

    fn scan_directory<F>(self, directory_cluster: u32, mut visit: F) -> Result<(), Error>
    where
        F: FnMut(u64, usize, &[u8; 32], LongName) -> Result<bool, Error>,
    {
        if !self.valid_cluster(directory_cluster) {
            return Err(Error::BadClusterChain);
        }
        let mut cluster = directory_cluster;
        let mut long_name = LongName::EMPTY;
        for _ in 0..self.cluster_count {
            for sector in 0..self.sectors_per_cluster as u64 {
                let mut data = [0u8; BYTES_PER_SECTOR];
                if !self.read_sector(self.cluster_sector(cluster) + sector, &mut data) {
                    return Err(Error::Io);
                }
                for entry_offset in (0..BYTES_PER_SECTOR).step_by(32) {
                    let mut entry = [0u8; 32];
                    entry.copy_from_slice(&data[entry_offset..entry_offset + 32]);
                    if entry[0] == 0 {
                        return Ok(());
                    }
                    if entry[0] == 0xe5 {
                        long_name.reset();
                        continue;
                    }
                    if entry[11] == 0x0f {
                        long_name.accept(&entry);
                        continue;
                    }
                    let pending_long_name = long_name;
                    long_name.reset();
                    if entry[11] & 0x08 != 0 {
                        continue;
                    }
                    if visit(
                        self.cluster_sector(cluster) + sector,
                        entry_offset,
                        &entry,
                        pending_long_name,
                    )? {
                        return Ok(());
                    }
                }
            }
            let Some(next) = self.next_cluster(cluster)? else {
                return Ok(());
            };
            cluster = next;
        }
        Err(Error::BadClusterChain)
    }

    fn entry_metadata(self, entry: &[u8; 32]) -> Result<EntryMetadata, Error> {
        let directory = entry[11] & 0x10 != 0;
        let cluster = (le_u16(entry, 20) as u32) << 16 | le_u16(entry, 26) as u32;
        let size = le_u32(entry, 28);
        if directory {
            if !self.valid_cluster(cluster) {
                return Err(Error::BadClusterChain);
            }
        } else if cluster != 0 && !self.valid_cluster(cluster) {
            return Err(Error::BadClusterChain);
        } else if size != 0 && cluster == 0 {
            return Err(Error::BadClusterChain);
        }
        Ok(EntryMetadata {
            directory,
            cluster,
            size,
        })
    }

    fn read_chain(self, start: u32, size: usize, output: &mut [u8]) -> Result<usize, Error> {
        if size > output.len() {
            return Err(Error::BufferTooSmall);
        }
        if size == 0 {
            return Ok(0);
        }
        let mut cluster = start;
        let mut copied = 0;
        for _ in 0..self.cluster_count {
            if cluster < 2 || cluster > self.cluster_count + 1 {
                return Err(Error::BadClusterChain);
            }
            for sector in 0..self.sectors_per_cluster as u64 {
                if copied == size {
                    return Ok(copied);
                }
                let mut data = [0u8; BYTES_PER_SECTOR];
                if !self.read_sector(self.cluster_sector(cluster) + sector, &mut data) {
                    return Err(Error::Io);
                }
                let length = (size - copied).min(BYTES_PER_SECTOR);
                output[copied..copied + length].copy_from_slice(&data[..length]);
                copied += length;
            }
            let Some(next) = self.next_cluster(cluster)? else {
                return (copied == size)
                    .then_some(copied)
                    .ok_or(Error::BadClusterChain);
            };
            cluster = next;
        }
        Err(Error::BadClusterChain)
    }

    fn valid_cluster(self, cluster: u32) -> bool {
        (2..=self.cluster_count + 1).contains(&cluster)
    }

    fn chain_length_and_last(self, start: u32) -> Result<(usize, Option<u32>), Error> {
        if start == 0 {
            return Ok((0, None));
        }
        let mut cluster = start;
        for count in 0..self.cluster_count as usize {
            if !self.valid_cluster(cluster) {
                return Err(Error::BadClusterChain);
            }
            let Some(next) = self.next_cluster(cluster)? else {
                return Ok((count + 1, Some(cluster)));
            };
            cluster = next;
        }
        Err(Error::BadClusterChain)
    }

    fn chain_split(self, start: u32, keep: usize) -> Result<(u32, Option<u32>), Error> {
        if keep == 0 || !self.valid_cluster(start) || keep > self.cluster_count as usize {
            return Err(Error::BadClusterChain);
        }
        let mut cluster = start;
        for index in 1..=keep {
            if !self.valid_cluster(cluster) {
                return Err(Error::BadClusterChain);
            }
            let next = self.next_cluster(cluster)?;
            if index == keep {
                return Ok((cluster, next));
            }
            cluster = next.ok_or(Error::BadClusterChain)?;
        }
        Err(Error::BadClusterChain)
    }

    fn free_chain(self, start: u32) -> Result<(), Error> {
        if start == 0 {
            return Ok(());
        }
        let mut cluster = start;
        for _ in 0..self.cluster_count {
            if !self.valid_cluster(cluster) {
                return Err(Error::BadClusterChain);
            }
            let next = self.next_cluster(cluster)?;
            self.write_fat_entry(cluster, 0)?;
            let Some(next) = next else {
                return Ok(());
            };
            cluster = next;
        }
        Err(Error::BadClusterChain)
    }

    fn allocate_chain(self, count: usize) -> Result<(u32, u32), Error> {
        if count == 0 || count > self.cluster_count as usize {
            return Err(Error::NoSpace);
        }
        let mut first = 0;
        let mut previous = 0;
        let mut allocated = 0;
        for offset in 0..self.cluster_count {
            let cluster = offset + 2;
            if self.read_fat_entry(cluster)? != 0 {
                continue;
            }
            if let Err(error) = self.write_fat_entry(cluster, EOC_MIN) {
                if first != 0 {
                    let _ = self.free_chain(first);
                }
                return Err(error);
            }
            if first == 0 {
                first = cluster;
            } else if let Err(error) = self.write_fat_entry(previous, cluster) {
                let _ = self.free_chain(first);
                let _ = self.write_fat_entry(cluster, 0);
                return Err(error);
            }
            previous = cluster;
            allocated += 1;
            if allocated == count {
                return Ok((first, previous));
            }
        }
        if first != 0 {
            let _ = self.free_chain(first);
        }
        Err(Error::NoSpace)
    }

    fn rollback_extension(self, old_last: Option<u32>, new_first: u32) {
        if let Some(last) = old_last {
            let _ = self.write_fat_entry(last, EOC_MIN);
        }
        let _ = self.free_chain(new_first);
    }

    fn write_chain(self, start: u32, input: &[u8]) -> Result<(), Error> {
        let Some(_writer) = self.writer else {
            return Err(Error::ReadOnly);
        };
        let mut cluster = start;
        let mut copied = 0;
        for _ in 0..self.cluster_count {
            if cluster < 2 || cluster > self.cluster_count + 1 {
                return Err(Error::BadClusterChain);
            }
            for sector in 0..self.sectors_per_cluster as u64 {
                if copied == input.len() {
                    return Ok(());
                }
                let length = (input.len() - copied).min(BYTES_PER_SECTOR);
                let mut data = [0u8; BYTES_PER_SECTOR];
                data[..length].copy_from_slice(&input[copied..copied + length]);
                if !self.write_sector(self.cluster_sector(cluster) + sector, &data) {
                    return Err(Error::Io);
                }
                copied += length;
            }
            let Some(next) = self.next_cluster(cluster)? else {
                return (copied == input.len())
                    .then_some(())
                    .ok_or(Error::BadClusterChain);
            };
            cluster = next;
        }
        Err(Error::BadClusterChain)
    }

    fn next_cluster(self, cluster: u32) -> Result<Option<u32>, Error> {
        let value = self.read_fat_entry(cluster)?;
        if value >= EOC_MIN {
            return Ok(None);
        }
        if value == 0 || value == BAD_CLUSTER || !self.valid_cluster(value) {
            return Err(Error::BadClusterChain);
        }
        Ok(Some(value))
    }

    fn fat_entry_location(self, fat_index: u8, cluster: u32) -> Result<(u64, usize), Error> {
        if fat_index >= self.fat_count || !self.valid_cluster(cluster) {
            return Err(Error::BadClusterChain);
        }
        let fat_offset = (cluster as u64)
            .checked_mul(4)
            .ok_or(Error::BadClusterChain)?;
        let sector_offset = fat_offset / BYTES_PER_SECTOR as u64;
        if sector_offset >= self.fat_size as u64 {
            return Err(Error::BadClusterChain);
        }
        let sector = (self.reserved_sectors as u64)
            .checked_add(
                (fat_index as u64)
                    .checked_mul(self.fat_size as u64)
                    .ok_or(Error::BadClusterChain)?,
            )
            .and_then(|value| value.checked_add(sector_offset))
            .ok_or(Error::BadClusterChain)?;
        Ok((sector, (fat_offset % BYTES_PER_SECTOR as u64) as usize))
    }

    fn read_fat_entry(self, cluster: u32) -> Result<u32, Error> {
        let mut data = [0u8; BYTES_PER_SECTOR];
        let (sector, offset) = self.fat_entry_location(0, cluster)?;
        if !self.read_sector(sector, &mut data) {
            return Err(Error::Io);
        }
        let expected = le_u32(&data, offset) & FAT_VALUE_MASK;
        for fat_index in 1..self.fat_count {
            let (sector, offset) = self.fat_entry_location(fat_index, cluster)?;
            if !self.read_sector(sector, &mut data) {
                return Err(Error::Io);
            }
            if le_u32(&data, offset) & FAT_VALUE_MASK != expected {
                return Err(Error::FatMismatch);
            }
        }
        Ok(expected)
    }

    fn write_fat_entry(self, cluster: u32, value: u32) -> Result<(), Error> {
        let Some(_writer) = self.writer else {
            return Err(Error::ReadOnly);
        };
        if value != 0
            && value < EOC_MIN
            && (value == 1 || value == BAD_CLUSTER || !self.valid_cluster(value))
        {
            return Err(Error::BadClusterChain);
        }
        let mut old_high = [0u32; 256];
        let mut old_value = None;
        let mut data = [0u8; BYTES_PER_SECTOR];
        for fat_index in 0..self.fat_count {
            let (sector, offset) = self.fat_entry_location(fat_index, cluster)?;
            if !self.read_sector(sector, &mut data) {
                return Err(Error::Io);
            }
            let raw = le_u32(&data, offset);
            old_high[fat_index as usize] = raw & FAT_HIGH_BITS;
            let current = raw & FAT_VALUE_MASK;
            if let Some(expected) = old_value {
                if current != expected {
                    return Err(Error::FatMismatch);
                }
            } else {
                old_value = Some(current);
            }
        }
        let old_value = old_value.ok_or(Error::BadClusterChain)?;
        if old_value == value {
            return Ok(());
        }

        for fat_index in 0..self.fat_count {
            let (sector, offset) = self.fat_entry_location(fat_index, cluster)?;
            if !self.read_sector(sector, &mut data) {
                let _ = self.restore_fat_entry(cluster, old_value, &old_high, fat_index);
                return Err(Error::Io);
            }
            let raw = (le_u32(&data, offset) & FAT_HIGH_BITS) | value;
            data[offset..offset + 4].copy_from_slice(&raw.to_le_bytes());
            if !self.write_sector(sector, &data) {
                let _ = self.restore_fat_entry(cluster, old_value, &old_high, fat_index + 1);
                return Err(Error::Io);
            }
            let mut verify = [0u8; BYTES_PER_SECTOR];
            if !self.read_sector(sector, &mut verify)
                || le_u32(&verify, offset) & FAT_VALUE_MASK != value
            {
                let _ = self.restore_fat_entry(cluster, old_value, &old_high, fat_index + 1);
                return Err(Error::Io);
            }
        }
        Ok(())
    }

    fn restore_fat_entry(
        self,
        cluster: u32,
        value: u32,
        old_high: &[u32; 256],
        copies: u8,
    ) -> bool {
        let mut success = true;
        let mut data = [0u8; BYTES_PER_SECTOR];
        for fat_index in 0..copies.min(self.fat_count) {
            let Ok((sector, offset)) = self.fat_entry_location(fat_index, cluster) else {
                success = false;
                continue;
            };
            if !self.read_sector(sector, &mut data) {
                success = false;
                continue;
            }
            let raw = old_high[fat_index as usize] | value;
            data[offset..offset + 4].copy_from_slice(&raw.to_le_bytes());
            if !self.write_sector(sector, &data) {
                success = false;
            }
        }
        success
    }

    fn cluster_sector(self, cluster: u32) -> u64 {
        self.first_data_sector + (cluster as u64 - 2) * self.sectors_per_cluster as u64
    }

    fn read_sector(self, lba: u64, output: &mut [u8; BYTES_PER_SECTOR]) -> bool {
        self.base_lba
            .checked_add(lba)
            .is_some_and(|absolute| (self.reader)(absolute, output))
    }

    fn write_sector(self, lba: u64, input: &[u8; BYTES_PER_SECTOR]) -> bool {
        self.writer
            .and_then(|writer| {
                self.base_lba
                    .checked_add(lba)
                    .map(|absolute| writer(absolute, input))
            })
            .unwrap_or(false)
    }
}

pub fn probe_block() -> Result<Mount, Error> {
    let partition = block::partition(0).ok_or(Error::InvalidBpb)?;
    Mount::open_at(
        ramdisk_read_sector,
        None,
        partition.start_lba,
        partition.sectors,
    )
}

pub fn probe_block_rw() -> Result<Mount, Error> {
    let partition = block::partition(0).ok_or(Error::InvalidBpb)?;
    Mount::open_at(
        ramdisk_read_sector,
        Some(ramdisk_write_sector),
        partition.start_lba,
        partition.sectors,
    )
}

pub const PERSISTENCE_FILE: &str = "/NORX.PST";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PersistenceStatus {
    pub previous_sequence: u64,
    pub sequence: u64,
}

pub fn fixed_file_persistence_check() -> Result<PersistenceStatus, Error> {
    if !block::persistent() || block::read_only() {
        return Err(Error::ReadOnly);
    }
    let volume = probe_block_rw()?;
    let mut record = [0u8; BYTES_PER_SECTOR];
    let length = volume.read_file(PERSISTENCE_FILE, &mut record)?;
    if length != record.len() {
        return Err(Error::InvalidPersistenceRecord);
    }
    let previous_sequence = u64::from_le_bytes(record[..8].try_into().unwrap());
    let sequence = previous_sequence
        .checked_add(1)
        .ok_or(Error::InvalidPersistenceRecord)?;
    record[..8].copy_from_slice(&sequence.to_le_bytes());
    volume.write_file(PERSISTENCE_FILE, &record)?;
    block::flush_cache().map_err(|_| Error::Io)?;
    let mut verify = [0u8; BYTES_PER_SECTOR];
    if volume.read_file(PERSISTENCE_FILE, &mut verify)? != record.len()
        || verify[..8] != record[..8]
    {
        return Err(Error::Io);
    }
    Ok(PersistenceStatus {
        previous_sequence,
        sequence,
    })
}

pub fn contract_self_check() {
    assert!(fixture_check());
}

fn ramdisk_read_sector(lba: u64, output: &mut [u8; BYTES_PER_SECTOR]) -> bool {
    usize::try_from(lba)
        .ok()
        .is_some_and(|lba| block::read_sector(lba, output))
}

fn ramdisk_write_sector(lba: u64, input: &[u8; BYTES_PER_SECTOR]) -> bool {
    usize::try_from(lba)
        .ok()
        .is_some_and(|lba| block::write_sector(lba, input))
}

static mut FIXTURE_DIRECTORY: [u8; BYTES_PER_SECTOR] = [0; BYTES_PER_SECTOR];
static mut FIXTURE_DATA: [u8; BYTES_PER_SECTOR] = [0; BYTES_PER_SECTOR];
static mut FIXTURE_DATA_TAIL: [u8; BYTES_PER_SECTOR] = [0; BYTES_PER_SECTOR];
static mut FIXTURE_DATA_GROWN: [u8; BYTES_PER_SECTOR] = [0; BYTES_PER_SECTOR];
static mut FIXTURE_FAT_PRIMARY: [u8; BYTES_PER_SECTOR] = [0; BYTES_PER_SECTOR];
static mut FIXTURE_FAT_SECONDARY: [u8; BYTES_PER_SECTOR] = [0; BYTES_PER_SECTOR];
static mut FIXTURE_DIRECTORY_VALID: bool = false;
static mut FIXTURE_DATA_VALID: bool = false;
static mut FIXTURE_DATA_TAIL_VALID: bool = false;
static mut FIXTURE_DATA_GROWN_VALID: bool = false;
static mut FIXTURE_FAT_PRIMARY_VALID: bool = false;
static mut FIXTURE_FAT_SECONDARY_VALID: bool = false;

const FIXTURE_PARTITION_BASE: u64 = 100;

fn fixture_check() -> bool {
    unsafe {
        FIXTURE_DIRECTORY_VALID = false;
        FIXTURE_DATA_VALID = false;
        FIXTURE_DATA_TAIL_VALID = false;
        FIXTURE_DATA_GROWN_VALID = false;
        FIXTURE_FAT_PRIMARY_VALID = false;
        FIXTURE_FAT_SECONDARY_VALID = false;
    }
    let Ok(read_only_volume) = Mount::open(fixture_read_sector) else {
        return false;
    };
    if read_only_volume.stat("relative") != Err(Error::InvalidPath) {
        return false;
    }
    let mut oversized_path = [b'a'; MAX_PATH_BYTES + 1];
    oversized_path[0] = b'/';
    let Ok(oversized_path) = core::str::from_utf8(&oversized_path) else {
        return false;
    };
    if read_only_volume.stat(oversized_path) != Err(Error::InvalidPath) {
        return false;
    }
    if !read_only_volume
        .write_file("/Long Name.txt", b"rejected")
        .is_err_and(|error| error == Error::ReadOnly)
    {
        return false;
    }
    let Ok(root) = read_only_volume.stat("/") else {
        return false;
    };
    if !root.directory || root.cluster != 2 || root.size != 0 {
        return false;
    }
    let Ok(long_info) = read_only_volume.stat("/Long Name.txt") else {
        return false;
    };
    if long_info.directory || long_info.cluster != 3 || long_info.size != 14 {
        return false;
    }
    let Ok(short_info) = read_only_volume.stat("/LONGNA~1.TXT") else {
        return false;
    };
    if short_info != long_info {
        return false;
    }
    let Ok(subdir_info) = read_only_volume.stat("/SUBDIR") else {
        return false;
    };
    if !subdir_info.directory || subdir_info.cluster != 4 || subdir_info.size != 0 {
        return false;
    }
    let mut entries = [DirectoryEntry::EMPTY; 2];
    let Ok(entry_count) = read_only_volume.read_dir("/", &mut entries) else {
        return false;
    };
    if entry_count != 2
        || entries[0].name_bytes() != b"Long Name.txt"
        || entries[0].directory
        || entries[0].cluster != 3
        || entries[0].size != 14
        || entries[1].name_bytes() != b"SUBDIR"
        || !entries[1].directory
        || entries[1].cluster != 4
    {
        return false;
    }
    let mut bounded_entries = [DirectoryEntry::EMPTY; 1];
    if read_only_volume.read_dir("/", &mut bounded_entries) != Ok(1) {
        return false;
    }
    let mut empty_entries: [DirectoryEntry; 0] = [];
    if read_only_volume.read_dir("/SUBDIR", &mut empty_entries) != Ok(0) {
        return false;
    }
    let Ok(volume) = Mount::open_rw(fixture_read_sector, fixture_write_sector) else {
        return false;
    };
    if volume.geometry().read_only {
        return false;
    }
    let mut output = [0u8; 32];
    let Ok(length) = volume.read_file("/Long Name.txt", &mut output) else {
        return false;
    };
    if length != 14 || &output[..length] != b"FAT32 fixture\n" {
        return false;
    }
    if volume
        .write_file("/Long Name.txt", b"FAT32 update\n")
        .is_err()
    {
        return false;
    }
    let mut updated = [0u8; 32];
    let Ok(length) = volume.read_file("/Long Name.txt", &mut updated) else {
        return false;
    };
    if length != 13 || &updated[..length] != b"FAT32 update\n" {
        return false;
    }
    let Ok(partitioned) = Mount::open_at(
        fixture_partition_read_sector,
        Some(fixture_partition_write_sector),
        FIXTURE_PARTITION_BASE,
        131_072,
    ) else {
        return false;
    };
    if partitioned.geometry().read_only {
        return false;
    }
    let mut partitioned_output = [0u8; 32];
    if partitioned
        .read_file("/Long Name.txt", &mut partitioned_output)
        .is_err()
    {
        return false;
    }
    let grown = [0xa5u8; BYTES_PER_SECTOR + 37];
    if volume.write_file("/Long Name.txt", &grown).is_err() {
        return false;
    }
    let mut grown_output = [0u8; BYTES_PER_SECTOR + 37];
    let Ok(length) = volume.read_file("/Long Name.txt", &mut grown_output) else {
        return false;
    };
    if length != grown.len() || grown_output != grown {
        return false;
    }
    if fixture_fat_entry(1, 3) != Some(5)
        || fixture_fat_entry(1, 4) != Some(EOC_MIN)
        || fixture_fat_entry(1, 5) != Some(EOC_MIN)
        || fixture_fat_entry(1025, 3) != Some(5)
        || fixture_fat_entry(1025, 4) != Some(EOC_MIN)
        || fixture_fat_entry(1025, 5) != Some(EOC_MIN)
    {
        return false;
    }

    if volume.write_file("/Long Name.txt", b"shrunk").is_err() {
        return false;
    }
    let mut shrunk_output = [0u8; 16];
    let Ok(length) = volume.read_file("/Long Name.txt", &mut shrunk_output) else {
        return false;
    };
    if length != 6 || &shrunk_output[..length] != b"shrunk" {
        return false;
    }
    if fixture_fat_entry(1, 5) != Some(0) || fixture_fat_entry(1025, 5) != Some(0) {
        return false;
    }

    if volume.write_file("/Long Name.txt", &[]).is_err() {
        return false;
    }
    let mut empty_output = [0u8; 1];
    let Ok(length) = volume.read_file("/Long Name.txt", &mut empty_output) else {
        return false;
    };
    if length != 0 || fixture_fat_entry(1, 3) != Some(0) || fixture_fat_entry(1025, 3) != Some(0) {
        return false;
    }
    if volume.write_file("/Long Name.txt", b"again").is_err() {
        return false;
    }

    unsafe {
        write_u32(&mut *core::ptr::addr_of_mut!(FIXTURE_FAT_SECONDARY), 12, 0);
    }
    volume
        .read_file("/Long Name.txt", &mut output)
        .is_err_and(|error| error == Error::FatMismatch)
}

fn fixture_read_sector(lba: u64, output: &mut [u8; BYTES_PER_SECTOR]) -> bool {
    output.fill(0);
    match lba {
        0 => {
            output[11..13].copy_from_slice(&512u16.to_le_bytes());
            output[13] = 1;
            output[14..16].copy_from_slice(&1u16.to_le_bytes());
            output[16] = 2;
            output[17..19].copy_from_slice(&0u16.to_le_bytes());
            output[19..21].copy_from_slice(&0u16.to_le_bytes());
            output[21] = 0xf8;
            output[22..24].copy_from_slice(&0u16.to_le_bytes());
            output[32..36].copy_from_slice(&131_072u32.to_le_bytes());
            output[36..40].copy_from_slice(&1024u32.to_le_bytes());
            output[44..48].copy_from_slice(&2u32.to_le_bytes());
            output[510..512].copy_from_slice(&[0x55, 0xaa]);
        }
        1 => unsafe {
            if FIXTURE_FAT_PRIMARY_VALID {
                output.copy_from_slice(&*core::ptr::addr_of!(FIXTURE_FAT_PRIMARY));
            } else {
                fixture_fat(output);
            }
        },
        1025 => unsafe {
            if FIXTURE_FAT_SECONDARY_VALID {
                output.copy_from_slice(&*core::ptr::addr_of!(FIXTURE_FAT_SECONDARY));
            } else {
                fixture_fat(output);
            }
        },
        2049 => unsafe {
            if FIXTURE_DIRECTORY_VALID {
                output.copy_from_slice(&*core::ptr::addr_of!(FIXTURE_DIRECTORY));
            } else {
                fixture_directory(output);
            }
        },
        2050 => unsafe {
            if FIXTURE_DATA_VALID {
                output.copy_from_slice(&*core::ptr::addr_of!(FIXTURE_DATA));
            } else {
                output[..14].copy_from_slice(b"FAT32 fixture\n");
            }
        },
        2051 => unsafe {
            if FIXTURE_DATA_TAIL_VALID {
                output.copy_from_slice(&*core::ptr::addr_of!(FIXTURE_DATA_TAIL));
            }
        },
        2052 => unsafe {
            if FIXTURE_DATA_GROWN_VALID {
                output.copy_from_slice(&*core::ptr::addr_of!(FIXTURE_DATA_GROWN));
            }
        },
        _ => {}
    }
    true
}

fn fixture_write_sector(lba: u64, input: &[u8; BYTES_PER_SECTOR]) -> bool {
    match lba {
        1 => unsafe {
            (&mut *core::ptr::addr_of_mut!(FIXTURE_FAT_PRIMARY)).copy_from_slice(input);
            FIXTURE_FAT_PRIMARY_VALID = true;
            true
        },
        1025 => unsafe {
            (&mut *core::ptr::addr_of_mut!(FIXTURE_FAT_SECONDARY)).copy_from_slice(input);
            FIXTURE_FAT_SECONDARY_VALID = true;
            true
        },
        2049 => unsafe {
            (&mut *core::ptr::addr_of_mut!(FIXTURE_DIRECTORY)).copy_from_slice(input);
            FIXTURE_DIRECTORY_VALID = true;
            true
        },
        2050 => unsafe {
            (&mut *core::ptr::addr_of_mut!(FIXTURE_DATA)).copy_from_slice(input);
            FIXTURE_DATA_VALID = true;
            true
        },
        2051 => unsafe {
            (&mut *core::ptr::addr_of_mut!(FIXTURE_DATA_TAIL)).copy_from_slice(input);
            FIXTURE_DATA_TAIL_VALID = true;
            true
        },
        2052 => unsafe {
            (&mut *core::ptr::addr_of_mut!(FIXTURE_DATA_GROWN)).copy_from_slice(input);
            FIXTURE_DATA_GROWN_VALID = true;
            true
        },
        _ => false,
    }
}

fn fixture_partition_read_sector(lba: u64, output: &mut [u8; BYTES_PER_SECTOR]) -> bool {
    lba.checked_sub(FIXTURE_PARTITION_BASE)
        .is_some_and(|relative| fixture_read_sector(relative, output))
}

fn fixture_partition_write_sector(lba: u64, input: &[u8; BYTES_PER_SECTOR]) -> bool {
    lba.checked_sub(FIXTURE_PARTITION_BASE)
        .is_some_and(|relative| fixture_write_sector(relative, input))
}

fn fixture_fat(output: &mut [u8; BYTES_PER_SECTOR]) {
    write_u32(output, 8, EOC_MIN);
    write_u32(output, 12, EOC_MIN);
    write_u32(output, 16, EOC_MIN);
}

fn fixture_fat_entry(lba: u64, cluster: u32) -> Option<u32> {
    let mut data = [0u8; BYTES_PER_SECTOR];
    fixture_read_sector(lba, &mut data).then(|| {
        let offset = cluster as usize * 4;
        le_u32(&data, offset) & FAT_VALUE_MASK
    })
}

fn fixture_directory(output: &mut [u8; BYTES_PER_SECTOR]) {
    let short = *b"LONGNA~1TXT";
    output[0] = 0x41;
    output[11] = 0x0f;
    output[13] = short_checksum(&short);
    for (index, character) in "Long Name.txt".encode_utf16().enumerate() {
        let offset = if index < 5 {
            1 + index * 2
        } else if index < 11 {
            14 + (index - 5) * 2
        } else {
            28 + (index - 11) * 2
        };
        output[offset..offset + 2].copy_from_slice(&character.to_le_bytes());
    }
    output[32..43].copy_from_slice(&short);
    output[32 + 11] = 0x20;
    output[32 + 20..32 + 22].copy_from_slice(&0u16.to_le_bytes());
    output[32 + 26..32 + 28].copy_from_slice(&3u16.to_le_bytes());
    output[32 + 28..32 + 32].copy_from_slice(&14u32.to_le_bytes());
    output[64..75].copy_from_slice(b"SUBDIR     ");
    output[64 + 11] = 0x10;
    output[64 + 20..64 + 22].copy_from_slice(&0u16.to_le_bytes());
    output[64 + 26..64 + 28].copy_from_slice(&4u16.to_le_bytes());
}

fn components(path: &str) -> Result<([&str; MAX_COMPONENTS], usize), Error> {
    let bytes = path.as_bytes();
    if !bytes.starts_with(b"/") || bytes.len() > MAX_PATH_BYTES {
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
            if count == MAX_COMPONENTS || index - start > MAX_COMPONENT_BYTES {
                return Err(Error::InvalidPath);
            }
            result[count] = &path[start..index];
            count += 1;
        }
        start = index + 1;
    }
    if start < bytes.len() {
        if count == MAX_COMPONENTS || bytes.len() - start > MAX_COMPONENT_BYTES {
            return Err(Error::InvalidPath);
        }
        result[count] = &path[start..];
        count += 1;
    }
    if count == 0 {
        return Err(Error::InvalidPath);
    }
    Ok((result, count))
}

fn components_allow_root(path: &str) -> Result<([&str; MAX_COMPONENTS], usize), Error> {
    if path == "/" {
        return Ok(([""; MAX_COMPONENTS], 0));
    }
    components(path)
}

fn encode_short_name(entry: &[u8; 32], output: &mut [u8]) -> Option<usize> {
    let mut length = 0;
    for byte in entry[..8].iter().copied() {
        if byte != b' ' {
            *output.get_mut(length)? = byte;
            length += 1;
        }
    }
    if entry[8..11].iter().any(|byte| *byte != b' ') {
        *output.get_mut(length)? = b'.';
        length += 1;
        for byte in entry[8..11].iter().copied() {
            if byte != b' ' {
                *output.get_mut(length)? = byte;
                length += 1;
            }
        }
    }
    (length != 0).then_some(length)
}

fn short_name_matches(entry: &[u8; 32], target: &str) -> bool {
    let mut short = [0u8; 12];
    encode_short_name(entry, &mut short)
        .is_some_and(|length| ascii_eq_ignore_case(&short[..length], target.as_bytes()))
}

fn ascii_eq_ignore_case(left: &[u8], right: &[u8]) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn short_checksum(name: &[u8]) -> u8 {
    name.iter().fold(0, |checksum, byte| {
        (checksum >> 1)
            .wrapping_add((checksum & 1) << 7)
            .wrapping_add(*byte)
    })
}

fn encode_utf8(character: u32, output: &mut [u8], offset: usize) -> Option<usize> {
    if character > 0x10ffff || (0xd800..=0xdfff).contains(&character) {
        return None;
    }
    if character <= 0x7f {
        *output.get_mut(offset)? = character as u8;
        Some(offset + 1)
    } else if character <= 0x7ff {
        output.get_mut(offset..offset + 2)?.copy_from_slice(&[
            0xc0 | (character >> 6) as u8,
            0x80 | (character & 0x3f) as u8,
        ]);
        Some(offset + 2)
    } else if character <= 0xffff {
        output.get_mut(offset..offset + 3)?.copy_from_slice(&[
            0xe0 | (character >> 12) as u8,
            0x80 | ((character >> 6) & 0x3f) as u8,
            0x80 | (character & 0x3f) as u8,
        ]);
        Some(offset + 3)
    } else {
        output.get_mut(offset..offset + 4)?.copy_from_slice(&[
            0xf0 | (character >> 18) as u8,
            0x80 | ((character >> 12) & 0x3f) as u8,
            0x80 | ((character >> 6) & 0x3f) as u8,
            0x80 | (character & 0x3f) as u8,
        ]);
        Some(offset + 4)
    }
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

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
