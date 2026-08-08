use crate::drivers::framework::{
    DmaBuffer, DmaDirection, Driver, DriverError, DriverOps, Resource,
};
use crate::drivers::pci;
use crate::drivers::usb::hid;
use crate::io::MmioRegion;

const XHCI_CLASS: u32 = 0x000c_0330;

const CAP_LENGTH: usize = 0x00;
const HCSPARAMS1: usize = 0x04;
const HCCPARAMS1: usize = 0x10;
const CAP_DBOFF: usize = 0x14;
const CAP_RTSOFF: usize = 0x18;
const OP_USBCMD: usize = 0x00;
const OP_USBSTS: usize = 0x04;
const OP_PAGESIZE: usize = 0x08;
const OP_DNCTRL: usize = 0x14;
const OP_CRCR: usize = 0x18;
const OP_DCBAAP: usize = 0x30;
const OP_CONFIG: usize = 0x38;

const RT_INTR0: usize = 0x20;
const RT_IMAN: usize = 0x00;
const RT_IMOD: usize = 0x04;
const RT_ERSTSZ: usize = 0x08;
const RT_ERSTBA: usize = 0x10;
const RT_ERDP: usize = 0x18;

const PORT_BASE: usize = 0x400;
const PORT_STRIDE: usize = 0x10;
const PORTSC: usize = 0x00;
const PORTSC_CONNECTED: u32 = 1 << 0;
const PORTSC_RESET: u32 = 1 << 4;
const PORTSC_SPEED_SHIFT: u32 = 10;
const PORTSC_SPEED_MASK: u32 = 0x0f << PORTSC_SPEED_SHIFT;
const PORTSC_RESET_CHANGE: u32 = 1 << 21;
const PORTSC_CHANGE_BITS: u32 = 0x00fe_0000;

const USBCMD_RUN_STOP: u32 = 1 << 0;
const USBCMD_HOST_CONTROLLER_RESET: u32 = 1 << 1;
const USBCMD_INTERRUPTER_ENABLE: u32 = 1 << 2;
const USBSTS_HALTED: u32 = 1 << 0;
const USBSTS_CONTROLLER_NOT_READY: u32 = 1 << 11;
const USBSTS_HOST_SYSTEM_ERROR: u32 = 1 << 2;
const ERDP_EVENT_HANDLER_BUSY: u64 = 1 << 3;
const MMIO_SIZE: usize = 0x4000;
const PAGE_SIZE: usize = 4096;
const RING_TRBS: usize = PAGE_SIZE / 16;
const LINK_INDEX: usize = RING_TRBS - 1;
const POLL_LIMIT: usize = 100_000;
const INTERRUPT_TIMEOUT_POLLS: usize = 1_000_000;
const HID_REPORT_DESCRIPTOR_MAX: usize = 256;
const FTDI_VENDOR_ID: u16 = 0x0403;
const FTDI_SERIAL_PRODUCT_ID: u16 = 0x6001;
pub const SERVICE_DEVICE_ID: crate::drivers::framework::DeviceId = 7;

const TRB_NORMAL: u8 = 1;
const TRB_SETUP_STAGE: u8 = 2;
const TRB_DATA_STAGE: u8 = 3;
const TRB_STATUS_STAGE: u8 = 4;
const TRB_LINK: u8 = 6;
const TRB_ENABLE_SLOT: u8 = 9;
const TRB_DISABLE_SLOT: u8 = 10;
const TRB_ADDRESS_DEVICE: u8 = 11;
const TRB_CONFIGURE_ENDPOINT: u8 = 12;
const TRB_STOP_ENDPOINT: u8 = 15;
const TRB_COMMAND_COMPLETION: u8 = 33;
const TRB_TRANSFER_EVENT: u8 = 32;
const TRB_INTERRUPT_ON_COMPLETION: u32 = 1 << 5;
const TRB_CHAIN: u32 = 1 << 4;
const TRB_IMMEDIATE_DATA: u32 = 1 << 6;
const TRB_LINK_TOGGLE_CYCLE: u32 = 1 << 1;

