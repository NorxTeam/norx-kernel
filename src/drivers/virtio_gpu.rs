use crate::boot::{PixelFormat, RawFramebuffer};
use crate::drivers::framework::{DmaBuffer, DmaDirection};
use crate::drivers::pci;
use crate::io::{MmioRegion, PioRegion};

const VIRTIO_GPU_CLASS: u32 = 0x0003_0000;
const VIRTIO_VENDOR: u16 = 0x1af4;
const PAGE_SIZE: usize = 4096;
const MAX_QUEUE_SIZE: u16 = 256;
const CONTROL_QUEUE: u16 = 0;
const QUEUE_PAGES: usize = 2;
const REQUEST_SIZE: usize = 64;
const RESPONSE_SIZE: usize = 512;
const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;

const DEVICE_FEATURES: usize = 0x00;
const GUEST_FEATURES: usize = 0x04;
const QUEUE_ADDRESS: usize = 0x08;
const QUEUE_SIZE: usize = 0x0c;
const QUEUE_SELECT: usize = 0x0e;
const QUEUE_NOTIFY: usize = 0x10;
const DEVICE_STATUS: usize = 0x12;

const PCI_CAP_ID_VENDOR: u8 = 0x09;
const PCI_CAP_COMMON_CFG: u8 = 1;
const PCI_CAP_NOTIFY_CFG: u8 = 2;
const VIRTIO_F_VERSION_1: u64 = 1 << 32;

const COMMON_DEVICE_FEATURE_SELECT: usize = 0x00;
const COMMON_DEVICE_FEATURE: usize = 0x04;
const COMMON_DRIVER_FEATURE_SELECT: usize = 0x08;
const COMMON_DRIVER_FEATURE: usize = 0x0c;
const COMMON_NUM_QUEUES: usize = 0x12;
const COMMON_DEVICE_STATUS: usize = 0x14;
const COMMON_QUEUE_SELECT: usize = 0x16;
const COMMON_QUEUE_SIZE: usize = 0x18;
const COMMON_QUEUE_ENABLE: usize = 0x1c;
const COMMON_QUEUE_NOTIFY_OFFSET: usize = 0x1e;
const COMMON_QUEUE_DESC: usize = 0x20;
const COMMON_QUEUE_DRIVER: usize = 0x28;
const COMMON_QUEUE_DEVICE: usize = 0x30;

const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 1 << 1;
const STATUS_DRIVER_OK: u8 = 1 << 2;
const STATUS_FAILED: u8 = 1 << 7;

const CMD_GET_DISPLAY_INFO: u32 = 0x0100;
const CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
const CMD_SET_SCANOUT: u32 = 0x0103;
const CMD_RESOURCE_FLUSH: u32 = 0x0104;
const CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const RESP_OK_NODATA: u32 = 0x1100;
const RESP_OK_DISPLAY_INFO: u32 = 0x1101;
const RESOURCE_ID: u32 = 1;
const FORMAT_B8G8R8A8_UNORM: u32 = 1;
const FORMAT_R8G8B8A8_UNORM: u32 = 67;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidPciBar,
    PciCommand,
    Device,
    InvalidQueue,
    DmaUnavailable,
    AddressTooWide,
    Timeout,
    Protocol,
    InvalidCapability,
    InvalidUsedDescriptor,
    InvalidDisplayResponse(u32),
    ModeMismatch,
    UnsupportedFeatures,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    pub vendor: u16,
    pub device: u16,
    pub irq: u8,
    pub control_queue: u16,
    pub scanout_enabled: bool,
    pub scanout_width: u32,
    pub scanout_height: u32,
    pub guest_mode_width: u32,
    pub guest_mode_height: u32,
    pub mode_switch_supported: bool,
    pub two_d: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitResult {
    Ready(Status),
    Unsupported,
    Failed(Error),
}

#[derive(Clone, Copy)]
struct DmaPage {
    physical: u64,
    virtual_address: usize,
}

impl DmaPage {
    const fn empty() -> Self {
        Self {
            physical: 0,
            virtual_address: 0,
        }
    }

