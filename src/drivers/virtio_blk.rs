use crate::drivers::framework::{DmaBuffer, DmaDirection};
use crate::drivers::pci;
use crate::io::MmioRegion;

const VIRTIO_BLK_CLASS: u32 = 0x0001_0000;
const VIRTIO_VENDOR: u16 = 0x1af4;
const PAGE_SIZE: usize = 4096;
const SECTOR_SIZE: usize = 512;
const QUEUE_PAGES: usize = 3;
const MAX_QUEUE_SIZE: u16 = 8;
const POLL_LIMIT: usize = 100_000;

const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;

const PCI_CAP_ID_VENDOR: u8 = 0x09;
const PCI_CAP_COMMON_CFG: u8 = 1;
const PCI_CAP_NOTIFY_CFG: u8 = 2;
const PCI_CAP_DEVICE_CFG: u8 = 4;

const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const VIRTIO_BLK_F_RO: u64 = 1 << 5;
const VIRTIO_BLK_F_FLUSH: u64 = 1 << 9;

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
const COMMON_CONFIG_GENERATION: usize = 0x15;
const COMMON_MIN_SIZE: usize = 0x38;

const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 1 << 1;
const STATUS_DRIVER_OK: u8 = 1 << 2;
const STATUS_FEATURES_OK: u8 = 1 << 3;
const STATUS_DEVICE_NEEDS_RESET: u8 = 1 << 6;
const STATUS_FAILED: u8 = 1 << 7;

const REQUEST_IN: u32 = 0;
const REQUEST_OUT: u32 = 1;
const REQUEST_FLUSH: u32 = 4;
const REQUEST_HEADER_SIZE: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidPciBar,
    PciCommand,
    Device,
    InvalidQueue,
    DmaUnavailable,
    AddressTooWide,
    Timeout,
    InvalidCapability,
    UnsupportedFeatures,
    FlushUnsupported,
    FeaturesRejected,
    InvalidUsedDescriptor(u32),
    InvalidUsedLength(u32),
    InvalidUsedCount(u16),
    InvalidRequestStatus(u8),
    RequestFailed(u8),
    DeviceState(u8),
    InvalidCapacity,
    OutOfRange,
    Busy,
    NotReady,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    pub vendor: u16,
    pub device: u16,
    pub capacity_sectors: u64,
    pub sectors: u64,
    pub read_only: bool,
    pub queue_size: u16,
    pub flush: bool,
    pub device_status: u8,
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
            owner: 11,
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

    fn write_u8(self, offset: usize, value: u8) {
        assert!(offset < PAGE_SIZE);
        unsafe { ((self.virtual_address + offset) as *mut u8).write_volatile(value) };
    }

    fn read_u8(self, offset: usize) -> u8 {
        assert!(offset < PAGE_SIZE);
        unsafe { ((self.virtual_address + offset) as *const u8).read_volatile() }
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

    fn copy_from(self, input: &[u8; SECTOR_SIZE]) {
        unsafe {
            core::ptr::copy_nonoverlapping(
                input.as_ptr(),
                self.virtual_address as *mut u8,
                SECTOR_SIZE,
            );
        }
    }

    fn copy_to(self, output: &mut [u8; SECTOR_SIZE]) {
        unsafe {
            core::ptr::copy_nonoverlapping(
                self.virtual_address as *const u8,
                output.as_mut_ptr(),
                SECTOR_SIZE,
            );
        }
    }
}

struct QueueMemory {
    pages: [DmaPage; QUEUE_PAGES],
    size: u16,
}

impl QueueMemory {
    fn new(size: u16) -> Result<Self, Error> {
        let Some((_, _, total)) = queue_layout(size) else {
            return Err(Error::InvalidQueue);
        };
        if total > QUEUE_PAGES * PAGE_SIZE {
            return Err(Error::InvalidQueue);
        }
        let mut pages = [DmaPage::empty(); QUEUE_PAGES];
        for page in &mut pages {
            *page = DmaPage::new()?;
        }
        let memory = Self { pages, size };
        memory.clear();
        memory.sync_for_device()?;
        Ok(memory)
    }