const COMPLETION_SUCCESS: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostControllerKind {
    Xhci,
    Ehci,
    Ohci,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostState {
    Ready,
}

#[derive(Clone, Copy)]
pub struct HostCapabilities {
    pub slots: u8,
    pub interrupters: u16,
    pub ports: u8,
    pub address64: bool,
    pub context_size: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbSpeed {
    Low,
    Full,
    High,
    Super,
    SuperPlus,
    Unknown(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbClass {
    Hid(hid::Kind),
    Serial,
    Storage,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SerialProtocol {
    CdcAcm,
    Ftdi,
}

impl SerialProtocol {
    const fn name(self) -> &'static str {
        match self {
            Self::CdcAcm => "cdc-acm",
            Self::Ftdi => "ftdi",
        }
    }
}

impl UsbClass {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Hid(kind) => kind.name(),
            Self::Serial => "serial",
            Self::Storage => "storage",
            Self::Other => "other",
        }
    }
}

impl UsbSpeed {
    fn from_port_status(value: u32) -> Self {
        match ((value & PORTSC_SPEED_MASK) >> PORTSC_SPEED_SHIFT) as u8 {
            1 => Self::Full,
            2 => Self::Low,
            3 => Self::High,
            4 => Self::Super,
            5 => Self::SuperPlus,
            value => Self::Unknown(value),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Full => "full",
            Self::High => "high",
            Self::Super => "super",
            Self::SuperPlus => "super-plus",
            Self::Unknown(_) => "unknown",
        }
    }

    fn raw(self) -> u8 {
        match self {
            Self::Low => 2,
            Self::Full => 1,
            Self::High => 3,
            Self::Super => 4,
            Self::SuperPlus => 5,
            Self::Unknown(value) => value,
        }
    }

    fn max_packet_size(self) -> u16 {
        match self {
            Self::Low | Self::Full => 8,
            Self::High => 64,
            Self::Super | Self::SuperPlus => 512,
            Self::Unknown(_) => 8,
        }
    }
}

#[derive(Clone, Copy)]
pub struct UsbDeviceInfo {
    pub slot: u8,
    pub address: u8,
    pub speed: UsbSpeed,
    pub vendor_id: u16,
    pub product_id: u16,
    pub configuration: u8,
    pub hub_ports: u8,
    pub hid: hid::Kind,
    pub class: UsbClass,
}

#[derive(Clone, Copy)]
pub struct Status {
    pub kind: HostControllerKind,
    pub state: HostState,
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    pub vendor: u16,
    pub device: u16,
    pub bar: u64,
    pub capabilities: HostCapabilities,
    pub connected_ports: u8,
    pub devices: u8,
    pub configured_devices: u8,
    pub first_device: Option<UsbDeviceInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostError {
    InvalidPciBar,
    InvalidCapabilities,
    RegisterAccess,
    Timeout,
    DmaUnavailable,
    RingFull,
    InvalidPort,
    Protocol,
    Completion(u8),
    DeviceAbsent,
}

#[derive(Clone, Copy)]
pub enum InitResult {
    Ready(Status),
    Unsupported,
    Failed(HostError),
}

pub trait HostController {
    fn kind(&self) -> HostControllerKind;
    fn capabilities(&self) -> HostCapabilities;
    fn reset(&mut self) -> Result<(), HostError>;
    fn connected_ports(&self) -> u8;
}

struct Xhci {
    mmio: MmioRegion,
    operational: usize,
    doorbell: usize,
    runtime: usize,
    capabilities: HostCapabilities,
}

static mut STATUS: Option<Status> = None;
static mut RUNTIME: Option<Runtime> = None;

pub fn contract_self_check() {
    let kinds = [
        HostControllerKind::Xhci,
        HostControllerKind::Ehci,
        HostControllerKind::Ohci,
    ];
    assert_eq!(kinds[0], HostControllerKind::Xhci);
    assert_eq!(kind_name(HostControllerKind::Ehci), "ehci");
    assert_eq!(kind_name(HostControllerKind::Ohci), "ohci");
    assert_eq!(parse_capabilities(0x0500_0408, 1), Some((8, 4, 5, true)));
    assert!(parse_capabilities(0, 0).is_none());
    assert_eq!(UsbSpeed::from_port_status(1 << 10), UsbSpeed::Full);
    assert_eq!(UsbSpeed::High.name(), "high");

    let setup = SetupPacket::get_descriptor(0x0100, 18);
    let trb = Trb::setup(setup, false);
    assert_eq!(trb.kind(), TRB_SETUP_STAGE);
    assert_eq!(trb.words[2] & 0x1ffff, 8);
    assert_eq!(Trb::normal(0x1000, 64).kind(), TRB_NORMAL);
    assert_eq!(Trb::link(0x2000).kind(), TRB_LINK);
}

pub fn init() -> InitResult {
    let result = probe();
    unsafe {
        core::ptr::addr_of_mut!(STATUS).write(match result {
            InitResult::Ready(status) => Some(status),
            InitResult::Unsupported | InitResult::Failed(_) => None,
        });
        if !matches!(result, InitResult::Ready(_)) {
            core::ptr::addr_of_mut!(RUNTIME).write(None);
        }
    }
    result
}

pub fn status() -> Option<Status> {
    unsafe { core::ptr::addr_of!(STATUS).read() }
}

pub fn poll() {
    unsafe {
        let runtime = core::ptr::addr_of_mut!(RUNTIME);
        if let Some(runtime) = (*runtime).as_mut() {
            runtime.poll();
        }
    }
}

pub fn service_driver() -> Driver {
    Driver::with_ops(
        SERVICE_DEVICE_ID,
        "xhci-userspace-service",
        crate::drivers::framework::Class::UsbHost,
        crate::drivers::framework::BusKind::Pci,
        DriverOps {
            probe: service_probe,
            suspend: None,
            resume: None,
            quiesce: Some(service_quiesce_device),
            remove: service_remove_device,
        },
    )
}

pub fn service_dma() -> Option<DmaBuffer> {
    unsafe {
        let runtime = core::ptr::addr_of_mut!(RUNTIME);
        (*runtime)
            .as_ref()
            .map(|runtime| runtime.dcbaa.buffer(DmaDirection::Bidirectional))
    }
}

pub fn service_quiesce() -> Result<(), HostError> {
    unsafe {
        let runtime = core::ptr::addr_of_mut!(RUNTIME);
        (*runtime)
            .as_mut()
            .ok_or(HostError::DeviceAbsent)?
            .stop_host()
    }
}

pub fn service_remove() -> Result<(), HostError> {
    service_quiesce()?;
    unsafe {
        core::ptr::addr_of_mut!(RUNTIME).write(None);
        core::ptr::addr_of_mut!(STATUS).write(None);
    }
    Ok(())
}

fn service_probe(device: &mut crate::drivers::framework::Device) -> Result<(), DriverError> {
    if status().is_none() {
        match init() {
            InitResult::Ready(_) => {}
            InitResult::Unsupported => return Err(DriverError::DeviceAbsent),
            InitResult::Failed(_) => return Err(DriverError::ProbeFailed),
        }
    }
    let status = status().ok_or(DriverError::DeviceAbsent)?;
    device.add_resource(Resource::Mmio {
        base: status.bar,
        size: MMIO_SIZE as u64,
        owner: device.id,
    })?;
    let dma = service_dma().ok_or(DriverError::DmaFailure)?;
    if dma.owner != device.id {
        return Err(DriverError::ResourceOwnerMismatch);
    }
    device.add_resource(Resource::Dma(dma))
}

fn service_quiesce_device(
    device: &mut crate::drivers::framework::Device,
) -> Result<(), DriverError> {
    let _ = device;
    service_quiesce().map_err(|_| DriverError::Timeout)
}

fn service_remove_device(
    device: &mut crate::drivers::framework::Device,
) -> Result<(), DriverError> {
    let _ = device;
    service_remove().map_err(|_| DriverError::RemoveFailed)
}

fn probe() -> InitResult {
    let Some(pci) = pci::find_class(XHCI_CLASS) else {
        return InitResult::Unsupported;
    };
    let Some(pci::Bar::Mmio(bar)) = pci.bar(0) else {
        return InitResult::Failed(HostError::InvalidPciBar);
    };
    if !pci.enable(false, true, true) {
        return InitResult::Failed(HostError::InvalidPciBar);
    }
    let Some(mmio) = (unsafe { MmioRegion::new(bar as usize, MMIO_SIZE) }) else {
        return InitResult::Failed(HostError::InvalidPciBar);
    };
    let Some((cap_length, capabilities)) = read_capabilities(mmio) else {
        return InitResult::Failed(HostError::InvalidCapabilities);
    };
    let Some(mut controller) = Xhci::new(mmio, cap_length, capabilities) else {
        return InitResult::Failed(HostError::InvalidCapabilities);
    };
    if let Err(error) = controller.reset() {
        crate::bootlog::warn_fmt(format_args!("xHCI controller reset failed: {:?}", error));
        return InitResult::Failed(error);
    }
    crate::bootlog::info("xHCI controller reset complete");
    let mut runtime = match Runtime::new(controller) {
        Ok(runtime) => runtime,
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!("xHCI DMA ring allocation failed: {:?}", error));
            return InitResult::Failed(error);
        }
    };
    if let Err(error) = runtime.start_host() {
        crate::bootlog::warn_fmt(format_args!("xHCI runtime start failed: {:?}", error));
        return InitResult::Failed(error);
    }
    crate::bootlog::info("xHCI command and event rings active");
    match runtime.enumerate_first() {
        Ok(_) => {}
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!("xHCI USB enumeration failed: {:?}", error));
            return InitResult::Failed(error);
        }
    }
    runtime.connected_ports = runtime.controller.connected_ports();
    let status = runtime.status(pci, bar);
    unsafe { core::ptr::addr_of_mut!(RUNTIME).write(Some(runtime)) };
    InitResult::Ready(Status {
        state: HostState::Ready,
        ..status
    })
}

impl Xhci {
    fn new(mmio: MmioRegion, cap_length: usize, capabilities: HostCapabilities) -> Option<Self> {
        let operational = cap_length;
        operational.checked_add(OP_CONFIG)?;
        let doorbell = (mmio.read_u32_le(CAP_DBOFF)? as usize) & !3;
        let runtime = (mmio.read_u32_le(CAP_RTSOFF)? as usize) & !31;
        if doorbell.checked_add(4)? > MMIO_SIZE || runtime.checked_add(RT_INTR0 + 0x40)? > MMIO_SIZE
        {
            return None;
        }
        Some(Self {
            mmio,
            operational,
            doorbell,
            runtime,
            capabilities,
        })
    }

    fn port_offset(&self, port: u8) -> Option<usize> {
        if port >= self.capabilities.ports {
            return None;
        }
        self.operational
            .checked_add(PORT_BASE)?
            .checked_add(port as usize * PORT_STRIDE)
    }

    fn port_status(&self, port: u8) -> Option<u32> {
        self.port_offset(port)
            .and_then(|offset| self.mmio.read_u32_le(offset + PORTSC))
    }

    fn reset_port(&self, port: u8) -> Result<UsbSpeed, HostError> {
        let offset = self.port_offset(port).ok_or(HostError::InvalidPort)?;
        let current = self
            .mmio
            .read_u32_le(offset + PORTSC)
            .ok_or(HostError::RegisterAccess)?;
        if current & PORTSC_CONNECTED == 0 {
            return Err(HostError::InvalidPort);
        }
        let command = (current & !PORTSC_CHANGE_BITS) | PORTSC_RESET;
        if !self.mmio.write_u32_le(offset + PORTSC, command) {
            return Err(HostError::RegisterAccess);
        }
        for _ in 0..POLL_LIMIT {
            let value = self
                .mmio
                .read_u32_le(offset + PORTSC)
                .ok_or(HostError::RegisterAccess)?;
            if value & PORTSC_RESET == 0 {
                if value & PORTSC_CONNECTED == 0 {
                    return Err(HostError::InvalidPort);
                }
                let speed = UsbSpeed::from_port_status(value);
                let clear_changes = value & PORTSC_CHANGE_BITS;
                if clear_changes != 0 {
                    let _ = self
                        .mmio
                        .write_u32_le(offset + PORTSC, clear_changes | PORTSC_RESET_CHANGE);
                }
                return Ok(speed);
            }
        }
        Err(HostError::Timeout)
    }
}

impl HostController for Xhci {
    fn kind(&self) -> HostControllerKind {
        HostControllerKind::Xhci
    }

    fn capabilities(&self) -> HostCapabilities {
        self.capabilities
    }