    fn new() -> Result<Self, Error> {
        let physical = crate::memory::alloc_frame().ok_or(Error::DmaUnavailable)?;
        let virtual_address =
            crate::arch::physical_to_virtual(crate::address::PhysAddr::new(physical))
                .ok_or(Error::DmaUnavailable)?
                .value();
        let page = Self {
            physical,
            virtual_address,
        };
        page.clear();
        page.sync_for_device()?;
        Ok(page)
    }

    fn buffer(self) -> DmaBuffer {
        DmaBuffer {
            physical: crate::address::PhysAddr::new(self.physical),
            virtual_address: crate::address::VirtAddr::new(self.virtual_address),
            length: PAGE_SIZE,
            alignment: PAGE_SIZE,
            direction: DmaDirection::Bidirectional,
            owner: 10,
        }
    }

    fn sync_for_device(self) -> Result<(), Error> {
        unsafe { self.buffer().sync_for_device() }.map_err(|_| Error::DmaUnavailable)
    }

    fn sync_for_cpu(self) -> Result<(), Error> {
        unsafe { self.buffer().sync_for_cpu() }.map_err(|_| Error::DmaUnavailable)
    }

    fn clear(self) {
        unsafe { core::ptr::write_bytes(self.virtual_address as *mut u8, 0, PAGE_SIZE) };
    }

    fn write_u32(self, offset: usize, value: u32) {
        assert!(offset + 4 <= PAGE_SIZE && offset.is_multiple_of(4));
        unsafe {
            ((self.virtual_address + offset) as *mut u32).write_volatile(value.to_le());
        }
    }

    fn write_u64(self, offset: usize, value: u64) {
        assert!(offset + 8 <= PAGE_SIZE && offset.is_multiple_of(8));
        unsafe {
            ((self.virtual_address + offset) as *mut u64).write_volatile(value.to_le());
        }
    }

    fn read_u32(self, offset: usize) -> u32 {
        assert!(offset + 4 <= PAGE_SIZE && offset.is_multiple_of(4));
        let value = unsafe { ((self.virtual_address + offset) as *const u32).read_volatile() };
        u32::from_le(value)
    }
}

struct QueueMemory {
    pages: [DmaPage; QUEUE_PAGES],
    length: usize,
    used_offset: usize,
    size: u16,
}

impl QueueMemory {
    fn new(size: u16) -> Result<Self, Error> {
        let Some((_, used_offset, total)) = queue_layout(size) else {
            return Err(Error::InvalidQueue);
        };
        if total > QUEUE_PAGES * PAGE_SIZE {
            return Err(Error::InvalidQueue);
        }
        let mut pages = [DmaPage::empty(); QUEUE_PAGES];
        for page in &mut pages {
            *page = DmaPage::new()?;
        }
        if pages[1].physical != pages[0].physical.saturating_add(PAGE_SIZE as u64) {
            return Err(Error::DmaUnavailable);
        }
        let memory = Self {
            pages,
            length: QUEUE_PAGES * PAGE_SIZE,
            used_offset,
            size,
        };
        memory.clear();
        memory.sync_for_device()?;
        Ok(memory)
    }

    fn physical(&self) -> u64 {
        self.pages[0].physical
    }

    fn virtual_address(&self) -> usize {
        self.pages[0].virtual_address
    }

    fn available_offset(&self) -> usize {
        self.size as usize * 16
    }

    fn clear(&self) {
        unsafe { core::ptr::write_bytes(self.virtual_address() as *mut u8, 0, self.length) };
    }

    fn sync_for_device(&self) -> Result<(), Error> {
        for page in self.pages {
            page.sync_for_device()?;
        }
        Ok(())
    }

    fn sync_for_cpu(&self) -> Result<(), Error> {
        for page in self.pages {
            page.sync_for_cpu()?;
        }
        Ok(())
    }

    fn write_u16(&self, offset: usize, value: u16) {
        assert!(offset + 2 <= self.length && offset.is_multiple_of(2));
        unsafe {
            ((self.virtual_address() + offset) as *mut u16).write_volatile(value.to_le());
        }
    }

