const MAGIC: &[u8; 4] = b"\0asm";
const VERSION: &[u8; 4] = b"\x01\0\0\0";
const PAGE_SIZE: usize = 65_536;
const MAX_MEMORY: usize = PAGE_SIZE;
const MAX_STACK: usize = 256;
const MAX_CONTROL: usize = 16;
const MAX_HANDLES: usize = 8;
const MAX_TYPES: usize = 8;
const MAX_MODULE: usize = 256 * 1024;
const MAX_FUEL: u64 = 1_000_000;
const PROFILE_ITERATIONS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Truncated,
    BadMagic,
    BadVersion,
    InvalidLeb,
    InvalidSection,
    DuplicateSection,
    UnsupportedSection,
    MissingProfile,
    InvalidProfile,
    InvalidType,
    InvalidImport,
    InvalidExport,
    InvalidMemory,
    InvalidCode,
    InvalidControl,
    InvalidStack,
    TypeMismatch,
    UnsupportedOpcode,
    ModuleTooLarge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trap {
    FuelExhausted,
    StackOverflow,
    StackUnderflow,
    MemoryFault,
    TypeMismatch,
    HostDenied,
    Unreachable,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    I32(i32),
    I64(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleMetrics {
    pub startup_ticks: u64,
    pub module_bytes: usize,
    pub linear_memory_bytes: usize,
    pub host_calls: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Type {
    I32,
    I64,
}

#[derive(Clone, Copy)]
struct Signature {
    parameters: [Type; 2],
    parameter_count: usize,
    result: Option<Type>,
}

#[derive(Clone, Copy)]
struct DataSegment<'a> {
    offset: usize,
    bytes: &'a [u8],
}

impl Signature {
    const EMPTY: Self = Self {
        parameters: [Type::I32; 2],
        parameter_count: 0,
        result: None,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Import {
    Log,
    FdRead,
    FdWrite,
    Exit,
}

pub trait Host {
    fn call(
        &mut self,
        import: Import,
        args: &[Value],
        memory: &mut LinearMemory<'_>,
        handles: &mut HandleTable,
    ) -> Result<Option<Value>, Trap>;
}

pub struct Module<'a> {
    code: &'a [u8],
    types: [Signature; MAX_TYPES],
    type_count: usize,
    import_count: usize,
    imports: [Option<Import>; 4],
    memory: bool,
    data: Option<DataSegment<'a>>,
}

impl<'a> Module<'a> {
    pub fn parse(image: &'a [u8]) -> Result<Self, Error> {
        if image.len() > MAX_MODULE {
            return Err(Error::ModuleTooLarge);
        }
        if image.get(..4) != Some(MAGIC) {
            return Err(Error::BadMagic);
        }
        if image.get(4..8) != Some(VERSION) {
            return Err(Error::BadVersion);
        }
        let mut module = Self {
            code: &[],
            types: [Signature::EMPTY; MAX_TYPES],
            type_count: 0,
            import_count: 0,
            imports: [None; 4],
            memory: false,
            data: None,
        };
        let mut cursor = Cursor::new(&image[8..]);
        let mut seen = [false; 13];
        let mut profile = false;
        let mut entry = false;
        let mut function_type = None;
        while !cursor.empty() {
            let id = cursor.byte()?;
            let payload_length = cursor.leb_u32()? as usize;
            let payload = cursor.bytes(payload_length)?;
            if id == 0 {
                if parse_profile(payload)? {
                    if profile {
                        return Err(Error::DuplicateSection);
                    }
                    profile = true;
                }
                continue;
            }
            if id > 12 || seen[id as usize] {
                return Err(if id > 12 {
                    Error::UnsupportedSection
                } else {
                    Error::DuplicateSection
                });
            }
            seen[id as usize] = true;
            let mut section = Cursor::new(payload);
            match id {
                1 => module.parse_types(&mut section)?,
                2 => module.parse_imports(&mut section)?,
                3 => {
                    if section.count(1)? != 1 {
                        return Err(Error::InvalidCode);
                    }
                    function_type = Some(module.type_index(section.leb_u32()?)?);
                }
                5 => module.parse_memory(&mut section)?,
                7 => entry = module.parse_exports(&mut section)?,
                10 => {
                    if section.count(1)? != 1 || function_type.is_none() {
                        return Err(Error::InvalidCode);
                    }
                    let body_length = section.leb_u32()? as usize;
                    let body = section.bytes(body_length)?;
                    verify_body(&module, body, function_type.unwrap())?;
                    module.code = body;
                }
                11 => module.parse_data(&mut section)?,
                _ => return Err(Error::UnsupportedSection),
            }
            if !section.empty() {
                return Err(Error::InvalidSection);
            }
        }
        if module.data.is_some() && !module.memory {
            return Err(Error::InvalidMemory);
        }
        if !profile || !entry || !seen[1] || !seen[3] || !seen[7] || !seen[10] {
            return Err(if !profile {
                Error::MissingProfile
            } else {
                Error::InvalidExport
            });
        }
        Ok(module)
    }

    fn parse_types(&mut self, cursor: &mut Cursor<'_>) -> Result<(), Error> {
        let count = cursor.count(MAX_TYPES)?;
        for index in 0..count {
            if cursor.byte()? != 0x60 {
                return Err(Error::InvalidType);
            }
            let parameter_count = cursor.count(2)?;
            let mut parameters = [Type::I32; 2];
            for parameter in parameters.iter_mut().take(parameter_count) {
                *parameter = cursor.value_type()?;
            }
            let result_count = cursor.count(1)?;
            let result = if result_count == 0 {
                None
            } else {
                Some(cursor.value_type()?)
            };
            self.types[index] = Signature {
                parameters,
                parameter_count,
                result,
            };
        }
        self.type_count = count;
        Ok(())
    }

    fn parse_imports(&mut self, cursor: &mut Cursor<'_>) -> Result<(), Error> {
        let count = cursor.count(4)?;
        for index in 0..count {
            if cursor.name()? != b"norx" {
                return Err(Error::InvalidImport);
            }
            let name = cursor.name()?;
            let import = match name {
                b"log" => Import::Log,
                b"fd_read" => Import::FdRead,
                b"fd_write" => Import::FdWrite,
                b"exit" => Import::Exit,
                _ => return Err(Error::InvalidImport),
            };
            if cursor.byte()? != 0 {
                return Err(Error::InvalidImport);
            }
            let type_index = self.type_index(cursor.leb_u32()?)?;
            let signature = self.types[type_index];
            if !valid_import(import, signature) {
                return Err(Error::InvalidImport);
            }
            self.imports[index] = Some(import);
        }
        self.import_count = count;
        Ok(())
    }

    fn parse_memory(&mut self, cursor: &mut Cursor<'_>) -> Result<(), Error> {
        if cursor.count(1)? != 1 || cursor.leb_u32()? != 0 || cursor.leb_u32()? > 1 {
            return Err(Error::InvalidMemory);
        }
        self.memory = true;
        Ok(())
    }

    fn parse_data(&mut self, cursor: &mut Cursor<'a>) -> Result<(), Error> {
        if cursor.count(1)? != 1 || cursor.leb_u32()? != 0 || cursor.byte()? != 0x41 {
            return Err(Error::InvalidMemory);
        }
        let offset = cursor.leb_i32()?;
        if offset < 0 || cursor.byte()? != 0x0b {
            return Err(Error::InvalidMemory);
        }
        let offset = offset as usize;
        let length = cursor.leb_u32()? as usize;
        let bytes = cursor.bytes(length)?;
        if offset
            .checked_add(length)
            .filter(|end| *end <= MAX_MEMORY)
            .is_none()
        {
            return Err(Error::InvalidMemory);
        }
        self.data = Some(DataSegment { offset, bytes });
        Ok(())
    }

    fn parse_exports(&self, cursor: &mut Cursor<'_>) -> Result<bool, Error> {
        let count = cursor.count(4)?;
        let mut entry = false;
        for _ in 0..count {
            let name = cursor.name()?;
            let kind = cursor.byte()?;
            let index = cursor.leb_u32()? as usize;
            if name == b"norx_main" {
                if kind != 0 || index != self.import_count || entry {
                    return Err(Error::InvalidExport);
                }
                entry = true;
            } else if kind == 2 && index == 0 {
                if !self.memory {
                    return Err(Error::InvalidExport);
                }
            } else {
                return Err(Error::InvalidExport);
            }
        }
        Ok(entry)
    }

    fn type_index(&self, index: u32) -> Result<usize, Error> {
        let index = index as usize;
        if index < self.type_count {
            Ok(index)
        } else {
            Err(Error::InvalidType)
        }
    }
}

fn valid_import(import: Import, signature: Signature) -> bool {
    match import {
        Import::Log | Import::FdRead | Import::FdWrite => {
            signature.parameter_count == 2
                && signature.parameters == [Type::I32, Type::I32]
                && signature.result == Some(Type::I32)
        }
        Import::Exit => {
            signature.parameter_count == 1
                && signature.parameters[0] == Type::I32
                && signature.result.is_none()
        }
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    fn empty(&self) -> bool {
        self.position == self.bytes.len()
    }
    fn byte(&mut self) -> Result<u8, Error> {
        let byte = *self.bytes.get(self.position).ok_or(Error::Truncated)?;
        self.position += 1;
        Ok(byte)
    }
    fn bytes(&mut self, length: usize) -> Result<&'a [u8], Error> {
        let end = self.position.checked_add(length).ok_or(Error::Truncated)?;
        let bytes = self.bytes.get(self.position..end).ok_or(Error::Truncated)?;
        self.position = end;
        Ok(bytes)
    }
    fn count(&mut self, maximum: usize) -> Result<usize, Error> {
        let count = self.leb_u32()? as usize;
        if count <= maximum {
            Ok(count)
        } else {
            Err(Error::InvalidSection)
        }
    }
    fn name(&mut self) -> Result<&'a [u8], Error> {
        let length = self.count(32)?;
        let name = self.bytes(length)?;
        if name.contains(&0) {
            Err(Error::InvalidSection)
        } else {
            Ok(name)
        }
    }
    fn value_type(&mut self) -> Result<Type, Error> {
        match self.byte()? {
            0x7f => Ok(Type::I32),
            0x7e => Ok(Type::I64),
            _ => Err(Error::InvalidType),
        }
    }
    fn leb_u32(&mut self) -> Result<u32, Error> {
        let mut value = 0;
        let mut shift = 0;
        loop {
            let byte = self.byte()?;
            if shift >= 32 || (shift == 28 && byte > 0x0f) {
                return Err(Error::InvalidLeb);
            }
            value |= u32::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
        }
    }
    fn leb_i32(&mut self) -> Result<i32, Error> {
        let mut value = 0i32;
        let mut shift = 0;
        loop {
            let byte = self.byte()?;
            if shift >= 32 {
                return Err(Error::InvalidLeb);
            }
            value |= i32::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 32 && byte & 0x40 != 0 {
                    value |= !0 << shift;
                }
                return Ok(value);
            }
        }
    }

    fn leb_i64(&mut self) -> Result<i64, Error> {
        let mut value = 0i64;
        let mut shift = 0;
        loop {
            let byte = self.byte()?;
            if shift >= 64 {
                return Err(Error::InvalidLeb);
            }
            value |= i64::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 64 && byte & 0x40 != 0 {
                    value |= !0 << shift;
                }
                return Ok(value);
            }
        }
    }
}

