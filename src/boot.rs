use core::str;

#[cfg(target_arch = "x86_64")]
pub const MULTIBOOT2_BOOTLOADER_MAGIC: u32 = 0x36d7_6289;

const MAX_MEMORY_REGIONS: usize = 64;
const MAX_RESERVED_REGIONS: usize = 32;
const MAX_MODULES: usize = 8;
const CMDLINE_MAX: usize = 128;
#[cfg(target_arch = "x86_64")]
const MAX_MULTIBOOT_INFO: usize = 16 * 1024 * 1024;
#[cfg(target_arch = "aarch64")]
const MAX_FDT_SIZE: usize = 16 * 1024 * 1024;

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    r#"
    .section .multiboot,"a"
    .align 8
norx_multiboot2_header:
    .long 0xe85250d6
    .long 0
    .long norx_multiboot2_header_end - norx_multiboot2_header
    .long -(0xe85250d6 + (norx_multiboot2_header_end - norx_multiboot2_header))

    .short 1
    .short 0
    .long 20
    .long 6
    .long 8
    .long 1
    .align 8

    .short 5
    .short 0
    .long 20
    .long 0
    .long 0
    .long 0
    .align 8

    .short 7
    .short 0
    .long 8
    .align 8

    .short 9
    .short 0
    .long 12
    .long _start
    .align 8

    .short 0
    .short 0
    .long 8
norx_multiboot2_header_end:

    .section .text.boot,"ax"
    .global _start
    .type _start,@function
_start:
    cli
    cld
    mov edi, eax
    mov rsi, rbx
    call norx_multiboot2_entry
1:
    hlt
    jmp 1b
    .size _start, .-_start
"#
);

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    r#"
    .section .image_header,"a"
    .align 3
    .long 0
    .long 0
    .quad 0x80000
    .quad 0
    .quad 0
    .quad 0
    .quad 0
    .quad 0
    .long 0x644d5241
    .long 0

    .section .text.boot,"ax"
    .global _start
    .type _start,%function
_start:
    bl norx_fdt_entry
1:
    wfi
    b 1b
    .size _start, .-_start
"#
);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    X86_64,
    #[cfg_attr(target_arch = "x86_64", allow(dead_code))]
    Aarch64,
    Unknown,
}

impl Architecture {
    pub const fn name(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy)]
pub enum PixelFormat {
    Rgb,
    Bgr,
}

#[derive(Clone, Copy)]
pub struct RawFramebuffer {
    pub base: *mut u8,
    pub size: usize,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
}

#[derive(Clone, Copy)]
pub struct MemoryRegion {
    pub base: u64,
    pub length: u64,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct Module {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Copy)]
pub struct BootInfo {
    pub architecture: Architecture,
    pub memory: [MemoryRegion; MAX_MEMORY_REGIONS],
    pub memory_len: usize,
    pub reserved: [MemoryRegion; MAX_RESERVED_REGIONS],
    pub reserved_len: usize,
    pub modules: [Module; MAX_MODULES],
    pub modules_len: usize,
    pub framebuffer: Option<RawFramebuffer>,
    pub cmdline: [u8; CMDLINE_MAX],
    pub cmdline_len: usize,
    pub handoff_address: u64,
}

impl BootInfo {
    const fn empty(architecture: Architecture) -> Self {
        Self {
            architecture,
            memory: [MemoryRegion { base: 0, length: 0 }; MAX_MEMORY_REGIONS],
            memory_len: 0,
            reserved: [MemoryRegion { base: 0, length: 0 }; MAX_RESERVED_REGIONS],
            reserved_len: 0,
            modules: [Module { start: 0, end: 0 }; MAX_MODULES],
            modules_len: 0,
            framebuffer: None,
            cmdline: [0; CMDLINE_MAX],
            cmdline_len: 0,
            handoff_address: 0,
        }
    }

    pub fn cmdline(&self) -> &str {
        str::from_utf8(&self.cmdline[..self.cmdline_len]).unwrap_or("")
    }
}