    fn write_u32(&self, offset: usize, value: u32) {
        assert!(offset + 4 <= self.length && offset.is_multiple_of(4));
        unsafe {
            ((self.virtual_address() + offset) as *mut u32).write_volatile(value.to_le());
        }
    }

    fn write_u64(&self, offset: usize, value: u64) {
        assert!(offset + 8 <= self.length && offset.is_multiple_of(8));
        unsafe {
            ((self.virtual_address() + offset) as *mut u64).write_volatile(value.to_le());
        }
    }

    fn read_u16(&self, offset: usize) -> u16 {
        assert!(offset + 2 <= self.length && offset.is_multiple_of(2));
        let value = unsafe { ((self.virtual_address() + offset) as *const u16).read_volatile() };
        u16::from_le(value)
    }

    fn read_u32(&self, offset: usize) -> u32 {
        assert!(offset + 4 <= self.length && offset.is_multiple_of(4));
        let value = unsafe { ((self.virtual_address() + offset) as *const u32).read_volatile() };
        u32::from_le(value)
    }
}

struct Queue {
    memory: QueueMemory,
    index: u16,
    available: u16,
    used: u16,
}

impl Queue {
    fn new(io: PioRegion, index: u16, size: u16) -> Result<Self, Error> {
        let memory = QueueMemory::new(size)?;
        if memory.physical() / PAGE_SIZE as u64 > u32::MAX as u64 {
            return Err(Error::AddressTooWide);
        }
        if !io.write_u16(QUEUE_SELECT, index)
            || !io.write_u32(QUEUE_ADDRESS, (memory.physical() / PAGE_SIZE as u64) as u32)
        {
            return Err(Error::Device);
        }
        Ok(Self {
            memory,
            index,
            available: 0,
            used: 0,
        })
    }

    fn descriptor_offset(&self, index: u16) -> usize {
        index as usize * 16
    }

    fn available_offset(&self) -> usize {
        self.memory.size as usize * 16
    }

    fn set_descriptor(&self, index: u16, physical: u64, length: u32, flags: u16, next: u16) {
        let offset = self.descriptor_offset(index);
        self.memory.write_u64(offset, physical);
        self.memory.write_u32(offset + 8, length);
        self.memory.write_u16(offset + 12, flags);
        self.memory.write_u16(offset + 14, next);
    }

    fn submit(&mut self, io: PioRegion, request: DmaPage, response: DmaPage) -> Result<(), Error> {
        self.set_descriptor(0, request.physical, REQUEST_SIZE as u32, DESC_NEXT, 1);
        self.set_descriptor(1, response.physical, RESPONSE_SIZE as u32, DESC_WRITE, 0);
        let ring = self.available_offset() + 4 + (self.available % self.memory.size) as usize * 2;
        self.memory.write_u16(ring, 0);
        self.available = self.available.wrapping_add(1);
        self.memory
            .write_u16(self.available_offset() + 2, self.available);
        request.sync_for_device()?;
        response.sync_for_device()?;
        self.memory.sync_for_device()?;
        if !io.write_u16(QUEUE_NOTIFY, self.index) {
            return Err(Error::Device);
        }
        Ok(())
    }