fn parse_profile(payload: &[u8]) -> Result<bool, Error> {
    let mut cursor = Cursor::new(payload);
    if cursor.name()? != b"norx.profile" {
        return Ok(false);
    }
    if cursor.bytes(4)? != b"NRXV"
        || cursor.bytes(2)? != [1, 0]
        || cursor.bytes(2)? != [0, 0]
        || cursor.bytes(4)? != [0, 0, 0, 0]
        || !cursor.empty()
    {
        return Err(Error::InvalidProfile);
    }
    Ok(true)
}

#[derive(Clone, Copy)]
struct Control {
    kind: u8,
    height: usize,
    has_else: bool,
}

fn verify_body(module: &Module<'_>, body: &[u8], type_index: usize) -> Result<(), Error> {
    let signature = module.types[type_index];
    let mut cursor = Cursor::new(body);
    if cursor.count(0)? != 0 {
        return Err(Error::InvalidCode);
    }
    let mut stack = [Type::I32; MAX_STACK];
    let mut stack_len = 0;
    let mut controls = [Control {
        kind: 3,
        height: 0,
        has_else: false,
    }; MAX_CONTROL];
    let mut control_len = 1;
    let mut ended = false;
    while !cursor.empty() {
        match cursor.byte()? {
            0x00 => return Err(Error::UnsupportedOpcode),
            0x01 => {}
            opcode @ 0x02..=0x04 => {
                if opcode == 0x04 {
                    pop(&mut stack_len, &stack, Type::I32)?;
                }
                if cursor.byte()? != 0x40 || control_len == MAX_CONTROL {
                    return Err(Error::InvalidControl);
                }
                controls[control_len] = Control {
                    kind: opcode,
                    height: stack_len,
                    has_else: false,
                };
                control_len += 1;
            }
            0x05 => {
                if control_len == 0
                    || controls[control_len - 1].kind != 0x04
                    || controls[control_len - 1].has_else
                {
                    return Err(Error::InvalidControl);
                }
                if stack_len != controls[control_len - 1].height {
                    return Err(Error::InvalidStack);
                }
                controls[control_len - 1].has_else = true;
            }
            0x0b => {
                if control_len == 0 {
                    return Err(Error::InvalidControl);
                }
                let control = controls[control_len - 1];
                if control.kind == 3 {
                    if let Some(result) = signature.result {
                        pop(&mut stack_len, &stack, result)?;
                    }
                } else if stack_len != control.height {
                    return Err(Error::InvalidStack);
                }
                control_len -= 1;
                if control.kind == 3 {
                    if !cursor.empty() || control_len != 0 {
                        return Err(Error::InvalidCode);
                    }
                    ended = true;
                }
            }
            0x1a => {
                stack_len = stack_len.checked_sub(1).ok_or(Error::InvalidStack)?;
            }
            0x10 => {
                let index = cursor.leb_u32()? as usize;
                if index >= module.import_count {
                    return Err(Error::InvalidCode);
                }
                let import = module.imports[index].ok_or(Error::InvalidImport)?;
                if import != Import::Log {
                    return Err(Error::InvalidImport);
                }
                for _ in 0..2 {
                    pop(&mut stack_len, &stack, Type::I32)?;
                }
                push(&mut stack_len, &mut stack, Type::I32)?;
            }
            0x28 => {
                if !module.memory || cursor.leb_u32()? > 2 {
                    return Err(Error::InvalidMemory);
                }
                cursor.leb_u32()?;
                pop(&mut stack_len, &stack, Type::I32)?;
                push(&mut stack_len, &mut stack, Type::I32)?;
            }
            0x36 => {
                if !module.memory || cursor.leb_u32()? > 2 {
                    return Err(Error::InvalidMemory);
                }
                cursor.leb_u32()?;
                pop(&mut stack_len, &stack, Type::I32)?;
                pop(&mut stack_len, &stack, Type::I32)?;
            }
            0x41 => {
                cursor.leb_i32()?;
                push(&mut stack_len, &mut stack, Type::I32)?;
            }
            0x42 => {
                cursor.leb_i64()?;
                push(&mut stack_len, &mut stack, Type::I64)?;
            }
            0x45 => {
                pop(&mut stack_len, &stack, Type::I32)?;
                push(&mut stack_len, &mut stack, Type::I32)?;
            }
            0x6a..=0x6c => {
                pop(&mut stack_len, &stack, Type::I32)?;
                pop(&mut stack_len, &stack, Type::I32)?;
                push(&mut stack_len, &mut stack, Type::I32)?;
            }
            0x7c => {
                pop(&mut stack_len, &stack, Type::I64)?;
                pop(&mut stack_len, &stack, Type::I64)?;
                push(&mut stack_len, &mut stack, Type::I64)?;
            }
            _ => return Err(Error::UnsupportedOpcode),
        }
    }
    if ended {
        Ok(())
    } else {
        Err(Error::InvalidCode)
    }
}

