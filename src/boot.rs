use core::str;

#[cfg(target_arch = "x86_64")]
pub const MULTIBOOT2_BOOTLOADER_MAGIC: u32 = 0x36d7_6289;

const MAX_MEMORY_REGIONS: usize = 64;
const MAX_RESERVED_REGIONS: usize = 32;
const MAX_MODULES: usize = 8;
const CMDLINE_MAX: usize = 128;
#[cfg(target_arch = "x86_64")]
const MAX_MULTIBOOT_INFO: usize = 16 * 1024 * 1024;
#[cfg(target_arch = "x86_64")]
const EFI_MEMORY_MAP_SIZE: usize = 64 * 1024;
#[cfg(target_arch = "aarch64")]
const MAX_FDT_SIZE: usize = 16 * 1024 * 1024;

#[cfg(target_arch = "x86_64")]
static mut EFI_MEMORY_MAP: [u8; EFI_MEMORY_MAP_SIZE] = [0; EFI_MEMORY_MAP_SIZE];

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
    .long 24
    .long 6
    .long 8
    .long 1
    .long 12
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
    lea rsp, [rip + norx_boot_stack_top]
    mov edi, eax
    mov rsi, rbx
    call norx_multiboot2_entry
1:
    hlt
    jmp 1b
    .size _start, .-_start

    .section .bss,"aw",@nobits
    .align 16
norx_boot_stack:
    .skip 131072
norx_boot_stack_top:
"#
);

#[cfg(all(target_arch = "aarch64", target_os = "uefi"))]
core::arch::global_asm!(
    r#"
    .section .bss,"aw"
    .align 12
norx_efi_stack:
    .skip 131072
norx_efi_stack_top:

    .text
    .align 2
    .global efi_main
efi_main:
    adrp x16, norx_efi_stack_top
    add x16, x16, :lo12:norx_efi_stack_top
    mov sp, x16
    b norx_efi_main
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub bytes_per_pixel: usize,
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
    pub efi_system_table: u64,
    #[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
    pub efi_runtime_el: u8,
    #[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
    pub efi_runtime_vbar: u64,
    #[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
    pub efi_runtime_sp_el0: u64,
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
            efi_system_table: 0,
            efi_runtime_el: 0,
            efi_runtime_vbar: 0,
            efi_runtime_sp_el0: 0,
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

pub fn efi_system_table() -> u64 {
    info().efi_system_table
}

#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub fn efi_runtime_context() -> (u8, u64, u64) {
    let boot = info();
    (
        boot.efi_runtime_el,
        boot.efi_runtime_vbar,
        boot.efi_runtime_sp_el0,
    )
}

pub fn contract_self_check() {
    assert!(framebuffer(0, 12, 4, 1, 24, PixelFormat::Rgb).is_none());
    assert!(framebuffer(0x1000, 0, 4, 1, 24, PixelFormat::Rgb).is_none());
    assert!(framebuffer(0x1000, 11, 4, 1, 24, PixelFormat::Rgb).is_none());
    assert!(framebuffer(0x1000, 12, 4, 1, 24, PixelFormat::Rgb).is_some());
    assert!(framebuffer(0x1000, u64::MAX, u64::MAX, 1, 32, PixelFormat::Rgb).is_none());

    #[cfg(target_arch = "x86_64")]
    {
        let mut malformed = [0u8; 32];
        malformed[..4].copy_from_slice(&8u32.to_le_bytes());
        assert!(unsafe { parse_multiboot2(malformed.as_ptr() as usize) }.is_none());

        malformed[..4].copy_from_slice(&16u32.to_le_bytes());
        malformed[8..12].copy_from_slice(&1u32.to_le_bytes());
        malformed[12..16].copy_from_slice(&4u32.to_le_bytes());
        assert!(unsafe { parse_multiboot2(malformed.as_ptr() as usize) }.is_none());

        malformed[..4].copy_from_slice(&32u32.to_le_bytes());
        malformed[12..16].copy_from_slice(&40u32.to_le_bytes());
        assert!(unsafe { parse_multiboot2(malformed.as_ptr() as usize) }.is_none());
    }

    #[cfg(target_arch = "aarch64")]
    {
        let mut malformed = [0u8; 64];
        malformed[..4].copy_from_slice(&0u32.to_be_bytes());
        assert!(unsafe { parse_fdt(malformed.as_ptr() as usize) }.is_none());

        malformed[..4].copy_from_slice(&0xd00d_feed_u32.to_be_bytes());
        malformed[4..8].copy_from_slice(&39u32.to_be_bytes());
        assert!(unsafe { parse_fdt(malformed.as_ptr() as usize) }.is_none());

        malformed[4..8].copy_from_slice(&40u32.to_be_bytes());
        malformed[8..12].copy_from_slice(&64u32.to_be_bytes());
        assert!(unsafe { parse_fdt(malformed.as_ptr() as usize) }.is_none());
        assert!(unsafe { read_cells(malformed.as_ptr() as usize, 8, 0) }.is_none());
        assert!(unsafe { align4(usize::MAX) }.is_none());
    }
}

