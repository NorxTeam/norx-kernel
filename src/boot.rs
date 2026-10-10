#![allow(
    dead_code,
    reason = "Staged ABI and hardware hooks are not all enabled in the bootstrap profile"
)]

use core::str;

#[cfg(target_arch = "x86_64")]
pub const MULTIBOOT2_BOOTLOADER_MAGIC: u32 = 0x36d7_6289;

const MAX_MEMORY_REGIONS: usize = 64;
const MAX_RESERVED_REGIONS: usize = 32;
const MAX_MODULES: usize = 8;
const CMDLINE_MAX: usize = 128;
const POINTER_REGION_MIN: usize = 1;
const EFI_TABLE_HEADER_SIZE: usize = 24;
const EFI_SYSTEM_TABLE_SIZE: usize = 120;
const EFI_BOOT_SERVICES_SIZE: usize = 328;
const EFI_CONFIG_TABLE_ENTRY_SIZE: usize = 24;
const MAX_EFI_CONFIG_TABLES: usize = 1024;
const EFI_FILE_PROTOCOL_SIZE: usize = 72;
const EFI_GOP_SIZE: usize = 32;
const EFI_GOP_MODE_SIZE: usize = 40;
const EFI_GOP_INFO_SIZE: usize = 36;
const MAX_FDT_NODE_NAME: usize = 256;
#[cfg(target_arch = "x86_64")]
const MAX_MULTIBOOT_INFO: usize = 16 * 1024 * 1024;
#[cfg(target_arch = "x86_64")]
const EFI_MEMORY_MAP_SIZE: usize = 64 * 1024;
#[cfg(target_arch = "aarch64")]
const MAX_FDT_SIZE: usize = 16 * 1024 * 1024;

#[cfg(target_arch = "x86_64")]
static mut EFI_MEMORY_MAP: [u8; EFI_MEMORY_MAP_SIZE] = [0; EFI_MEMORY_MAP_SIZE];

#[derive(Clone, Copy)]
struct PointerRegion {
    address: usize,
    length: usize,
}

impl PointerRegion {
    fn new(address: usize, length: usize) -> Option<Self> {
        if length < POINTER_REGION_MIN || !valid_pointer(address) {
            return None;
        }
        let end = address.checked_add(length)?;
        if !valid_pointer(end.checked_sub(1)?) {
            return None;
        }
        Some(Self { address, length })
    }

    fn from_u64(address: u64, length: usize) -> Option<Self> {
        Self::new(usize::try_from(address).ok()?, length)
    }

    fn subregion(self, offset: usize, length: usize) -> Option<Self> {
        let end = offset.checked_add(length)?;
        if end > self.length {
            return None;
        }
        Some(Self {
            address: self.address.checked_add(offset)?,
            length,
        })
    }

    fn address_at(self, offset: usize, length: usize) -> Option<usize> {
        self.subregion(offset, length).map(|region| region.address)
    }
}

fn valid_pointer(address: usize) -> bool {
    if address == 0 {
        return false;
    }
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        let upper = address >> 48;
        upper == 0 || upper == usize::MAX >> 48
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        true
    }
}

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
    .long 32
    .long 6
    .long 8
    .long 1
    .long 12
    .long 14
    .long 15
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

    .section .boot_stack,"aw",@nobits
    .align 16
norx_boot_stack:
    .skip 524288
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

#[cfg(target_arch = "aarch64")]
#[derive(Clone, Copy)]
pub struct Aarch64InterruptInfo {
    pub gic_version: u8,
    pub gic_distributor: u64,
    pub gic_cpu_interface: u64,
    pub gic_redistributor: u64,
    pub timer_irq: u32,
    pub timer_flags: u32,
}