    fn wait(&mut self, response: DmaPage) -> Result<(), Error> {
        for _ in 0..100_000 {
            self.memory.sync_for_cpu()?;
            let device_used = self.memory.read_u16(self.memory.used_offset + 2);
            if device_used != self.used {
                let offset =
                    self.memory.used_offset + 4 + (self.used % self.memory.size) as usize * 8;
                let descriptor = self.memory.read_u32(offset);
                self.used = self.used.wrapping_add(1);
                if descriptor != 0 {
                    return Err(Error::Protocol);
                }
                response.sync_for_cpu()?;
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Error::Timeout)
    }
}

struct ModernQueue {
    memory: QueueMemory,
    common: MmioRegion,
    notify: MmioRegion,
    notify_multiplier: u32,
    index: u16,
    available: u16,
    used: u16,
}

impl ModernQueue {
    fn new(
        common: MmioRegion,
        notify: MmioRegion,
        notify_multiplier: u32,
        index: u16,
        size: u16,
    ) -> Result<Self, Error> {
        let memory = QueueMemory::new(size)?;
        let available = memory
            .physical()
            .checked_add(memory.available_offset() as u64);
        let used = memory.physical().checked_add(memory.used_offset as u64);
        let Some(available) = available else {
            return Err(Error::AddressTooWide);
        };
        let Some(used) = used else {
            return Err(Error::AddressTooWide);
        };
        if !common.write_u16_le(COMMON_QUEUE_SELECT, index)
            || !common.write_u64_le(COMMON_QUEUE_DESC, memory.physical())
            || !common.write_u64_le(COMMON_QUEUE_DRIVER, available)
            || !common.write_u64_le(COMMON_QUEUE_DEVICE, used)
            || !common.write_u16_le(COMMON_QUEUE_ENABLE, 1)
        {
            return Err(Error::Device);
        }
        Ok(Self {
            memory,
            common,
            notify,
            notify_multiplier,
            index,
            available: 0,
            used: 0,
        })
    }

    fn descriptor_offset(&self, index: u16) -> usize {
        index as usize * 16
    }

    fn set_descriptor(&self, index: u16, physical: u64, length: u32, flags: u16, next: u16) {
        let offset = self.descriptor_offset(index);
        self.memory.write_u64(offset, physical);
        self.memory.write_u32(offset + 8, length);
        self.memory.write_u16(offset + 12, flags);
        self.memory.write_u16(offset + 14, next);
    }

    fn submit(&mut self, request: DmaPage, response: DmaPage) -> Result<(), Error> {
        self.set_descriptor(0, request.physical, REQUEST_SIZE as u32, DESC_NEXT, 1);
        self.set_descriptor(1, response.physical, RESPONSE_SIZE as u32, DESC_WRITE, 0);
        let ring =
            self.memory.available_offset() + 4 + (self.available % self.memory.size) as usize * 2;
        self.memory.write_u16(ring, 0);
        self.available = self.available.wrapping_add(1);
        self.memory
            .write_u16(self.memory.available_offset() + 2, self.available);
        request.sync_for_device()?;
        response.sync_for_device()?;
        self.memory.sync_for_device()?;
        let notify_index = self
            .common
            .read_u16_le(COMMON_QUEUE_NOTIFY_OFFSET)
            .ok_or(Error::Device)?;
        let offset = (notify_index as u32)
            .checked_mul(self.notify_multiplier)
            .ok_or(Error::Device)? as usize;
        if !self.notify.write_u16_le(offset, self.index) {
            return Err(Error::Device);
        }
        Ok(())
    }