fn pop(length: &mut usize, stack: &[Type; MAX_STACK], expected: Type) -> Result<(), Error> {
    if *length == 0 {
        return Err(Error::InvalidStack);
    }
    *length -= 1;
    if stack[*length] == expected {
        Ok(())
    } else {
        Err(Error::TypeMismatch)
    }
}

fn push(length: &mut usize, stack: &mut [Type; MAX_STACK], value: Type) -> Result<(), Error> {
    if *length == MAX_STACK {
        return Err(Error::InvalidStack);
    }
    stack[*length] = value;
    *length += 1;
    Ok(())
}

fn find_control_end(bytes: &[u8], start: usize) -> Result<(Option<usize>, usize), Trap> {
    let mut cursor = Cursor::new(&bytes[start..]);
    let mut depth = 0usize;
    let mut else_position = None;
    loop {
        let position = start + cursor.position;
        let opcode = cursor.byte().map_err(|_| Trap::TypeMismatch)?;
        match opcode {
            0x02..=0x04 => {
                if cursor.byte().map_err(|_| Trap::TypeMismatch)? != 0x40 {
                    return Err(Trap::TypeMismatch);
                }
                depth += 1;
            }
            0x05 if depth == 0 => {
                if else_position.is_some() {
                    return Err(Trap::TypeMismatch);
                }
                else_position = Some(position);
            }
            0x0b => {
                if depth == 0 {
                    return Ok((else_position, position));
                }
                depth -= 1;
            }
            0x10 | 0x20..=0x22 => {
                cursor.leb_u32().map_err(|_| Trap::TypeMismatch)?;
            }
            0x28 | 0x36 => {
                cursor.leb_u32().map_err(|_| Trap::TypeMismatch)?;
                cursor.leb_u32().map_err(|_| Trap::TypeMismatch)?;
            }
            0x41 => {
                cursor.leb_i32().map_err(|_| Trap::TypeMismatch)?;
            }
            _ => {}
        }
    }
}