#[cfg(target_os = "uefi")]
pub fn set_efi_system_table(system_table: u64) {
    unsafe {
        (*core::ptr::addr_of_mut!(INFO)).efi_system_table = system_table;
    }
}

#[cfg(target_os = "uefi")]
pub fn set_efi_runtime_context(el: u8, vbar: u64, sp_el0: u64) {
    unsafe {
        let info = &mut *core::ptr::addr_of_mut!(INFO);
        info.efi_runtime_el = el;
        info.efi_runtime_vbar = vbar;
        info.efi_runtime_sp_el0 = sp_el0;
    }
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn norx_multiboot2_entry(magic: u32, info_address: u64) -> ! {
    crate::arch::init();
    if !init_multiboot2(magic, info_address) {
        crate::drivers::serial::write(format_args!(
            "Norx: invalid Multiboot2 hand-off magic=0x{:08x} info=0x{:016x}\r\n",
            magic, info_address
        ));
        crate::arch::halt();
    }
    crate::kernel_start()
}

#[cfg(target_os = "uefi")]
#[no_mangle]
pub extern "efiapi" fn norx_efi_main(_image_handle: u64, system_table: u64) -> ! {
    crate::arch::init();
    let Some(fdt_address) =
        (unsafe { efi_fdt(system_table).or_else(|| efi_file_fdt(_image_handle, system_table)) })
    else {
        crate::drivers::serial::write_str("Norx: EFI DTB table unavailable\r\n");
        crate::arch::halt();
    };
    if !init_fdt(fdt_address) {
        crate::drivers::serial::write_str("Norx: invalid EFI DTB hand-off\r\n");
        crate::arch::halt();
    }
    set_efi_system_table(system_table);
    let (efi_runtime_el, efi_runtime_vbar, efi_runtime_sp_el0) =
        crate::arch::firmware_runtime_context();
    set_efi_runtime_context(efi_runtime_el, efi_runtime_vbar, efi_runtime_sp_el0);
    if let Some((base, size)) = unsafe { efi_image_region(_image_handle, system_table) } {
        unsafe { add_reserved(&mut *core::ptr::addr_of_mut!(INFO), base, size) };
    }
    if let Some(framebuffer) = unsafe { efi_framebuffer(system_table) } {
        unsafe {
            add_reserved(
                &mut *core::ptr::addr_of_mut!(INFO),
                framebuffer.base as u64,
                framebuffer.size as u64,
            );
            (*core::ptr::addr_of_mut!(INFO)).framebuffer = Some(framebuffer);
        }
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

#[cfg(not(target_os = "uefi"))]
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
    let bytes_per_pixel: usize = match bpp {
        24 => 3,
        32 => 4,
        _ => return None,
    };
    let minimum_pitch = width.checked_mul(bytes_per_pixel as u64)?;
    if base == 0 || width == 0 || height == 0 || pitch < minimum_pitch {
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
        bytes_per_pixel,
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
    let mut efi_system_table = 0u64;
    let mut efi_image_handle = 0u64;

    while offset.checked_add(8)? <= total_size {
        let tag = address.checked_add(offset)?;
        let tag_type = read_u32(tag)?;
        let size = read_u32(tag + 4)? as usize;
        if size < 8 || offset.checked_add(size)? > total_size {
            return None;
        }
        match tag_type {
            1 => copy_c_string(tag + 8, &mut info.cmdline, &mut info.cmdline_len, size - 8),
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
                let pixel_type = read_u8(tag + 29)?;
                if pixel_type == 1 && size >= 38 {
                    let red = read_u8(tag + 32)?;
                    let blue = read_u8(tag + 36)?;
                    if red == 0 && blue == 16 {
                        framebuffer_info =
                            framebuffer(base, pitch, width, height, bpp, PixelFormat::Rgb);
                    } else if red == 16 && blue == 0 {
                        framebuffer_info =
                            framebuffer(base, pitch, width, height, bpp, PixelFormat::Bgr);
                    }
                }
            }
            12 if size >= 16 => efi_system_table = read_u64(tag + 8)?,
            20 if size >= 16 => efi_image_handle = read_u64(tag + 8)?,
            0 => break,
            _ => {}
        }
        offset = offset.checked_add((size + 7) & !7)?;
    }

    if info.memory_len == 0 {
        efi_memory_map(efi_system_table, efi_image_handle, &mut info)?;
    }
    if let Some(framebuffer) = framebuffer_info {
        add_reserved(&mut info, framebuffer.base as u64, framebuffer.size as u64);
    }
    info.framebuffer = framebuffer_info;
    info.handoff_address = address as u64;
    info.efi_system_table = efi_system_table;
    add_reserved(&mut info, address as u64, total_size as u64);
    #[cfg(not(target_os = "uefi"))]
    reserve_kernel(&mut info);
    Some(info)
}

#[cfg(target_arch = "x86_64")]
type EfiGetMemoryMap = unsafe extern "efiapi" fn(
    memory_map_size: *mut usize,
    memory_map: *mut u8,
    map_key: *mut usize,
    descriptor_size: *mut usize,
    descriptor_version: *mut u32,
) -> usize;

#[cfg(target_arch = "x86_64")]
type EfiExitBootServices = unsafe extern "efiapi" fn(image_handle: u64, map_key: usize) -> usize;

#[cfg(target_arch = "x86_64")]
unsafe fn efi_memory_map(system_table: u64, image_handle: u64, info: &mut BootInfo) -> Option<()> {
    if system_table == 0 || image_handle == 0 {
        return None;
    }

    let system_table = system_table as usize;
    let boot_services = read_u64(system_table + 96)? as usize;
    let get_memory_map = read_u64(boot_services + 56)? as usize;
    let exit_boot_services = read_u64(boot_services + 232)? as usize;
    if get_memory_map == 0 || exit_boot_services == 0 {
        return None;
    }

    let mut map_size = EFI_MEMORY_MAP_SIZE;
    let mut map_key = 0usize;
    let mut descriptor_size = 0usize;
    let mut descriptor_version = 0u32;
    let map_ptr = core::ptr::addr_of_mut!(EFI_MEMORY_MAP) as *mut u8;
    let get_memory_map: EfiGetMemoryMap = core::mem::transmute(get_memory_map);
    if get_memory_map(
        &mut map_size,
        map_ptr,
        &mut map_key,
        &mut descriptor_size,
        &mut descriptor_version,
    ) != 0
        || descriptor_size < 40
        || map_size > EFI_MEMORY_MAP_SIZE
    {
        return None;
    }

    let exit_boot_services: EfiExitBootServices = core::mem::transmute(exit_boot_services);
    if exit_boot_services(image_handle, map_key) != 0 {
        return None;
    }

    let mut offset = 0usize;
    while offset.checked_add(descriptor_size)? <= map_size {
        let descriptor = map_ptr.add(offset) as usize;
        let kind = read_u32(descriptor)?;
        let base = read_u64(descriptor + 8)?;
        let pages = read_u64(descriptor + 24)?;
        if matches!(kind, 1..=4 | 7) {
            add_memory(info, base, pages.checked_mul(0x1000)?);
        }
        offset = offset.checked_add(descriptor_size)?;
    }
    let _ = descriptor_version;
    Some(())
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
                add_reserved(&mut info, fb.base as u64, fb.size as u64);
                info.framebuffer = Some(fb);
            }
        }
    }
    info.handoff_address = address as u64;
    add_reserved(&mut info, address as u64, total_size as u64);
    #[cfg(not(target_os = "uefi"))]
    reserve_kernel(&mut info);
    Some(info)
}

