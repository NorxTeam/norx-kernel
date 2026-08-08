use crate::elf::{LoadPlan, Machine};
use crate::user_runtime::NativeRuntime;
use crate::{address_space::AslrHook, elf, process::ProcessId};

const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const PT_DYNAMIC: u32 = 2;
const PT_TLS: u32 = 7;
const ET_DYN: u16 = 3;
const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_PLTRELSZ: u64 = 2;
const DT_STRTAB: u64 = 5;
const DT_SYMTAB: u64 = 6;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const DT_STRSZ: u64 = 10;
const DT_SYMENT: u64 = 11;
const DT_SONAME: u64 = 14;
const DT_REL: u64 = 17;
const DT_RELSZ: u64 = 18;
const DT_RELENT: u64 = 19;
const DT_JMPREL: u64 = 23;
const DT_PLTREL: u64 = 20;
const DT_HASH: u64 = 4;
const DT_GNU_HASH: u64 = 0x6fff_fef5;
const DT_RELA_FORMAT: u64 = 7;
const MAX_DYNAMIC_ENTRIES: usize = 128;
const MAX_NEEDED: usize = 8;
const MAX_RELOCATIONS: usize = 256;
const MAX_SYMBOLS: usize = 256;
const MAX_STRING_TABLE: usize = 16 * 1024;
const MAX_TLS_BYTES: usize = 64 * 1024;
const MAX_LIBRARY_NAME: usize = 128;
const MAX_GNU_HASH_BUCKETS: usize = 256;
const MAX_GNU_HASH_BLOOM: usize = 256;
const RELA_SIZE: usize = 24;
const SYMBOL_SIZE: usize = 24;

const R_X86_64_64: u32 = 1;
const R_X86_64_GLOB_DAT: u32 = 6;
const R_X86_64_JUMP_SLOT: u32 = 7;
const R_X86_64_RELATIVE: u32 = 8;
const R_AARCH64_ABS64: u32 = 257;
const R_AARCH64_GLOB_DAT: u32 = 1025;
const R_AARCH64_JUMP_SLOT: u32 = 1026;
const R_AARCH64_RELATIVE: u32 = 1027;
const SMOKE_IMAGE_SIZE: usize = 4096 + 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Truncated,
    WrongType,
    WrongMachine,
    ProgramHeaderOverflow,
    MissingDynamicTerminator,
    DuplicateDynamicSegment,
    InvalidTable,
    TableTooLarge,
    MissingStringTable,
    MissingSymbolHash,
    InvalidString,
    InvalidSymbolIndex,
    InvalidRelocationTarget,
    UnsupportedRelocationFormat,
    UnsupportedRelocation,
    SymbolRequired,
    InvalidRelocation,
    InvalidTls,
    TooManyNeeded,
    InvalidLibraryName,
    MissingDependency,
}

pub const LIBRARY_SEARCH_ROOTS: [&[u8]; 2] = [b"/lib", b"/lib64"];