pub struct Instance<'module, 'memory> {
    module: &'module Module<'module>,
    memory: LinearMemory<'memory>,
    handles: HandleTable,
    stack: [Value; MAX_STACK],
    stack_len: usize,
    fuel: u64,
    cancelled: bool,
}

pub struct LinearMemory<'a> {
    bytes: &'a mut [u8; MAX_MEMORY],
    live: bool,
}

impl<'a> LinearMemory<'a> {
    fn new(module: &Module<'_>, bytes: &'a mut [u8; MAX_MEMORY]) -> Self {
        bytes.fill(0);
        if let Some(data) = module.data {
            bytes[data.offset..data.offset + data.bytes.len()].copy_from_slice(data.bytes);
        }
        Self { bytes, live: true }
    }

    fn range(&self, address: usize, offset: usize, length: usize) -> Result<usize, Trap> {
        if !self.live {
            return Err(Trap::Cancelled);
        }
        address
            .checked_add(offset)
            .and_then(|start| start.checked_add(length))
            .filter(|end| *end <= self.bytes.len())
            .and_then(|end| end.checked_sub(length))
            .ok_or(Trap::MemoryFault)
    }

    fn load_i32(&self, address: usize, offset: usize) -> Result<i32, Trap> {
        let start = self.range(address, offset, 4)?;
        Ok(i32::from_le_bytes(
            self.bytes[start..start + 4].try_into().unwrap(),
        ))
    }