    fn physical(&self) -> u64 {
        self.pages[0].physical
    }

    fn available_physical(&self) -> u64 {
        self.pages[1].physical
    }

    fn used_physical(&self) -> u64 {
        self.pages[2].physical
    }

    fn available_offset(&self) -> usize {
        0
    }

    fn used_offset(&self) -> usize {
        0
    }

    fn clear(&self) {
        for page in self.pages {
            page.clear();
        }
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
        assert!(offset + 2 <= PAGE_SIZE && offset.is_multiple_of(2));
        unsafe {
            ((self.pages[0].virtual_address + offset) as *mut u16).write_volatile(value.to_le());
        }
    }

    fn write_u32(&self, offset: usize, value: u32) {
        assert!(offset + 4 <= PAGE_SIZE && offset.is_multiple_of(4));
        unsafe {
            ((self.pages[0].virtual_address + offset) as *mut u32).write_volatile(value.to_le());
        }
    }

    fn write_u64(&self, offset: usize, value: u64) {
        assert!(offset + 8 <= PAGE_SIZE && offset.is_multiple_of(8));
        unsafe {
            ((self.pages[0].virtual_address + offset) as *mut u64).write_volatile(value.to_le());
        }
    }

    fn write_available_u16(&self, offset: usize, value: u16) {
        assert!(offset + 2 <= PAGE_SIZE && offset.is_multiple_of(2));
        unsafe {
            ((self.pages[1].virtual_address + offset) as *mut u16).write_volatile(value.to_le());
        }
    }

    fn read_u16(&self, offset: usize) -> u16 {
        assert!(offset + 2 <= PAGE_SIZE && offset.is_multiple_of(2));
        let value =
            unsafe { ((self.pages[2].virtual_address + offset) as *const u16).read_volatile() };
        u16::from_le(value)
    }

    fn read_u32(&self, offset: usize) -> u32 {
        assert!(offset + 4 <= PAGE_SIZE && offset.is_multiple_of(4));
        let value =
            unsafe { ((self.pages[2].virtual_address + offset) as *const u32).read_volatile() };
        u32::from_le(value)
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Read,
    Write,
    Flush,
}

impl Operation {
    fn request_type(self) -> u32 {
        match self {
            Self::Read => REQUEST_IN,
            Self::Write => REQUEST_OUT,
            Self::Flush => REQUEST_FLUSH,
        }
    }

    fn has_data(self) -> bool {
        !matches!(self, Self::Flush)
    }

    fn data_is_writable(self) -> bool {
        matches!(self, Self::Read)
    }

    fn expected_used_length(self) -> u32 {
        match self {
            Self::Read => (SECTOR_SIZE + 1) as u32,
            Self::Write | Self::Flush => 1,
        }
    }
}

struct Queue {
    memory: QueueMemory,
    common: MmioRegion,
    notify: MmioRegion,
    notify_multiplier: u32,
    notify_offset: u16,
    index: u16,
    available: u16,
    used: u16,
}

impl Queue {
    fn new(
        common: MmioRegion,
        notify: MmioRegion,
        notify_multiplier: u32,
        index: u16,
        size: u16,
    ) -> Result<Self, Error> {
        if !common.write_u16_le(COMMON_QUEUE_SELECT, index)
            || !common.write_u16_le(COMMON_QUEUE_SIZE, size)
        {
            return Err(Error::InvalidCapability);
        }
        if common.read_u16_le(COMMON_QUEUE_SIZE) != Some(size) {
            return Err(Error::InvalidQueue);
        }
        let notify_offset = common
            .read_u16_le(COMMON_QUEUE_NOTIFY_OFFSET)
            .ok_or(Error::Device)?;
        let memory = QueueMemory::new(size)?;
        if !common.write_u64_le(COMMON_QUEUE_DESC, memory.physical())
            || !common.write_u64_le(COMMON_QUEUE_DRIVER, memory.available_physical())
            || !common.write_u64_le(COMMON_QUEUE_DEVICE, memory.used_physical())
            || !common.write_u16_le(COMMON_QUEUE_ENABLE, 1)
        {
            return Err(Error::Device);
        }
        Ok(Self {
            memory,
            common,
            notify,
            notify_multiplier,
            notify_offset,
            index,
            available: 0,
            used: 0,
        })
    }