pub fn validate_library_name(name: &[u8]) -> Result<(), Error> {
    if name.is_empty()
        || name.len() > MAX_LIBRARY_NAME
        || name.contains(&0)
        || name.contains(&b'/')
        || name == b"."
        || name == b".."
    {
        return Err(Error::InvalidLibraryName);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Table {
    pub virtual_address: usize,
    pub file_offset: usize,
    pub size: usize,
    pub entry_size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StringRef {
    offset: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name_offset: u32,
    pub value: usize,
    pub size: usize,
    pub info: u8,
    pub defined: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relocation {
    pub address: usize,
    pub symbol: u32,
    pub kind: u32,
    pub addend: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TlsPlan {
    pub virtual_address: usize,
    pub file_offset: usize,
    pub file_size: usize,
    pub memory_size: usize,
    pub alignment: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynamicPlan {
    machine: Machine,
    pub base: usize,
    pub dynamic: Option<Table>,
    pub strings: Option<Table>,
    pub symbols: Option<Table>,
    pub symbol_count: usize,
    pub needed_count: usize,
    needed: [StringRef; MAX_NEEDED],
    pub soname_offset: Option<usize>,
    pub relocation_count: usize,
    relocations: [Relocation; MAX_RELOCATIONS],
    pub tls: Option<TlsPlan>,
}

impl DynamicPlan {
    pub fn needed<'a>(&self, image: &'a [u8], index: usize) -> Result<Option<&'a [u8]>, Error> {
        if index >= self.needed_count {
            return Ok(None);
        }
        let strings = self.strings.ok_or(Error::MissingStringTable)?;
        string_at(image, &strings, self.needed[index].offset).map(Some)
    }

    pub fn soname<'a>(&self, image: &'a [u8]) -> Result<Option<&'a [u8]>, Error> {
        let Some(offset) = self.soname_offset else {
            return Ok(None);
        };
        let strings = self.strings.ok_or(Error::MissingStringTable)?;
        string_at(image, &strings, offset).map(Some)
    }

    pub fn lookup_symbol(&self, image: &[u8], name: &[u8]) -> Result<Option<Symbol>, Error> {
        let Some(symbols) = self.symbols else {
            return Ok(None);
        };
        let strings = self.strings.ok_or(Error::MissingStringTable)?;
        for index in 0..self.symbol_count {
            let offset = symbols
                .file_offset
                .checked_add(index.checked_mul(SYMBOL_SIZE).ok_or(Error::InvalidTable)?)
                .ok_or(Error::InvalidTable)?;
            let entry = image
                .get(offset..offset + SYMBOL_SIZE)
                .ok_or(Error::Truncated)?;
            let name_offset = read_u32(entry, 0)?;
            if string_at(image, &strings, name_offset as usize)? != name {
                continue;
            }
            let value = usize::try_from(read_u64(entry, 8)?).map_err(|_| Error::InvalidTable)?;
            let size = usize::try_from(read_u64(entry, 16)?).map_err(|_| Error::InvalidTable)?;
            return Ok(Some(Symbol {
                name_offset,
                value,
                size,
                info: entry[4],
                defined: read_u16(entry, 6)? != 0,
            }));
        }
        Ok(None)
    }

    pub fn relocation(&self, index: usize) -> Option<Relocation> {
        (index < self.relocation_count).then_some(self.relocations[index])
    }

    pub fn relative_value(&self, index: usize) -> Result<usize, Error> {
        let relocation = self.relocation(index).ok_or(Error::InvalidRelocation)?;
        if relocation.kind != relative_kind(self.machine) {
            return Err(if relocation.kind_is_symbolic(self.machine) {
                Error::SymbolRequired
            } else {
                Error::UnsupportedRelocation
            });
        }
        add_signed(self.base, relocation.addend).ok_or(Error::InvalidRelocation)
    }

    pub fn relocation_value(
        &self,
        index: usize,
        symbol_value: Option<usize>,
    ) -> Result<usize, Error> {
        let relocation = self.relocation(index).ok_or(Error::InvalidRelocation)?;
        if relocation.kind == relative_kind(self.machine) {
            return add_signed(self.base, relocation.addend).ok_or(Error::InvalidRelocation);
        }
        if !relocation.kind_is_symbolic(self.machine) {
            return Err(Error::UnsupportedRelocation);
        }
        add_signed(
            symbol_value.ok_or(Error::SymbolRequired)?,
            relocation.addend,
        )
        .ok_or(Error::InvalidRelocation)
    }
}

impl Relocation {
    fn kind_is_symbolic(self, machine: Machine) -> bool {
        matches!(
            (machine, self.kind),
            (
                Machine::X86_64,
                R_X86_64_64 | R_X86_64_GLOB_DAT | R_X86_64_JUMP_SLOT
            ) | (
                Machine::Aarch64,
                R_AARCH64_ABS64 | R_AARCH64_GLOB_DAT | R_AARCH64_JUMP_SLOT
            )
        )
    }
}

#[derive(Clone, Copy)]
struct ProgramHeader {
    kind: u32,
    file_offset: u64,
    virtual_address: u64,
    file_size: u64,
    memory_size: u64,
    alignment: u64,
}

#[derive(Clone, Copy)]
struct Tags {
    string_address: Option<u64>,
    string_size: Option<u64>,
    symbol_address: Option<u64>,
    symbol_entry_size: Option<u64>,
    hash_address: Option<u64>,
    gnu_hash_address: Option<u64>,
    rela_address: Option<u64>,
    rela_size: Option<u64>,
    rela_entry_size: Option<u64>,
    plt_address: Option<u64>,
    plt_size: Option<u64>,
    plt_format: Option<u64>,
    soname_offset: Option<u64>,
    needed: [Option<u64>; MAX_NEEDED],
    needed_count: usize,
    has_rel: bool,
}

impl Tags {
    const fn new() -> Self {
        Self {
            string_address: None,
            string_size: None,
            symbol_address: None,
            symbol_entry_size: None,
            hash_address: None,
            gnu_hash_address: None,
            rela_address: None,
            rela_size: None,
            rela_entry_size: None,
            plt_address: None,
            plt_size: None,
            plt_format: None,
            soname_offset: None,
            needed: [None; MAX_NEEDED],
            needed_count: 0,
            has_rel: false,
        }
    }
}

pub fn parse(image: &[u8], plan: &LoadPlan, machine: Machine) -> Result<DynamicPlan, Error> {
    if image.len() < ELF_HEADER_SIZE {
        return Err(Error::Truncated);
    }
    if read_u16(image, 16)? != ET_DYN {
        return Err(Error::WrongType);
    }
    if read_u16(image, 18)? != machine as u16 {
        return Err(Error::WrongMachine);
    }
    let program_header_offset =
        usize::try_from(read_u64(image, 32)?).map_err(|_| Error::InvalidTable)?;
    let program_header_count = read_u16(image, 56)? as usize;
    let program_header_bytes = program_header_count
        .checked_mul(PROGRAM_HEADER_SIZE)
        .ok_or(Error::ProgramHeaderOverflow)?;
    let program_header_end = program_header_offset
        .checked_add(program_header_bytes)
        .ok_or(Error::ProgramHeaderOverflow)?;
    if program_header_end > image.len() {
        return Err(Error::Truncated);
    }

    let mut dynamic_header = None;
    let mut tls = None;
    for index in 0..program_header_count {
        let offset = program_header_offset + index * PROGRAM_HEADER_SIZE;
        let header = ProgramHeader {
            kind: read_u32(image, offset)?,
            file_offset: read_u64(image, offset + 8)?,
            virtual_address: read_u64(image, offset + 16)?,
            file_size: read_u64(image, offset + 32)?,
            memory_size: read_u64(image, offset + 40)?,
            alignment: read_u64(image, offset + 48)?,
        };
        match header.kind {
            PT_DYNAMIC => {
                if dynamic_header.replace(header).is_some() {
                    return Err(Error::DuplicateDynamicSegment);
                }
            }
            PT_TLS => {
                tls = Some(parse_tls(image, plan, header)?);
            }
            _ => {}
        }
    }

    let Some(header) = dynamic_header else {
        return Ok(DynamicPlan {
            machine,
            base: plan.base,
            dynamic: None,
            strings: None,
            symbols: None,
            symbol_count: 0,
            needed_count: 0,
            needed: [StringRef { offset: 0 }; MAX_NEEDED],
            soname_offset: None,
            relocation_count: 0,
            relocations: [Relocation {
                address: 0,
                symbol: 0,
                kind: 0,
                addend: 0,
            }; MAX_RELOCATIONS],
            tls,
        });
    };
    let dynamic_size = usize::try_from(header.file_size).map_err(|_| Error::TableTooLarge)?;
    if dynamic_size == 0
        || dynamic_size > MAX_DYNAMIC_ENTRIES * 16
        || !dynamic_size.is_multiple_of(16)
        || header.file_size > header.memory_size
    {
        return Err(Error::InvalidTable);
    }
    let dynamic_address = plan
        .base
        .checked_add(usize::try_from(header.virtual_address).map_err(|_| Error::InvalidTable)?)
        .ok_or(Error::InvalidTable)?;
    let dynamic_file_offset = plan
        .file_offset(dynamic_address, dynamic_size)
        .ok_or(Error::InvalidTable)?;
    let dynamic = Table {
        virtual_address: dynamic_address,
        file_offset: dynamic_file_offset,
        size: dynamic_size,
        entry_size: 16,
    };
    let mut tags = Tags::new();
    let mut terminated = false;
    for index in 0..(dynamic_size / 16) {
        let offset = dynamic_file_offset + index * 16;
        let tag = read_u64(image, offset)?;
        let value = read_u64(image, offset + 8)?;
        if tag == DT_NULL {
            terminated = true;
            break;
        }
        match tag {
            DT_NEEDED => {
                if tags.needed_count == MAX_NEEDED {
                    return Err(Error::TooManyNeeded);
                }
                tags.needed[tags.needed_count] = Some(value);
                tags.needed_count += 1;
            }
            DT_STRTAB => tags.string_address = Some(value),
            DT_STRSZ => tags.string_size = Some(value),
            DT_SYMTAB => tags.symbol_address = Some(value),
            DT_SYMENT => tags.symbol_entry_size = Some(value),
            DT_HASH => tags.hash_address = Some(value),
            DT_GNU_HASH => tags.gnu_hash_address = Some(value),
            DT_RELA => tags.rela_address = Some(value),
            DT_RELASZ => tags.rela_size = Some(value),
            DT_RELAENT => tags.rela_entry_size = Some(value),
            DT_JMPREL => tags.plt_address = Some(value),
            DT_PLTRELSZ => tags.plt_size = Some(value),
            DT_PLTREL => tags.plt_format = Some(value),
            DT_SONAME => tags.soname_offset = Some(value),
            DT_REL | DT_RELSZ | DT_RELENT => tags.has_rel = true,
            _ => {}
        }
    }
    if !terminated {
        return Err(Error::MissingDynamicTerminator);
    }

    let strings = match (tags.string_address, tags.string_size) {
        (Some(address), Some(size)) => Some(table(image, plan, address, size, 1)?),
        (None, None) if tags.needed_count == 0 && tags.soname_offset.is_none() => None,
        _ => return Err(Error::MissingStringTable),
    };
    let (symbols, symbol_count) = match tags.symbol_address {
        Some(address) => {
            let entry_size = tags.symbol_entry_size.ok_or(Error::InvalidTable)?;
            if entry_size as usize != SYMBOL_SIZE {
                return Err(Error::InvalidTable);
            }
            let symbol_count = match (tags.hash_address, tags.gnu_hash_address) {
                (Some(hash), _) => sysv_symbol_count(image, plan, hash)?,
                (None, Some(hash)) => gnu_symbol_count(image, plan, hash)?,
                (None, None) => return Err(Error::MissingSymbolHash),
            };
            let symbol_size = symbol_count
                .checked_mul(SYMBOL_SIZE)
                .ok_or(Error::TableTooLarge)?;
            let symbols = table(image, plan, address, symbol_size as u64, SYMBOL_SIZE)?;
            (Some(symbols), symbol_count)
        }
        None => (None, 0),
    };
    if tags.needed_count != 0 && strings.is_none() {
        return Err(Error::MissingStringTable);
    }
    for offset in tags
        .needed
        .iter()
        .take(tags.needed_count)
        .filter_map(|value| *value)
    {
        let strings = strings.ok_or(Error::MissingStringTable)?;
        string_at(
            image,
            &strings,
            usize::try_from(offset).map_err(|_| Error::InvalidString)?,
        )?;
    }
    if let Some(offset) = tags.soname_offset {
        let strings = strings.ok_or(Error::MissingStringTable)?;
        string_at(
            image,
            &strings,
            usize::try_from(offset).map_err(|_| Error::InvalidString)?,
        )?;
    }
    if tags.has_rel {
        return Err(Error::UnsupportedRelocationFormat);
    }

    let mut relocations = [Relocation {
        address: 0,
        symbol: 0,
        kind: 0,
        addend: 0,
    }; MAX_RELOCATIONS];
    let mut relocation_count = 0;
    if let Some(address) = tags.rela_address {
        let size = tags.rela_size.ok_or(Error::InvalidTable)?;
        let entry_size = tags.rela_entry_size.ok_or(Error::InvalidTable)?;
        let mut context = RelocationContext {
            image,
            plan,
            machine,
            symbol_count,
            relocations: &mut relocations,
            relocation_count: &mut relocation_count,
        };
        append_relocations(&mut context, address, size, entry_size)?;
    } else if tags.rela_size.is_some() || tags.rela_entry_size.is_some() {
        return Err(Error::InvalidTable);
    }
    if let Some(address) = tags.plt_address {
        if tags.plt_format != Some(DT_RELA_FORMAT) {
            return Err(Error::UnsupportedRelocationFormat);
        }
        let mut context = RelocationContext {
            image,
            plan,
            machine,
            symbol_count,
            relocations: &mut relocations,
            relocation_count: &mut relocation_count,
        };
        append_relocations(
            &mut context,
            address,
            tags.plt_size.ok_or(Error::InvalidTable)?,
            tags.rela_entry_size.unwrap_or(RELA_SIZE as u64),
        )?;
    } else if tags.plt_size.is_some() || tags.plt_format.is_some() {
        return Err(Error::InvalidTable);
    }

    Ok(DynamicPlan {
        machine,
        base: plan.base,
        dynamic: Some(dynamic),
        strings,
        symbols,
        symbol_count,
        needed_count: tags.needed_count,
        needed: tags.needed.map(|offset| StringRef {
            offset: usize::try_from(offset.unwrap_or(0)).unwrap_or(0),
        }),
        soname_offset: tags
            .soname_offset
            .map(|offset| usize::try_from(offset).unwrap_or(0)),
        relocation_count,
        relocations,
        tls,
    })
}

fn parse_tls(image: &[u8], plan: &LoadPlan, header: ProgramHeader) -> Result<TlsPlan, Error> {
    let file_size = usize::try_from(header.file_size).map_err(|_| Error::InvalidTls)?;
    let memory_size = usize::try_from(header.memory_size).map_err(|_| Error::InvalidTls)?;
    let alignment = usize::try_from(header.alignment).map_err(|_| Error::InvalidTls)?;
    if file_size > memory_size
        || memory_size > MAX_TLS_BYTES
        || (alignment > 1 && !alignment.is_power_of_two())
    {
        return Err(Error::InvalidTls);
    }
    let file_offset = usize::try_from(header.file_offset).map_err(|_| Error::InvalidTls)?;
    if file_offset
        .checked_add(file_size)
        .and_then(|end| image.get(file_offset..end))
        .is_none()
    {
        return Err(Error::InvalidTls);
    }
    let virtual_address = plan
        .base
        .checked_add(usize::try_from(header.virtual_address).map_err(|_| Error::InvalidTls)?)
        .ok_or(Error::InvalidTls)?;
    if !plan.contains_address(virtual_address, memory_size) {
        return Err(Error::InvalidTls);
    }
    Ok(TlsPlan {
        virtual_address,
        file_offset,
        file_size,
        memory_size,
        alignment,
    })
}

fn table(
    image: &[u8],
    plan: &LoadPlan,
    virtual_address: u64,
    size: u64,
    entry_size: usize,
) -> Result<Table, Error> {
    let size = usize::try_from(size).map_err(|_| Error::TableTooLarge)?;
    if size > MAX_STRING_TABLE || (entry_size != 1 && !size.is_multiple_of(entry_size)) {
        return Err(Error::TableTooLarge);
    }
    let virtual_address = plan
        .base
        .checked_add(usize::try_from(virtual_address).map_err(|_| Error::InvalidTable)?)
        .ok_or(Error::InvalidTable)?;
    let file_offset = plan
        .file_offset(virtual_address, size)
        .ok_or(Error::InvalidTable)?;
    image
        .get(file_offset..file_offset.checked_add(size).ok_or(Error::InvalidTable)?)
        .ok_or(Error::Truncated)?;
    Ok(Table {
        virtual_address,
        file_offset,
        size,
        entry_size,
    })
}

fn file_slice_at<'a>(
    image: &'a [u8],
    plan: &LoadPlan,
    virtual_address: u64,
    size: usize,
) -> Result<&'a [u8], Error> {
    let address = plan
        .base
        .checked_add(usize::try_from(virtual_address).map_err(|_| Error::InvalidTable)?)
        .ok_or(Error::InvalidTable)?;
    let offset = plan.file_offset(address, size).ok_or(Error::InvalidTable)?;
    image
        .get(offset..offset.checked_add(size).ok_or(Error::InvalidTable)?)
        .ok_or(Error::Truncated)
}

