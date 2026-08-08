use crate::address_space::{PAGE_SIZE, USER_LIMIT};

const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const MAX_LOAD_SEGMENTS: usize = 8;
const MAX_LOAD_PAGES: usize = 256;
const MAX_ARGUMENTS: usize = 16;
const MAX_ENVIRONMENT: usize = 16;
const MAX_STACK_WORDS: usize = 64;
const MAX_STACK_BYTES: usize = 4096;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const PT_LOAD: u32 = 1;
const PT_INTERP: u32 = 3;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const EM_X86_64: u16 = 62;
const EM_AARCH64: u16 = 183;
const AT_PAGESZ: u64 = 6;
const AT_ENTRY: u64 = 9;
const MAX_INTERPRETER: usize = 128;

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Machine {
    X86_64 = EM_X86_64,
    Aarch64 = EM_AARCH64,
}

impl Machine {
    pub const fn current() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            Self::X86_64
        }
        #[cfg(target_arch = "aarch64")]
        {
            Self::Aarch64
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentFlags {
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
}

impl SegmentFlags {
    const fn from_raw(raw: u32) -> Self {
        Self {
            readable: raw & 4 != 0,
            writable: raw & PF_W != 0,
            executable: raw & PF_X != 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentPlan {
    pub virtual_start: usize,
    pub virtual_end: usize,
    pub file_offset: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub zero_fill: u64,
    pub flags: SegmentFlags,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadPlan {
    pub base: usize,
    pub entry: usize,
    pub segment_count: usize,
    pub total_pages: usize,
    segments: [Option<SegmentPlan>; MAX_LOAD_SEGMENTS],
}

impl LoadPlan {
    pub fn segment(&self, index: usize) -> Option<SegmentPlan> {
        self.segments.get(index).copied().flatten()
    }

    pub fn build_initial_stack<'a>(
        &self,
        stack_top: usize,
        arguments: &[&'a [u8]],
        environment: &[&'a [u8]],
    ) -> Result<InitialStack, Error> {
        InitialStack::build(self.entry, stack_top, arguments, environment)
    }

    pub fn contains_address(&self, virtual_address: usize, length: usize) -> bool {
        let Some(end) = virtual_address.checked_add(length) else {
            return false;
        };
        self.segments
            .iter()
            .flatten()
            .any(|segment| virtual_address >= segment.virtual_start && end <= segment.virtual_end)
    }

    pub fn file_offset(&self, virtual_address: usize, length: usize) -> Option<usize> {
        let end = virtual_address.checked_add(length)?;
        self.segments.iter().flatten().find_map(|segment| {
            let file_end = segment
                .virtual_start
                .checked_add(usize_from_u64(segment.file_size).ok()?)?;
            if virtual_address < segment.virtual_start || end > file_end {
                return None;
            }
            usize_from_u64(segment.file_offset)
                .ok()?
                .checked_add(virtual_address - segment.virtual_start)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitialRegisters {
    pub instruction_pointer: usize,
    pub stack_pointer: usize,
    pub flags: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitialStack {
    pub stack_pointer: usize,
    pub word_count: usize,
    words: [u64; MAX_STACK_WORDS],
    bytes: [u8; MAX_STACK_BYTES],
    byte_count: usize,
}

impl InitialStack {
    fn build<'a>(
        entry: usize,
        stack_top: usize,
        arguments: &[&'a [u8]],
        environment: &[&'a [u8]],
    ) -> Result<Self, Error> {
        if arguments.len() > MAX_ARGUMENTS || environment.len() > MAX_ENVIRONMENT {
            return Err(Error::TooManyStrings);
        }
        if !stack_top.is_multiple_of(PAGE_SIZE)
            || !(MAX_STACK_BYTES + PAGE_SIZE..=USER_LIMIT).contains(&stack_top)
        {
            return Err(Error::InvalidStack);
        }
        let word_count = 1usize
            .checked_add(arguments.len())
            .and_then(|count| count.checked_add(1))
            .and_then(|count| count.checked_add(environment.len()))
            .and_then(|count| count.checked_add(1))
            .and_then(|count| count.checked_add(6))
            .ok_or(Error::StackOverflow)?;
        if word_count > MAX_STACK_WORDS {
            return Err(Error::StackOverflow);
        }

        let stack_bottom = stack_top - MAX_STACK_BYTES;
        let mut bytes = [0; MAX_STACK_BYTES];
        let mut cursor = stack_top;
        let mut argument_pointers = [0; MAX_ARGUMENTS];
        let mut environment_pointers = [0; MAX_ENVIRONMENT];
        let mut byte_count = 0;
        for (index, string) in arguments.iter().enumerate() {
            argument_pointers[index] = place_string(
                string,
                stack_bottom,
                &mut cursor,
                &mut bytes,
                &mut byte_count,
            )?;
        }
        for (index, string) in environment.iter().enumerate() {
            environment_pointers[index] = place_string(
                string,
                stack_bottom,
                &mut cursor,
                &mut bytes,
                &mut byte_count,
            )?;
        }

        let word_bytes = word_count
            .checked_mul(core::mem::size_of::<u64>())
            .ok_or(Error::StackOverflow)?;
        let stack_pointer = align_down(
            cursor.checked_sub(word_bytes).ok_or(Error::StackOverflow)?,
            16,
        );
        if stack_pointer < stack_bottom {
            return Err(Error::StackOverflow);
        }
        let mut words = [0; MAX_STACK_WORDS];
        let mut index = 0;
        words[index] = arguments.len() as u64;
        index += 1;
        for pointer in argument_pointers.iter().take(arguments.len()) {
            words[index] = *pointer;
            index += 1;
        }
        words[index] = 0;
        index += 1;
        for pointer in environment_pointers.iter().take(environment.len()) {
            words[index] = *pointer;
            index += 1;
        }
        words[index] = 0;
        index += 1;
        words[index] = AT_PAGESZ;
        words[index + 1] = PAGE_SIZE as u64;
        words[index + 2] = AT_ENTRY;
        words[index + 3] = entry as u64;
        words[index + 4] = 0;
        words[index + 5] = 0;
        Ok(Self {
            stack_pointer,
            word_count,
            words,
            bytes,
            byte_count,
        })
    }

    pub fn word(&self, index: usize) -> Option<u64> {
        (index < self.word_count).then_some(self.words[index])
    }

    pub fn string_bytes(&self) -> &[u8] {
        &self.bytes[..self.byte_count]
    }

    pub fn words(&self) -> &[u64] {
        &self.words[..self.word_count]
    }

    pub fn registers(&self, entry: usize) -> InitialRegisters {
        #[cfg(target_arch = "x86_64")]
        let (flags, arg0, arg1, arg2) = (0x202, 0, 0, 0);
        #[cfg(target_arch = "aarch64")]
        let (flags, arg0, arg1, arg2) = (
            0,
            self.words[0],
            (self.stack_pointer + core::mem::size_of::<u64>()) as u64,
            (self.stack_pointer + core::mem::size_of::<u64>() * (2 + self.words[0] as usize))
                as u64,
        );
        InitialRegisters {
            instruction_pointer: entry,
            stack_pointer: self.stack_pointer,
            flags,
            arg0,
            arg1,
            arg2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Truncated,
    BadMagic,
    UnsupportedClass,
    UnsupportedEndian,
    BadVersion,
    WrongType,
    WrongMachine,
    BadHeaderSize,
    BadProgramHeaderSize,
    ProgramHeaderOverflow,
    TooManySegments,
    NoLoadSegments,
    InvalidAlignment,
    InvalidRange,
    FileRangeOverflow,
    FileSizeExceedsMemory,
    SegmentOverlap,
    WriteExecute,
    InvalidEntry,
    TooManyStrings,
    InteriorNul,
    InvalidStack,
    StackOverflow,
    InterpreterTooLong,
    InvalidInterpreter,
}

pub fn parse(image: &[u8], machine: Machine, load_bias: usize) -> Result<LoadPlan, Error> {
    if image.len() < ELF_HEADER_SIZE {
        return Err(Error::Truncated);
    }
    if image[0..4] != *b"\x7fELF" {
        return Err(Error::BadMagic);
    }
    if image[4] != 2 {
        return Err(Error::UnsupportedClass);
    }
    if image[5] != 1 {
        return Err(Error::UnsupportedEndian);
    }
    if image[6] != 1 {
        return Err(Error::BadVersion);
    }
    let object_type = read_u16(image, 16)?;
    if object_type != ET_EXEC && object_type != ET_DYN {
        return Err(Error::WrongType);
    }
    if read_u16(image, 18)? != machine as u16 {
        return Err(Error::WrongMachine);
    }
    if read_u16(image, 52)? as usize != ELF_HEADER_SIZE {
        return Err(Error::BadHeaderSize);
    }
    if read_u16(image, 54)? as usize != PROGRAM_HEADER_SIZE {
        return Err(Error::BadProgramHeaderSize);
    }
    if object_type == ET_EXEC && load_bias != 0 {
        return Err(Error::InvalidRange);
    }
    if object_type == ET_DYN && load_bias == 0 {
        return Err(Error::InvalidRange);
    }
    if !load_bias.is_multiple_of(PAGE_SIZE) {
        return Err(Error::InvalidAlignment);
    }
    let entry_raw = read_u64(image, 24)?;
    let program_header_offset = usize_from_u64(read_u64(image, 32)?)?;
    let program_header_count = read_u16(image, 56)? as usize;
    if program_header_count == 0 || program_header_count > MAX_LOAD_SEGMENTS * 4 {
        return Err(Error::TooManySegments);
    }
    let program_header_bytes = program_header_count
        .checked_mul(PROGRAM_HEADER_SIZE)
        .ok_or(Error::ProgramHeaderOverflow)?;
    let program_header_end = program_header_offset
        .checked_add(program_header_bytes)
        .ok_or(Error::ProgramHeaderOverflow)?;
    if program_header_end > image.len() {
        return Err(Error::Truncated);
    }

    let entry = load_bias
        .checked_add(usize_from_u64(entry_raw)?)
        .ok_or(Error::InvalidRange)?;
    let mut plan = LoadPlan {
        base: load_bias,
        entry,
        segment_count: 0,
        total_pages: 0,
        segments: [None; MAX_LOAD_SEGMENTS],
    };
    for index in 0..program_header_count {
        let offset = program_header_offset + index * PROGRAM_HEADER_SIZE;
        if read_u32(image, offset)? != PT_LOAD {
            continue;
        }
        if plan.segment_count == MAX_LOAD_SEGMENTS {
            return Err(Error::TooManySegments);
        }
        let flags_raw = read_u32(image, offset + 4)?;
        let file_offset = read_u64(image, offset + 8)?;
        let virtual_raw = read_u64(image, offset + 16)?;
        let file_size = read_u64(image, offset + 32)?;
        let memory_size = read_u64(image, offset + 40)?;
        let alignment = read_u64(image, offset + 48)?;
        if file_size > memory_size {
            return Err(Error::FileSizeExceedsMemory);
        }
        if alignment > 1 && !alignment.is_power_of_two() {
            return Err(Error::InvalidAlignment);
        }
        if file_offset % PAGE_SIZE as u64 != virtual_raw % PAGE_SIZE as u64 {
            return Err(Error::InvalidAlignment);
        }
        let file_end = file_offset
            .checked_add(file_size)
            .ok_or(Error::FileRangeOverflow)?;
        if file_end > image.len() as u64 {
            return Err(Error::FileRangeOverflow);
        }
        let virtual_raw = usize_from_u64(virtual_raw)?;
        let virtual_start = load_bias
            .checked_add(align_down(virtual_raw, PAGE_SIZE))
            .ok_or(Error::InvalidRange)?;
        let memory_end_raw = virtual_raw
            .checked_add(usize_from_u64(memory_size)?)
            .ok_or(Error::InvalidRange)?;
        let virtual_end = load_bias
            .checked_add(align_up(memory_end_raw, PAGE_SIZE).ok_or(Error::InvalidRange)?)
            .ok_or(Error::InvalidRange)?;
        if virtual_start < PAGE_SIZE || virtual_start >= virtual_end || virtual_end > USER_LIMIT {
            return Err(Error::InvalidRange);
        }
        let flags = SegmentFlags::from_raw(flags_raw);
        if flags.writable && flags.executable {
            return Err(Error::WriteExecute);
        }
        let segment = SegmentPlan {
            virtual_start,
            virtual_end,
            file_offset,
            file_size,
            memory_size,
            zero_fill: memory_size - file_size,
            flags,
        };
        if plan.segments.iter().flatten().any(|previous| {
            segment.virtual_start < previous.virtual_end
                && previous.virtual_start < segment.virtual_end
        }) {
            return Err(Error::SegmentOverlap);
        }
        let pages = (virtual_end - virtual_start) / PAGE_SIZE;
        plan.total_pages = plan
            .total_pages
            .checked_add(pages)
            .ok_or(Error::InvalidRange)?;
        if plan.total_pages > MAX_LOAD_PAGES {
            return Err(Error::InvalidRange);
        }
        plan.segments[plan.segment_count] = Some(segment);
        plan.segment_count += 1;
    }
    if plan.segment_count == 0 {
        return Err(Error::NoLoadSegments);
    }
    if !plan.segments.iter().flatten().any(|segment| {
        segment.flags.executable
            && segment.virtual_start <= plan.entry
            && plan.entry < segment.virtual_end
    }) {
        return Err(Error::InvalidEntry);
    }
    Ok(plan)
}

pub fn select_interpreter<'a>(
    image: &'a [u8],
    requested: Option<&'a [u8]>,
) -> Result<Option<&'a [u8]>, Error> {
    if let Some(interpreter) = interpreter(image)? {
        return Ok(Some(interpreter));
    }
    requested.map(validate_interpreter).transpose()
}

fn interpreter(image: &[u8]) -> Result<Option<&[u8]>, Error> {
    if image.len() < ELF_HEADER_SIZE {
        return Err(Error::Truncated);
    }
    let program_header_offset = usize_from_u64(read_u64(image, 32)?)?;
    let program_header_count = read_u16(image, 56)? as usize;
    let program_header_bytes = program_header_count
        .checked_mul(PROGRAM_HEADER_SIZE)
        .ok_or(Error::ProgramHeaderOverflow)?;
    let end = program_header_offset
        .checked_add(program_header_bytes)
        .ok_or(Error::ProgramHeaderOverflow)?;
    if end > image.len() {
        return Err(Error::Truncated);
    }
    for index in 0..program_header_count {
        let offset = program_header_offset + index * PROGRAM_HEADER_SIZE;
        if read_u32(image, offset)? != PT_INTERP {
            continue;
        }
        let file_offset = usize_from_u64(read_u64(image, offset + 8)?)?;
        let file_size = usize_from_u64(read_u64(image, offset + 32)?)?;
        if file_size == 0 || file_size > MAX_INTERPRETER {
            return Err(Error::InterpreterTooLong);
        }
        let bytes = image
            .get(
                file_offset
                    ..file_offset
                        .checked_add(file_size)
                        .ok_or(Error::FileRangeOverflow)?,
            )
            .ok_or(Error::FileRangeOverflow)?;
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(Error::InvalidInterpreter)?;
        return validate_interpreter(&bytes[..end]).map(Some);
    }
    Ok(None)
}

fn validate_interpreter(path: &[u8]) -> Result<&[u8], Error> {
    if path.is_empty() || path.len() > MAX_INTERPRETER || path.contains(&0) {
        return Err(Error::InvalidInterpreter);
    }
    Ok(path)
}

fn place_string(
    string: &[u8],
    stack_bottom: usize,
    cursor: &mut usize,
    bytes: &mut [u8; MAX_STACK_BYTES],
    byte_count: &mut usize,
) -> Result<u64, Error> {
    if string.contains(&0) {
        return Err(Error::InteriorNul);
    }
    let length = string.len().checked_add(1).ok_or(Error::StackOverflow)?;
    let start = cursor.checked_sub(length).ok_or(Error::StackOverflow)?;
    if start < stack_bottom {
        return Err(Error::StackOverflow);
    }
    let offset = start - stack_bottom;
    bytes[offset..offset + string.len()].copy_from_slice(string);
    bytes[offset + string.len()] = 0;
    *cursor = start;
    *byte_count = byte_count.saturating_add(length);
    Ok(start as u64)
}

fn usize_from_u64(value: u64) -> Result<usize, Error> {
    if value > usize::MAX as u64 {
        Err(Error::InvalidRange)
    } else {
        Ok(value as usize)
    }
}

fn read_u16(image: &[u8], offset: usize) -> Result<u16, Error> {
    let bytes = image.get(offset..offset + 2).ok_or(Error::Truncated)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(image: &[u8], offset: usize) -> Result<u32, Error> {
    let bytes = image.get(offset..offset + 4).ok_or(Error::Truncated)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(image: &[u8], offset: usize) -> Result<u64, Error> {
    let bytes = image.get(offset..offset + 8).ok_or(Error::Truncated)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn align_down(value: usize, alignment: usize) -> usize {
    value & !(alignment - 1)
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
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

pub(crate) fn contract_image(machine: Machine) -> [u8; PAGE_SIZE + 4] {
    let mut image = [0u8; PAGE_SIZE + 4];
    image[0..4].copy_from_slice(b"\x7fELF");
    image[4] = 2;
    image[5] = 1;
    image[6] = 1;
    write_u16(&mut image, 16, ET_EXEC);
    write_u16(&mut image, 18, machine as u16);
    write_u32(&mut image, 20, 1);
    write_u64(&mut image, 24, 0x400000);
    write_u64(&mut image, 32, ELF_HEADER_SIZE as u64);
    write_u16(&mut image, 52, ELF_HEADER_SIZE as u16);
    write_u16(&mut image, 54, PROGRAM_HEADER_SIZE as u16);
    write_u16(&mut image, 56, 1);
    let program_header = ELF_HEADER_SIZE;
    write_u32(&mut image, program_header, PT_LOAD);
    write_u32(&mut image, program_header + 4, 5);
    write_u64(&mut image, program_header + 8, PAGE_SIZE as u64);
    write_u64(&mut image, program_header + 16, 0x400000);
    write_u64(&mut image, program_header + 32, 4);
    write_u64(&mut image, program_header + 40, PAGE_SIZE as u64);
    write_u64(&mut image, program_header + 48, PAGE_SIZE as u64);
    image[PAGE_SIZE] = 0xc3;
    image
}

#[allow(dead_code)]
pub(crate) fn service_image(machine: Machine) -> [u8; PAGE_SIZE + 16] {
    let mut image = [0u8; PAGE_SIZE + 16];
    image[0..4].copy_from_slice(b"\x7fELF");
    image[4] = 2;
    image[5] = 1;
    image[6] = 1;
    write_u16(&mut image, 16, ET_DYN);
    write_u16(&mut image, 18, machine as u16);
    write_u32(&mut image, 20, 1);
    write_u64(&mut image, 24, 0);
    write_u64(&mut image, 32, ELF_HEADER_SIZE as u64);
    write_u16(&mut image, 52, ELF_HEADER_SIZE as u16);
    write_u16(&mut image, 54, PROGRAM_HEADER_SIZE as u16);
    write_u16(&mut image, 56, 1);
    let program_header = ELF_HEADER_SIZE;
    write_u32(&mut image, program_header, PT_LOAD);
    write_u32(&mut image, program_header + 4, PF_X | 4);
    write_u64(&mut image, program_header + 8, PAGE_SIZE as u64);
    write_u64(&mut image, program_header + 16, 0);
    write_u64(&mut image, program_header + 32, 12);
    write_u64(&mut image, program_header + 40, PAGE_SIZE as u64);
    write_u64(&mut image, program_header + 48, PAGE_SIZE as u64);
    #[cfg(target_arch = "x86_64")]
    let code = [
        0xb8, 0x3c, 0x00, 0x00, 0x00, 0xbf, 0x2a, 0x00, 0x00, 0x00, 0x0f, 0x05,
    ];
    #[cfg(target_arch = "aarch64")]
    let code = [
        0x88, 0x07, 0x80, 0xd2, 0x40, 0x05, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4,
    ];
    image[PAGE_SIZE..PAGE_SIZE + code.len()].copy_from_slice(&code);
    image
}

pub(crate) fn representative_image() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/nordix-userspace-smoke.elf"))
}

pub(crate) fn representative_c_image() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/nordix-userspace-c.elf"))
}

pub(crate) fn representative_cxx_image() -> &'static [u8] {
    include_bytes!(concat!(env!("OUT_DIR"), "/nordix-userspace-cxx.elf"))
}

pub(crate) fn load_bias_for_image(image: &[u8], dynamic_bias: usize) -> usize {
    if image.len() >= 18 && u16::from_le_bytes([image[16], image[17]]) == ET_EXEC {
        0
    } else {
        dynamic_bias
    }
}

pub fn contract_self_check() {
    let _machines = [Machine::X86_64, Machine::Aarch64];
    let _errors = [
        Error::Truncated,
        Error::BadMagic,
        Error::UnsupportedClass,
        Error::UnsupportedEndian,
        Error::BadVersion,
        Error::WrongType,
        Error::WrongMachine,
        Error::BadHeaderSize,
        Error::BadProgramHeaderSize,
        Error::ProgramHeaderOverflow,
        Error::TooManySegments,
        Error::NoLoadSegments,
        Error::InvalidAlignment,
        Error::InvalidRange,
        Error::FileRangeOverflow,
        Error::FileSizeExceedsMemory,
        Error::SegmentOverlap,
        Error::WriteExecute,
        Error::InvalidEntry,
        Error::TooManyStrings,
        Error::InteriorNul,
        Error::InvalidStack,
        Error::StackOverflow,
        Error::InterpreterTooLong,
        Error::InvalidInterpreter,
    ];
    let image = contract_image(Machine::current());
    let program_header = ELF_HEADER_SIZE;
    let plan = parse(&image, Machine::current(), 0).unwrap();
    assert_eq!(plan.segment_count, 1);
    assert_eq!(plan.total_pages, 1);
    assert_eq!(plan.entry, 0x400000);
    let segment = plan.segment(0).unwrap();
    assert!(segment.flags.executable && !segment.flags.writable);
    assert_eq!(segment.zero_fill, PAGE_SIZE as u64 - 4);
    assert_eq!(
        select_interpreter(&image, Some(b"/lib/norx-ld.so")),
        Ok(Some(&b"/lib/norx-ld.so"[..]))
    );
    let arguments = [b"init".as_slice(), b"--check".as_slice()];
    let environment = [b"PATH=/sbin".as_slice()];
    let stack = plan
        .build_initial_stack(USER_LIMIT - PAGE_SIZE, &arguments, &environment)
        .unwrap();
    assert_eq!(stack.word(0), Some(2));
    assert_eq!(stack.string_bytes().last(), Some(&0));
    let registers = stack.registers(plan.entry);
    assert_eq!(registers.instruction_pointer, plan.entry);
    assert_eq!(registers.stack_pointer, stack.stack_pointer);
    #[cfg(target_arch = "x86_64")]
    assert_eq!(registers.flags, 0x202);
    #[cfg(target_arch = "aarch64")]
    {
        assert_eq!(registers.flags, 0);
        assert_eq!(registers.arg0, 2);
        assert_eq!(registers.arg1, stack.stack_pointer as u64 + 8);
        assert_eq!(registers.arg2, stack.stack_pointer as u64 + 8 * 4);
    }
    let mut variant = image;
    write_u32(&mut variant, program_header + 4, PF_W | PF_X);
    assert_eq!(
        parse(&variant, Machine::current(), 0),
        Err(Error::WriteExecute)
    );
    variant = image;
    variant[0] = 0;
    assert_eq!(parse(&variant, Machine::current(), 0), Err(Error::BadMagic));
    variant = image;
    write_u16(&mut variant, 56, 2);
    let second_header = program_header + PROGRAM_HEADER_SIZE;
    write_u32(&mut variant, second_header, PT_LOAD);
    write_u32(&mut variant, second_header + 4, 5);
    write_u64(&mut variant, second_header + 8, PAGE_SIZE as u64);
    write_u64(&mut variant, second_header + 16, 0x400000);
    write_u64(&mut variant, second_header + 32, 4);
    write_u64(&mut variant, second_header + 40, PAGE_SIZE as u64);
    write_u64(&mut variant, second_header + 48, PAGE_SIZE as u64);
    assert_eq!(
        parse(&variant, Machine::current(), 0),
        Err(Error::SegmentOverlap)
    );
    variant = image;
    write_u64(&mut variant, program_header + 16, USER_LIMIT as u64);
    assert_eq!(
        parse(&variant, Machine::current(), 0),
        Err(Error::InvalidRange)
    );
    variant = image;
    write_u64(&mut variant, 24, 0x500000);
    assert_eq!(
        parse(&variant, Machine::current(), 0),
        Err(Error::InvalidEntry)
    );
    let too_many_arguments = [b"x".as_slice(); MAX_ARGUMENTS + 1];
    let too_many_environment = [b"x".as_slice(); MAX_ENVIRONMENT + 1];
    assert_eq!(
        plan.build_initial_stack(USER_LIMIT - PAGE_SIZE, &too_many_arguments, &[]),
        Err(Error::TooManyStrings)
    );
    assert_eq!(
        plan.build_initial_stack(USER_LIMIT - PAGE_SIZE, &[], &too_many_environment),
        Err(Error::TooManyStrings)
    );
    assert_eq!(
        parse(&image[..PAGE_SIZE + 3], Machine::current(), 0),
        Err(Error::FileRangeOverflow)
    );
}