    fn bytes(&self, address: usize, length: usize) -> Result<&[u8], Trap> {
        let start = self.range(address, 0, length)?;
        Ok(&self.bytes[start..start + length])
    }

    fn store_i32(&mut self, address: usize, offset: usize, value: i32) -> Result<(), Trap> {
        let start = self.range(address, offset, 4)?;
        self.bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    fn revoke(&mut self) {
        self.live = false;
    }
}

pub struct HandleTable {
    slots: [Option<u32>; MAX_HANDLES],
}

impl HandleTable {
    const fn new() -> Self {
        Self {
            slots: [None; MAX_HANDLES],
        }
    }

    pub fn insert(&mut self, value: u32) -> Option<u32> {
        let (index, slot) = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())?;
        *slot = Some(value);
        Some(index as u32)
    }

    pub fn valid(&self, handle: u32) -> bool {
        self.slots.get(handle as usize).is_some_and(Option::is_some)
    }

    fn revoke_all(&mut self) {
        self.slots.fill(None);
    }
}

impl<'module, 'memory> Instance<'module, 'memory> {
    pub fn new(module: &'module Module<'module>, memory: &'memory mut [u8; MAX_MEMORY]) -> Self {
        Self {
            module,
            memory: LinearMemory::new(module, memory),
            handles: HandleTable::new(),
            stack: [Value::I32(0); MAX_STACK],
            stack_len: 0,
            fuel: MAX_FUEL,
            cancelled: false,
        }
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.memory.revoke();
        self.handles.revoke_all();
    }

