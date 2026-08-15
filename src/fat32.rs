use crate::drivers::block;

const BYTES_PER_SECTOR: usize = 512;
const MAX_LFN_CHARS: usize = 260;
const MAX_COMPONENTS: usize = 16;
const EOC_MIN: u32 = 0x0fff_fff8;
const BAD_CLUSTER: u32 = 0x0fff_fff7;

pub type ReadSector = fn(u64, &mut [u8; BYTES_PER_SECTOR]) -> bool;
pub type WriteSector = fn(u64, &[u8; BYTES_PER_SECTOR]) -> bool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Io,
    InvalidBpb,
    InvalidPath,
    NotFound,
    NotDirectory,
    IsDirectory,
    BadClusterChain,
    BufferTooSmall,
    NoSpace,
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
    reserved_sectors: u16,
    fat_size: u32,
    total_sectors: u64,
    first_data_sector: u64,
    sectors_per_cluster: u8,
    cluster_count: u32,
    root_cluster: u32,
}

#[derive(Clone, Copy)]
struct DirectoryEntry {
    directory: bool,
    cluster: u32,
    size: u32,
}

#[derive(Clone, Copy)]
struct EntryLocation {
    entry: DirectoryEntry,
    sector: u64,
    offset: usize,
}

#[derive(Clone, Copy)]
struct LongName {
    chars: [u16; MAX_LFN_CHARS],
    checksum: u8,
    max_sequence: u8,
    seen: u32,
    valid: bool,
}

impl LongName {
    const EMPTY: Self = Self {
        chars: [0; MAX_LFN_CHARS],
        checksum: 0,
        max_sequence: 0,
        seen: 0,
        valid: false,
    };

    fn reset(&mut self) {
        *self = Self::EMPTY;
    }

    fn accept(&mut self, entry: &[u8; 32]) {
        let sequence = entry[0] & 0x1f;
        if sequence == 0 || sequence as usize > MAX_LFN_CHARS / 13 {
            self.reset();
            return;
        }
        if entry[0] & 0x40 != 0 {
            self.max_sequence = sequence;
            self.checksum = entry[13];
            self.seen = 0;
            self.valid = true;
        }
        if !self.valid || self.checksum != entry[13] {
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
    }

    fn matches(&self, target: &str, short_checksum: u8) -> bool {
        if !self.valid
            || self.max_sequence == 0
            || self.checksum != short_checksum
            || self.seen & ((1u32 << self.max_sequence) - 1) != (1u32 << self.max_sequence) - 1
        {
            return false;
        }
        let mut encoded = [0u8; MAX_LFN_CHARS * 3];
        let mut length = 0;
        for character in self.chars.iter().take(self.max_sequence as usize * 13) {
            if *character == 0 || *character == 0xffff {
                break;
            }
            let Some(next) = encode_utf8(*character, &mut encoded, length) else {
                return false;
            };
            length = next;
        }
        encoded[..length] == target.as_bytes()[..]
    }
}

impl Mount {
    pub fn open(reader: ReadSector) -> Result<Self, Error> {
        Self::open_with_writer(reader, None)
    }

    pub fn open_rw(reader: ReadSector, writer: WriteSector) -> Result<Self, Error> {
        Self::open_with_writer(reader, Some(writer))
    }