    fn set_descriptor(&self, index: usize, physical: u64, length: u32, flags: u16, next: u16) {
        assert!(index < self.memory.size as usize);
        let offset = index * 16;
        self.memory.write_u64(offset, physical);
        self.memory.write_u32(offset + 8, length);
        self.memory.write_u16(offset + 12, flags);
        self.memory.write_u16(offset + 14, next);
    }

    fn submit(
        &mut self,
        operation: Operation,
        request: DmaPage,
        data: Option<DmaPage>,
        status: DmaPage,
    ) -> Result<u32, Error> {
        if self.available != self.used {
            return Err(Error::Busy);
        }
        if operation.has_data() != data.is_some() {
            return Err(Error::Device);
        }
        self.set_descriptor(
            0,
            request.physical,
            REQUEST_HEADER_SIZE as u32,
            DESC_NEXT,
            1,
        );
        let status_index = if let Some(data) = data {
            self.set_descriptor(
                1,
                data.physical,
                SECTOR_SIZE as u32,
                DESC_NEXT
                    | if operation.data_is_writable() {
                        DESC_WRITE
                    } else {
                        0
                    },
                2,
            );
            2
        } else {
            1
        };
        self.set_descriptor(status_index, status.physical, 1, DESC_WRITE, 0);
        let ring =
            self.memory.available_offset() + 4 + (self.available % self.memory.size) as usize * 2;
        self.memory.write_available_u16(ring, 0);
        self.available = self.available.wrapping_add(1);
        self.memory
            .write_available_u16(self.memory.available_offset() + 2, self.available);

        request.sync_for_device()?;
        if let Some(data) = data {
            data.sync_for_device()?;
        }
        status.sync_for_device()?;
        self.memory.sync_for_device()?;
        let offset = (self.notify_offset as u32)
            .checked_mul(self.notify_multiplier)
            .and_then(|offset| usize::try_from(offset).ok())
            .ok_or(Error::AddressTooWide)?;
        if !self.notify.write_u16_le(offset, self.index) {
            return Err(Error::Device);
        }
        Ok(operation.expected_used_length())
    }

    fn wait(
        &mut self,
        operation: Operation,
        data: Option<DmaPage>,
        status: DmaPage,
    ) -> Result<(), Error> {
        let expected_length = operation.expected_used_length();
        for _ in 0..POLL_LIMIT {
            let device_status = self
                .common
                .read_u8(COMMON_DEVICE_STATUS)
                .ok_or(Error::Device)?;
            if device_status & (STATUS_FAILED | STATUS_DEVICE_NEEDS_RESET) != 0
                || device_status & STATUS_DRIVER_OK == 0
            {
                return Err(Error::DeviceState(device_status));
            }
            self.memory.sync_for_cpu()?;
            let device_used = self.memory.read_u16(self.memory.used_offset() + 2);
            let completed = device_used.wrapping_sub(self.used);
            if completed == 0 {
                core::hint::spin_loop();
                continue;
            }
            let offset =
                self.memory.used_offset() + 4 + (self.used % self.memory.size) as usize * 8;
            let descriptor = self.memory.read_u32(offset);
            let used_length = self.memory.read_u32(offset + 4);
            self.used = self.used.wrapping_add(1);
            let validation = validate_used(
                completed,
                descriptor,
                used_length,
                expected_length,
                self.memory.size,
            );
            if let Err(error) = validation {
                self.fail_device();
                return Err(error);
            }
            if let Some(data) = data.filter(|_| operation.data_is_writable()) {
                data.sync_for_cpu()?;
            }
            status.sync_for_cpu()?;
            match status.read_u8(0) {
                0 => return Ok(()),
                1 | 2 => return Err(Error::RequestFailed(status.read_u8(0))),
                value => return Err(Error::InvalidRequestStatus(value)),
            }
        }
        self.fail_device();
        Err(Error::Timeout)
    }