    pub fn insert_handle(&mut self, value: u32) -> Option<u32> {
        self.handles.insert(value)
    }

    pub fn run<H: Host>(&mut self, host: &mut H, fuel: u64) -> Result<i32, Trap> {
        if self.cancelled {
            return Err(Trap::Cancelled);
        }
        self.fuel = fuel.min(MAX_FUEL);
        self.stack_len = 0;
        let mut cursor = Cursor::new(self.module.code);
        if cursor.leb_u32().map_err(|_| Trap::TypeMismatch)? != 0 {
            return Err(Trap::TypeMismatch);
        }
        let mut control_ends = [0usize; MAX_CONTROL];
        let mut control_kinds = [0u8; MAX_CONTROL];
        let mut control_len = 0;
        while !cursor.empty() {
            if self.fuel == 0 {
                return Err(Trap::FuelExhausted);
            }
            self.fuel -= 1;
            match cursor.byte().map_err(|_| Trap::TypeMismatch)? {
                0x00 => return Err(Trap::Unreachable),
                0x01 => {}
                opcode @ 0x02..=0x04 => {
                    let condition = if opcode == 0x04 {
                        Some(self.pop_i32()?)
                    } else {
                        None
                    };
                    if cursor.byte().map_err(|_| Trap::TypeMismatch)? != 0x40 {
                        return Err(Trap::TypeMismatch);
                    }
                    let (else_position, end_position) =
                        find_control_end(self.module.code, cursor.position)?;
                    if control_len == MAX_CONTROL {
                        return Err(Trap::StackOverflow);
                    }
                    control_ends[control_len] = end_position;
                    control_kinds[control_len] = opcode;
                    control_len += 1;
                    if condition == Some(0) {
                        if let Some(else_position) = else_position {
                            cursor.position = else_position + 1;
                        } else {
                            cursor.position = end_position + 1;
                            control_len -= 1;
                        }
                    }
                }
                0x05 => {
                    if control_len == 0 || control_kinds[control_len - 1] != 0x04 {
                        return Err(Trap::TypeMismatch);
                    }
                    cursor.position = control_ends[control_len - 1] + 1;
                    control_len -= 1;
                }
                0x10 => {
                    if cursor.leb_u32().map_err(|_| Trap::TypeMismatch)? != 0 {
                        return Err(Trap::HostDenied);
                    }
                    let len = self.pop_i32()?;
                    let ptr = self.pop_i32()?;
                    let result = host.call(
                        Import::Log,
                        &[Value::I32(ptr), Value::I32(len)],
                        &mut self.memory,
                        &mut self.handles,
                    )?;
                    if let Some(result) = result {
                        self.push(result)?;
                    }
                }
                0x1a => {
                    self.pop()?;
                }
                0x28 => {
                    let _align = cursor.leb_u32().map_err(|_| Trap::TypeMismatch)?;
                    let offset = cursor.leb_u32().map_err(|_| Trap::TypeMismatch)? as usize;
                    let address = self.pop_i32()? as usize;
                    self.push(Value::I32(self.memory.load_i32(address, offset)?))?;
                }
                0x36 => {
                    let _align = cursor.leb_u32().map_err(|_| Trap::TypeMismatch)?;
                    let offset = cursor.leb_u32().map_err(|_| Trap::TypeMismatch)? as usize;
                    let value = self.pop_i32()?;
                    let address = self.pop_i32()? as usize;
                    self.memory.store_i32(address, offset, value)?;
                }
                0x41 => self.push(Value::I32(
                    cursor.leb_i32().map_err(|_| Trap::TypeMismatch)?,
                ))?,
                0x42 => self.push(Value::I64(
                    cursor.leb_i64().map_err(|_| Trap::TypeMismatch)?,
                ))?,
                0x45 => {
                    let value = self.pop_i32()?;
                    self.push(Value::I32((value == 0) as i32))?;
                }
                opcode @ 0x6a..=0x6c => {
                    let right = self.pop_i32()?;
                    let left = self.pop_i32()?;
                    let value = match opcode {
                        0x6a => left.wrapping_add(right),
                        0x6b => left.wrapping_sub(right),
                        _ => left.wrapping_mul(right),
                    };
                    self.push(Value::I32(value))?;
                }
                0x7c => {
                    let right = self.pop_i64()?;
                    let left = self.pop_i64()?;
                    self.push(Value::I64(left.wrapping_add(right)))?;
                }
                0x0b => {
                    if control_len != 0 && control_ends[control_len - 1] == cursor.position - 1 {
                        control_len -= 1;
                    } else {
                        return match self.pop()? {
                            Value::I32(value) => Ok(value),
                            _ => Err(Trap::TypeMismatch),
                        };
                    }
                }
                _ => return Err(Trap::TypeMismatch),
            }
        }
        Err(Trap::TypeMismatch)
    }
    fn push(&mut self, value: Value) -> Result<(), Trap> {
        if self.stack_len == MAX_STACK {
            return Err(Trap::StackOverflow);
        }
        self.stack[self.stack_len] = value;
        self.stack_len += 1;
        Ok(())
    }
    fn pop(&mut self) -> Result<Value, Trap> {
        if self.stack_len == 0 {
            return Err(Trap::StackUnderflow);
        }
        self.stack_len -= 1;
        Ok(self.stack[self.stack_len])
    }
    fn pop_i32(&mut self) -> Result<i32, Trap> {
        match self.pop()? {
            Value::I32(value) => Ok(value),
            Value::I64(_) => Err(Trap::TypeMismatch),
        }
    }