static mut INFO: BootInfo = BootInfo::empty(Architecture::Unknown);

extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
}

pub fn info() -> BootInfo {
    unsafe { INFO }
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn norx_multiboot2_entry(magic: u32, info_address: u64) -> ! {
    crate::arch::init();
    if !init_multiboot2(magic, info_address) {
        crate::drivers::serial::write_str("Norx: invalid Multiboot2 hand-off\r\n");
        crate::arch::halt();
    }
    crate::kernel_start()
}

#[cfg(target_arch = "aarch64")]
#[no_mangle]
pub extern "C" fn norx_fdt_entry(fdt_address: u64) -> ! {
    crate::arch::init();
    if !init_fdt(fdt_address) {
        crate::drivers::serial::write_str("Norx: invalid ARM64 FDT hand-off\r\n");
        crate::arch::halt();
    }
    crate::kernel_start()
}

#[cfg(target_arch = "x86_64")]
pub fn init_multiboot2(magic: u32, info_address: u64) -> bool {
    if magic != MULTIBOOT2_BOOTLOADER_MAGIC {
        return false;
    }
    let Some(info) = (unsafe { parse_multiboot2(info_address as usize) }) else {
        return false;
    };
    unsafe { INFO = info };
    true
}

#[cfg(target_arch = "aarch64")]
pub fn init_fdt(fdt_address: u64) -> bool {
    let Some(info) = (unsafe { parse_fdt(fdt_address as usize) }) else {
        return false;
    };
    unsafe { INFO = info };
    true
}

fn reserve_kernel(info: &mut BootInfo) {
    let start = (&raw const __kernel_start) as u64;
    let end = (&raw const __kernel_end) as u64;
    if end > start {
        add_reserved(info, start, end - start);
    }
}

fn add_memory(info: &mut BootInfo, base: u64, length: u64) {
    if length == 0 || base.checked_add(length).is_none() {
        return;
    }
    if info.memory_len < info.memory.len() {
        info.memory[info.memory_len] = MemoryRegion { base, length };
        info.memory_len += 1;
    }
}

fn add_reserved(info: &mut BootInfo, base: u64, length: u64) {
    if length == 0 || base.checked_add(length).is_none() {
        return;
    }
    if info.reserved_len < info.reserved.len() {
        info.reserved[info.reserved_len] = MemoryRegion { base, length };
        info.reserved_len += 1;
    }
}

fn add_module(info: &mut BootInfo, start: u64, end: u64) {
    if end <= start {
        return;
    }
    add_reserved(info, start, end - start);
    if info.modules_len < info.modules.len() {
        info.modules[info.modules_len] = Module { start, end };
        info.modules_len += 1;
    }
}

fn framebuffer(
    base: u64,
    pitch: u64,
    width: u64,
    height: u64,
    bpp: u8,
    format: PixelFormat,
) -> Option<RawFramebuffer> {
    let minimum_pitch = width.checked_mul(4)?;
    if base == 0 || width == 0 || height == 0 || bpp != 32 || pitch < minimum_pitch {
        return None;
    }
    let stride = usize::try_from(pitch).ok()?;
    let width = u32::try_from(width).ok()?;
    let height = u32::try_from(height).ok()?;
    let size = stride.checked_mul(height as usize)?;
    Some(RawFramebuffer {
        base: base as *mut u8,
        size,
        width,
        height,
        stride,
        format,
    })
}

#[cfg(target_arch = "x86_64")]
unsafe fn parse_multiboot2(address: usize) -> Option<BootInfo> {
    let total_size = read_u32(address)? as usize;
    if !(16..=MAX_MULTIBOOT_INFO).contains(&total_size) {
        return None;
    }
    let end = address.checked_add(total_size)?;
    let mut info = BootInfo::empty(Architecture::X86_64);
    let mut offset = 8usize;
    let mut framebuffer_info = None;

    while offset.checked_add(8)? <= total_size {
        let tag = address.checked_add(offset)?;
        let tag_type = read_u32(tag)?;
        let size = read_u32(tag + 4)? as usize;
        if size < 8 || offset.checked_add(size)? > total_size {
            return None;
        }
        match tag_type {
            1 => copy_c_string(read_ptr(tag + 8)?, &mut info.cmdline, &mut info.cmdline_len),
            3 if size >= 16 => {
                let start = read_u32(tag + 8)? as u64;
                let end = read_u32(tag + 12)? as u64;
                add_module(&mut info, start, end);
            }
            6 if size >= 16 => {
                let entry_size = read_u32(tag + 8)? as usize;
                if entry_size < 24 || tag + 16 > end {
                    return None;
                }
                let mut entry = tag + 16;
                let entries_end = tag + size;
                while entry.checked_add(24)? <= entries_end {
                    let base = read_u64(entry)?;
                    let length = read_u64(entry + 8)?;
                    if read_u32(entry + 16)? == 1 {
                        add_memory(&mut info, base, length);
                    }
                    entry = entry.checked_add(entry_size)?;
                }
            }
            8 if size >= 32 => {
                let base = read_u64(tag + 8)?;
                let pitch = read_u32(tag + 16)? as u64;
                let width = read_u32(tag + 20)? as u64;
                let height = read_u32(tag + 24)? as u64;
                let bpp = read_u8(tag + 28)?;
                if read_u8(tag + 29)? == 1 && size >= 38 {
                    let red = read_u8(tag + 32)?;
                    let blue = read_u8(tag + 36)?;
                    if red == 0 && blue == 16 {
                        framebuffer_info =
                            framebuffer(base, pitch, width, height, bpp, PixelFormat::Bgr);
                    } else if red == 16 && blue == 0 {
                        framebuffer_info =
                            framebuffer(base, pitch, width, height, bpp, PixelFormat::Rgb);
                    }
                }
            }
            0 => break,
            _ => {}
        }
        offset = offset.checked_add((size + 7) & !7)?;
    }

    if info.memory_len == 0 {
        return None;
    }
    info.framebuffer = framebuffer_info;
    info.handoff_address = address as u64;
    add_reserved(&mut info, address as u64, total_size as u64);
    reserve_kernel(&mut info);
    Some(info)
}

#[cfg(target_arch = "aarch64")]
unsafe fn parse_fdt(address: usize) -> Option<BootInfo> {
    if read_be32(address)? != 0xd00d_feed {
        return None;
    }
    let total_size = read_be32(address + 4)? as usize;
    if !(40..=MAX_FDT_SIZE).contains(&total_size) {
        return None;
    }
    let structure_offset = read_be32(address + 8)? as usize;
    let strings_offset = read_be32(address + 12)? as usize;
    let structure_size = read_be32(address + 36)? as usize;
    let strings_size = read_be32(address + 32)? as usize;
    let structure = address.checked_add(structure_offset)?;
    let structure_end = structure.checked_add(structure_size)?;
    let strings = address.checked_add(strings_offset)?;
    let strings_end = strings.checked_add(strings_size)?;
    let blob_end = address.checked_add(total_size)?;
    if structure < address
        || structure_end > blob_end
        || strings < address
        || strings_end > blob_end
    {
        return None;
    }

    let mut info = BootInfo::empty(Architecture::Aarch64);
    let mut cursor = structure;
    let mut depth = 0usize;
    let mut nodes = [NodeKind::Other; 16];
    let mut address_cells = 2usize;
    let mut size_cells = 2usize;
    let mut initrd_start = 0u64;
    let mut initrd_end = 0u64;
    let mut framebuffer_base = 0u64;
    let mut framebuffer_size = 0u64;
    let mut framebuffer_width = 0u64;
    let mut framebuffer_height = 0u64;
    let mut framebuffer_stride = 0u64;
    let mut framebuffer_format = [0u8; 16];
    let mut framebuffer_format_len = 0usize;

    loop {
        let token = read_be32(cursor)?;
        cursor = cursor.checked_add(4)?;
        match token {
            1 => {
                let (name_len, kind) = node_kind(cursor);
                cursor = align4(cursor.checked_add(name_len + 1)?)?;
                depth += 1;
                nodes[depth.min(nodes.len() - 1)] = kind;
            }
            2 => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            3 => {
                let length = read_be32(cursor)? as usize;
                let name_offset = read_be32(cursor + 4)? as usize;
                cursor = cursor.checked_add(8)?;
                let data = cursor;
                cursor = align4(cursor.checked_add(length)?)?;
                if cursor > structure_end || strings + name_offset >= strings_end {
                    return None;
                }
                let name = strings + name_offset;
                let kind = nodes[depth.min(nodes.len() - 1)];
                if depth == 1 && c_string_eq(name, b"#address-cells") {
                    address_cells = read_be32(data)? as usize;
                } else if depth == 1 && c_string_eq(name, b"#size-cells") {
                    size_cells = read_be32(data)? as usize;
                } else if kind == NodeKind::Memory && c_string_eq(name, b"reg") {
                    parse_reg(&mut info, data, length, address_cells, size_cells)?;
                } else if kind == NodeKind::Chosen && c_string_eq(name, b"bootargs") {
                    copy_bytes(&mut info.cmdline, &mut info.cmdline_len, data, length);
                } else if kind == NodeKind::Chosen && c_string_eq(name, b"linux,initrd-start") {
                    initrd_start = read_cells(data, length, size_cells)?;
                } else if kind == NodeKind::Chosen && c_string_eq(name, b"linux,initrd-end") {
                    initrd_end = read_cells(data, length, size_cells)?;
                } else if kind == NodeKind::SimpleFramebuffer && c_string_eq(name, b"reg") {
                    framebuffer_base = read_cells(data, length, address_cells)?;
                    let offset = address_cells.checked_mul(4)?;
                    framebuffer_size = read_cells(
                        data.checked_add(offset)?,
                        length.checked_sub(offset)?,
                        size_cells,
                    )?;
                } else if kind == NodeKind::SimpleFramebuffer && c_string_eq(name, b"width") {
                    framebuffer_width = read_be32(data)? as u64;
                } else if kind == NodeKind::SimpleFramebuffer && c_string_eq(name, b"height") {
                    framebuffer_height = read_be32(data)? as u64;
                } else if kind == NodeKind::SimpleFramebuffer && c_string_eq(name, b"stride") {
                    framebuffer_stride = read_be32(data)? as u64;
                } else if kind == NodeKind::SimpleFramebuffer && c_string_eq(name, b"format") {
                    framebuffer_format_len = copy_bytes(
                        &mut framebuffer_format,
                        &mut framebuffer_format_len,
                        data,
                        length,
                    );
                }
            }
            4 => {}
            9 => break,
            _ => return None,
        }
        if cursor >= structure_end {
            break;
        }
    }

    if info.memory_len == 0 {
        return None;
    }
    if initrd_end > initrd_start {
        add_module(&mut info, initrd_start, initrd_end);
    }
    if framebuffer_base != 0 && framebuffer_size != 0 {
        let format = if framebuffer_format[..framebuffer_format_len].starts_with(b"r8g8b8a8") {
            PixelFormat::Rgb
        } else {
            PixelFormat::Bgr
        };
        if let Some(fb) = framebuffer(
            framebuffer_base,
            framebuffer_stride,
            framebuffer_width,
            framebuffer_height,
            32,
            format,
        ) {
            if fb.size as u64 <= framebuffer_size {
                info.framebuffer = Some(fb);
            }
        }
    }
    info.handoff_address = address as u64;
    add_reserved(&mut info, address as u64, total_size as u64);
    reserve_kernel(&mut info);
    Some(info)
}

#[cfg(target_arch = "aarch64")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    Chosen,
    Memory,
    SimpleFramebuffer,
    Other,
}