    fn fail_device(&self) {
        if let Some(status) = self.common.read_u8(COMMON_DEVICE_STATUS) {
            let _ = self
                .common
                .write_u8(COMMON_DEVICE_STATUS, status | STATUS_FAILED);
        }
    }
}

struct Runtime {
    common: MmioRegion,
    queue: Queue,
    request: DmaPage,
    data: DmaPage,
    status_page: DmaPage,
    status: Status,
}

impl Runtime {
    fn current_status(&self) -> Status {
        let mut status = self.status;
        if let Some(device_status) = self.common.read_u8(COMMON_DEVICE_STATUS) {
            status.device_status = device_status;
        }
        status
    }

    fn check_ready(&self) -> Result<(), Error> {
        let device_status = self
            .common
            .read_u8(COMMON_DEVICE_STATUS)
            .ok_or(Error::Device)?;
        if device_status & (STATUS_FAILED | STATUS_DEVICE_NEEDS_RESET) != 0
            || device_status & STATUS_DRIVER_OK == 0
        {
            return Err(Error::DeviceState(device_status));
        }
        Ok(())
    }

    fn read_sector(&mut self, lba: u64, output: &mut [u8; SECTOR_SIZE]) -> Result<(), Error> {
        self.request(Operation::Read, lba, Some(output), None)
    }

    fn write_sector(&mut self, lba: u64, input: &[u8; SECTOR_SIZE]) -> Result<(), Error> {
        self.request(Operation::Write, lba, None, Some(input))
    }

    fn flush(&mut self) -> Result<(), Error> {
        self.check_ready()?;
        if !self.status.flush {
            return Err(Error::FlushUnsupported);
        }
        self.request.clear();
        self.request.write_u32(0, REQUEST_FLUSH);
        self.request.write_u32(4, 0);
        self.request.write_u64(8, 0);
        self.status_page.clear();
        self.status_page.write_u8(0, 0xff);
        self.queue
            .submit(Operation::Flush, self.request, None, self.status_page)?;
        self.queue.wait(Operation::Flush, None, self.status_page)
    }