    fn wait(&mut self, response: DmaPage) -> Result<(), Error> {
        for _ in 0..100_000 {
            self.memory.sync_for_cpu()?;
            let device_used = self.memory.read_u16(self.memory.used_offset + 2);
            if device_used != self.used {
                let offset =
                    self.memory.used_offset + 4 + (self.used % self.memory.size) as usize * 8;
                let descriptor = self.memory.read_u32(offset);
                self.used = self.used.wrapping_add(1);
                if descriptor != 0 {
                    return Err(Error::InvalidUsedDescriptor);
                }
                response.sync_for_cpu()?;
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Error::Timeout)
    }
}

struct ModernRuntime {
    control: ModernQueue,
    request: DmaPage,
    response: DmaPage,
    status: Status,
}

impl ModernRuntime {
    fn new(
        device: pci::Device,
        common: MmioRegion,
        notify: MmioRegion,
        notify_multiplier: u32,
        framebuffer: Option<RawFramebuffer>,
    ) -> Result<Self, Error> {
        if !common.write_u8(COMMON_DEVICE_STATUS, 0)
            || !common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE)
            || !common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)
        {
            return Err(Error::Device);
        }
        common.write_u32_le(COMMON_DEVICE_FEATURE_SELECT, 0);
        let _device_features_low = common
            .read_u32_le(COMMON_DEVICE_FEATURE)
            .ok_or(Error::Device)?;
        if !common.write_u32_le(COMMON_DEVICE_FEATURE_SELECT, 1) {
            return Err(Error::Device);
        }
        let device_features_high = common
            .read_u32_le(COMMON_DEVICE_FEATURE)
            .ok_or(Error::Device)?;
        if device_features_high & (VIRTIO_F_VERSION_1 >> 32) as u32 == 0 {
            return Err(Error::UnsupportedFeatures);
        }
        common.write_u32_le(COMMON_DRIVER_FEATURE_SELECT, 0);
        if !common.write_u32_le(COMMON_DRIVER_FEATURE, 0)
            || !common.write_u32_le(COMMON_DRIVER_FEATURE_SELECT, 1)
            || !common.write_u32_le(COMMON_DRIVER_FEATURE, 1)
        {
            return Err(Error::Device);
        }
        let queue_count = common.read_u16_le(COMMON_NUM_QUEUES).ok_or(Error::Device)?;
        if queue_count == CONTROL_QUEUE {
            return Err(Error::InvalidQueue);
        }
        if !common.write_u16_le(COMMON_QUEUE_SELECT, CONTROL_QUEUE) {
            return Err(Error::Device);
        }
        let queue_size = common
            .read_u16_le(COMMON_QUEUE_SIZE)
            .ok_or(Error::Device)?
            .min(8);
        if queue_size < 2 || !queue_size.is_power_of_two() {
            return Err(Error::InvalidQueue);
        }
        let control =
            ModernQueue::new(common, notify, notify_multiplier, CONTROL_QUEUE, queue_size)?;
        let request = DmaPage::new()?;
        let response = DmaPage::new()?;
        if !common.write_u8(
            COMMON_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
        ) {
            return Err(Error::Device);
        }
        let mut runtime = Self {
            control,
            request,
            response,
            status: Status {
                bus: device.address.bus,
                slot: device.address.slot,
                function: device.address.function,
                vendor: device.vendor,
                device: device.device,
                irq: device.irq_line(),
                control_queue: queue_size,
                scanout_enabled: false,
                scanout_width: 0,
                scanout_height: 0,
                guest_mode_width: 0,
                guest_mode_height: 0,
                mode_switch_supported: false,
                two_d: false,
            },
        };
        runtime.query_display_info()?;
        if let Some(framebuffer) = framebuffer {
            runtime.configure_framebuffer(framebuffer)?;
        }
        Ok(runtime)
    }

    fn query_display_info(&mut self) -> Result<(), Error> {
        self.request.clear();
        self.response.clear();
        self.request.write_u32(0, CMD_GET_DISPLAY_INFO);
        self.control.submit(self.request, self.response)?;
        self.control.wait(self.response)?;
        let response_type = self.response.read_u32(0);
        if response_type != RESP_OK_DISPLAY_INFO {
            return Err(Error::InvalidDisplayResponse(response_type));
        }
        self.status.scanout_width = self.response.read_u32(24 + 8);
        self.status.scanout_height = self.response.read_u32(24 + 12);
        self.status.scanout_enabled = self.response.read_u32(24 + 16) != 0;
        Ok(())
    }