fn sysv_symbol_count(image: &[u8], plan: &LoadPlan, address: u64) -> Result<usize, Error> {
    let header = file_slice_at(image, plan, address, 8)?;
    let bucket_count = usize::try_from(read_u32(header, 0)?).map_err(|_| Error::TableTooLarge)?;
    let symbol_count = usize::try_from(read_u32(header, 4)?).map_err(|_| Error::TableTooLarge)?;
    if symbol_count > MAX_SYMBOLS {
        return Err(Error::TableTooLarge);
    }
    let words = bucket_count
        .checked_add(symbol_count)
        .ok_or(Error::TableTooLarge)?;
    let size = 8usize
        .checked_add(words.checked_mul(4).ok_or(Error::TableTooLarge)?)
        .ok_or(Error::TableTooLarge)?;
    file_slice_at(image, plan, address, size)?;
    Ok(symbol_count)
}

fn gnu_symbol_count(image: &[u8], plan: &LoadPlan, address: u64) -> Result<usize, Error> {
    let header = file_slice_at(image, plan, address, 16)?;
    let bucket_count = usize::try_from(read_u32(header, 0)?).map_err(|_| Error::TableTooLarge)?;
    let symbol_offset = usize::try_from(read_u32(header, 4)?).map_err(|_| Error::TableTooLarge)?;
    let bloom_size = usize::try_from(read_u32(header, 8)?).map_err(|_| Error::TableTooLarge)?;
    if bucket_count > MAX_GNU_HASH_BUCKETS || bloom_size == 0 || bloom_size > MAX_GNU_HASH_BLOOM {
        return Err(Error::TableTooLarge);
    }
    let buckets_offset = 16usize
        .checked_add(bloom_size.checked_mul(8).ok_or(Error::TableTooLarge)?)
        .ok_or(Error::TableTooLarge)?;
    file_slice_at(
        image,
        plan,
        address,
        buckets_offset
            .checked_add(bucket_count.checked_mul(4).ok_or(Error::TableTooLarge)?)
            .ok_or(Error::TableTooLarge)?,
    )?;
    let chains_address = address
        .checked_add(u64::try_from(buckets_offset).map_err(|_| Error::TableTooLarge)?)
        .and_then(|value| value.checked_add(u64::try_from(bucket_count.checked_mul(4)?).ok()?))
        .ok_or(Error::TableTooLarge)?;
    let mut symbol_count = 0;
    for bucket in 0..bucket_count {
        let bucket_offset = buckets_offset + bucket * 4;
        let bucket_bytes = file_slice_at(
            image,
            plan,
            address + u64::try_from(bucket_offset).map_err(|_| Error::TableTooLarge)?,
            4,
        )?;
        let mut index =
            usize::try_from(read_u32(bucket_bytes, 0)?).map_err(|_| Error::TableTooLarge)?;
        if index < symbol_offset {
            continue;
        }
        loop {
            let chain_index = index
                .checked_sub(symbol_offset)
                .ok_or(Error::TableTooLarge)?;
            if chain_index >= MAX_SYMBOLS {
                return Err(Error::TableTooLarge);
            }
            let chain_address = chains_address
                .checked_add(
                    u64::try_from(chain_index.checked_mul(4).ok_or(Error::TableTooLarge)?)
                        .map_err(|_| Error::TableTooLarge)?,
                )
                .ok_or(Error::TableTooLarge)?;
            let chain = file_slice_at(image, plan, chain_address, 4)?;
            let value = read_u32(chain, 0)?;
            symbol_count = symbol_count.max(index.saturating_add(1));
            if value & 1 != 0 {
                break;
            }
            index = index.checked_add(1).ok_or(Error::TableTooLarge)?;
        }
    }
    Ok(symbol_count)
}