    fn pop_i64(&mut self) -> Result<i64, Trap> {
        match self.pop()? {
            Value::I64(value) => Ok(value),
            Value::I32(_) => Err(Trap::TypeMismatch),
        }
    }
}

struct ContractHost {
    calls: usize,
}

impl Host for ContractHost {
    fn call(
        &mut self,
        import: Import,
        args: &[Value],
        memory: &mut LinearMemory<'_>,
        _handles: &mut HandleTable,
    ) -> Result<Option<Value>, Trap> {
        if import != Import::Log
            || args != [Value::I32(0), Value::I32(12)]
            || memory.bytes(0, 12).ok() != Some(b"norx sample\n")
        {
            return Err(Trap::HostDenied);
        }
        self.calls += 1;
        Ok(Some(Value::I32(0)))
    }
}

static CONTRACT: &[u8] = &[
    0, 97, 115, 109, 1, 0, 0, 0, 0, 25, 12, 110, 111, 114, 120, 46, 112, 114, 111, 102, 105, 108,
    101, 78, 82, 88, 86, 1, 0, 0, 0, 0, 0, 0, 0, 1, 11, 2, 96, 2, 127, 127, 1, 127, 96, 0, 1, 127,
    2, 12, 1, 4, 110, 111, 114, 120, 3, 108, 111, 103, 0, 0, 3, 2, 1, 1, 5, 3, 1, 0, 1, 7, 13, 1,
    9, 110, 111, 114, 120, 95, 109, 97, 105, 110, 0, 1, 10, 18, 1, 16, 0, 65, 0, 65, 12, 16, 0, 26,
    65, 1, 4, 64, 11, 65, 7, 11, 11, 18, 1, 0, 65, 0, 11, 12, 110, 111, 114, 120, 32, 115, 97, 109,
    112, 108, 101, 10,
];