    fn configure_framebuffer(&mut self, framebuffer: RawFramebuffer) -> Result<(), Error> {
        if framebuffer.bytes_per_pixel != 4
            || framebuffer.width == 0
            || framebuffer.height == 0
            || framebuffer.stride < framebuffer.width as usize * 4
            || framebuffer
                .stride
                .checked_mul(framebuffer.height as usize)
                .is_none_or(|size| size > framebuffer.size)
        {
            return Err(Error::UnsupportedFeatures);
        }
        let Some(framebuffer_size) = u32::try_from(framebuffer.size).ok() else {
            return Err(Error::AddressTooWide);
        };
        let (display_width, display_height) = crate::drivers::display::validate_scanout(
            framebuffer.width,
            framebuffer.height,
            self.status.scanout_width,
            self.status.scanout_height,
        )
        .ok_or(Error::ModeMismatch)?;
        let framebuffer_dma = DmaBuffer {
            physical: crate::address::PhysAddr::new(framebuffer.base as u64),
            virtual_address: crate::address::VirtAddr::new(framebuffer.base as usize),
            length: framebuffer.size,
            alignment: 1,
            direction: DmaDirection::ToDevice,
            owner: 10,
        };
        unsafe { framebuffer_dma.sync_for_device() }.map_err(|_| Error::DmaUnavailable)?;
        let format = match framebuffer.format {
            PixelFormat::Bgr => FORMAT_B8G8R8A8_UNORM,
            PixelFormat::Rgb => FORMAT_R8G8B8A8_UNORM,
        };
        self.command(CMD_RESOURCE_CREATE_2D);
        self.request.write_u32(24, RESOURCE_ID);
        self.request.write_u32(28, format);
        self.request.write_u32(32, framebuffer.width);
        self.request.write_u32(36, framebuffer.height);
        self.expect_ok_nodata()?;

        self.command(CMD_RESOURCE_ATTACH_BACKING);
        self.request.write_u32(24, RESOURCE_ID);
        self.request.write_u32(28, 1);
        self.request.write_u64(32, framebuffer.base as u64);
        self.request.write_u32(40, framebuffer_size);
        self.request.write_u32(44, 0);
        self.expect_ok_nodata()?;

        self.command(CMD_SET_SCANOUT);
        self.write_rect(24, display_width, display_height);
        self.request.write_u32(40, 0);
        self.request.write_u32(44, RESOURCE_ID);
        self.expect_ok_nodata()?;

        self.command(CMD_TRANSFER_TO_HOST_2D);
        self.write_rect(24, display_width, display_height);
        self.request.write_u64(40, 0);
        self.request.write_u32(48, RESOURCE_ID);
        self.request.write_u32(52, 0);
        self.expect_ok_nodata()?;

        self.command(CMD_RESOURCE_FLUSH);
        self.write_rect(24, display_width, display_height);
        self.request.write_u32(40, RESOURCE_ID);
        self.expect_ok_nodata()?;
        self.status.two_d = true;
        self.status.scanout_enabled = true;
        self.status.guest_mode_width = framebuffer.width;
        self.status.guest_mode_height = framebuffer.height;
        Ok(())
    }

    fn command(&mut self, command: u32) {
        self.request.clear();
        self.response.clear();
        self.request.write_u32(0, command);
    }

    fn write_rect(&self, offset: usize, width: u32, height: u32) {
        self.request.write_u32(offset, 0);
        self.request.write_u32(offset + 4, 0);
        self.request.write_u32(offset + 8, width);
        self.request.write_u32(offset + 12, height);
    }

    fn expect_ok_nodata(&mut self) -> Result<(), Error> {
        self.control.submit(self.request, self.response)?;
        self.control.wait(self.response)?;
        let response_type = self.response.read_u32(0);
        if response_type != RESP_OK_NODATA {
            return Err(Error::InvalidDisplayResponse(response_type));
        }
        Ok(())
    }
}

struct Runtime {
    io: PioRegion,
    control: Queue,
    request: DmaPage,
    response: DmaPage,
    status: Status,
}

static mut RUNTIME: Option<Runtime> = None;
static mut MODERN_RUNTIME: Option<ModernRuntime> = None;

pub fn contract_self_check() {
    let (available, used, total) = queue_layout(8).expect("valid virtio queue layout");
    assert_eq!(available, 128);
    assert_eq!(used, PAGE_SIZE);
    assert!(total > used);
    assert_eq!(CMD_GET_DISPLAY_INFO, 0x0100);
    assert_eq!(RESP_OK_NODATA, 0x1100);
    assert_eq!(RESP_OK_DISPLAY_INFO, 0x1101);
    assert!(queue_layout(0).is_none());
    assert!(queue_layout(3).is_none());
    assert!(queue_layout(MAX_QUEUE_SIZE - 1).is_none());
}