    fn reset(&mut self) -> Result<(), HostError> {
        let command = self
            .mmio
            .read_u32_le(self.operational + OP_USBCMD)
            .ok_or(HostError::RegisterAccess)?;
        if command & USBCMD_RUN_STOP != 0
            && !self
                .mmio
                .write_u32_le(self.operational + OP_USBCMD, command & !USBCMD_RUN_STOP)
        {
            return Err(HostError::RegisterAccess);
        }
        if !wait_for_status(self.mmio, self.operational, USBSTS_HALTED, true) {
            return Err(HostError::Timeout);
        }
        if !self
            .mmio
            .write_u32_le(self.operational + OP_USBCMD, USBCMD_HOST_CONTROLLER_RESET)
        {
            return Err(HostError::RegisterAccess);
        }
        if !wait_for_command_clear(self.mmio, self.operational, USBCMD_HOST_CONTROLLER_RESET) {
            return Err(HostError::Timeout);
        }
        if !wait_for_status(
            self.mmio,
            self.operational,
            USBSTS_CONTROLLER_NOT_READY,
            false,
        ) {
            return Err(HostError::Timeout);
        }
        if !self
            .mmio
            .write_u32_le(self.operational + OP_CONFIG, self.capabilities.slots as u32)
        {
            return Err(HostError::RegisterAccess);
        }
        Ok(())
    }

    fn connected_ports(&self) -> u8 {
        let mut connected: u8 = 0;
        let mut port = 0;
        while port < self.capabilities.ports {
            if self
                .port_status(port)
                .is_some_and(|portsc| portsc & PORTSC_CONNECTED != 0)
            {
                connected = connected.saturating_add(1);
            }
            port += 1;
        }
        connected
    }
}

fn read_capabilities(mmio: MmioRegion) -> Option<(usize, HostCapabilities)> {
    let cap_length = mmio.read_u8(CAP_LENGTH)? as usize;
    if !(0x20..MMIO_SIZE).contains(&cap_length) {
        return None;
    }
    let params = mmio.read_u32_le(HCSPARAMS1)?;
    let hcc = mmio.read_u32_le(HCCPARAMS1)?;
    let packed = parse_capabilities(params, hcc)?;
    Some((
        cap_length,
        HostCapabilities {
            slots: packed.0,
            interrupters: packed.1,
            ports: packed.2,
            address64: packed.3,
            context_size: if hcc & (1 << 2) != 0 { 64 } else { 32 },
        },
    ))
}

fn parse_capabilities(params: u32, hcc: u32) -> Option<(u8, u16, u8, bool)> {
    let slots = (params & 0xff) as u8;
    let interrupters = ((params >> 8) & 0x7ff) as u16;
    let ports = (params >> 24) as u8;
    if slots == 0 || interrupters == 0 || ports == 0 {
        return None;
    }
    Some((slots, interrupters, ports, hcc & 1 != 0))
}

fn wait_for_status(mmio: MmioRegion, operational: usize, mask: u32, set: bool) -> bool {
    for _ in 0..POLL_LIMIT {
        let Some(status) = mmio.read_u32_le(operational + OP_USBSTS) else {
            return false;
        };
        if (status & mask != 0) == set {
            return status & USBSTS_HOST_SYSTEM_ERROR == 0;
        }
    }
    false
}