    fn open_with_writer(reader: ReadSector, writer: Option<WriteSector>) -> Result<Self, Error> {
        let mut bpb = [0u8; BYTES_PER_SECTOR];
        if !reader(0, &mut bpb) {
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
        if total_sectors <= first_data_sector {
            return Err(Error::InvalidBpb);
        }
        let cluster_count = ((total_sectors - first_data_sector) / sectors_per_cluster as u64)
            .try_into()
            .map_err(|_| Error::InvalidBpb)?;
        if cluster_count < 65_525
            || (cluster_count as u64 + 2) * 4 > fat_size as u64 * BYTES_PER_SECTOR as u64
            || root_cluster > cluster_count + 1
        {
            return Err(Error::InvalidBpb);
        }
        Ok(Self {
            reader,
            writer,
            reserved_sectors,
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

    pub fn write_file(self, path: &str, input: &[u8]) -> Result<usize, Error> {
        let Some(writer) = self.writer else {
            return Err(Error::ReadOnly);
        };
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
        // ponytail: reuse the existing chain; add allocation/free-list work with file creation.
        let capacity = self.chain_capacity(location.entry.cluster)?;
        if input.len() > capacity {
            return Err(Error::NoSpace);
        }
        self.write_chain(location.entry.cluster, input)?;
        let mut data = [0u8; BYTES_PER_SECTOR];
        if !(self.reader)(location.sector, &mut data) {
            return Err(Error::Io);
        }
        data[location.offset + 28..location.offset + 32]
            .copy_from_slice(&(input.len() as u32).to_le_bytes());
        if !writer(location.sector, &data) {
            return Err(Error::Io);
        }
        Ok(input.len())
    }

    fn find_entry(self, directory_cluster: u32, target: &str) -> Result<DirectoryEntry, Error> {
        Ok(self.find_entry_location(directory_cluster, target)?.entry)
    }

    fn find_entry_location(
        self,
        directory_cluster: u32,
        target: &str,
    ) -> Result<EntryLocation, Error> {
        let mut cluster = directory_cluster;
        let mut long_name = LongName::EMPTY;
        for _ in 0..self.cluster_count {
            for sector in 0..self.sectors_per_cluster as u64 {
                let mut data = [0u8; BYTES_PER_SECTOR];
                if !(self.reader)(self.cluster_sector(cluster) + sector, &mut data) {
                    return Err(Error::Io);
                }
                for entry_offset in (0..BYTES_PER_SECTOR).step_by(32) {
                    let mut entry = [0u8; 32];
                    entry.copy_from_slice(&data[entry_offset..entry_offset + 32]);
                    if entry[0] == 0 {
                        return Err(Error::NotFound);
                    }
                    if entry[0] == 0xe5 {
                        long_name.reset();
                        continue;
                    }
                    if entry[11] == 0x0f {
                        long_name.accept(&entry);
                        continue;
                    }
                    let checksum = short_checksum(&entry[..11]);
                    let matches =
                        long_name.matches(target, checksum) || short_name_matches(&entry, target);
                    let result = if matches {
                        Some(EntryLocation {
                            entry: DirectoryEntry {
                                directory: entry[11] & 0x10 != 0,
                                cluster: (le_u16(&entry, 20) as u32) << 16
                                    | le_u16(&entry, 26) as u32,
                                size: le_u32(&entry, 28),
                            },
                            sector: self.cluster_sector(cluster) + sector,
                            offset: entry_offset,
                        })
                    } else {
                        None
                    };
                    long_name.reset();
                    if let Some(result) = result {
                        return Ok(result);
                    }
                }
            }
            let Some(next) = self.next_cluster(cluster)? else {
                return Err(Error::NotFound);
            };
            cluster = next;
        }
        Err(Error::BadClusterChain)
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
                if !(self.reader)(self.cluster_sector(cluster) + sector, &mut data) {
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

    fn chain_capacity(self, start: u32) -> Result<usize, Error> {
        let mut cluster = start;
        let cluster_bytes = self.sectors_per_cluster as usize * BYTES_PER_SECTOR;
        let mut capacity = 0usize;
        for _ in 0..self.cluster_count {
            if cluster < 2 || cluster > self.cluster_count + 1 {
                return Err(Error::BadClusterChain);
            }
            capacity = capacity
                .checked_add(cluster_bytes)
                .ok_or(Error::BadClusterChain)?;
            let Some(next) = self.next_cluster(cluster)? else {
                return Ok(capacity);
            };
            cluster = next;
        }
        Err(Error::BadClusterChain)
    }

    fn write_chain(self, start: u32, input: &[u8]) -> Result<(), Error> {
        let Some(writer) = self.writer else {
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
                if !writer(self.cluster_sector(cluster) + sector, &data) {
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
        let fat_offset = cluster as u64 * 4;
        let sector = self.reserved_sectors as u64 + fat_offset / BYTES_PER_SECTOR as u64;
        if sector >= self.reserved_sectors as u64 + self.fat_size as u64 {
            return Err(Error::BadClusterChain);
        }
        let mut data = [0u8; BYTES_PER_SECTOR];
        if !(self.reader)(sector, &mut data) {
            return Err(Error::Io);
        }
        let value = le_u32(&data, (fat_offset as usize) % BYTES_PER_SECTOR) & 0x0fff_ffff;
        if value >= EOC_MIN {
            return Ok(None);
        }
        if value == 0 || value == BAD_CLUSTER || value > self.cluster_count + 1 {
            return Err(Error::BadClusterChain);
        }
        Ok(Some(value))
    }

    fn cluster_sector(self, cluster: u32) -> u64 {
        self.first_data_sector + (cluster as u64 - 2) * self.sectors_per_cluster as u64
    }
}

pub fn probe_ramdisk() -> Result<Mount, Error> {
    Mount::open(ramdisk_read_sector)
}

pub fn probe_block_rw() -> Result<Mount, Error> {
    Mount::open_rw(ramdisk_read_sector, ramdisk_write_sector)
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
static mut FIXTURE_DIRECTORY_VALID: bool = false;
static mut FIXTURE_DATA_VALID: bool = false;

fn fixture_check() -> bool {
    unsafe {
        FIXTURE_DIRECTORY_VALID = false;
        FIXTURE_DATA_VALID = false;
    }
    let Ok(read_only_volume) = Mount::open(fixture_read_sector) else {
        return false;
    };
    if !read_only_volume
        .write_file("/Long Name.txt", b"rejected")
        .is_err_and(|error| error == Error::ReadOnly)
    {
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
    let too_large = [0u8; BYTES_PER_SECTOR + 1];
    volume
        .write_file("/Long Name.txt", &too_large)
        .is_err_and(|error| error == Error::NoSpace)
}

fn fixture_read_sector(lba: u64, output: &mut [u8; BYTES_PER_SECTOR]) -> bool {
    output.fill(0);
    match lba {
        0 => {
            output[11..13].copy_from_slice(&512u16.to_le_bytes());
            output[13] = 1;
            output[14..16].copy_from_slice(&1u16.to_le_bytes());
            output[16] = 1;
            output[17..19].copy_from_slice(&0u16.to_le_bytes());
            output[19..21].copy_from_slice(&0u16.to_le_bytes());
            output[21] = 0xf8;
            output[22..24].copy_from_slice(&0u16.to_le_bytes());
            output[32..36].copy_from_slice(&131_072u32.to_le_bytes());
            output[36..40].copy_from_slice(&1024u32.to_le_bytes());
            output[44..48].copy_from_slice(&2u32.to_le_bytes());
            output[510..512].copy_from_slice(&[0x55, 0xaa]);
        }
        1 => {
            write_u32(output, 8, 0x0fff_fff8);
            write_u32(output, 12, 0x0fff_ffff);
            write_u32(output, 16, 0x0fff_ffff);
            write_u32(output, 20, 0x0fff_ffff);
        }
        1025 => unsafe {
            if FIXTURE_DIRECTORY_VALID {
                output.copy_from_slice(&*core::ptr::addr_of!(FIXTURE_DIRECTORY));
            } else {
                fixture_directory(output);
            }
        },
        1026 => unsafe {
            if FIXTURE_DATA_VALID {
                output.copy_from_slice(&*core::ptr::addr_of!(FIXTURE_DATA));
            } else {
                output[..14].copy_from_slice(b"FAT32 fixture\n");
            }
        },
        _ => {}
    }
    true
}

fn fixture_write_sector(lba: u64, input: &[u8; BYTES_PER_SECTOR]) -> bool {
    match lba {
        1025 => unsafe {
            (&mut *core::ptr::addr_of_mut!(FIXTURE_DIRECTORY)).copy_from_slice(input);
            FIXTURE_DIRECTORY_VALID = true;
            true
        },
        1026 => unsafe {
            (&mut *core::ptr::addr_of_mut!(FIXTURE_DATA)).copy_from_slice(input);
            FIXTURE_DATA_VALID = true;
            true
        },
        _ => false,
    }
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
            if count == MAX_COMPONENTS {
                return Err(Error::InvalidPath);
            }
            result[count] = &path[start..index];
            count += 1;
        }
        start = index + 1;
    }
    if start < bytes.len() {
        if count == MAX_COMPONENTS {
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

fn short_name_matches(entry: &[u8; 32], target: &str) -> bool {
    let mut short = [b' '; 12];
    let mut length = 0;
    for byte in entry[..8].iter().copied() {
        if byte != b' ' {
            short[length] = byte;
            length += 1;
        }
    }
    if entry[8..11].iter().any(|byte| *byte != b' ') {
        short[length] = b'.';
        length += 1;
        for byte in entry[8..11].iter().copied() {
            if byte != b' ' {
                short[length] = byte;
                length += 1;
            }
        }
    }
    ascii_eq_ignore_case(&short[..length], target.as_bytes())
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

fn encode_utf8(character: u16, output: &mut [u8], offset: usize) -> Option<usize> {
    let character = character as u32;
    if character <= 0x7f {
        *output.get_mut(offset)? = character as u8;
        Some(offset + 1)
    } else if character <= 0x7ff {
        output.get_mut(offset..offset + 2)?.copy_from_slice(&[
            0xc0 | (character >> 6) as u8,
            0x80 | (character & 0x3f) as u8,
        ]);
        Some(offset + 2)
    } else {
        output.get_mut(offset..offset + 3)?.copy_from_slice(&[
            0xe0 | (character >> 12) as u8,
            0x80 | ((character >> 6) & 0x3f) as u8,
            0x80 | (character & 0x3f) as u8,
        ]);
        Some(offset + 3)
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