    fn request(
        &mut self,
        operation: Operation,
        lba: u64,
        output: Option<&mut [u8; SECTOR_SIZE]>,
        input: Option<&[u8; SECTOR_SIZE]>,
    ) -> Result<(), Error> {
        self.check_ready()?;
        if lba >= self.status.capacity_sectors {
            return Err(Error::OutOfRange);
        }
        self.request.clear();
        self.request.write_u32(0, operation.request_type());
        self.request.write_u32(4, 0);
        self.request.write_u64(8, lba);
        self.data.clear();
        if let Some(input) = input {
            self.data.copy_from(input);
        }
        self.status_page.clear();
        self.status_page.write_u8(0, 0xff);
        self.queue
            .submit(operation, self.request, Some(self.data), self.status_page)?;
        self.queue
            .wait(operation, Some(self.data), self.status_page)?;
        if let Some(output) = output {
            self.data.copy_to(output);
        }
        Ok(())
    }
}

static mut RUNTIME: Option<Runtime> = None;

pub fn contract_self_check() {
    let (available, used, total) = queue_layout(8).expect("valid virtio-blk queue layout");
    assert_eq!(available, 128);
    assert_eq!(used, PAGE_SIZE);
    assert_eq!(total, PAGE_SIZE + 68);
    assert!(queue_layout(2).is_none());
    assert!(queue_layout(3).is_none());
    assert!(queue_layout(16).is_none());
    assert_eq!(Operation::Read.expected_used_length(), 513);
    assert_eq!(Operation::Write.expected_used_length(), 1);
    assert_eq!(Operation::Flush.expected_used_length(), 1);
    assert!(validate_used(1, 0, 513, 513, 8).is_ok());
    assert!(validate_used(1, 0, 1, 1, 8).is_ok());
    assert_eq!(
        validate_used(1, 1, 513, 513, 8),
        Err(Error::InvalidUsedDescriptor(1))
    );
    assert_eq!(
        validate_used(1, 0, 512, 513, 8),
        Err(Error::InvalidUsedLength(512))
    );
    assert_eq!(VIRTIO_BLK_F_FLUSH, 1 << 9);
    assert_eq!(VIRTIO_F_VERSION_1, 1 << 32);
}

pub fn init() -> InitResult {
    match probe() {
        Ok(Some(runtime)) => {
            let status = runtime.status;
            unsafe {
                core::ptr::addr_of_mut!(RUNTIME).write(Some(runtime));
            }
            InitResult::Ready(status)
        }
        Ok(None) => {
            unsafe {
                core::ptr::addr_of_mut!(RUNTIME).write(None);
            }
            InitResult::Unsupported
        }
        Err(error) => {
            unsafe {
                core::ptr::addr_of_mut!(RUNTIME).write(None);
            }
            InitResult::Failed(error)
        }
    }
}

pub fn status() -> Option<Status> {
    unsafe {
        (*core::ptr::addr_of_mut!(RUNTIME))
            .as_ref()
            .map(Runtime::current_status)
    }
}

pub fn read_sector(lba: u64, output: &mut [u8; SECTOR_SIZE]) -> Result<(), Error> {
    let runtime = unsafe { core::ptr::addr_of_mut!(RUNTIME).as_mut() }
        .and_then(|runtime| runtime.as_mut())
        .ok_or(Error::NotReady)?;
    runtime.read_sector(lba, output)
}

pub fn write_sector(lba: u64, input: &[u8; SECTOR_SIZE]) -> Result<(), Error> {
    let runtime = unsafe { core::ptr::addr_of_mut!(RUNTIME).as_mut() }
        .and_then(|runtime| runtime.as_mut())
        .ok_or(Error::NotReady)?;
    runtime.write_sector(lba, input)
}

pub fn flush() -> Result<(), Error> {
    let runtime = unsafe { core::ptr::addr_of_mut!(RUNTIME).as_mut() }
        .and_then(|runtime| runtime.as_mut())
        .ok_or(Error::NotReady)?;
    runtime.flush()
}

fn probe() -> Result<Option<Runtime>, Error> {
    let Some(device) = pci::find_class_vendor(VIRTIO_BLK_CLASS, Some(VIRTIO_VENDOR)) else {
        return Ok(None);
    };
    if !device.enable(false, true, true) {
        return Err(Error::PciCommand);
    }
    let Some((common, _)) = cap_region(device, PCI_CAP_COMMON_CFG)? else {
        return Err(Error::UnsupportedFeatures);
    };
    let Some((notify, notify_multiplier)) = cap_region(device, PCI_CAP_NOTIFY_CFG)? else {
        return Err(Error::UnsupportedFeatures);
    };
    let Some((config, _)) = cap_region(device, PCI_CAP_DEVICE_CFG)? else {
        return Err(Error::UnsupportedFeatures);
    };
    Runtime::new(device, common, notify, notify_multiplier, config).map(Some)
}

impl Runtime {
    fn new(
        device: pci::Device,
        common: MmioRegion,
        notify: MmioRegion,
        notify_multiplier: u32,
        config: MmioRegion,
    ) -> Result<Self, Error> {
        let result = Self::initialize(device, common, notify, notify_multiplier, config);
        if result.is_err() {
            if let Some(status) = common.read_u8(COMMON_DEVICE_STATUS) {
                let _ = common.write_u8(COMMON_DEVICE_STATUS, status | STATUS_FAILED);
            }
        }
        result
    }