static MEMORY_TRAP: &[u8] = &[
    0, 97, 115, 109, 1, 0, 0, 0, 0, 25, 12, 110, 111, 114, 120, 46, 112, 114, 111, 102, 105, 108,
    101, 78, 82, 88, 86, 1, 0, 0, 0, 0, 0, 0, 0, 1, 5, 1, 96, 0, 1, 127, 3, 2, 1, 0, 5, 3, 1, 0, 1,
    7, 13, 1, 9, 110, 111, 114, 120, 95, 109, 97, 105, 110, 0, 0, 10, 11, 1, 9, 0, 65, 255, 255, 3,
    40, 0, 0, 11,
];
static mut CONTRACT_MEMORY: [u8; MAX_MEMORY] = [0; MAX_MEMORY];

pub fn contract_self_check() {
    let module = Module::parse(CONTRACT).unwrap();
    let mut host = ContractHost { calls: 0 };
    let memory = unsafe { &mut *core::ptr::addr_of_mut!(CONTRACT_MEMORY) };
    let mut instance = Instance::new(&module, memory);
    assert_eq!(instance.run(&mut host, 100), Ok(7));
    assert_eq!(host.calls, 1);
    let handle = instance.insert_handle(42).unwrap();
    assert!(instance.handles.valid(handle));
    instance.cancel();
    assert!(!instance.handles.valid(handle));
    assert_eq!(instance.run(&mut host, 100), Err(Trap::Cancelled));
    assert!(matches!(
        Module::parse(&CONTRACT[..CONTRACT.len() - 1]),
        Err(Error::Truncated)
    ));

    let module = Module::parse(MEMORY_TRAP).unwrap();
    let mut host = ContractHost { calls: 0 };
    let memory = unsafe { &mut *core::ptr::addr_of_mut!(CONTRACT_MEMORY) };
    let mut instance = Instance::new(&module, memory);
    assert_eq!(instance.run(&mut host, 100), Err(Trap::MemoryFault));
}

pub fn profile_self_check() -> u64 {
    let start = crate::time::ticks();
    for _ in 0..PROFILE_ITERATIONS {
        let module = Module::parse(CONTRACT).unwrap();
        let mut host = ContractHost { calls: 0 };
        let memory = unsafe { &mut *core::ptr::addr_of_mut!(CONTRACT_MEMORY) };
        let mut instance = Instance::new(&module, memory);
        assert_eq!(instance.run(&mut host, 100), Ok(7));
    }
    crate::time::ticks().wrapping_sub(start)
}

pub fn sample_profile_self_check() -> SampleMetrics {
    let start = crate::time::ticks();
    let module = Module::parse(CONTRACT).unwrap();
    let mut host = ContractHost { calls: 0 };
    let memory = unsafe { &mut *core::ptr::addr_of_mut!(CONTRACT_MEMORY) };
    let mut instance = Instance::new(&module, memory);
    assert_eq!(instance.run(&mut host, 100), Ok(7));
    SampleMetrics {
        startup_ticks: crate::time::ticks().wrapping_sub(start),
        module_bytes: CONTRACT.len(),
        linear_memory_bytes: MAX_MEMORY,
        host_calls: host.calls,
    }
}