pub fn init(framebuffer: Option<RawFramebuffer>) -> InitResult {
    match probe() {
        Ok(Some(runtime)) => {
            let status = runtime.status;
            unsafe { core::ptr::addr_of_mut!(RUNTIME).write(Some(runtime)) };
            InitResult::Ready(status)
        }
        Ok(None) => match probe_modern(framebuffer) {
            Ok(Some(runtime)) => {
                let status = runtime.status;
                unsafe { core::ptr::addr_of_mut!(MODERN_RUNTIME).write(Some(runtime)) };
                InitResult::Ready(status)
            }
            Ok(None) => {
                unsafe {
                    core::ptr::addr_of_mut!(RUNTIME).write(None);
                    core::ptr::addr_of_mut!(MODERN_RUNTIME).write(None);
                }
                InitResult::Unsupported
            }
            Err(error) => {
                unsafe {
                    core::ptr::addr_of_mut!(RUNTIME).write(None);
                    core::ptr::addr_of_mut!(MODERN_RUNTIME).write(None);
                }
                InitResult::Failed(error)
            }
        },
        Err(error) => {
            unsafe {
                core::ptr::addr_of_mut!(RUNTIME).write(None);
                core::ptr::addr_of_mut!(MODERN_RUNTIME).write(None);
            }
            InitResult::Failed(error)
        }
    }
}

fn probe() -> Result<Option<Runtime>, Error> {
    let Some(device) = pci::find_class_vendor(VIRTIO_GPU_CLASS, Some(VIRTIO_VENDOR)) else {
        return Ok(None);
    };
    let Some(pci::Bar::Pio(base)) = device.bar(0) else {
        return Ok(None);
    };
    if !device.enable(true, false, true) {
        return Err(Error::PciCommand);
    }
    let Some(io) = PioRegion::new(base, 0x100) else {
        return Err(Error::InvalidPciBar);
    };
    Runtime::new(device, io).map(Some)
}

fn config_byte(device: pci::Device, offset: u8) -> u8 {
    let value = device.read(offset & !3);
    (value >> ((offset & 3) * 8)) as u8
}

fn config_u32(device: pci::Device, offset: u8) -> u32 {
    device.read(offset)
}

fn cap_region(device: pci::Device, requested_type: u8) -> Result<Option<(MmioRegion, u32)>, Error> {
    let mut pointer = config_byte(device, 0x34) as u16;
    for _ in 0..48 {
        if !(0x40..0x100).contains(&pointer) || pointer & 3 != 0 || pointer + 4 > 0x100 {
            return Ok(None);
        }
        let pointer_byte = pointer as u8;
        if config_byte(device, pointer_byte) == PCI_CAP_ID_VENDOR {
            let length = config_byte(device, pointer_byte + 2);
            let cfg_type = config_byte(device, pointer_byte + 3);
            if cfg_type == requested_type {
                let required_length = if requested_type == PCI_CAP_NOTIFY_CFG {
                    20
                } else {
                    16
                };
                if length < required_length || pointer + length as u16 > 0x100 {
                    return Err(Error::InvalidCapability);
                }
                let bar = config_byte(device, pointer_byte + 4);
                let Some(pci::Bar::Mmio(base)) = device.bar(bar) else {
                    return Err(Error::InvalidPciBar);
                };
                let region_base = base
                    .checked_add(config_u32(device, pointer_byte + 8) as u64)
                    .ok_or(Error::InvalidPciBar)?;
                let region_length = config_u32(device, pointer_byte + 12) as usize;
                let Some(region_base) = usize::try_from(region_base).ok() else {
                    return Err(Error::InvalidPciBar);
                };
                let Some(region) = (unsafe { MmioRegion::new(region_base, region_length) }) else {
                    return Err(Error::InvalidPciBar);
                };
                let multiplier = if requested_type == PCI_CAP_NOTIFY_CFG {
                    config_u32(device, pointer_byte + 16)
                } else {
                    0
                };
                return Ok(Some((region, multiplier)));
            }
        }
        let next = config_byte(device, pointer_byte + 1) as u16;
        if next == pointer {
            return Ok(None);
        }
        pointer = next;
    }
    Err(Error::InvalidCapability)
}