#[cfg(target_os = "uefi")]
const EFI_DTB_TABLE_GUID: [u8; 16] = [
    0xd5, 0x21, 0xb6, 0xb1, 0x9c, 0xf1, 0xa5, 0x41, 0x83, 0x0b, 0xd9, 0x15, 0x2c, 0x69, 0xaa, 0xe0,
];

#[cfg(target_os = "uefi")]
const EFI_LOADED_IMAGE_PROTOCOL_GUID: [u8; 16] = [
    0xa1, 0x31, 0x1b, 0x5b, 0x62, 0x95, 0xd2, 0x11, 0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];

#[cfg(target_os = "uefi")]
const EFI_SIMPLE_FILE_SYSTEM_PROTOCOL_GUID: [u8; 16] = [
    0x22, 0x5b, 0x4e, 0x96, 0x59, 0x64, 0xd2, 0x11, 0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];

#[cfg(target_os = "uefi")]
const EFI_FILE_INFO_GUID: [u8; 16] = [
    0x92, 0x6e, 0x57, 0x09, 0x3f, 0x6d, 0xd2, 0x11, 0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b,
];

#[cfg(target_os = "uefi")]
const EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID: [u8; 16] = [
    0xde, 0xa9, 0x42, 0x90, 0xdc, 0x23, 0x38, 0x4a, 0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0xa7,
];