struct RelocationContext<'a> {
    image: &'a [u8],
    plan: &'a LoadPlan,
    machine: Machine,
    symbol_count: usize,
    relocations: &'a mut [Relocation; MAX_RELOCATIONS],
    relocation_count: &'a mut usize,
}

fn append_relocations(
    context: &mut RelocationContext,
    address: u64,
    size: u64,
    entry_size: u64,
) -> Result<(), Error> {
    let entry_size = usize::try_from(entry_size).map_err(|_| Error::TableTooLarge)?;
    let size = usize::try_from(size).map_err(|_| Error::TableTooLarge)?;
    if entry_size != RELA_SIZE || !size.is_multiple_of(RELA_SIZE) {
        return Err(Error::UnsupportedRelocationFormat);
    }
    let table = file_slice_at(context.image, context.plan, address, size)?;
    for index in 0..(size / RELA_SIZE) {
        if *context.relocation_count == MAX_RELOCATIONS {
            return Err(Error::TableTooLarge);
        }
        let entry = &table[index * RELA_SIZE..(index + 1) * RELA_SIZE];
        let target = context
            .plan
            .base
            .checked_add(
                usize::try_from(read_u64(entry, 0)?).map_err(|_| Error::InvalidRelocationTarget)?,
            )
            .ok_or(Error::InvalidRelocationTarget)?;
        if !context
            .plan
            .contains_address(target, core::mem::size_of::<u64>())
        {
            return Err(Error::InvalidRelocationTarget);
        }
        let info = read_u64(entry, 8)?;
        let symbol = u32::try_from(info >> 32).map_err(|_| Error::InvalidSymbolIndex)?;
        if usize::try_from(symbol).map_err(|_| Error::InvalidSymbolIndex)? >= context.symbol_count
            && symbol != 0
        {
            return Err(Error::InvalidSymbolIndex);
        }
        let kind =
            u32::try_from(info & u64::from(u32::MAX)).map_err(|_| Error::UnsupportedRelocation)?;
        if !supported_relocation(context.machine, kind) {
            return Err(Error::UnsupportedRelocation);
        }
        context.relocations[*context.relocation_count] = Relocation {
            address: target,
            symbol,
            kind,
            addend: read_i64(entry, 16)?,
        };
        *context.relocation_count += 1;
    }
    Ok(())
}