    fn initialize(
        device: pci::Device,
        common: MmioRegion,
        notify: MmioRegion,
        notify_multiplier: u32,
        config: MmioRegion,
    ) -> Result<Self, Error> {
        if !common.write_u8(COMMON_DEVICE_STATUS, 0)
            || common.read_u8(COMMON_DEVICE_STATUS) != Some(0)
            || !common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE)
            || !common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)
        {
            return Err(Error::Device);
        }
        if !common.write_u32_le(COMMON_DEVICE_FEATURE_SELECT, 0) {
            return Err(Error::Device);
        }
        let device_features_low = common
            .read_u32_le(COMMON_DEVICE_FEATURE)
            .ok_or(Error::Device)?;
        if !common.write_u32_le(COMMON_DEVICE_FEATURE_SELECT, 1) {
            return Err(Error::Device);
        }
        let device_features_high = common
            .read_u32_le(COMMON_DEVICE_FEATURE)
            .ok_or(Error::Device)?;
        let device_features = (device_features_high as u64) << 32 | device_features_low as u64;
        if device_features & VIRTIO_F_VERSION_1 == 0 {
            return Err(Error::UnsupportedFeatures);
        }
        let read_only = device_features & VIRTIO_BLK_F_RO != 0;
        let flush = device_features & VIRTIO_BLK_F_FLUSH != 0;
        let driver_features = (if read_only { VIRTIO_BLK_F_RO } else { 0 })
            | (if flush { VIRTIO_BLK_F_FLUSH } else { 0 });
        if !common.write_u32_le(COMMON_DRIVER_FEATURE_SELECT, 0)
            || !common.write_u32_le(COMMON_DRIVER_FEATURE, driver_features as u32)
            || !common.write_u32_le(COMMON_DRIVER_FEATURE_SELECT, 1)
            || !common.write_u32_le(COMMON_DRIVER_FEATURE, (VIRTIO_F_VERSION_1 >> 32) as u32)
            || !common.write_u8(
                COMMON_DEVICE_STATUS,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
            )
        {
            return Err(Error::Device);
        }
        let negotiated_status = common.read_u8(COMMON_DEVICE_STATUS).ok_or(Error::Device)?;
        if negotiated_status & STATUS_FEATURES_OK == 0 {
            return Err(Error::FeaturesRejected);
        }
        if negotiated_status & (STATUS_FAILED | STATUS_DEVICE_NEEDS_RESET) != 0 {
            return Err(Error::DeviceState(negotiated_status));
        }
        let capacity_sectors = read_capacity(common, config)?;
        if capacity_sectors == 0 {
            return Err(Error::InvalidCapacity);
        }
        let queue_count = common.read_u16_le(COMMON_NUM_QUEUES).ok_or(Error::Device)?;
        if queue_count == 0 || !common.write_u16_le(COMMON_QUEUE_SELECT, 0) {
            return Err(Error::InvalidQueue);
        }
        let queue_size = common
            .read_u16_le(COMMON_QUEUE_SIZE)
            .ok_or(Error::Device)?
            .min(MAX_QUEUE_SIZE);
        if queue_size < 4 || !queue_size.is_power_of_two() {
            return Err(Error::InvalidQueue);
        }
        let queue = Queue::new(common, notify, notify_multiplier, 0, queue_size)?;
        if !common.write_u8(
            COMMON_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        ) {
            return Err(Error::Device);
        }
        let device_status = common.read_u8(COMMON_DEVICE_STATUS).ok_or(Error::Device)?;
        if device_status & STATUS_DRIVER_OK == 0
            || device_status & (STATUS_FAILED | STATUS_DEVICE_NEEDS_RESET) != 0
        {
            return Err(Error::DeviceState(device_status));
        }
        Ok(Self {
            common,
            queue,
            request: DmaPage::new()?,
            data: DmaPage::new()?,
            status_page: DmaPage::new()?,
            status: Status {
                bus: device.address.bus,
                slot: device.address.slot,
                function: device.address.function,
                vendor: device.vendor,
                device: device.device,
                capacity_sectors,
                sectors: capacity_sectors,
                read_only,
                queue_size,
                flush,
                device_status,
            },
        })
    }
}