#[cfg(target_os = "uefi")]
const EFI_DTB_PATH: [u16; 15] = [
    b'\\' as u16,
    b'b' as u16,
    b'o' as u16,
    b'o' as u16,
    b't' as u16,
    b'\\' as u16,
    b'n' as u16,
    b'o' as u16,
    b'r' as u16,
    b'x' as u16,
    b'.' as u16,
    b'd' as u16,
    b't' as u16,
    b'b' as u16,
    0,
];

#[cfg(target_os = "uefi")]
type EfiHandleProtocol = unsafe extern "efiapi" fn(u64, *const u8, *mut u64) -> usize;
#[cfg(target_os = "uefi")]
type EfiOpenVolume = unsafe extern "efiapi" fn(u64, *mut u64) -> usize;
#[cfg(target_os = "uefi")]
type EfiFileOpen = unsafe extern "efiapi" fn(u64, *mut u64, *const u16, u64, u64) -> usize;
#[cfg(target_os = "uefi")]
type EfiFileClose = unsafe extern "efiapi" fn(u64) -> usize;
#[cfg(target_os = "uefi")]
type EfiFileRead = unsafe extern "efiapi" fn(u64, *mut usize, *mut u8) -> usize;
#[cfg(target_os = "uefi")]
type EfiFileGetInfo = unsafe extern "efiapi" fn(u64, *const u8, *mut usize, *mut u8) -> usize;
#[cfg(target_os = "uefi")]
type EfiAllocatePool = unsafe extern "efiapi" fn(u32, usize, *mut u64) -> usize;
#[cfg(target_os = "uefi")]
type EfiLocateProtocol = unsafe extern "efiapi" fn(*const u8, *const u8, *mut u64) -> usize;

#[cfg(target_os = "uefi")]
unsafe fn efi_fdt(system_table: u64) -> Option<u64> {
    if system_table == 0 {
        return None;
    }
    let system_table = system_table as usize;
    let count = read_u64(system_table + 104)? as usize;
    let tables = read_u64(system_table + 112)? as usize;
    for index in 0..count {
        let entry = tables.checked_add(index.checked_mul(24)?)?;
        let guid = core::slice::from_raw_parts(entry as *const u8, 16);
        if guid == EFI_DTB_TABLE_GUID {
            return read_u64(entry + 16);
        }
    }
    None
}

#[cfg(target_os = "uefi")]
unsafe fn efi_image_region(image_handle: u64, system_table: u64) -> Option<(u64, u64)> {
    if image_handle == 0 || system_table == 0 {
        return None;
    }
    let loaded_image = efi_loaded_image(image_handle, system_table)?;
    let image_base = read_u64(loaded_image as usize + 64)?;
    let image_size = read_u64(loaded_image as usize + 72)?;
    Some((image_base, image_size))
}

#[cfg(target_os = "uefi")]
unsafe fn efi_loaded_image(image_handle: u64, system_table: u64) -> Option<u64> {
    let boot_services = read_u64(system_table as usize + 96)? as usize;
    let handle_protocol = read_u64(boot_services + 152)? as usize;
    if handle_protocol == 0 {
        return None;
    }
    let handle_protocol: EfiHandleProtocol = core::mem::transmute(handle_protocol);
    let mut loaded_image = 0u64;
    if handle_protocol(
        image_handle,
        EFI_LOADED_IMAGE_PROTOCOL_GUID.as_ptr(),
        &mut loaded_image,
    ) != 0
    {
        return None;
    }
    Some(loaded_image)
}