fn supported_relocation(machine: Machine, kind: u32) -> bool {
    match machine {
        Machine::X86_64 => matches!(
            kind,
            R_X86_64_64 | R_X86_64_GLOB_DAT | R_X86_64_JUMP_SLOT | R_X86_64_RELATIVE
        ),
        Machine::Aarch64 => matches!(
            kind,
            R_AARCH64_ABS64 | R_AARCH64_GLOB_DAT | R_AARCH64_JUMP_SLOT | R_AARCH64_RELATIVE
        ),
    }
}

fn relative_kind(machine: Machine) -> u32 {
    match machine {
        Machine::X86_64 => R_X86_64_RELATIVE,
        Machine::Aarch64 => R_AARCH64_RELATIVE,
    }
}

fn symbolic_kind(machine: Machine) -> u32 {
    match machine {
        Machine::X86_64 => R_X86_64_GLOB_DAT,
        Machine::Aarch64 => R_AARCH64_GLOB_DAT,
    }
}

fn resolve_library<'a>(
    name: &[u8],
    path: &'a [u8],
    image: &'a [u8],
    plan: &'a DynamicPlan,
) -> Result<(&'a [u8], &'a DynamicPlan), Error> {
    validate_library_name(name)?;
    let found = LIBRARY_SEARCH_ROOTS.iter().any(|root| {
        path.starts_with(root)
            && path.len() == root.len() + 1 + name.len()
            && path[root.len()] == b'/'
            && path[root.len() + 1..] == *name
    });
    found
        .then_some((image, plan))
        .ok_or(Error::MissingDependency)
}