#[cfg(target_arch = "aarch64")]
unsafe fn node_kind(address: usize) -> (usize, NodeKind) {
    let mut len = 0usize;
    while len < 256 && read_u8(address + len).unwrap_or(0) != 0 {
        len += 1;
    }
    let kind = if c_string_eq(address, b"chosen") {
        NodeKind::Chosen
    } else if read_u8(address).unwrap_or(0) == b'm' && c_string_starts_with(address, b"memory") {
        NodeKind::Memory
    } else if c_string_starts_with(address, b"simple-framebuffer") {
        NodeKind::SimpleFramebuffer
    } else {
        NodeKind::Other
    };
    (len, kind)
}

#[cfg(target_arch = "aarch64")]
unsafe fn parse_reg(
    info: &mut BootInfo,
    data: usize,
    length: usize,
    address_cells: usize,
    size_cells: usize,
) -> Option<()> {
    let entry_cells = address_cells.checked_add(size_cells)?;
    let entry_bytes = entry_cells.checked_mul(4)?;
    if entry_cells == 0 || entry_bytes == 0 || !length.is_multiple_of(entry_bytes) {
        return None;
    }
    let mut offset = 0usize;
    while offset < length {
        let base = read_cells(data + offset, address_cells * 4, address_cells)?;
        let length = read_cells(
            data + offset + address_cells * 4,
            size_cells * 4,
            size_cells,
        )?;
        add_memory(info, base, length);
        offset += entry_bytes;
    }
    Some(())
}