fn probe_modern(framebuffer: Option<RawFramebuffer>) -> Result<Option<ModernRuntime>, Error> {
    let Some(device) = pci::find_class_vendor(VIRTIO_GPU_CLASS, Some(VIRTIO_VENDOR)) else {
        return Ok(None);
    };
    if !device.enable(false, true, true) {
        return Err(Error::PciCommand);
    }
    let Some((common, _)) = cap_region(device, PCI_CAP_COMMON_CFG)? else {
        return Ok(None);
    };
    let Some((notify, notify_multiplier)) = cap_region(device, PCI_CAP_NOTIFY_CFG)? else {
        return Ok(None);
    };
    ModernRuntime::new(device, common, notify, notify_multiplier, framebuffer).map(Some)
}

impl Runtime {
    fn new(device: pci::Device, io: PioRegion) -> Result<Self, Error> {
        let features = io.read_u32(DEVICE_FEATURES).ok_or(Error::Device)?;
        if !io.write_u8(DEVICE_STATUS, 0)
            || !io.write_u8(DEVICE_STATUS, STATUS_ACKNOWLEDGE)
            || !io.write_u8(DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)
        {
            return Err(Error::Device);
        }
        let _ = features;
        if !io.write_u32(GUEST_FEATURES, 0) {
            let _ = io.write_u8(DEVICE_STATUS, STATUS_FAILED);
            return Err(Error::Device);
        }
        let queue_size = queue_size(io, CONTROL_QUEUE)?;
        let queue_size = queue_size.min(8);
        if queue_size < 2 {
            return Err(Error::InvalidQueue);
        }
        let control = Queue::new(io, CONTROL_QUEUE, queue_size)?;
        let request = DmaPage::new()?;
        let response = DmaPage::new()?;
        if !io.write_u8(
            DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
        ) {
            return Err(Error::Device);
        }
        let mut runtime = Self {
            io,
            control,
            request,
            response,
            status: Status {
                bus: device.address.bus,
                slot: device.address.slot,
                function: device.address.function,
                vendor: device.vendor,
                device: device.device,
                irq: device.irq_line(),
                control_queue: queue_size,
                scanout_enabled: false,
                scanout_width: 0,
                scanout_height: 0,
                guest_mode_width: 0,
                guest_mode_height: 0,
                mode_switch_supported: false,
                two_d: false,
            },
        };
        runtime.query_display_info()?;
        Ok(runtime)
    }

    fn query_display_info(&mut self) -> Result<(), Error> {
        self.request.clear();
        self.response.clear();
        self.request.write_u32(0, CMD_GET_DISPLAY_INFO);
        self.control.submit(self.io, self.request, self.response)?;
        self.control.wait(self.response)?;
        let response_type = self.response.read_u32(0);
        if response_type != RESP_OK_DISPLAY_INFO {
            return Err(Error::InvalidDisplayResponse(response_type));
        }
        self.status.scanout_width = self.response.read_u32(24 + 8);
        self.status.scanout_height = self.response.read_u32(24 + 12);
        self.status.scanout_enabled = self.response.read_u32(24 + 16) != 0;
        Ok(())
    }
}

fn queue_size(io: PioRegion, index: u16) -> Result<u16, Error> {
    if !io.write_u16(QUEUE_SELECT, index) {
        return Err(Error::Device);
    }
    let size = io.read_u16(QUEUE_SIZE).ok_or(Error::Device)?;
    if size == 0 || size > MAX_QUEUE_SIZE || !size.is_power_of_two() {
        return Err(Error::InvalidQueue);
    }
    Ok(size)
}

fn queue_layout(queue_size: u16) -> Option<(usize, usize, usize)> {
    if queue_size == 0 || queue_size > MAX_QUEUE_SIZE || !queue_size.is_power_of_two() {
        return None;
    }
    let available = queue_size as usize * 16;
    let available_end = available.checked_add(4 + queue_size as usize * 2)?;
    let used = available_end.checked_add(PAGE_SIZE - 1)? & !(PAGE_SIZE - 1);
    let total = used.checked_add(4 + queue_size as usize * 8)?;
    Some((available, used, total))
}