#[cfg(target_arch = "aarch64")]
impl Aarch64InterruptInfo {
    pub(crate) const fn empty() -> Self {
        Self {
            gic_version: 0,
            gic_distributor: 0,
            gic_cpu_interface: 0,
            gic_redistributor: 0,
            timer_irq: 0,
            timer_flags: 0,
        }
    }
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
    #[cfg(target_arch = "x86_64")]
    pub acpi_rsdp: u64,
    #[cfg(target_arch = "aarch64")]
    pub interrupt_info: Aarch64InterruptInfo,
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
            #[cfg(target_arch = "x86_64")]
            acpi_rsdp: 0,
            #[cfg(target_arch = "aarch64")]
            interrupt_info: Aarch64InterruptInfo::empty(),
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
    assert!(framebuffer(u64::MAX - 3, 16, 4, 1, 32, PixelFormat::Rgb).is_none());

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
        assert!(unsafe {
            read_cells(
                &PointerRegion::new(malformed.as_ptr() as usize, malformed.len()).unwrap(),
                0,
                8,
                0,
            )
        }
        .is_none());
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
    #[cfg(target_arch = "aarch64")]
    unsafe {
        reserve_efi_mapped_regions(&mut *core::ptr::addr_of_mut!(INFO))
    };
    #[cfg(target_arch = "aarch64")]
    crate::arch::start_kernel();
    #[cfg(not(target_arch = "aarch64"))]
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

#[cfg(target_arch = "aarch64")]
unsafe fn reserve_efi_mapped_regions(info: &mut BootInfo) {
    let original_len = info.reserved_len;
    let mut index = 0;
    while index < original_len {
        let region = info.reserved[index];
        let start = region.base & !(4096 - 1);
        let Some(end) = region.base.checked_add(region.length) else {
            index += 1;
            continue;
        };
        let end = end.checked_add(4095).map(|value| value & !(4096 - 1));
        let Some(end) = end else {
            index += 1;
            continue;
        };
        if start >= end {
            index += 1;
            continue;
        }
        let Some(physical_start) =
            crate::arch::virtual_to_physical(crate::address::VirtAddr::new(start as usize))
        else {
            index += 1;
            continue;
        };
        let Some(physical_end) =
            crate::arch::virtual_to_physical(crate::address::VirtAddr::new((end - 1) as usize))
        else {
            index += 1;
            continue;
        };
        let Some(physical_length) = physical_end
            .value()
            .checked_sub(physical_start.value())
            .and_then(|value| value.checked_add(4096))
        else {
            index += 1;
            continue;
        };
        if physical_length == end - start {
            add_reserved(info, physical_start.value(), physical_length);
        }
        index += 1;
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
    base.checked_add(size as u64)?;
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
    let header = PointerRegion::new(address, 8)?;
    let total_size = read_u32(&header, 0)? as usize;
    if !(16..=MAX_MULTIBOOT_INFO).contains(&total_size) {
        return None;
    }
    let region = PointerRegion::new(address, total_size)?;
    let mut info = BootInfo::empty(Architecture::X86_64);
    let mut offset = 8usize;
    let mut framebuffer_info = None;
    let mut efi_system_table = 0u64;
    let mut efi_image_handle = 0u64;
    let mut acpi_rsdp = 0u64;
    let mut end_tag = false;

    while offset < total_size {
        let tag_header = region.subregion(offset, 8)?;
        let tag_type = read_u32(&tag_header, 0)?;
        let size = read_u32(&tag_header, 4)? as usize;
        if size < 8 {
            return None;
        }
        let tag = region.subregion(offset, size)?;
        match tag_type {
            1 => copy_c_string(&tag, 8, size - 8, &mut info.cmdline, &mut info.cmdline_len)?,
            3 if size >= 16 => {
                let start = read_u32(&tag, 8)? as u64;
                let end = read_u32(&tag, 12)? as u64;
                add_module(&mut info, start, end);
            }
            6 if size >= 16 => {
                let entry_size = read_u32(&tag, 8)? as usize;
                let entries_size = size - 16;
                if entry_size < 24 || !entries_size.is_multiple_of(entry_size) {
                    return None;
                }
                let mut entry_offset = 16usize;
                while entry_offset < size {
                    let entry = tag.subregion(entry_offset, entry_size)?;
                    let base = read_u64(&entry, 0)?;
                    let length = read_u64(&entry, 8)?;
                    if read_u32(&entry, 16)? == 1 {
                        add_memory(&mut info, base, length);
                    }
                    entry_offset = entry_offset.checked_add(entry_size)?;
                }
            }
            8 if size >= 32 => {
                let base = read_u64(&tag, 8)?;
                let pitch = read_u32(&tag, 16)? as u64;
                let width = read_u32(&tag, 20)? as u64;
                let height = read_u32(&tag, 24)? as u64;
                let bpp = read_u8(&tag, 28)?;
                let pixel_type = read_u8(&tag, 29)?;
                if pixel_type == 1 && size >= 38 {
                    let red = read_u8(&tag, 32)?;
                    let blue = read_u8(&tag, 36)?;
                    if red == 0 && blue == 16 {
                        framebuffer_info =
                            framebuffer(base, pitch, width, height, bpp, PixelFormat::Rgb);
                    } else if red == 16 && blue == 0 {
                        framebuffer_info =
                            framebuffer(base, pitch, width, height, bpp, PixelFormat::Bgr);
                    }
                }
            }
            12 if size >= 16 => efi_system_table = read_u64(&tag, 8)?,
            14 if size >= 28 && acpi_rsdp == 0 => acpi_rsdp = tag.address_at(8, 1)? as u64,
            15 if size >= 44 => acpi_rsdp = tag.address_at(8, 1)? as u64,
            20 if size >= 16 => efi_image_handle = read_u64(&tag, 8)?,
            0 if size == 8 => {
                end_tag = true;
                offset = total_size;
                continue;
            }
            _ => {}
        }
        offset = offset.checked_add(size)?.checked_add(7)? & !7;
    }

    if !end_tag {
        return None;
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
    info.acpi_rsdp = acpi_rsdp;
    add_reserved(&mut info, address as u64, total_size as u64);
    #[cfg(not(target_os = "uefi"))]
    reserve_kernel(&mut info);
    Some(info)
}

#[cfg(any(target_arch = "x86_64", target_os = "uefi"))]
const EFI_SYSTEM_TABLE_SIGNATURE: u64 = 0x5453_5953_2049_4249;
#[cfg(any(target_arch = "x86_64", target_os = "uefi"))]
const EFI_BOOT_SERVICES_SIGNATURE: u64 = 0x5652_4553_544f_4f42;

#[cfg(any(target_arch = "x86_64", target_os = "uefi"))]
unsafe fn efi_table_region(address: u64, size: usize, signature: u64) -> Option<PointerRegion> {
    let header = PointerRegion::from_u64(address, EFI_TABLE_HEADER_SIZE)?;
    if read_u64(&header, 0)? != signature {
        return None;
    }
    let header_size = read_u32(&header, 12)? as usize;
    if !(EFI_TABLE_HEADER_SIZE..=4096).contains(&header_size) {
        return None;
    }
    PointerRegion::from_u64(address, size)
}

#[cfg(any(target_arch = "x86_64", target_os = "uefi"))]
unsafe fn efi_boot_services_region(system_table: u64) -> Option<PointerRegion> {
    let system_table = efi_table_region(
        system_table,
        EFI_SYSTEM_TABLE_SIZE,
        EFI_SYSTEM_TABLE_SIGNATURE,
    )?;
    let boot_services = read_u64(&system_table, 96)?;
    efi_table_region(
        boot_services,
        EFI_BOOT_SERVICES_SIZE,
        EFI_BOOT_SERVICES_SIGNATURE,
    )
}

#[cfg(any(target_arch = "x86_64", target_os = "uefi"))]
fn efi_function_address(address: u64) -> Option<usize> {
    let region = PointerRegion::from_u64(address, POINTER_REGION_MIN)?;
    #[cfg(target_arch = "aarch64")]
    if !region.address.is_multiple_of(4) {
        return None;
    }
    Some(region.address)
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

    let boot_services = efi_boot_services_region(system_table)?;
    let get_memory_map = efi_function_address(read_u64(&boot_services, 56)?)?;
    let exit_boot_services = efi_function_address(read_u64(&boot_services, 232)?)?;

    let mut map_size = EFI_MEMORY_MAP_SIZE;
    let mut map_key = 0usize;
    let mut descriptor_size = 0usize;
    let mut descriptor_version = 0u32;
    let map_ptr = core::ptr::addr_of_mut!(EFI_MEMORY_MAP) as *mut u8;
    let map_region = PointerRegion::new(map_ptr as usize, EFI_MEMORY_MAP_SIZE)?;
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
        || map_size == 0
        || !map_size.is_multiple_of(descriptor_size)
    {
        return None;
    }

    let exit_boot_services: EfiExitBootServices = core::mem::transmute(exit_boot_services);
    if exit_boot_services(image_handle, map_key) != 0 {
        return None;
    }

    let mut offset = 0usize;
    while offset.checked_add(descriptor_size)? <= map_size {
        let descriptor = map_region.subregion(offset, descriptor_size)?;
        let kind = read_u32(&descriptor, 0)?;
        let base = read_u64(&descriptor, 8)?;
        let pages = read_u64(&descriptor, 24)?;
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
    let header = PointerRegion::new(address, 40)?;
    if read_be32(&header, 0)? != 0xd00d_feed {
        return None;
    }
    let total_size = read_be32(&header, 4)? as usize;
    if !(40..=MAX_FDT_SIZE).contains(&total_size) {
        return None;
    }
    let blob = PointerRegion::new(address, total_size)?;
    let structure_offset = read_be32(&header, 8)? as usize;
    let strings_offset = read_be32(&header, 12)? as usize;
    let memory_reserve_offset = read_be32(&header, 16)? as usize;
    let version = read_be32(&header, 20)?;
    let last_compatible_version = read_be32(&header, 24)?;
    let strings_size = read_be32(&header, 32)? as usize;
    let structure_size = read_be32(&header, 36)? as usize;
    if version < 17 || last_compatible_version > version {
        return None;
    }
    let structure = blob.subregion(structure_offset, structure_size)?;
    let strings = blob.subregion(strings_offset, strings_size)?;
    validate_fdt_reservations(&blob, memory_reserve_offset)?;

    let mut info = BootInfo::empty(Architecture::Aarch64);
    let mut cursor = 0usize;
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
    let mut gic_version = 0u8;
    let mut gic_distributor = 0u64;
    let mut gic_second_region = 0u64;
    let mut timer_irq = 0u32;
    let mut timer_flags = 0u32;
    let end_token;

    loop {
        let token = read_be32(&structure, cursor)?;
        cursor = cursor.checked_add(4)?;
        match token {
            1 => {
                let (name_len, kind) = node_kind(&structure, cursor)?;
                cursor = align4(cursor.checked_add(name_len)?.checked_add(1)?)?;
                if depth >= nodes.len() {
                    return None;
                }
                nodes[depth] = kind;
                depth += 1;
            }
            2 => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            3 => {
                let property = structure.subregion(cursor, 8)?;
                let length = read_be32(&property, 0)? as usize;
                let name_offset = read_be32(&property, 4)? as usize;
                cursor = cursor.checked_add(8)?;
                let data = cursor;
                let aligned_end = align4(cursor.checked_add(length)?)?;
                structure.subregion(data, length)?;
                if aligned_end > structure.length {
                    return None;
                }
                cursor = aligned_end;
                string_length(&strings, name_offset)?;
                let kind = if depth == 0 {
                    NodeKind::Other
                } else {
                    nodes[depth - 1]
                };
                if depth == 1 && c_string_eq(&strings, name_offset, b"#address-cells") {
                    if length < 4 {
                        return None;
                    }
                    address_cells = read_be32(&structure, data)? as usize;
                } else if depth == 1 && c_string_eq(&strings, name_offset, b"#size-cells") {
                    if length < 4 {
                        return None;
                    }
                    size_cells = read_be32(&structure, data)? as usize;
                } else if kind == NodeKind::Memory && c_string_eq(&strings, name_offset, b"reg") {
                    parse_reg(
                        &mut info,
                        &structure,
                        data,
                        length,
                        address_cells,
                        size_cells,
                    )?;
                } else if kind == NodeKind::Chosen
                    && c_string_eq(&strings, name_offset, b"bootargs")
                {
                    copy_bytes(
                        &mut info.cmdline,
                        &mut info.cmdline_len,
                        &structure,
                        data,
                        length,
                    )?;
                } else if kind == NodeKind::Chosen
                    && c_string_eq(&strings, name_offset, b"linux,initrd-start")
                {
                    initrd_start = read_cells(&structure, data, length, size_cells)?;
                } else if kind == NodeKind::Chosen
                    && c_string_eq(&strings, name_offset, b"linux,initrd-end")
                {
                    initrd_end = read_cells(&structure, data, length, size_cells)?;
                } else if kind == NodeKind::SimpleFramebuffer
                    && c_string_eq(&strings, name_offset, b"reg")
                {
                    framebuffer_base = read_cells(&structure, data, length, address_cells)?;
                    let offset = address_cells.checked_mul(4)?;
                    framebuffer_size = read_cells(
                        &structure,
                        data.checked_add(offset)?,
                        length.checked_sub(offset)?,
                        size_cells,
                    )?;
                } else if kind == NodeKind::SimpleFramebuffer
                    && c_string_eq(&strings, name_offset, b"width")
                {
                    if length < 4 {
                        return None;
                    }
                    framebuffer_width = read_be32(&structure, data)? as u64;
                } else if kind == NodeKind::SimpleFramebuffer
                    && c_string_eq(&strings, name_offset, b"height")
                {
                    if length < 4 {
                        return None;
                    }
                    framebuffer_height = read_be32(&structure, data)? as u64;
                } else if kind == NodeKind::SimpleFramebuffer
                    && c_string_eq(&strings, name_offset, b"stride")
                {
                    if length < 4 {
                        return None;
                    }
                    framebuffer_stride = read_be32(&structure, data)? as u64;
                } else if kind == NodeKind::SimpleFramebuffer
                    && c_string_eq(&strings, name_offset, b"format")
                {
                    framebuffer_format_len = copy_bytes(
                        &mut framebuffer_format,
                        &mut framebuffer_format_len,
                        &structure,
                        data,
                        length,
                    )?;
                } else if kind == NodeKind::InterruptController
                    && c_string_eq(&strings, name_offset, b"compatible")
                {
                    if property_c_string_starts_with(&structure, data, length, b"arm,gic-v3") {
                        gic_version = 3;
                    } else if property_c_string_starts_with(
                        &structure,
                        data,
                        length,
                        b"arm,cortex-a15-gic",
                    ) || property_c_string_starts_with(
                        &structure,
                        data,
                        length,
                        b"arm,gic-400",
                    ) {
                        gic_version = 2;
                    }
                } else if kind == NodeKind::InterruptController
                    && c_string_eq(&strings, name_offset, b"reg")
                {
                    let entry_cells = address_cells.checked_add(size_cells)?;
                    let entry_bytes = entry_cells.checked_mul(4)?;
                    if entry_bytes == 0 || !length.is_multiple_of(entry_bytes) {
                        return None;
                    }
                    let address_bytes = address_cells.checked_mul(4)?;
                    gic_distributor = read_cells(&structure, data, length, address_cells)?;
                    if length >= entry_bytes.checked_mul(2)? {
                        gic_second_region = read_cells(
                            &structure,
                            data.checked_add(entry_bytes)?,
                            length.checked_sub(entry_bytes)?,
                            address_cells,
                        )?;
                    }
                    if address_bytes > length {
                        return None;
                    }
                } else if kind == NodeKind::Timer
                    && c_string_eq(&strings, name_offset, b"interrupts")
                {
                    let spec_bytes = 12usize;
                    let virtual_offset = spec_bytes.checked_mul(2)?;
                    if length >= virtual_offset.checked_add(spec_bytes)? {
                        let spec = data.checked_add(virtual_offset)?;
                        if read_be32(&structure, spec)? == 1 {
                            let irq = read_be32(&structure, spec.checked_add(4)?)?;
                            if irq < 16 {
                                timer_irq = 16 + irq;
                                timer_flags = read_be32(&structure, spec.checked_add(8)?)?;
                            }
                        }
                    }
                }
            }
            4 => {}
            9 => {
                if depth != 0 {
                    return None;
                }
                end_token = true;
                break;
            }
            _ => return None,
        }
        if cursor >= structure.length {
            return None;
        }
    }

    if !end_token || info.memory_len == 0 {
        return None;
    }
    if initrd_end > initrd_start {
        add_module(&mut info, initrd_start, initrd_end);
    }
    if framebuffer_base != 0 && framebuffer_size != 0 {
        let format = if framebuffer_format[..framebuffer_format_len].starts_with(b"r8g8b8a8") {
            PixelFormat::Rgb
        } else if framebuffer_format[..framebuffer_format_len].starts_with(b"b8g8r8a8") {
            PixelFormat::Bgr
        } else {
            return None;
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
    info.interrupt_info = Aarch64InterruptInfo {
        gic_version,
        gic_distributor,
        gic_cpu_interface: if gic_version == 2 {
            gic_second_region
        } else {
            0
        },
        gic_redistributor: if gic_version >= 3 {
            gic_second_region
        } else {
            0
        },
        timer_irq,
        timer_flags,
    };
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
type EfiFreePool = unsafe extern "efiapi" fn(u64) -> usize;
#[cfg(target_os = "uefi")]
type EfiLocateProtocol = unsafe extern "efiapi" fn(*const u8, *const u8, *mut u64) -> usize;

#[cfg(target_os = "uefi")]
unsafe fn efi_fdt(system_table: u64) -> Option<u64> {
    let system_table = efi_table_region(
        system_table,
        EFI_SYSTEM_TABLE_SIZE,
        EFI_SYSTEM_TABLE_SIGNATURE,
    )?;
    let count = usize::try_from(read_u64(&system_table, 104)?).ok()?;
    if count == 0 || count > MAX_EFI_CONFIG_TABLES {
        return None;
    }
    let table_bytes = count.checked_mul(EFI_CONFIG_TABLE_ENTRY_SIZE)?;
    let tables = PointerRegion::from_u64(read_u64(&system_table, 112)?, table_bytes)?;
    for index in 0..count {
        let entry = tables.subregion(
            index.checked_mul(EFI_CONFIG_TABLE_ENTRY_SIZE)?,
            EFI_CONFIG_TABLE_ENTRY_SIZE,
        )?;
        if bytes_equal(&entry, 0, &EFI_DTB_TABLE_GUID)? {
            let fdt = read_u64(&entry, 16)?;
            PointerRegion::from_u64(fdt, 40)?;
            return Some(fdt);
        }
    }
    None
}

#[cfg(target_os = "uefi")]
unsafe fn efi_image_region(image_handle: u64, system_table: u64) -> Option<(u64, u64)> {
    if image_handle == 0 || system_table == 0 {
        return None;
    }
    let loaded_image = PointerRegion::from_u64(efi_loaded_image(image_handle, system_table)?, 80)?;
    let image_base = read_u64(&loaded_image, 64)?;
    let image_size = read_u64(&loaded_image, 72)?;
    let image_size_usize = usize::try_from(image_size).ok()?;
    PointerRegion::from_u64(image_base, image_size_usize)?;
    Some((image_base, image_size))
}

#[cfg(target_os = "uefi")]
unsafe fn efi_loaded_image(image_handle: u64, system_table: u64) -> Option<u64> {
    if image_handle == 0 {
        return None;
    }
    let boot_services = efi_boot_services_region(system_table)?;
    let handle_protocol = efi_function_address(read_u64(&boot_services, 152)?)?;
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
    PointerRegion::from_u64(loaded_image, 80)?;
    Some(loaded_image)
}

#[cfg(target_os = "uefi")]
unsafe fn efi_file_fdt(image_handle: u64, system_table: u64) -> Option<u64> {
    let loaded_image = PointerRegion::from_u64(efi_loaded_image(image_handle, system_table)?, 80)?;
    let device_handle = read_u64(&loaded_image, 24)?;
    if device_handle == 0 {
        return None;
    }
    let boot_services = efi_boot_services_region(system_table)?;
    let handle_protocol = efi_function_address(read_u64(&boot_services, 152)?)?;
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

    let file_system_region = PointerRegion::from_u64(file_system, 16)?;
    let open_volume = efi_function_address(read_u64(&file_system_region, 8)?)?;
    let open_volume: EfiOpenVolume = core::mem::transmute(open_volume);
    let mut root = 0u64;
    if open_volume(file_system, &mut root) != 0 {
        return None;
    }
    let root_region = PointerRegion::from_u64(root, EFI_FILE_PROTOCOL_SIZE)?;
    let root_close = efi_function_address(read_u64(&root_region, 16)?)?;
    let root_close: EfiFileClose = core::mem::transmute(root_close);
    let open = efi_function_address(read_u64(&root_region, 8)?)?;
    let open: EfiFileOpen = core::mem::transmute(open);
    let mut file = 0u64;
    if open(root, &mut file, EFI_DTB_PATH.as_ptr(), 1, 0) != 0 {
        root_close(root);
        return None;
    }
    let file_region = PointerRegion::from_u64(file, EFI_FILE_PROTOCOL_SIZE)?;
    let close = efi_function_address(read_u64(&file_region, 16)?)?;
    let close: EfiFileClose = core::mem::transmute(close);

    let get_info = efi_function_address(read_u64(&file_region, 64)?)?;
    let get_info: EfiFileGetInfo = core::mem::transmute(get_info);
    let read = efi_function_address(read_u64(&file_region, 32)?)?;
    let read: EfiFileRead = core::mem::transmute(read);
    let allocate_pool = efi_function_address(read_u64(&boot_services, 64)?)?;
    let allocate_pool: EfiAllocatePool = core::mem::transmute(allocate_pool);
    let free_pool = efi_function_address(read_u64(&boot_services, 72)?)?;
    let free_pool: EfiFreePool = core::mem::transmute(free_pool);
    let mut info_size = 128usize;
    let mut info = [0u8; 128];
    let info_region = PointerRegion::new(info.as_ptr() as usize, info.len())?;
    if get_info(
        file,
        EFI_FILE_INFO_GUID.as_ptr(),
        &mut info_size,
        info.as_mut_ptr(),
    ) != 0
    {
        close(file);
        root_close(root);
        return None;
    }
    if info_size < 16 || info_size > info.len() {
        close(file);
        root_close(root);
        return None;
    }
    let file_size = usize::try_from(read_u64(&info_region, 8)?).ok()?;
    if !(40..=MAX_FDT_SIZE).contains(&file_size) {
        close(file);
        root_close(root);
        return None;
    }

    let mut buffer = 0u64;
    if allocate_pool(2, file_size, &mut buffer) != 0 {
        close(file);
        root_close(root);
        return None;
    }
    let buffer_region = match PointerRegion::from_u64(buffer, file_size) {
        Some(region) => region,
        None => {
            close(file);
            root_close(root);
            return None;
        }
    };
    let mut read_size = file_size;
    if read(file, &mut read_size, buffer_region.address as *mut u8) != 0 || read_size != file_size {
        free_pool(buffer);
        close(file);
        root_close(root);
        return None;
    }
    if close(file) != 0 || root_close(root) != 0 {
        free_pool(buffer);
        return None;
    }
    Some(buffer)
}

#[cfg(target_os = "uefi")]
unsafe fn efi_framebuffer(system_table: u64) -> Option<RawFramebuffer> {
    let boot_services = efi_boot_services_region(system_table)?;
    let locate_protocol = efi_function_address(read_u64(&boot_services, 320)?)?;
    let locate_protocol: EfiLocateProtocol = core::mem::transmute(locate_protocol);
    let mut graphics = 0u64;
    if locate_protocol(
        EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID.as_ptr(),
        core::ptr::null(),
        &mut graphics,
    ) != 0
    {
        return None;
    }
    // EFI_GRAPHICS_OUTPUT_PROTOCOL: QueryMode, SetMode, Blt, then Mode.
    let graphics = PointerRegion::from_u64(graphics, EFI_GOP_SIZE)?;
    let mode = PointerRegion::from_u64(read_u64(&graphics, 24)?, EFI_GOP_MODE_SIZE)?;
    let info_size = usize::try_from(read_u64(&mode, 16)?).ok()?;
    if !(EFI_GOP_INFO_SIZE..=4096).contains(&info_size) {
        return None;
    }
    let info = PointerRegion::from_u64(read_u64(&mode, 8)?, info_size)?;
    let width = read_u32(&info, 4)? as u64;
    let height = read_u32(&info, 8)? as u64;
    let format = match read_u32(&info, 12)? {
        0 => PixelFormat::Rgb,
        1 => PixelFormat::Bgr,
        _ => return None,
    };
    let pixels_per_scan_line = read_u32(&info, 32)? as u64;
    let base = read_u64(&mode, 24)?;
    let size = read_u64(&mode, 32)?;
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
    InterruptController,
    Timer,
    Other,
}

#[cfg(target_arch = "aarch64")]
unsafe fn validate_fdt_reservations(blob: &PointerRegion, offset: usize) -> Option<()> {
    for index in 0..1024usize {
        let entry_offset = offset.checked_add(index.checked_mul(16)?)?;
        let entry = blob.subregion(entry_offset, 16)?;
        let address = read_be64(&entry, 0)?;
        let length = read_be64(&entry, 8)?;
        if address == 0 && length == 0 {
            return Some(());
        }
    }
    None
}

#[cfg(target_arch = "aarch64")]
unsafe fn node_kind(structure: &PointerRegion, offset: usize) -> Option<(usize, NodeKind)> {
    let len = string_length(structure, offset)?;
    if len > MAX_FDT_NODE_NAME {
        return None;
    }
    let kind = if c_string_eq(structure, offset, b"chosen") {
        NodeKind::Chosen
    } else if c_string_starts_with(structure, offset, b"memory") {
        NodeKind::Memory
    } else if c_string_starts_with(structure, offset, b"simple-framebuffer") {
        NodeKind::SimpleFramebuffer
    } else if c_string_starts_with(structure, offset, b"intc")
        || c_string_starts_with(structure, offset, b"interrupt-controller")
    {
        NodeKind::InterruptController
    } else if c_string_eq(structure, offset, b"timer") {
        NodeKind::Timer
    } else {
        NodeKind::Other
    };
    Some((len, kind))
}

#[cfg(target_arch = "aarch64")]
unsafe fn parse_reg(
    info: &mut BootInfo,
    structure: &PointerRegion,
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
    structure.subregion(data, length)?;
    let mut offset = 0usize;
    while offset < length {
        let entry_offset = data.checked_add(offset)?;
        let address_bytes = address_cells.checked_mul(4)?;
        let base = read_cells(
            structure,
            entry_offset,
            length.checked_sub(offset)?,
            address_cells,
        )?;
        let region_length = read_cells(
            structure,
            entry_offset.checked_add(address_bytes)?,
            length.checked_sub(offset)?.checked_sub(address_bytes)?,
            size_cells,
        )?;
        if region_length != 0 && base.checked_add(region_length).is_none() {
            return None;
        }
        add_memory(info, base, region_length);
        offset = offset.checked_add(entry_bytes)?;
    }
    Some(())
}

#[cfg(target_arch = "aarch64")]
unsafe fn read_cells(
    structure: &PointerRegion,
    address: usize,
    length: usize,
    cells: usize,
) -> Option<u64> {
    if cells == 0 || cells > 2 || length < cells.checked_mul(4)? {
        return None;
    }
    let mut value = 0u64;
    for i in 0..cells {
        let offset = address.checked_add(i.checked_mul(4)?)?;
        value = value
            .checked_shl(32)?
            .checked_add(read_be32(structure, offset)? as u64)?;
    }
    Some(value)
}

#[cfg(target_arch = "x86_64")]
unsafe fn copy_c_string(
    region: &PointerRegion,
    offset: usize,
    limit: usize,
    out: &mut [u8],
    len: &mut usize,
) -> Option<()> {
    let string = region.subregion(offset, limit)?;
    let mut copied = 0;
    let mut terminated = false;
    for index in 0..limit {
        let byte = read_u8(&string, index)?;
        if byte == 0 {
            terminated = true;
            break;
        }
        if copied < out.len() {
            out[copied] = byte;
            copied += 1;
        }
    }
    if !terminated {
        return None;
    }
    *len = copied;
    Some(())
}

#[cfg(target_arch = "aarch64")]
unsafe fn copy_bytes(
    out: &mut [u8],
    len: &mut usize,
    structure: &PointerRegion,
    address: usize,
    size: usize,
) -> Option<usize> {
    let property = structure.subregion(address, size)?;
    let copied = size.min(out.len());
    for (index, byte) in out.iter_mut().enumerate().take(copied) {
        *byte = read_u8(&property, index)?;
    }
    let copied = out[..copied]
        .iter()
        .rposition(|byte| *byte != 0)
        .map(|index| index + 1)
        .unwrap_or(0);
    *len = copied;
    Some(copied)
}

#[cfg(target_arch = "aarch64")]
unsafe fn string_length(region: &PointerRegion, offset: usize) -> Option<usize> {
    if offset >= region.length {
        return None;
    }
    for index in offset..region.length {
        if read_u8(region, index)? == 0 {
            return Some(index - offset);
        }
    }
    None
}

#[cfg(target_arch = "aarch64")]
unsafe fn c_string_eq(region: &PointerRegion, offset: usize, expected: &[u8]) -> bool {
    string_length(region, offset) == Some(expected.len())
        && c_string_starts_with(region, offset, expected)
}

#[cfg(target_arch = "aarch64")]
unsafe fn c_string_starts_with(region: &PointerRegion, offset: usize, expected: &[u8]) -> bool {
    let Some(bytes) = region.subregion(offset, expected.len()) else {
        return false;
    };
    expected
        .iter()
        .enumerate()
        .all(|(index, byte)| read_u8(&bytes, index).is_some_and(|value| value == *byte))
}

#[cfg(target_arch = "aarch64")]
unsafe fn property_c_string_starts_with(
    structure: &PointerRegion,
    offset: usize,
    length: usize,
    expected: &[u8],
) -> bool {
    let Some(property) = structure.subregion(offset, length) else {
        return false;
    };
    c_string_starts_with(&property, 0, expected)
}

#[cfg(target_arch = "aarch64")]
unsafe fn align4(value: usize) -> Option<usize> {
    value.checked_add(3).map(|value| value & !3)
}

unsafe fn read_u8(region: &PointerRegion, offset: usize) -> Option<u8> {
    let address = region.address_at(offset, 1)?;
    Some((address as *const u8).read_unaligned())
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn read_u32(region: &PointerRegion, offset: usize) -> Option<u32> {
    let address = region.address_at(offset, 4)?;
    Some((address as *const u32).read_unaligned())
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn read_u64(region: &PointerRegion, offset: usize) -> Option<u64> {
    let address = region.address_at(offset, 8)?;
    Some((address as *const u64).read_unaligned())
}

#[cfg(target_arch = "aarch64")]
unsafe fn read_be32(region: &PointerRegion, offset: usize) -> Option<u32> {
    Some(u32::from_be(read_u32(region, offset)?))
}

#[cfg(target_arch = "aarch64")]
unsafe fn read_be64(region: &PointerRegion, offset: usize) -> Option<u64> {
    let high = read_be32(region, offset)? as u64;
    let low = read_be32(region, offset.checked_add(4)?)? as u64;
    Some((high << 32) | low)
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe fn bytes_equal(region: &PointerRegion, offset: usize, expected: &[u8]) -> Option<bool> {
    let bytes = region.subregion(offset, expected.len())?;
    for (index, expected) in expected.iter().enumerate() {
        if read_u8(&bytes, index)? != *expected {
            return Some(false);
        }
    }
    Some(true)
}