#[cfg(target_os = "uefi")]
unsafe fn efi_file_fdt(image_handle: u64, system_table: u64) -> Option<u64> {
    let loaded_image = efi_loaded_image(image_handle, system_table)?;
    let device_handle = read_u64(loaded_image as usize + 24)?;
    let boot_services = read_u64(system_table as usize + 96)? as usize;
    let handle_protocol = read_u64(boot_services + 152)? as usize;
    let handle_protocol: EfiHandleProtocol = core::mem::transmute(handle_protocol);
    let mut file_system = 0u64;
    if handle_protocol(
        device_handle,
        EFI_SIMPLE_FILE_SYSTEM_PROTOCOL_GUID.as_ptr(),
        &mut file_system,
    ) != 0
    {
        return None;
    }

    let open_volume: EfiOpenVolume = core::mem::transmute(read_u64(file_system as usize + 8)?);
    let mut root = 0u64;
    if open_volume(file_system, &mut root) != 0 {
        return None;
    }
    let open: EfiFileOpen = core::mem::transmute(read_u64(root as usize + 8)?);
    let mut file = 0u64;
    if open(root, &mut file, EFI_DTB_PATH.as_ptr(), 1, 0) != 0 {
        return None;
    }
    let close: EfiFileClose = core::mem::transmute(read_u64(file as usize + 16)?);

    let get_info: EfiFileGetInfo = core::mem::transmute(read_u64(file as usize + 64)?);
    let mut info_size = 128usize;
    let mut info = [0u8; 128];
    if get_info(
        file,
        EFI_FILE_INFO_GUID.as_ptr(),
        &mut info_size,
        info.as_mut_ptr(),
    ) != 0
    {
        close(file);
        return None;
    }
    let file_size = read_u64(info.as_ptr() as usize + 8)? as usize;
    if !(40..=MAX_FDT_SIZE).contains(&file_size) {
        close(file);
        return None;
    }

    let allocate_pool: EfiAllocatePool = core::mem::transmute(read_u64(boot_services + 64)?);
    let mut buffer = 0u64;
    if allocate_pool(2, file_size, &mut buffer) != 0 {
        close(file);
        return None;
    }
    let read: EfiFileRead = core::mem::transmute(read_u64(file as usize + 32)?);
    let mut read_size = file_size;
    if read(file, &mut read_size, buffer as *mut u8) != 0 || read_size != file_size {
        close(file);
        return None;
    }
    close(file);
    Some(buffer)
}

#[cfg(target_os = "uefi")]
unsafe fn efi_framebuffer(system_table: u64) -> Option<RawFramebuffer> {
    let boot_services = read_u64(system_table as usize + 96)? as usize;
    let locate_protocol: EfiLocateProtocol = core::mem::transmute(read_u64(boot_services + 320)?);
    let mut graphics = 0u64;
    if locate_protocol(
        EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID.as_ptr(),
        core::ptr::null(),
        &mut graphics,
    ) != 0
    {
        return None;
    }
    let mode = read_u64(graphics as usize + 32)? as usize;
    let info = read_u64(mode + 8)? as usize;
    let width = read_u32(info + 4)? as u64;
    let height = read_u32(info + 8)? as u64;
    let format = match read_u32(info + 12)? {
        0 => PixelFormat::Rgb,
        1 => PixelFormat::Bgr,
        _ => return None,
    };
    let pixels_per_scan_line = read_u32(info + 32)? as u64;
    let base = read_u64(mode + 24)?;
    let size = read_u64(mode + 32)?;
    let framebuffer = framebuffer(
        base,
        pixels_per_scan_line.checked_mul(4)?,
        width,
        height,
        32,
        format,
    )?;
    (framebuffer.size as u64 <= size).then_some(framebuffer)
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
unsafe fn copy_c_string(address: usize, out: &mut [u8], len: &mut usize, limit: usize) {
    let mut index = 0;
    while index < out.len() && index < limit {
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

unsafe fn read_u8(address: usize) -> Option<u8> {
    Some((address as *const u8).read_unaligned())
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn read_u32(address: usize) -> Option<u32> {
    Some((address as *const u32).read_unaligned())
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn read_u64(address: usize) -> Option<u64> {
    Some((address as *const u64).read_unaligned())
}

#[cfg(target_arch = "aarch64")]
unsafe fn read_be32(address: usize) -> Option<u32> {
    Some(u32::from_be((address as *const u32).read_unaligned()))
}