fn wait_for_command_clear(mmio: MmioRegion, operational: usize, mask: u32) -> bool {
    for _ in 0..POLL_LIMIT {
        let Some(command) = mmio.read_u32_le(operational + OP_USBCMD) else {
            return false;
        };
        if command & mask == 0 {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy)]
struct DmaPage {
    physical: u64,
    virtual_address: usize,
}

impl DmaPage {
    fn new() -> Result<Self, HostError> {
        let physical = crate::memory::alloc_frame().ok_or(HostError::DmaUnavailable)?;
        let virtual_address =
            crate::arch::physical_to_virtual(crate::address::PhysAddr::new(physical))
                .ok_or(HostError::DmaUnavailable)?
                .value();
        let page = Self {
            physical,
            virtual_address,
        };
        page.clear();
        page.buffer(DmaDirection::Bidirectional)
            .validate()
            .map_err(|_| HostError::DmaUnavailable)?;
        Ok(page)
    }

    fn buffer(self, direction: DmaDirection) -> DmaBuffer {
        DmaBuffer {
            physical: crate::address::PhysAddr::new(self.physical),
            virtual_address: crate::address::VirtAddr::new(self.virtual_address),
            length: PAGE_SIZE,
            alignment: PAGE_SIZE,
            direction,
            owner: SERVICE_DEVICE_ID,
        }
    }

    fn sync_for_device(self) -> Result<(), HostError> {
        unsafe { self.buffer(DmaDirection::Bidirectional).sync_for_device() }
            .map_err(|_| HostError::DmaUnavailable)
    }

    fn sync_for_cpu(self) -> Result<(), HostError> {
        unsafe { self.buffer(DmaDirection::Bidirectional).sync_for_cpu() }
            .map_err(|_| HostError::DmaUnavailable)
    }

    fn clear(self) {
        unsafe { core::ptr::write_bytes(self.virtual_address as *mut u8, 0, PAGE_SIZE) };
    }

    fn read_u8(self, offset: usize) -> u8 {
        assert!(offset < PAGE_SIZE);
        unsafe { (self.virtual_address as *const u8).add(offset).read() }
    }

    fn write_u8(self, offset: usize, value: u8) {
        assert!(offset < PAGE_SIZE);
        unsafe { (self.virtual_address as *mut u8).add(offset).write(value) };
    }

    fn write_u32(self, offset: usize, value: u32) {
        assert!(offset + 4 <= PAGE_SIZE && offset.is_multiple_of(4));
        unsafe {
            (self.virtual_address as *mut u32)
                .add(offset / 4)
                .write(value.to_le())
        };
    }

    fn read_u32(self, offset: usize) -> u32 {
        assert!(offset + 4 <= PAGE_SIZE && offset.is_multiple_of(4));
        unsafe { u32::from_le((self.virtual_address as *const u32).add(offset / 4).read()) }
    }

    fn write_u64(self, offset: usize, value: u64) {
        assert!(offset + 8 <= PAGE_SIZE && offset.is_multiple_of(8));
        unsafe {
            (self.virtual_address as *mut u64)
                .add(offset / 8)
                .write(value.to_le())
        };
    }

    fn read_trb(self, index: usize) -> Trb {
        let offset = index * 16;
        Trb {
            words: [
                self.read_u32(offset),
                self.read_u32(offset + 4),
                self.read_u32(offset + 8),
                self.read_u32(offset + 12),
            ],
        }
    }

    fn write_trb(self, index: usize, trb: Trb) {
        let offset = index * 16;
        self.write_u32(offset, trb.words[0]);
        self.write_u32(offset + 4, trb.words[1]);
        self.write_u32(offset + 8, trb.words[2]);
        self.write_u32(offset + 12, trb.words[3]);
    }
}

#[derive(Clone, Copy)]
struct Trb {
    words: [u32; 4],
}

impl Trb {
    fn new(parameter: u64, status: u32, control: u32) -> Self {
        Self {
            words: [parameter as u32, (parameter >> 32) as u32, status, control],
        }
    }

    fn kind(self) -> u8 {
        ((self.words[3] >> 10) & 0x3f) as u8
    }

    fn link(physical: u64) -> Self {
        Self::new(physical, 0, (TRB_LINK as u32) << 10 | TRB_LINK_TOGGLE_CYCLE)
    }

    fn setup(setup: SetupPacket, data_in: bool) -> Self {
        let transfer_type = if setup.length == 0 {
            0
        } else if data_in {
            3
        } else {
            2
        };
        Self::new(
            setup.value(),
            8,
            (TRB_SETUP_STAGE as u32) << 10 | TRB_IMMEDIATE_DATA | (transfer_type << 16),
        )
    }

    fn data(physical: u64, length: usize, data_in: bool) -> Self {
        let direction = if data_in { 1 << 16 } else { 0 };
        Self::new(
            physical,
            length as u32,
            (TRB_DATA_STAGE as u32) << 10 | direction | TRB_CHAIN,
        )
    }

    fn status(data_in: bool) -> Self {
        let direction = if data_in { 1 << 16 } else { 0 };
        Self::new(
            0,
            0,
            (TRB_STATUS_STAGE as u32) << 10 | direction | TRB_INTERRUPT_ON_COMPLETION,
        )
    }

    fn normal(physical: u64, length: usize) -> Self {
        Self::new(
            physical,
            length as u32,
            (TRB_NORMAL as u32) << 10 | TRB_INTERRUPT_ON_COMPLETION,
        )
    }
}

struct TrbRing {
    page: DmaPage,
    enqueue: usize,
    cycle: bool,
}

impl TrbRing {
    fn new() -> Result<Self, HostError> {
        let page = DmaPage::new()?;
        page.write_trb(LINK_INDEX, Trb::link(page.physical));
        page.sync_for_device()?;
        Ok(Self {
            page,
            enqueue: 0,
            cycle: true,
        })
    }

    fn next_pointer(&self) -> u64 {
        (self.page.physical + (self.enqueue * 16) as u64) | self.cycle as u64
    }

    fn push(&mut self, mut trb: Trb) -> Result<(), HostError> {
        if self.enqueue == LINK_INDEX {
            self.page.write_trb(
                LINK_INDEX,
                Trb::link_with_cycle(self.page.physical, self.cycle),
            );
            self.enqueue = 0;
            self.cycle = !self.cycle;
        }
        trb.words[3] = (trb.words[3] & !1) | self.cycle as u32;
        self.page.write_trb(self.enqueue, trb);
        self.enqueue += 1;
        Ok(())
    }

    fn sync_for_device(&self) -> Result<(), HostError> {
        self.page.sync_for_device()
    }

    fn reset(&mut self) {
        self.page.clear();
        self.enqueue = 0;
        self.cycle = true;
        self.page
            .write_trb(LINK_INDEX, Trb::link(self.page.physical));
    }
}

impl Trb {
    fn link_with_cycle(physical: u64, cycle: bool) -> Self {
        let mut trb = Self::link(physical);
        trb.words[3] = (trb.words[3] & !1) | cycle as u32;
        trb
    }
}

#[derive(Clone, Copy)]
struct Event {
    kind: u8,
    completion: u8,
    slot: u8,
    endpoint: u8,
    remaining: u32,
}

struct EventRing {
    page: DmaPage,
    erst: DmaPage,
    dequeue: usize,
    cycle: bool,
}

impl EventRing {
    fn new() -> Result<Self, HostError> {
        let page = DmaPage::new()?;
        let erst = DmaPage::new()?;
        erst.write_u64(0, page.physical);
        erst.write_u32(8, RING_TRBS as u32);
        erst.sync_for_device()?;
        Ok(Self {
            page,
            erst,
            dequeue: 0,
            cycle: true,
        })
    }

    fn poll(&mut self, controller: &Xhci) -> Result<Option<Event>, HostError> {
        self.page.sync_for_cpu()?;
        let trb = self.page.read_trb(self.dequeue);
        if (trb.words[3] & 1 != 0) != self.cycle {
            return Ok(None);
        }
        self.dequeue += 1;
        if self.dequeue == RING_TRBS {
            self.dequeue = 0;
            self.cycle = !self.cycle;
        }
        let pointer = self.page.physical + (self.dequeue * 16) as u64;
        if !controller.mmio.write_u64_le(
            controller.runtime + RT_INTR0 + RT_ERDP,
            pointer | ERDP_EVENT_HANDLER_BUSY,
        ) {
            return Err(HostError::RegisterAccess);
        }
        let iman = controller
            .mmio
            .read_u32_le(controller.runtime + RT_INTR0 + RT_IMAN)
            .ok_or(HostError::RegisterAccess)?;
        if !controller
            .mmio
            .write_u32_le(controller.runtime + RT_INTR0 + RT_IMAN, iman | 1)
        {
            return Err(HostError::RegisterAccess);
        }
        Ok(Some(Event {
            kind: ((trb.words[3] >> 10) & 0x3f) as u8,
            completion: (trb.words[2] >> 24) as u8,
            slot: (trb.words[3] >> 24) as u8,
            endpoint: ((trb.words[3] >> 16) & 0x1f) as u8,
            remaining: trb.words[2] & 0x00ff_ffff,
        }))
    }
}

#[derive(Clone, Copy)]
struct SetupPacket {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
}

impl SetupPacket {
    fn get_descriptor(value: u16, length: u16) -> Self {
        Self {
            request_type: 0x80,
            request: 6,
            value,
            index: 0,
            length,
        }
    }

    fn set_configuration(value: u8) -> Self {
        Self {
            request_type: 0,
            request: 9,
            value: value as u16,
            index: 0,
            length: 0,
        }
    }

    fn hub_descriptor(length: u16) -> Self {
        Self {
            request_type: 0xa0,
            request: 6,
            value: 0x2900,
            index: 0,
            length,
        }
    }

    fn hid_report_descriptor(interface: u8, length: u16) -> Self {
        Self {
            request_type: 0x81,
            request: 6,
            value: 0x2200,
            index: interface as u16,
            length,
        }
    }

    fn cdc_line_coding(interface: u8) -> Self {
        Self {
            request_type: 0x21,
            request: 0x20,
            value: 0,
            index: interface as u16,
            length: 7,
        }
    }

    fn cdc_control_line_state(interface: u8, state: u16) -> Self {
        Self {
            request_type: 0x21,
            request: 0x22,
            value: state,
            index: interface as u16,
            length: 0,
        }
    }

    fn ftdi_baud_rate(interface: u8) -> Self {
        Self {
            request_type: 0x40,
            request: 3,
            value: 0x4138,
            index: interface as u16,
            length: 0,
        }
    }

    fn ftdi_modem_control(interface: u8) -> Self {
        Self {
            request_type: 0x40,
            request: 1,
            value: 0x0303,
            index: interface as u16,
            length: 0,
        }
    }

    fn value(self) -> u64 {
        self.request_type as u64
            | (self.request as u64) << 8
            | (self.value as u64) << 16
            | (self.index as u64) << 32
            | (self.length as u64) << 48
    }

    fn direction_in(self) -> bool {
        self.request_type & 0x80 != 0
    }
}

#[derive(Clone, Copy)]
struct EndpointInfo {
    id: u8,
    max_packet: u16,
    interval: u8,
    transfer_type: u8,
}

#[derive(Clone, Copy)]
struct ConfigurationInfo {
    value: u8,
    interrupt_in: Option<EndpointInfo>,
    bulk_in: Option<EndpointInfo>,
    bulk_out: Option<EndpointInfo>,
    hub: bool,
    hid: bool,
    serial: bool,
    serial_protocol: Option<SerialProtocol>,
    storage: bool,
    interface: u8,
    control_interface: u8,
    report_length: u16,
}

struct InterruptEndpoint {
    info: EndpointInfo,
    ring: TrbRing,
    data: DmaPage,
    pending: bool,
    polls: usize,
}

struct BulkEndpoint {
    info: EndpointInfo,
    ring: TrbRing,
    data: DmaPage,
}

#[derive(Clone, Copy)]
enum DataTransferKind {
    Bulk,
    Interrupt,
}

struct Runtime {
    controller: Xhci,
    dcbaa: DmaPage,
    command: TrbRing,
    events: EventRing,
    input: DmaPage,
    device_context: DmaPage,
    ep0: TrbRing,
    data: DmaPage,
    port: u8,
    speed: UsbSpeed,
    slot: u8,
    connected_ports: u8,
    first_device: Option<UsbDeviceInfo>,
    interrupt: Option<InterruptEndpoint>,
    bulk_in: Option<BulkEndpoint>,
    bulk_out: Option<BulkEndpoint>,
    hid: Option<hid::Device>,
}

impl Runtime {
    fn new(controller: Xhci) -> Result<Self, HostError> {
        Ok(Self {
            controller,
            dcbaa: DmaPage::new()?,
            command: TrbRing::new()?,
            events: EventRing::new()?,
            input: DmaPage::new()?,
            device_context: DmaPage::new()?,
            ep0: TrbRing::new()?,
            data: DmaPage::new()?,
            port: 0,
            speed: UsbSpeed::Full,
            slot: 0,
            connected_ports: 0,
            first_device: None,
            interrupt: None,
            bulk_in: None,
            bulk_out: None,
            hid: None,
        })
    }

    fn poll(&mut self) {
        let connected = self.controller.connected_ports();
        if connected < self.connected_ports && self.first_device.is_some() {
            crate::bootlog::warn("xHCI USB device disconnected; removing slot");
            if let Err(error) = self.disable_slot() {
                crate::bootlog::warn_fmt(format_args!(
                    "xHCI disconnected-slot cleanup failed: {:?}",
                    error
                ));
            }
            self.slot = 0;
            self.first_device = None;
            self.interrupt = None;
            self.bulk_in = None;
            self.bulk_out = None;
            self.hid = None;
            self.ep0.reset();
        } else if connected > self.connected_ports && self.first_device.is_none() {
            crate::bootlog::info("xHCI USB connection detected; enumerating device");
            match self.enumerate_first() {
                Ok(Some(_)) => crate::bootlog::ok("xHCI USB device re-enumerated"),
                Ok(None) => crate::bootlog::warn("xHCI connection disappeared during enumeration"),
                Err(error) => crate::bootlog::warn_fmt(format_args!(
                    "xHCI hotplug enumeration failed: {:?}",
                    error
                )),
            }
        }
        self.connected_ports = connected;
        if self.first_device.is_some() && self.hid.is_some() {
            self.poll_hid();
        }
    }

    fn stop_host(&mut self) -> Result<(), HostError> {
        let command = self
            .controller
            .mmio
            .read_u32_le(self.controller.operational + OP_USBCMD)
            .ok_or(HostError::RegisterAccess)?;
        if !self.controller.mmio.write_u32_le(
            self.controller.operational + OP_USBCMD,
            command & !(USBCMD_RUN_STOP | USBCMD_INTERRUPTER_ENABLE),
        ) {
            return Err(HostError::RegisterAccess);
        }
        if !wait_for_status(
            self.controller.mmio,
            self.controller.operational,
            USBSTS_HALTED,
            true,
        ) {
            return Err(HostError::Timeout);
        }
        let _ = self
            .controller
            .mmio
            .write_u32_le(self.controller.runtime + RT_INTR0 + RT_IMAN, 0);
        Ok(())
    }

    fn poll_hid(&mut self) {
        let Some(mut endpoint) = self.interrupt.take() else {
            return;
        };
        let result = self.poll_interrupt_endpoint(&mut endpoint);
        if let Ok(Some(length)) = result {
            let mut report = [0u8; HID_REPORT_DESCRIPTOR_MAX];
            let length = length.min(report.len());
            for (index, byte) in report.iter_mut().take(length).enumerate() {
                *byte = endpoint.data.read_u8(index);
            }
            if let Some(device) = self.hid.as_mut() {
                device.feed(&report[..length]);
            }
        } else if let Err(error) = result {
            crate::bootlog::warn_fmt(format_args!(
                "xHCI HID interrupt transfer stopped: {:?}",
                error
            ));
            self.hid = None;
        }
        self.interrupt = Some(endpoint);
    }

    fn poll_interrupt_endpoint(
        &mut self,
        endpoint: &mut InterruptEndpoint,
    ) -> Result<Option<usize>, HostError> {
        let report_length = self
            .hid
            .as_ref()
            .map(hid::Device::report_bytes)
            .ok_or(HostError::Protocol)?;
        let length = report_length.min(endpoint.info.max_packet as usize);
        if length == 0 || length > PAGE_SIZE {
            return Err(HostError::Protocol);
        }
        if !endpoint.pending {
            endpoint.data.sync_for_device()?;
            endpoint
                .ring
                .push(Trb::normal(endpoint.data.physical, length))?;
            endpoint.ring.sync_for_device()?;
            self.ring_doorbell(endpoint.info.id)?;
            endpoint.pending = true;
            endpoint.polls = 0;
        }
        endpoint.polls = endpoint.polls.saturating_add(1);
        if let Some(event) = self.events.poll(&self.controller)? {
            if event.kind == TRB_TRANSFER_EVENT
                && event.endpoint == endpoint.info.id
                && event.slot == self.slot
            {
                endpoint.pending = false;
                endpoint.polls = 0;
                if event.completion != COMPLETION_SUCCESS {
                    return Err(HostError::Completion(event.completion));
                }
                endpoint.data.sync_for_cpu()?;
                return Ok(Some(
                    length.saturating_sub(event.remaining as usize).min(length),
                ));
            }
        }
        if endpoint.polls >= INTERRUPT_TIMEOUT_POLLS {
            endpoint.pending = false;
            let _ = self.cancel_endpoint(endpoint.info.id);
            return Err(HostError::Timeout);
        }
        Ok(None)
    }

    fn start_host(&mut self) -> Result<(), HostError> {
        if self
            .controller
            .mmio
            .read_u32_le(self.controller.operational + OP_PAGESIZE)
            .ok_or(HostError::RegisterAccess)?
            & 1
            == 0
        {
            return Err(HostError::InvalidCapabilities);
        }
        self.dcbaa.sync_for_device()?;
        self.command.sync_for_device()?;
        if !self
            .controller
            .mmio
            .write_u64_le(self.controller.operational + OP_DCBAAP, self.dcbaa.physical)
            || !self
                .controller
                .mmio
                .write_u32_le(self.controller.operational + OP_DNCTRL, 0)
            || !self.controller.mmio.write_u64_le(
                self.controller.operational + OP_CRCR,
                self.command.next_pointer(),
            )
            || !self
                .controller
                .mmio
                .write_u32_le(self.controller.runtime + RT_INTR0 + RT_IMAN, 1 << 1)
            || !self
                .controller
                .mmio
                .write_u32_le(self.controller.runtime + RT_INTR0 + RT_IMOD, 0)
            || !self
                .controller
                .mmio
                .write_u32_le(self.controller.runtime + RT_INTR0 + RT_ERSTSZ, 1)
            || !self.controller.mmio.write_u64_le(
                self.controller.runtime + RT_INTR0 + RT_ERSTBA,
                self.events.erst.physical,
            )
            || !self.controller.mmio.write_u64_le(
                self.controller.runtime + RT_INTR0 + RT_ERDP,
                self.events.page.physical | ERDP_EVENT_HANDLER_BUSY,
            )
        {
            return Err(HostError::RegisterAccess);
        }
        let command = self
            .controller
            .mmio
            .read_u32_le(self.controller.operational + OP_USBCMD)
            .ok_or(HostError::RegisterAccess)?;
        if !self.controller.mmio.write_u32_le(
            self.controller.operational + OP_USBCMD,
            command | USBCMD_RUN_STOP | USBCMD_INTERRUPTER_ENABLE,
        ) {
            return Err(HostError::RegisterAccess);
        }
        if !wait_for_status(
            self.controller.mmio,
            self.controller.operational,
            USBSTS_HALTED,
            false,
        ) {
            return Err(HostError::Timeout);
        }
        Ok(())
    }

    fn enumerate_first(&mut self) -> Result<Option<UsbDeviceInfo>, HostError> {
        let mut port = 0;
        while port < self.controller.capabilities.ports {
            if self
                .controller
                .port_status(port)
                .is_some_and(|value| value & PORTSC_CONNECTED != 0)
            {
                crate::bootlog::info("xHCI resetting connected USB port");
                let speed = self.controller.reset_port(port)?;
                crate::bootlog::info_fmt(format_args!(
                    "xHCI USB port reset complete speed={}",
                    speed.name()
                ));
                self.port = port;
                self.speed = speed;
                crate::bootlog::info("xHCI enabling USB slot");
                let slot = self.enable_slot()?;
                self.slot = slot;
                crate::bootlog::info_fmt(format_args!("xHCI USB slot {} enabled", slot));
                self.address_device(slot, port, speed)?;
                crate::bootlog::info_fmt(format_args!("xHCI USB slot {} addressed", slot));
                crate::bootlog::info("xHCI reading USB device descriptor");

                let device_length =
                    self.control_transfer(SetupPacket::get_descriptor(0x0100, 18), 18)?;
                crate::bootlog::info("xHCI USB device descriptor received");
                if device_length < 12 || self.data.read_u8(1) != 1 {
                    return Err(HostError::Protocol);
                }
                let device_class = self.data.read_u8(4);
                let vendor_id = (self.data.read_u8(8) as u16) | (self.data.read_u8(9) as u16) << 8;
                let product_id =
                    (self.data.read_u8(10) as u16) | (self.data.read_u8(11) as u16) << 8;

                let config_header_length =
                    self.control_transfer(SetupPacket::get_descriptor(0x0200, 9), 9)?;
                crate::bootlog::info("xHCI USB configuration header received");
                if config_header_length < 9 || self.data.read_u8(1) != 2 {
                    return Err(HostError::Protocol);
                }
                let total_length =
                    self.data.read_u8(2) as usize | (self.data.read_u8(3) as usize) << 8;
                if !(9..=PAGE_SIZE).contains(&total_length) {
                    return Err(HostError::Protocol);
                }
                if total_length > config_header_length {
                    crate::bootlog::info_fmt(format_args!(
                        "xHCI reading USB configuration descriptor length={}",
                        total_length
                    ));
                    self.control_transfer(
                        SetupPacket::get_descriptor(0x0200, total_length as u16),
                        total_length,
                    )?;
                }
                let configuration = parse_configuration(
                    self.data,
                    total_length,
                    device_class,
                    vendor_id,
                    product_id,
                )?;
                crate::bootlog::info_fmt(format_args!(
                    "xHCI USB device class={} hub={} serial={} storage={} serial-protocol={}",
                    device_class,
                    configuration.hub,
                    configuration.serial,
                    configuration.storage,
                    configuration
                        .serial_protocol
                        .map_or("none", SerialProtocol::name)
                ));
                crate::bootlog::info_fmt(format_args!(
                    "xHCI setting USB configuration {}",
                    configuration.value
                ));
                self.control_transfer(SetupPacket::set_configuration(configuration.value), 0)?;
                let hub_ports = if configuration.hub {
                    let ports = self.read_hub_descriptor()?;
                    crate::bootlog::info_fmt(format_args!(
                        "xHCI USB hub descriptor reports {} downstream ports",
                        ports
                    ));
                    ports
                } else {
                    0
                };
                let hid_device = if configuration.hid {
                    if configuration.report_length == 0
                        || configuration.report_length as usize > HID_REPORT_DESCRIPTOR_MAX
                    {
                        return Err(HostError::Protocol);
                    }
                    let descriptor_length = self.control_transfer(
                        SetupPacket::hid_report_descriptor(
                            configuration.interface,
                            configuration.report_length,
                        ),
                        configuration.report_length as usize,
                    )?;
                    let mut descriptor = [0u8; HID_REPORT_DESCRIPTOR_MAX];
                    for (index, byte) in descriptor.iter_mut().take(descriptor_length).enumerate() {
                        *byte = self.data.read_u8(index);
                    }
                    let device = hid::Device::from_descriptor(&descriptor[..descriptor_length])
                        .map_err(|_| HostError::Protocol)?;
                    crate::bootlog::info_fmt(format_args!(
                        "xHCI USB HID {} report-bytes={}",
                        device.kind().name(),
                        device.report_bytes()
                    ));
                    Some(device)
                } else {
                    None
                };
                crate::bootlog::info_fmt(format_args!(
                    "xHCI configuring USB endpoints interrupt-in={} bulk-in={} bulk-out={}",
                    configuration.interrupt_in.map_or(0, |endpoint| endpoint.id),
                    configuration.bulk_in.map_or(0, |endpoint| endpoint.id),
                    configuration.bulk_out.map_or(0, |endpoint| endpoint.id)
                ));
                self.configure_endpoints(configuration)?;
                if configuration.serial {
                    crate::bootlog::info_fmt(format_args!(
                        "xHCI configuring USB serial control requests protocol={}",
                        configuration
                            .serial_protocol
                            .map_or("unknown", SerialProtocol::name)
                    ));
                    self.configure_serial(
                        configuration.control_interface,
                        configuration.serial_protocol,
                    )?;
                }
                if configuration.storage {
                    crate::bootlog::info("xHCI probing USB mass-storage BOT");
                    self.probe_storage()?;
                }
                let hid_kind = hid_device
                    .as_ref()
                    .map(hid::Device::kind)
                    .unwrap_or(hid::Kind::Other);
                let class = if configuration.hid {
                    UsbClass::Hid(hid_kind)
                } else if configuration.storage {
                    UsbClass::Storage
                } else if configuration.serial {
                    UsbClass::Serial
                } else {
                    UsbClass::Other
                };
                let info = UsbDeviceInfo {
                    slot,
                    address: slot,
                    speed,
                    vendor_id,
                    product_id,
                    configuration: configuration.value,
                    hub_ports,
                    hid: hid_kind,
                    class,
                };
                self.first_device = Some(info);
                self.hid = hid_device;
                return Ok(Some(info));
            }
            port += 1;
        }
        Ok(None)
    }

    fn read_hub_descriptor(&mut self) -> Result<u8, HostError> {
        let length = self.control_transfer(SetupPacket::hub_descriptor(9), 9)?;
        if length < 3 || self.data.read_u8(1) != 0x29 {
            return Err(HostError::Protocol);
        }
        let ports = self.data.read_u8(2);
        if ports == 0 {
            return Err(HostError::Protocol);
        }
        Ok(ports)
    }

    fn enable_slot(&mut self) -> Result<u8, HostError> {
        let completion = self.command(Trb::new(0, 0, (TRB_ENABLE_SLOT as u32) << 10), 0)?;
        if completion.completion != COMPLETION_SUCCESS || completion.slot == 0 {
            return Err(HostError::Completion(completion.completion));
        }
        Ok(completion.slot)
    }

    fn disable_slot(&mut self) -> Result<(), HostError> {
        if self.slot == 0 {
            return Ok(());
        }
        let control = (TRB_DISABLE_SLOT as u32) << 10 | (self.slot as u32) << 24;
        let completion = self.command(Trb::new(0, 0, control), 0)?;
        if completion.completion == COMPLETION_SUCCESS {
            Ok(())
        } else {
            Err(HostError::Completion(completion.completion))
        }
    }

    fn address_device(&mut self, slot: u8, port: u8, speed: UsbSpeed) -> Result<(), HostError> {
        self.ep0.reset();
        self.device_context.clear();
        self.dcbaa
            .write_u64(slot as usize * 8, self.device_context.physical);
        self.input.clear();
        self.input.write_u32(4, 0x3);
        self.write_slot_context(port, speed, 1);
        self.write_endpoint_context(1, self.ep0.next_pointer(), speed.max_packet_size(), 4, 0);
        self.input.sync_for_device()?;
        self.dcbaa.sync_for_device()?;
        let control = (TRB_ADDRESS_DEVICE as u32) << 10 | (slot as u32) << 24;
        let completion = self.command(Trb::new(self.input.physical, 0, control), 0)?;
        if completion.completion != COMPLETION_SUCCESS {
            crate::bootlog::warn_fmt(format_args!(
                "xHCI Address Device failed completion={}",
                completion.completion
            ));
            return Err(HostError::Completion(completion.completion));
        }
        Ok(())
    }

    fn write_slot_context(&self, port: u8, speed: UsbSpeed, entries: u8) {
        let value = (speed.raw() as u32) << 20 | (entries as u32) << 27;
        self.input.write_u32(32, value);
        self.input.write_u32(36, (port as u32 + 1) << 16);
    }

    fn write_endpoint_context(
        &self,
        endpoint: u8,
        dequeue: u64,
        max_packet: u16,
        endpoint_type: u8,
        interval: u8,
    ) {
        let offset = 32 + self.controller.capabilities.context_size as usize * endpoint as usize;
        self.input.write_u32(offset, (interval as u32) << 16);
        self.input.write_u32(
            offset + 4,
            3 << 1 | (endpoint_type as u32) << 3 | (max_packet as u32) << 16,
        );
        self.input.write_u64(offset + 8, dequeue);
        self.input.write_u32(offset + 16, 8);
    }

    fn configure_endpoints(&mut self, configuration: ConfigurationInfo) -> Result<(), HostError> {
        let endpoints = [
            configuration.interrupt_in,
            configuration.bulk_in,
            configuration.bulk_out,
        ];
        let mut add_flags = 1u32;
        let mut entries = 1u8;
        for endpoint in endpoints.into_iter().flatten() {
            add_flags |= 1u32 << endpoint.id;
            entries = entries.max(endpoint.id);
        }
        let mut interrupt = None;
        let mut bulk_in = None;
        let mut bulk_out = None;
        for endpoint in endpoints.into_iter().flatten() {
            let ring = TrbRing::new()?;
            let data = DmaPage::new()?;
            if Some(endpoint.id) == configuration.interrupt_in.map(|value| value.id) {
                interrupt = Some(InterruptEndpoint {
                    info: endpoint,
                    ring,
                    data,
                    pending: false,
                    polls: 0,
                });
            } else if Some(endpoint.id) == configuration.bulk_in.map(|value| value.id) {
                bulk_in = Some(BulkEndpoint {
                    info: endpoint,
                    ring,
                    data,
                });
            } else {
                bulk_out = Some(BulkEndpoint {
                    info: endpoint,
                    ring,
                    data,
                });
            }
        }
        if add_flags == 1 {
            return Ok(());
        }
        self.input.clear();
        self.input.write_u32(4, add_flags);
        self.write_slot_context(self.port, self.speed, entries);
        self.write_endpoint_context(
            1,
            self.ep0.next_pointer(),
            self.speed.max_packet_size(),
            4,
            0,
        );
        for endpoint in endpoints.into_iter().flatten() {
            let ring = if Some(endpoint.id) == configuration.interrupt_in.map(|value| value.id) {
                interrupt.as_ref().map(|value| &value.ring)
            } else if Some(endpoint.id) == configuration.bulk_in.map(|value| value.id) {
                bulk_in.as_ref().map(|value| &value.ring)
            } else {
                bulk_out.as_ref().map(|value| &value.ring)
            }
            .ok_or(HostError::Protocol)?;
            self.write_endpoint_context(
                endpoint.id,
                ring.next_pointer(),
                endpoint.max_packet,
                endpoint.transfer_type,
                endpoint.interval.max(1),
            );
        }
        self.input.sync_for_device()?;
        let control = (TRB_CONFIGURE_ENDPOINT as u32) << 10 | (self.slot as u32) << 24;
        let completion = self.command(Trb::new(self.input.physical, 0, control), 0)?;
        if completion.completion != COMPLETION_SUCCESS {
            crate::bootlog::warn_fmt(format_args!(
                "xHCI Configure Endpoint failed completion={}",
                completion.completion
            ));
            return Err(HostError::Completion(completion.completion));
        }
        self.interrupt = interrupt;
        self.bulk_in = bulk_in;
        self.bulk_out = bulk_out;
        Ok(())
    }

    fn configure_serial(
        &mut self,
        interface: u8,
        protocol: Option<SerialProtocol>,
    ) -> Result<(), HostError> {
        match protocol.ok_or(HostError::Protocol)? {
            SerialProtocol::CdcAcm => {
                self.data.clear();
                self.data.write_u8(0, 0x00);
                self.data.write_u8(1, 0xc2);
                self.data.write_u8(2, 0x01);
                self.data.write_u8(3, 0x00);
                self.data.write_u8(4, 0);
                self.data.write_u8(5, 0);
                self.data.write_u8(6, 8);
                self.control_transfer(SetupPacket::cdc_line_coding(interface), 7)?;
                self.control_transfer(SetupPacket::cdc_control_line_state(interface, 3), 0)?;
            }
            SerialProtocol::Ftdi => {
                self.control_transfer(SetupPacket::ftdi_baud_rate(interface), 0)?;
                self.control_transfer(SetupPacket::ftdi_modem_control(interface), 0)?;
            }
        }
        crate::bootlog::info_fmt(format_args!(
            "xHCI USB serial configured protocol={} baud=115200 data=8n1",
            protocol.map_or("unknown", SerialProtocol::name)
        ));
        Ok(())
    }

    fn bulk_transfer_in(&mut self, length: usize) -> Result<usize, HostError> {
        let Some(mut endpoint) = self.bulk_in.take() else {
            return Err(HostError::Protocol);
        };
        let result = self.data_transfer(
            endpoint.info.id,
            &mut endpoint.ring,
            &mut endpoint.data,
            length,
            DataTransferKind::Bulk,
        );
        self.bulk_in = Some(endpoint);
        result
    }

    fn storage_command(
        &mut self,
        tag: u32,
        cdb: &[u8; 16],
        cdb_length: u8,
        transfer_length: usize,
    ) -> Result<usize, HostError> {
        if transfer_length > PAGE_SIZE {
            return Err(HostError::Protocol);
        }
        let mut cbw = [0u8; 31];
        cbw[0..4].copy_from_slice(&0x4342_5355u32.to_le_bytes());
        cbw[4..8].copy_from_slice(&tag.to_le_bytes());
        cbw[8..12].copy_from_slice(&(transfer_length as u32).to_le_bytes());
        cbw[12] = u8::from(transfer_length != 0) << 7;
        cbw[14] = cdb_length;
        cbw[15..31].copy_from_slice(cdb);
        let Some(mut out) = self.bulk_out.take() else {
            return Err(HostError::Protocol);
        };
        out.data.clear();
        for (index, byte) in cbw.into_iter().enumerate() {
            out.data.write_u8(index, byte);
        }
        let cbw_result = self.data_transfer(
            out.info.id,
            &mut out.ring,
            &mut out.data,
            31,
            DataTransferKind::Bulk,
        );
        self.bulk_out = Some(out);
        if let Err(error) = cbw_result {
            crate::bootlog::warn_fmt(format_args!(
                "xHCI USB mass-storage CBW transfer failed: {:?}",
                error
            ));
            return Err(error);
        }

        let data_result = if transfer_length == 0 {
            Ok(0)
        } else {
            self.bulk_transfer_in(transfer_length)
        };
        let data_result = match data_result {
            Ok(length) => length,
            Err(error) => {
                crate::bootlog::warn_fmt(format_args!(
                    "xHCI USB mass-storage data transfer failed: {:?}",
                    error
                ));
                return Err(error);
            }
        };
        let Some(mut input) = self.bulk_in.take() else {
            return Err(HostError::Protocol);
        };
        let csw_result = self.data_transfer(
            input.info.id,
            &mut input.ring,
            &mut input.data,
            13,
            DataTransferKind::Bulk,
        );
        self.bulk_in = Some(input);
        if let Err(error) = csw_result {
            crate::bootlog::warn_fmt(format_args!(
                "xHCI USB mass-storage CSW transfer failed: {:?}",
                error
            ));
            return Err(error);
        }
        let Some(input) = self.bulk_in.as_ref() else {
            return Err(HostError::Protocol);
        };
        let signature = input.data.read_u32(0);
        let received_tag = input.data.read_u32(4);
        let status = input.data.read_u8(12);
        if signature != 0x5342_5355 || received_tag != tag || status != 0 {
            crate::bootlog::warn_fmt(format_args!(
                "xHCI USB mass-storage invalid CSW signature=0x{:08x} tag={} status={}",
                signature, received_tag, status
            ));
            return Err(HostError::Protocol);
        }
        Ok(data_result)
    }

    fn probe_storage(&mut self) -> Result<(), HostError> {
        let mut inquiry = [0u8; 16];
        inquiry[0] = 0x12;
        inquiry[4] = 36;
        let inquiry_length = self.storage_command(1, &inquiry, 6, 36)?;
        if inquiry_length < 36 {
            return Err(HostError::Protocol);
        }
        crate::bootlog::info_fmt(format_args!(
            "xHCI USB mass-storage inquiry ok bytes={}",
            inquiry_length
        ));
        Ok(())
    }

    fn control_transfer(&mut self, setup: SetupPacket, length: usize) -> Result<usize, HostError> {
        if length > PAGE_SIZE || length != setup.length as usize {
            return Err(HostError::Protocol);
        }
        let data_in = setup.direction_in();
        self.ep0
            .push(Trb::setup(setup, data_in))
            .map_err(|_| HostError::RingFull)?;
        if length != 0 {
            self.data.sync_for_device()?;
            self.ep0
                .push(Trb::data(self.data.physical, length, data_in))
                .map_err(|_| HostError::RingFull)?;
        }
        self.ep0.push(Trb::status(length == 0 || !data_in))?;
        self.ep0.sync_for_device()?;
        self.ring_doorbell(1)?;
        let event = match self.wait_for_transfer(1, self.slot) {
            Ok(event) => event,
            Err(error) => {
                let _ = self.cancel_endpoint(1);
                return Err(error);
            }
        };
        if event.completion != COMPLETION_SUCCESS {
            return Err(HostError::Completion(event.completion));
        }
        if length != 0 {
            self.data.sync_for_cpu()?;
        }
        Ok(length.saturating_sub(event.remaining as usize).min(length))
    }

    fn data_transfer(
        &mut self,
        endpoint: u8,
        ring: &mut TrbRing,
        data: &mut DmaPage,
        length: usize,
        kind: DataTransferKind,
    ) -> Result<usize, HostError> {
        if length == 0 || length > PAGE_SIZE {
            return Err(HostError::Protocol);
        }
        let _ = kind;
        data.sync_for_device()?;
        ring.push(Trb::normal(data.physical, length))?;
        ring.sync_for_device()?;
        self.ring_doorbell(endpoint)?;
        let event = match self.wait_for_transfer(endpoint, self.slot) {
            Ok(event) => event,
            Err(error) => {
                let _ = self.cancel_endpoint(endpoint);
                return Err(error);
            }
        };
        if event.completion != COMPLETION_SUCCESS {
            return Err(HostError::Completion(event.completion));
        }
        data.sync_for_cpu()?;
        Ok(length.saturating_sub(event.remaining as usize).min(length))
    }

    #[allow(dead_code)]
    fn interrupt_transfer(&mut self, length: usize) -> Result<usize, HostError> {
        let Some(mut endpoint) = self.interrupt.take() else {
            return Err(HostError::Protocol);
        };
        let result = self.data_transfer(
            endpoint.info.id,
            &mut endpoint.ring,
            &mut endpoint.data,
            length,
            DataTransferKind::Interrupt,
        );
        self.interrupt = Some(endpoint);
        result
    }

    #[allow(dead_code)]
    fn bulk_transfer(
        &mut self,
        endpoint: u8,
        ring: &mut TrbRing,
        data: &mut DmaPage,
        length: usize,
    ) -> Result<usize, HostError> {
        self.data_transfer(endpoint, ring, data, length, DataTransferKind::Bulk)
    }

    fn command(&mut self, trb: Trb, _target: u8) -> Result<Event, HostError> {
        self.command.push(trb)?;
        self.command.sync_for_device()?;
        if !self
            .controller
            .mmio
            .write_u32_le(self.controller.doorbell, 0)
        {
            return Err(HostError::RegisterAccess);
        }
        self.wait_for_command()
    }

    fn wait_for_command(&mut self) -> Result<Event, HostError> {
        for _ in 0..POLL_LIMIT {
            if let Some(event) = self.events.poll(&self.controller)? {
                if event.kind == TRB_COMMAND_COMPLETION {
                    return Ok(event);
                }
            }
        }
        let command = self
            .controller
            .mmio
            .read_u32_le(self.controller.operational + OP_USBCMD)
            .unwrap_or(u32::MAX);
        let status = self
            .controller
            .mmio
            .read_u32_le(self.controller.operational + OP_USBSTS)
            .unwrap_or(u32::MAX);
        let raw = self.events.page.read_trb(self.events.dequeue);
        crate::bootlog::warn_fmt(format_args!(
            "xHCI command completion timeout usbcmd=0x{:08x} usbsts=0x{:08x} event-d3=0x{:08x}",
            command, status, raw.words[3]
        ));
        Err(HostError::Timeout)
    }

    fn wait_for_transfer(&mut self, endpoint: u8, slot: u8) -> Result<Event, HostError> {
        for _ in 0..POLL_LIMIT {
            if let Some(event) = self.events.poll(&self.controller)? {
                if event.kind == TRB_TRANSFER_EVENT
                    && event.endpoint == endpoint
                    && event.slot == slot
                {
                    return Ok(event);
                }
            }
        }
        Err(HostError::Timeout)
    }

    fn cancel_endpoint(&mut self, endpoint: u8) -> Result<(), HostError> {
        let control =
            (TRB_STOP_ENDPOINT as u32) << 10 | (endpoint as u32) << 16 | (self.slot as u32) << 24;
        let completion = self.command(Trb::new(0, 0, control), 0)?;
        if completion.completion == COMPLETION_SUCCESS {
            Ok(())
        } else {
            Err(HostError::Completion(completion.completion))
        }
    }

    fn ring_doorbell(&self, endpoint: u8) -> Result<(), HostError> {
        let offset = self
            .controller
            .doorbell
            .checked_add(self.slot as usize * 4)
            .ok_or(HostError::RegisterAccess)?;
        if !self.controller.mmio.write_u32_le(offset, endpoint as u32) {
            return Err(HostError::RegisterAccess);
        }
        Ok(())
    }

    fn status(&self, pci: pci::Device, bar: u64) -> Status {
        Status {
            kind: self.controller.kind(),
            state: HostState::Ready,
            bus: pci.address.bus,
            slot: pci.address.slot,
            function: pci.address.function,
            vendor: pci.vendor,
            device: pci.device,
            bar,
            capabilities: self.controller.capabilities(),
            connected_ports: self.controller.connected_ports(),
            devices: u8::from(self.first_device.is_some()),
            configured_devices: u8::from(self.first_device.is_some()),
            first_device: self.first_device,
        }
    }
}

fn parse_configuration(
    page: DmaPage,
    total_length: usize,
    device_class: u8,
    vendor_id: u16,
    product_id: u16,
) -> Result<ConfigurationInfo, HostError> {
    if total_length < 9 || page.read_u8(0) < 9 || page.read_u8(1) != 2 {
        return Err(HostError::Protocol);
    }
    let value = page.read_u8(5);
    let mut interrupt_in = None;
    let mut bulk_in = None;
    let mut bulk_out = None;
    let mut hub = device_class == 9;
    let mut hid = false;
    let mut serial = false;
    let ftdi = vendor_id == FTDI_VENDOR_ID && product_id == FTDI_SERIAL_PRODUCT_ID;
    let serial_protocol = if ftdi {
        Some(SerialProtocol::Ftdi)
    } else {
        None
    };
    let mut storage = false;
    let mut interface = 0;
    let mut control_interface = 0;
    let mut control_interface_selected = false;
    let mut report_length = 0;
    let mut hid_interface = false;
    let mut serial_interface = false;
    let mut storage_interface = false;
    let mut offset = 0;
    while offset + 2 <= total_length {
        let length = page.read_u8(offset) as usize;
        let kind = page.read_u8(offset + 1);
        if length < 2 || offset + length > total_length {
            return Err(HostError::Protocol);
        }
        if kind == 4 && length >= 9 {
            let number = page.read_u8(offset + 2);
            let class = page.read_u8(offset + 5);
            hid_interface = class == 3;
            serial_interface = class == 2 || class == 0x0a || ftdi;
            storage_interface =
                class == 8 && page.read_u8(offset + 6) == 6 && page.read_u8(offset + 7) == 0x50;
            if hid_interface && !hid {
                interface = number;
            }
            if (class == 2 || ftdi) && serial_interface && !control_interface_selected {
                control_interface = number;
                control_interface_selected = true;
            }
            hub |= class == 9;
            hid |= hid_interface;
            serial |= serial_interface;
            storage |= storage_interface;
        }
        if kind == 0x21 && length >= 9 && hid_interface && page.read_u8(offset + 6) == 0x22 {
            report_length =
                page.read_u8(offset + 7) as u16 | (page.read_u8(offset + 8) as u16) << 8;
        }
        if kind == 5 && length >= 7 {
            let address = page.read_u8(offset + 2);
            let attributes = page.read_u8(offset + 3) & 0x03;
            let max_packet =
                page.read_u8(offset + 4) as u16 | (page.read_u8(offset + 5) as u16) << 8;
            let max_packet = max_packet & 0x07ff;
            let number = address & 0x0f;
            if max_packet != 0 && number != 0 {
                let input = address & 0x80 != 0;
                let transfer_type = match attributes {
                    2 if input => 6,
                    2 => 2,
                    3 if input => 7,
                    3 => 3,
                    _ => 0,
                };
                let endpoint = EndpointInfo {
                    id: number.saturating_mul(2).saturating_add(u8::from(input)),
                    max_packet,
                    interval: page.read_u8(offset + 6),
                    transfer_type,
                };
                if hid_interface && transfer_type == 7 && interrupt_in.is_none() {
                    interrupt_in = Some(endpoint);
                } else if (serial_interface || storage_interface) && transfer_type == 6 {
                    if bulk_in.is_none() {
                        bulk_in = Some(endpoint);
                    }
                } else if (serial_interface || storage_interface)
                    && transfer_type == 2
                    && bulk_out.is_none()
                {
                    bulk_out = Some(endpoint);
                }
            }
        }
        offset += length;
    }
    Ok(ConfigurationInfo {
        value,
        interrupt_in,
        bulk_in,
        bulk_out,
        hub,
        hid,
        serial,
        serial_protocol: serial_protocol.or_else(|| serial.then_some(SerialProtocol::CdcAcm)),
        storage,
        interface,
        control_interface,
        report_length,
    })
}

pub fn kind_name(kind: HostControllerKind) -> &'static str {
    match kind {
        HostControllerKind::Xhci => "xhci",
        HostControllerKind::Ehci => "ehci",
        HostControllerKind::Ohci => "ohci",
    }
}