#[cfg(target_arch = "aarch64")]
unsafe fn read_cells(address: usize, length: usize, cells: usize) -> Option<u64> {
    if cells == 0 || cells > 2 || length < cells.checked_mul(4)? {
        return None;
    }
    let mut value = 0u64;
    for i in 0..cells {
        value = value
            .checked_shl(32)?
            .checked_add(read_be32(address + i * 4)? as u64)?;
    }
    Some(value)
}

#[cfg(target_arch = "x86_64")]
unsafe fn copy_c_string(address: usize, out: &mut [u8], len: &mut usize) {
    let mut index = 0;
    while index < out.len() {
        let byte = read_u8(address + index).unwrap_or(0);
        if byte == 0 {
            break;
        }
        out[index] = byte;
        index += 1;
    }
    *len = index;
}

#[cfg(target_arch = "aarch64")]
unsafe fn copy_bytes(out: &mut [u8], len: &mut usize, address: usize, size: usize) -> usize {
    let copied = size.min(out.len());
    for (index, byte) in out.iter_mut().enumerate().take(copied) {
        *byte = read_u8(address + index).unwrap_or(0);
    }
    let copied = out[..copied]
        .iter()
        .rposition(|byte| *byte != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    *len = copied;
    copied
}

#[cfg(target_arch = "aarch64")]
unsafe fn c_string_eq(address: usize, expected: &[u8]) -> bool {
    c_string_starts_with(address, expected) && read_u8(address + expected.len()).unwrap_or(1) == 0
}

#[cfg(target_arch = "aarch64")]
unsafe fn c_string_starts_with(address: usize, expected: &[u8]) -> bool {
    expected
        .iter()
        .enumerate()
        .all(|(index, byte)| read_u8(address + index).unwrap_or(0) == *byte)
}

#[cfg(target_arch = "aarch64")]
unsafe fn align4(value: usize) -> Option<usize> {
    value.checked_add(3).map(|value| value & !3)
}

#[cfg(target_arch = "x86_64")]
unsafe fn read_ptr(address: usize) -> Option<usize> {
    Some(read_u32(address)? as usize)
}

unsafe fn read_u8(address: usize) -> Option<u8> {
    Some((address as *const u8).read_unaligned())
}

#[cfg(target_arch = "x86_64")]
unsafe fn read_u32(address: usize) -> Option<u32> {
    Some((address as *const u32).read_unaligned())
}

#[cfg(target_arch = "x86_64")]
unsafe fn read_u64(address: usize) -> Option<u64> {
    Some((address as *const u64).read_unaligned())
}

#[cfg(target_arch = "aarch64")]
unsafe fn read_be32(address: usize) -> Option<u32> {
    Some(u32::from_be((address as *const u32).read_unaligned()))
}