fn validate_used(
    completed: u16,
    descriptor: u32,
    used_length: u32,
    expected_length: u32,
    queue_size: u16,
) -> Result<(), Error> {
    if completed != 1 {
        return Err(Error::InvalidUsedCount(completed));
    }
    if descriptor != 0 || descriptor >= queue_size as u32 {
        return Err(Error::InvalidUsedDescriptor(descriptor));
    }
    if used_length != expected_length {
        return Err(Error::InvalidUsedLength(used_length));
    }
    Ok(())
}

fn read_capacity(common: MmioRegion, config: MmioRegion) -> Result<u64, Error> {
    for _ in 0..4 {
        let before = common
            .read_u8(COMMON_CONFIG_GENERATION)
            .ok_or(Error::Device)?;
        let low = config.read_u32_le(0).ok_or(Error::Device)?;
        let high = config.read_u32_le(4).ok_or(Error::Device)?;
        let after = common
            .read_u8(COMMON_CONFIG_GENERATION)
            .ok_or(Error::Device)?;
        if before == after {
            return Ok((high as u64) << 32 | low as u64);
        }
    }
    Err(Error::Device)
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
    if pointer == 0 {
        return Ok(None);
    }
    for _ in 0..48 {
        if !(0x40..0x100).contains(&pointer) || pointer & 3 != 0 || pointer + 4 > 0x100 {
            return Err(Error::InvalidCapability);
        }
        let pointer_byte = pointer as u8;
        if config_byte(device, pointer_byte) == PCI_CAP_ID_VENDOR {
            let length = config_byte(device, pointer_byte + 2);
            let cfg_type = config_byte(device, pointer_byte + 3);
            let Some(end) = pointer.checked_add(length as u16) else {
                return Err(Error::InvalidCapability);
            };
            let minimum_capability_length = match cfg_type {
                PCI_CAP_COMMON_CFG => 16,
                PCI_CAP_NOTIFY_CFG => 20,
                PCI_CAP_DEVICE_CFG => 16,
                _ => 16,
            };
            if length < minimum_capability_length || end > 0x100 {
                return Err(Error::InvalidCapability);
            }
            if cfg_type == requested_type {
                let minimum_region_length = match requested_type {
                    PCI_CAP_COMMON_CFG => COMMON_MIN_SIZE,
                    PCI_CAP_NOTIFY_CFG => 2,
                    PCI_CAP_DEVICE_CFG => 8,
                    _ => return Err(Error::InvalidCapability),
                };
                let bar = config_byte(device, pointer_byte + 4);
                let Some(pci::Bar::Mmio(base)) = device.bar(bar) else {
                    return Err(Error::InvalidPciBar);
                };
                let region_base = base
                    .checked_add(config_u32(device, pointer_byte + 8) as u64)
                    .ok_or(Error::InvalidPciBar)?;
                let region_length = config_u32(device, pointer_byte + 12) as usize;
                if region_length < minimum_region_length {
                    return Err(Error::InvalidCapability);
                }
                let region_base = usize::try_from(region_base).map_err(|_| Error::InvalidPciBar)?;
                let region = unsafe { MmioRegion::new(region_base, region_length) }
                    .ok_or(Error::InvalidPciBar)?;
                let multiplier = if requested_type == PCI_CAP_NOTIFY_CFG {
                    config_u32(device, pointer_byte + 16)
                } else {
                    0
                };
                return Ok(Some((region, multiplier)));
            }
        }
        let next = config_byte(device, pointer_byte + 1) as u16;
        if next == 0 {
            return Ok(None);
        }
        if next == pointer {
            return Err(Error::InvalidCapability);
        }
        pointer = next;
    }
    Err(Error::InvalidCapability)
}

fn queue_layout(queue_size: u16) -> Option<(usize, usize, usize)> {
    if !(4..=MAX_QUEUE_SIZE).contains(&queue_size) || !queue_size.is_power_of_two() {
        return None;
    }
    let available = queue_size as usize * 16;
    let available_end = available.checked_add(4 + queue_size as usize * 2)?;
    let used = available_end.checked_add(PAGE_SIZE - 1)? & !(PAGE_SIZE - 1);
    let total = used.checked_add(4 + queue_size as usize * 8)?;
    Some((available, used, total))
}