fn add_signed(base: usize, addend: i64) -> Option<usize> {
    if addend >= 0 {
        base.checked_add(addend as usize)
    } else {
        base.checked_sub(addend.unsigned_abs() as usize)
    }
}

fn string_at<'a>(image: &'a [u8], table: &Table, offset: usize) -> Result<&'a [u8], Error> {
    if offset >= table.size {
        return Err(Error::InvalidString);
    }
    let start = table
        .file_offset
        .checked_add(offset)
        .ok_or(Error::InvalidString)?;
    let bytes = image
        .get(start..table.file_offset + table.size)
        .ok_or(Error::Truncated)?;
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Error::InvalidString)?;
    Ok(&bytes[..end])
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, Error> {
    let bytes = bytes.get(offset..offset + 2).ok_or(Error::Truncated)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    let bytes = bytes.get(offset..offset + 4).ok_or(Error::Truncated)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, Error> {
    let bytes = bytes.get(offset..offset + 8).ok_or(Error::Truncated)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn read_i64(bytes: &[u8], offset: usize) -> Result<i64, Error> {
    Ok(i64::from_le_bytes(read_u64(bytes, offset)?.to_le_bytes()))
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

fn contract_image(machine: Machine) -> [u8; 4096 + 4] {
    let mut image = [0u8; 4096 + 4];
    image[0..4].copy_from_slice(b"\x7fELF");
    image[4] = 2;
    image[5] = 1;
    image[6] = 1;
    write_u16(&mut image, 16, ET_DYN);
    write_u16(&mut image, 18, machine as u16);
    write_u32(&mut image, 20, 1);
    write_u64(&mut image, 24, 0x400100);
    write_u64(&mut image, 32, ELF_HEADER_SIZE as u64);
    write_u16(&mut image, 52, ELF_HEADER_SIZE as u16);
    write_u16(&mut image, 54, PROGRAM_HEADER_SIZE as u16);
    write_u16(&mut image, 56, 3);

    let load = ELF_HEADER_SIZE;
    write_u32(&mut image, load, 1);
    write_u32(&mut image, load + 4, 5);
    write_u64(&mut image, load + 8, 0);
    write_u64(&mut image, load + 16, 0x400000);
    write_u64(&mut image, load + 32, 0x600);
    write_u64(&mut image, load + 40, 0x2000);
    write_u64(&mut image, load + 48, 0x1000);

    let dynamic = load + PROGRAM_HEADER_SIZE;
    write_u32(&mut image, dynamic, PT_DYNAMIC);
    write_u64(&mut image, dynamic + 8, 0x200);
    write_u64(&mut image, dynamic + 16, 0x400200);
    write_u64(&mut image, dynamic + 32, 10 * 16);
    write_u64(&mut image, dynamic + 40, 10 * 16);
    write_u64(&mut image, dynamic + 48, 8);

    let tls = dynamic + PROGRAM_HEADER_SIZE;
    write_u32(&mut image, tls, PT_TLS);
    write_u64(&mut image, tls + 8, 0x500);
    write_u64(&mut image, tls + 16, 0x400500);
    write_u64(&mut image, tls + 32, 4);
    write_u64(&mut image, tls + 40, 8);
    write_u64(&mut image, tls + 48, 8);

    let dynamic_data = 0x200;
    let entries = [
        (DT_NEEDED, 1),
        (DT_STRTAB, 0x400300),
        (DT_STRSZ, 16),
        (DT_SYMTAB, 0x400320),
        (DT_SYMENT, SYMBOL_SIZE as u64),
        (DT_HASH, 0x400380),
        (DT_RELA, 0x4003a0),
        (DT_RELASZ, RELA_SIZE as u64),
        (DT_RELAENT, RELA_SIZE as u64),
        (DT_NULL, 0),
    ];
    for (index, (tag, value)) in entries.into_iter().enumerate() {
        write_u64(&mut image, dynamic_data + index * 16, tag);
        write_u64(&mut image, dynamic_data + index * 16 + 8, value);
    }
    image[0x300..0x310].copy_from_slice(b"\0libdep.so\0foo\0\0");
    write_u32(&mut image, 0x380, 1);
    write_u32(&mut image, 0x384, 2);
    write_u32(&mut image, 0x388, 1);
    write_u32(&mut image, 0x38c, 0);
    write_u32(&mut image, 0x390, 0);
    write_u32(&mut image, 0x320 + SYMBOL_SIZE, 11);
    write_u16(&mut image, 0x320 + SYMBOL_SIZE + 6, 1);
    write_u64(&mut image, 0x320 + SYMBOL_SIZE + 8, 0x400600);
    write_u64(&mut image, 0x320 + SYMBOL_SIZE + 16, 4);
    write_u64(&mut image, 0x3a0, 0x400600);
    write_u64(&mut image, 0x3a8, u64::from(relative_kind(machine)));
    write_u64(&mut image, 0x3b0, 0x10);
    image[0x500..0x504].copy_from_slice(b"TLS!");
    image[0x100] = 0xc3;
    image
}

static mut SMOKE_MAIN_IMAGE: [u8; SMOKE_IMAGE_SIZE] = [0; SMOKE_IMAGE_SIZE];
static mut SMOKE_LIBRARY_IMAGE: [u8; SMOKE_IMAGE_SIZE] = [0; SMOKE_IMAGE_SIZE];

fn make_smoke_library(image: &mut [u8; SMOKE_IMAGE_SIZE]) {
    let entries = [
        (DT_STRTAB, 0x400300),
        (DT_STRSZ, 16),
        (DT_SYMTAB, 0x400320),
        (DT_SYMENT, SYMBOL_SIZE as u64),
        (DT_HASH, 0x400380),
        (DT_RELA, 0x4003a0),
        (DT_RELASZ, RELA_SIZE as u64),
        (DT_RELAENT, RELA_SIZE as u64),
        (DT_NULL, 0),
    ];
    for (index, (tag, value)) in entries.into_iter().enumerate() {
        write_u64(image, 0x200 + index * 16, tag);
        write_u64(image, 0x208 + index * 16, value);
    }
}

pub fn smoke_self_check() {
    let machine = Machine::current();
    let (main_image, library_image) = unsafe {
        let main = core::ptr::addr_of_mut!(SMOKE_MAIN_IMAGE);
        let library = core::ptr::addr_of_mut!(SMOKE_LIBRARY_IMAGE);
        main.write(contract_image(machine));
        library.write(contract_image(machine));
        write_u16(&mut *main, 0x320 + SYMBOL_SIZE + 6, 0);
        write_u64(
            &mut *main,
            0x3a8,
            (1u64 << 32) | u64::from(symbolic_kind(machine)),
        );
        write_u64(&mut *main, 0x3b0, 4);
        make_smoke_library(&mut *library);
        (&*main, &*library)
    };
    let main_load = elf::parse(main_image, machine, 0x100000).unwrap();
    let library_load = elf::parse(library_image, machine, 0x200000).unwrap();
    let main_dynamic = parse(main_image, &main_load, machine).unwrap();
    let library_dynamic = parse(library_image, &library_load, machine).unwrap();
    let dependency = main_dynamic.needed(main_image, 0).unwrap().unwrap();
    let (resolved_image, resolved_plan) = resolve_library(
        dependency,
        b"/lib/libdep.so",
        library_image,
        &library_dynamic,
    )
    .unwrap();
    assert_eq!(resolved_image, library_image);
    let undefined = main_dynamic
        .lookup_symbol(main_image, b"foo")
        .unwrap()
        .unwrap();
    assert!(!undefined.defined);
    let definition = resolved_plan
        .lookup_symbol(resolved_image, b"foo")
        .unwrap()
        .unwrap();
    assert!(definition.defined);
    let relocation = main_dynamic.relocation(0).unwrap();
    assert_eq!(relocation.symbol, 1);
    assert_eq!(
        main_dynamic
            .relocation_value(
                0,
                Some(resolved_plan.base.checked_add(definition.value).unwrap()),
            )
            .unwrap(),
        0x600604
    );
    assert_eq!(
        resolve_library(
            b"missing.so",
            b"/lib/libdep.so",
            library_image,
            &library_dynamic,
        ),
        Err(Error::MissingDependency)
    );
    let dentry = crate::vfs::lookup("/lib/libdep.so").unwrap();
    assert_ne!(dentry.dentry().mount, crate::vfs::MountId::ROOT);
    crate::vfs::release_dentry(dentry).unwrap();
    assert!(matches!(
        crate::vfs::lookup("/lib/missing.so"),
        Err(crate::vfs::Error::NotFound)
    ));

    let arguments = [b"dynamic-smoke".as_slice()];
    let environment: [&[u8]; 0] = [];
    let mut runtime = NativeRuntime::prepare(
        ProcessId::INIT,
        &main_load,
        AslrHook::new(47),
        &arguments,
        &environment,
    )
    .unwrap();
    runtime.start().unwrap();
    runtime.exit(0).unwrap();
    assert!(runtime.is_exited());
}

pub fn contract_self_check() {
    let image = contract_image(Machine::current());
    let load_plan = crate::elf::parse(&image, Machine::current(), 0x100000).unwrap();
    let dynamic = parse(&image, &load_plan, Machine::current()).unwrap();
    assert!(dynamic.dynamic.is_some());
    assert_eq!(dynamic.needed(&image, 0).unwrap(), Some(&b"libdep.so"[..]));
    assert_eq!(dynamic.soname(&image).unwrap(), None);
    let symbol = dynamic.lookup_symbol(&image, b"foo").unwrap().unwrap();
    assert!(symbol.defined);
    assert_eq!(symbol.value, 0x400600);
    assert_eq!(dynamic.relocation_count, 1);
    assert_eq!(dynamic.relocation(0).unwrap().address, 0x500600);
    assert_eq!(dynamic.relative_value(0).unwrap(), 0x100010);
    assert_eq!(dynamic.tls.unwrap().memory_size, 8);
    assert_eq!(LIBRARY_SEARCH_ROOTS, [&b"/lib"[..], &b"/lib64"[..]]);
    validate_library_name(b"libdep.so").unwrap();
    assert_eq!(
        validate_library_name(b"../host.so"),
        Err(Error::InvalidLibraryName)
    );
    let other_machine = match Machine::current() {
        Machine::X86_64 => Machine::Aarch64,
        Machine::Aarch64 => Machine::X86_64,
    };
    assert_eq!(
        parse(&image, &load_plan, other_machine),
        Err(Error::WrongMachine)
    );
}
