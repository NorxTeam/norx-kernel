use core::{ffi::c_void, ptr::null_mut};

pub type Handle = *mut c_void;
pub type Status = usize;

const SUCCESS: Status = 0;
const ERROR_BIT: usize = 1usize << (usize::BITS as usize - 1);
const GOP_GUID: Guid = Guid::new(
    0x9042a9de,
    0x23dc,
    0x4a38,
    [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a],
);

#[repr(C)]
pub struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

impl Guid {
    const fn new(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        Self {
            data1,
            data2,
            data3,
            data4,
        }
    }
}

#[repr(C)]
pub struct TableHeader {
    signature: u64,
    revision: u32,
    header_size: u32,
    crc32: u32,
    reserved: u32,
}

#[repr(C)]
pub struct SystemTable {
    hdr: TableHeader,
    firmware_vendor: *mut u16,
    firmware_revision: u32,
    console_in_handle: Handle,
    pub con_in: *mut SimpleTextInputProtocol,
    console_out_handle: Handle,
    con_out: *mut c_void,
    stderr_handle: Handle,
    std_err: *mut c_void,
    runtime_services: *mut RuntimeServices,
    boot_services: *mut BootServices,
    number_of_table_entries: usize,
    configuration_table: *mut c_void,
}

#[repr(C)]
pub struct SimpleTextInputProtocol {
    reset: usize,
    pub read_key_stroke: extern "efiapi" fn(*mut SimpleTextInputProtocol, *mut InputKey) -> Status,
    wait_for_key: *mut c_void,
}

#[repr(C)]
pub struct InputKey {
    pub scan_code: u16,
    pub unicode_char: u16,
}

#[repr(C)]
pub struct RuntimeServices {
    hdr: TableHeader,
    get_time: extern "efiapi" fn(*mut Time, *mut c_void) -> Status,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Time {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub pad1: u8,
    pub nanosecond: u32,
    pub time_zone: i16,
    pub daylight: u8,
    pub pad2: u8,
}

#[repr(C)]
pub struct BootServices {
    hdr: TableHeader,
    raise_tpl: usize,
    restore_tpl: usize,
    allocate_pages: usize,
    free_pages: usize,
    get_memory_map: extern "efiapi" fn(
        *mut usize,
        *mut MemoryDescriptor,
        *mut usize,
        *mut usize,
        *mut u32,
    ) -> Status,
    allocate_pool: usize,
    free_pool: usize,
    create_event: usize,
    set_timer: usize,
    wait_for_event: usize,
    signal_event: usize,
    close_event: usize,
    check_event: usize,
    install_protocol_interface: usize,
    reinstall_protocol_interface: usize,
    uninstall_protocol_interface: usize,
    handle_protocol: usize,
    reserved: usize,
    register_protocol_notify: usize,
    locate_handle: usize,
    locate_device_path: usize,
    install_configuration_table: usize,
    load_image: usize,
    start_image: usize,
    exit: usize,
    unload_image: usize,
    exit_boot_services: extern "efiapi" fn(Handle, usize) -> Status,
    get_next_monotonic_count: usize,
    stall: usize,
    set_watchdog_timer: usize,
    connect_controller: usize,
    disconnect_controller: usize,
    open_protocol: usize,
    close_protocol: usize,
    open_protocol_information: usize,
    protocols_per_handle: usize,
    locate_handle_buffer: usize,
    locate_protocol: extern "efiapi" fn(*const Guid, *const c_void, *mut *mut c_void) -> Status,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryDescriptor {
    pub ty: u32,
    pub pad: u32,
    pub physical_start: u64,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}

pub const MEMORY_CONVENTIONAL: u32 = 7;

#[repr(C)]
struct GraphicsOutputProtocol {
    query_mode: usize,
    set_mode: usize,
    blt: usize,
    mode: *mut GraphicsOutputMode,
}

#[repr(C)]
struct GraphicsOutputMode {
    max_mode: u32,
    mode: u32,
    info: *mut GraphicsOutputModeInfo,
    size_of_info: usize,
    framebuffer_base: u64,
    framebuffer_size: usize,
}

#[repr(C)]
struct GraphicsOutputModeInfo {
    version: u32,
    horizontal_resolution: u32,
    vertical_resolution: u32,
    pixel_format: u32,
    pixel_info: [u32; 4],
    pixels_per_scan_line: u32,
}

#[derive(Clone, Copy)]
pub enum PixelFormat {
    Rgb,
    Bgr,
    Other,
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

pub unsafe fn gop_framebuffer(system_table: *mut SystemTable) -> Option<RawFramebuffer> {
    let boot_services = (*system_table).boot_services.as_ref()?;
    let mut protocol = null_mut();
    let status = (boot_services.locate_protocol)(&GOP_GUID, core::ptr::null(), &mut protocol);
    if status != SUCCESS {
        return None;
    }

    let gop = (protocol as *mut GraphicsOutputProtocol).as_ref()?;
    let mode = gop.mode.as_ref()?;
    let info = mode.info.as_ref()?;
    let format = match info.pixel_format {
        0 => PixelFormat::Rgb,
        1 => PixelFormat::Bgr,
        _ => PixelFormat::Other,
    };

    Some(RawFramebuffer {
        base: mode.framebuffer_base as *mut u8,
        size: mode.framebuffer_size,
        width: info.horizontal_resolution,
        height: info.vertical_resolution,
        stride: info.pixels_per_scan_line as usize,
        format,
    })
}

pub unsafe fn get_time(system_table: *mut SystemTable) -> Option<Time> {
    let runtime = (*system_table).runtime_services.as_ref()?;
    let mut time = Time {
        year: 0,
        month: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        pad1: 0,
        nanosecond: 0,
        time_zone: 0,
        daylight: 0,
        pad2: 0,
    };
    if crate::time::ok((runtime.get_time)(&mut time, core::ptr::null_mut())) {
        Some(time)
    } else {
        None
    }
}

pub fn is_error(status: Status) -> bool {
    status & ERROR_BIT != 0
}

pub unsafe fn memory_map(
    system_table: *mut SystemTable,
    buffer: *mut MemoryDescriptor,
    buffer_bytes: usize,
) -> Option<MemoryMapInfo> {
    let boot = (*system_table).boot_services.as_ref()?;
    let mut map_size = buffer_bytes;
    let mut map_key = 0;
    let mut descriptor_size = 0;
    let mut descriptor_version = 0;
    let status = (boot.get_memory_map)(
        &mut map_size,
        buffer,
        &mut map_key,
        &mut descriptor_size,
        &mut descriptor_version,
    );
    if is_error(status) || descriptor_size == 0 {
        return None;
    }
    Some(MemoryMapInfo {
        map_size,
        map_key,
        descriptor_size,
        descriptor_version,
    })
}

pub unsafe fn exit_boot_services(
    image: Handle,
    system_table: *mut SystemTable,
    map_key: usize,
) -> Status {
    let Some(boot) = (*system_table).boot_services.as_ref() else {
        return ERROR_BIT;
    };
    (boot.exit_boot_services)(image, map_key)
}

#[derive(Clone, Copy)]
pub struct MemoryMapInfo {
    pub map_size: usize,
    pub map_key: usize,
    pub descriptor_size: usize,
    pub descriptor_version: u32,
}
