const MAX_DRIVERS: usize = 32;
const MAX_DEVICES_PER_BUS: usize = 16;
const MAX_RESOURCES: usize = 8;

pub type BusId = u16;
pub type DeviceId = u16;
pub type DriverId = u16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BusKind {
    Platform,
    Pci,
    Virtio,
    Usb,
    Ps2,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Display,
    Serial,
    Block,
    Input,
    UsbHost,
    Audio,
    Network,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DriverStage {
    Early,
    Runtime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    Discovered,
    Probing,
    Ready,
    Suspended,
    Deferred,
    Unsupported,
    Busy,
    Quiescing,
    Removed,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IrqKind {
    Legacy,
    Msi,
    MsiX,
    Gic,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DmaDirection {
    ToDevice,
    FromDevice,
    Bidirectional,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Irq {
    pub kind: IrqKind,
    pub line: u32,
    pub owner: DeviceId,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DmaBuffer {
    pub physical: crate::address::PhysAddr,
    pub virtual_address: crate::address::VirtAddr,
    pub length: usize,
    pub alignment: usize,
    pub direction: DmaDirection,
    pub owner: DeviceId,
}

impl DmaBuffer {
    pub fn validate(self) -> Result<(), DriverError> {
        if self.length == 0
            || !self.alignment.is_power_of_two()
            || !self.physical.is_aligned(self.alignment)
            || !self.virtual_address.is_aligned(self.alignment)
        {
            return Err(DriverError::InvalidAlignment);
        }
        if self.physical.checked_add(self.length).is_none()
            || self.virtual_address.checked_add(self.length).is_none()
        {
            return Err(DriverError::InvalidAddressRange);
        }
        Ok(())
    }

    #[allow(dead_code)]
    ///
    /// # Safety
    /// The DMA buffer's virtual range must be mapped and valid for the device.
    pub unsafe fn sync_for_device(self) -> Result<(), DriverError> {
        self.validate()?;
        if !matches!(
            self.direction,
            DmaDirection::ToDevice | DmaDirection::Bidirectional
        ) {
            return Err(DriverError::InvalidDmaDirection);
        }
        if unsafe { crate::io::dma_for_device(self.virtual_address, self.length) } {
            Ok(())
        } else {
            Err(DriverError::InvalidAddressRange)
        }
    }

    #[allow(dead_code)]
    ///
    /// # Safety
    /// The DMA buffer's virtual range must be mapped and valid for the device.
    pub unsafe fn sync_for_cpu(self) -> Result<(), DriverError> {
        self.validate()?;
        if !matches!(
            self.direction,
            DmaDirection::FromDevice | DmaDirection::Bidirectional
        ) {
            return Err(DriverError::InvalidDmaDirection);
        }
        if unsafe { crate::io::dma_for_cpu(self.virtual_address, self.length) } {
            Ok(())
        } else {
            Err(DriverError::InvalidAddressRange)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    Mmio {
        base: u64,
        size: u64,
        owner: DeviceId,
    },
    Pio {
        base: u16,
        size: u16,
        owner: DeviceId,
    },
    Irq(Irq),
    Dma(DmaBuffer),
}

impl Resource {
    fn owner(self) -> DeviceId {
        match self {
            Self::Mmio { owner, .. } | Self::Pio { owner, .. } => owner,
            Self::Irq(irq) => irq.owner,
            Self::Dma(buffer) => buffer.owner,
        }
    }

    fn validate(self) -> Result<(), DriverError> {
        match self {
            Self::Mmio { base, size, .. } => {
                if size == 0 || base.checked_add(size).is_none() {
                    return Err(DriverError::InvalidResourceRange);
                }
            }
            Self::Pio { base, size, .. } => {
                if size == 0 || base as u32 + size as u32 > 0x1_0000 {
                    return Err(DriverError::InvalidResourceRange);
                }
            }
            Self::Dma(buffer) => buffer.validate()?,
            Self::Irq(_) => {}
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverError {
    InvalidState {
        expected: DeviceState,
        actual: DeviceState,
    },
    BusCapacity,
    ResourceCapacity,
    ResourceOwnerMismatch,
    DriverAlreadyBound,
    DriverNotBound,
    Unsupported,
    Deferred,
    Busy,
    Timeout,
    ProbeFailed,
    RemoveFailed,
    NotMatched,
    NotPublished,
    InvalidResourceRange,
    InvalidAlignment,
    InvalidAddressRange,
    InvalidDmaDirection,
    DeviceAbsent,
    MalformedDescriptor,
    DmaFailure,
    InterruptStorm,
}

const MAX_TRACE_EVENTS: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TraceOp {
    Discover,
    Match,
    Probe,
    Publish,
    Suspend,
    Resume,
    Quiesce,
    Remove,
    Revoke,
    HotUnplug,
    Rebind,
    ResourceAdd,
    ResourceRelease,
}

#[derive(Clone, Copy)]
pub struct TraceEvent {
    pub sequence: u64,
    pub operation: TraceOp,
    pub device: DeviceId,
    pub driver: Option<DriverId>,
    pub state: DeviceState,
    pub resources: usize,
    pub irqs: usize,
    pub dmas: usize,
    pub success: bool,
}

static mut TRACE: [Option<TraceEvent>; MAX_TRACE_EVENTS] = [None; MAX_TRACE_EVENTS];
static mut TRACE_NEXT: u64 = 0;
static mut TRACE_LEN: usize = 0;

pub type DeviceCallback = fn(&mut Device) -> Result<(), DriverError>;

#[derive(Clone, Copy)]
pub struct DriverOps {
    pub probe: DeviceCallback,
    pub suspend: Option<DeviceCallback>,
    pub resume: Option<DeviceCallback>,
    pub quiesce: Option<DeviceCallback>,
    pub remove: DeviceCallback,
}

#[derive(Clone, Copy)]
pub struct Bus {
    pub id: BusId,
    pub name: &'static str,
    pub kind: BusKind,
    devices: [Option<DeviceId>; MAX_DEVICES_PER_BUS],
    device_len: usize,
}

impl Bus {
    pub const fn new(id: BusId, name: &'static str, kind: BusKind) -> Self {
        Self {
            id,
            name,
            kind,
            devices: [None; MAX_DEVICES_PER_BUS],
            device_len: 0,
        }
    }

    pub fn attach(&mut self, device: DeviceId) -> Result<(), DriverError> {
        if self.device_len == MAX_DEVICES_PER_BUS {
            return Err(DriverError::BusCapacity);
        }
        if self.devices[..self.device_len].contains(&Some(device)) {
            return Ok(());
        }
        self.devices[self.device_len] = Some(device);
        self.device_len += 1;
        Ok(())
    }

    pub fn devices(&self) -> &[Option<DeviceId>] {
        &self.devices[..self.device_len]
    }
}

#[derive(Clone, Copy)]
pub struct Device {
    pub id: DeviceId,
    pub bus: BusId,
    pub bus_kind: BusKind,
    pub name: &'static str,
    pub class: Class,
    pub state: DeviceState,
    resources: [Option<Resource>; MAX_RESOURCES],
    resource_len: usize,
    driver: Option<DriverId>,
    published: bool,
}

impl Device {
    pub const fn new(
        id: DeviceId,
        bus: BusId,
        bus_kind: BusKind,
        name: &'static str,
        class: Class,
    ) -> Self {
        Self {
            id,
            bus,
            bus_kind,
            name,
            class,
            state: DeviceState::Discovered,
            resources: [None; MAX_RESOURCES],
            resource_len: 0,
            driver: None,
            published: false,
        }
    }

    pub fn transition(&mut self, next: DeviceState) -> Result<(), DriverError> {
        let valid = matches!(
            (self.state, next),
            (DeviceState::Discovered, DeviceState::Probing)
                | (DeviceState::Probing, DeviceState::Ready)
                | (DeviceState::Ready, DeviceState::Suspended)
                | (DeviceState::Suspended, DeviceState::Ready)
                | (DeviceState::Probing, DeviceState::Deferred)
                | (DeviceState::Probing, DeviceState::Unsupported)
                | (DeviceState::Probing, DeviceState::Busy)
                | (DeviceState::Probing, DeviceState::Failed)
                | (DeviceState::Ready, DeviceState::Quiescing)
                | (DeviceState::Suspended, DeviceState::Quiescing)
                | (DeviceState::Quiescing, DeviceState::Removed)
                | (DeviceState::Quiescing, DeviceState::Failed)
                | (DeviceState::Failed, DeviceState::Removed)
                | (DeviceState::Failed, DeviceState::Probing)
                | (DeviceState::Removed, DeviceState::Probing)
                | (DeviceState::Deferred, DeviceState::Probing)
                | (DeviceState::Unsupported, DeviceState::Probing)
                | (DeviceState::Busy, DeviceState::Probing)
        );
        if !valid {
            return Err(DriverError::InvalidState {
                expected: next,
                actual: self.state,
            });
        }
        self.state = next;
        Ok(())
    }

    pub fn add_resource(&mut self, resource: Resource) -> Result<(), DriverError> {
        if resource.owner() != self.id {
            return Err(DriverError::ResourceOwnerMismatch);
        }
        resource.validate()?;
        if self.resource_len == MAX_RESOURCES {
            return Err(DriverError::ResourceCapacity);
        }
        self.resources[self.resource_len] = Some(resource);
        self.resource_len += 1;
        trace_event(TraceOp::ResourceAdd, self, self.driver, true);
        Ok(())
    }

    pub fn resources(&self) -> &[Option<Resource>] {
        &self.resources[..self.resource_len]
    }

    pub fn release_resources(&mut self) -> Result<(), DriverError> {
        if !matches!(
            self.state,
            DeviceState::Failed
                | DeviceState::Deferred
                | DeviceState::Unsupported
                | DeviceState::Busy
                | DeviceState::Removed
        ) {
            return Err(DriverError::InvalidState {
                expected: DeviceState::Removed,
                actual: self.state,
            });
        }
        self.resources = [None; MAX_RESOURCES];
        self.resource_len = 0;
        trace_event(TraceOp::ResourceRelease, self, self.driver, true);
        Ok(())
    }

    pub fn bind_driver(&mut self, driver: DriverId) -> Result<(), DriverError> {
        if self.driver.is_some() {
            return Err(DriverError::DriverAlreadyBound);
        }
        if self.state != DeviceState::Probing {
            return Err(DriverError::InvalidState {
                expected: DeviceState::Probing,
                actual: self.state,
            });
        }
        self.driver = Some(driver);
        Ok(())
    }

    pub fn unbind_driver(&mut self) -> Result<DriverId, DriverError> {
        self.driver.take().ok_or(DriverError::DriverNotBound)
    }

    pub fn bound_driver(&self) -> Option<DriverId> {
        self.driver
    }

    pub fn publish(&mut self) -> Result<(), DriverError> {
        if self.state != DeviceState::Ready {
            return Err(DriverError::InvalidState {
                expected: DeviceState::Ready,
                actual: self.state,
            });
        }
        self.published = true;
        Ok(())
    }

    pub fn unpublish(&mut self) -> Result<(), DriverError> {
        if !self.published {
            return Err(DriverError::NotPublished);
        }
        self.published = false;
        Ok(())
    }

    pub fn is_published(&self) -> bool {
        self.published
    }

    fn resource_summary(&self) -> (usize, usize, usize) {
        let mut irqs = 0;
        let mut dmas = 0;
        for resource in self.resources[..self.resource_len].iter().flatten() {
            match resource {
                Resource::Irq(_) => irqs += 1,
                Resource::Dma(_) => dmas += 1,
                Resource::Mmio { .. } | Resource::Pio { .. } => {}
            }
        }
        (self.resource_len, irqs, dmas)
    }
}

fn trace_event(operation: TraceOp, device: &Device, driver: Option<DriverId>, success: bool) {
    let (resources, irqs, dmas) = device.resource_summary();
    unsafe {
        let sequence = TRACE_NEXT;
        let index = (sequence as usize) % MAX_TRACE_EVENTS;
        TRACE[index] = Some(TraceEvent {
            sequence,
            operation,
            device: device.id,
            driver,
            state: device.state,
            resources,
            irqs,
            dmas,
            success,
        });
        TRACE_NEXT = TRACE_NEXT.saturating_add(1);
        TRACE_LEN = TRACE_LEN.saturating_add(1).min(MAX_TRACE_EVENTS);
    }
}

pub fn trace(mut f: impl FnMut(TraceEvent)) {
    unsafe {
        let trace = core::ptr::addr_of!(TRACE);
        let start = TRACE_NEXT.saturating_sub(TRACE_LEN as u64);
        let mut offset = 0;
        while offset < TRACE_LEN {
            let index = ((start + offset as u64) as usize) % MAX_TRACE_EVENTS;
            if let Some(event) = (*trace)[index] {
                f(event);
            }
            offset += 1;
        }
    }
}

pub fn trace_op_name(operation: TraceOp) -> &'static str {
    match operation {
        TraceOp::Discover => "discover",
        TraceOp::Match => "match",
        TraceOp::Probe => "probe",
        TraceOp::Publish => "publish",
        TraceOp::Suspend => "suspend",
        TraceOp::Resume => "resume",
        TraceOp::Quiesce => "quiesce",
        TraceOp::Remove => "remove",
        TraceOp::Revoke => "revoke",
        TraceOp::HotUnplug => "hot-unplug",
        TraceOp::Rebind => "rebind",
        TraceOp::ResourceAdd => "resource-add",
        TraceOp::ResourceRelease => "resource-release",
    }
}

#[derive(Clone, Copy)]
pub struct Driver {
    pub id: DriverId,
    pub name: &'static str,
    pub class: Class,
    pub bus: BusKind,
    pub stage: DriverStage,
    pub ops: Option<DriverOps>,
}

impl Driver {
    pub const fn new(id: DriverId, name: &'static str, class: Class, bus: BusKind) -> Self {
        Self {
            id,
            name,
            class,
            bus,
            stage: DriverStage::Runtime,
            ops: None,
        }
    }

    pub const fn early(id: DriverId, name: &'static str, class: Class, bus: BusKind) -> Self {
        Self {
            id,
            name,
            class,
            bus,
            stage: DriverStage::Early,
            ops: None,
        }
    }

    pub const fn with_ops(
        id: DriverId,
        name: &'static str,
        class: Class,
        bus: BusKind,
        ops: DriverOps,
    ) -> Self {
        Self {
            id,
            name,
            class,
            bus,
            stage: DriverStage::Runtime,
            ops: Some(ops),
        }
    }

    pub fn matches(&self, device: &Device) -> bool {
        self.class == device.class && self.bus == device.bus_kind
    }
}

pub fn discover(bus: &mut Bus, device: &mut Device) -> Result<(), DriverError> {
    if device.bus != bus.id || device.bus_kind != bus.kind {
        return Err(DriverError::NotMatched);
    }
    if device.state != DeviceState::Discovered {
        return Err(DriverError::InvalidState {
            expected: DeviceState::Discovered,
            actual: device.state,
        });
    }
    let result = bus.attach(device.id);
    trace_event(TraceOp::Discover, device, None, result.is_ok());
    result
}

pub fn match_driver(driver: Driver, device: &Device) -> bool {
    let matched = driver.matches(device);
    trace_event(TraceOp::Match, device, Some(driver.id), matched);
    matched
}

pub fn probe(device: &mut Device, driver: Driver) -> Result<(), DriverError> {
    if !driver.matches(device) {
        trace_event(TraceOp::Probe, device, Some(driver.id), false);
        return Err(DriverError::NotMatched);
    }
    if let Err(error) = device.transition(DeviceState::Probing) {
        trace_event(TraceOp::Probe, device, Some(driver.id), false);
        return Err(error);
    }
    let Some(ops) = driver.ops else {
        if device.transition(DeviceState::Unsupported).is_err() {
            trace_event(TraceOp::Probe, device, Some(driver.id), false);
            return Err(DriverError::InvalidState {
                expected: DeviceState::Unsupported,
                actual: device.state,
            });
        }
        let _ = device.release_resources();
        trace_event(TraceOp::Probe, device, Some(driver.id), false);
        return Err(DriverError::Unsupported);
    };
    if let Err(error) = (ops.probe)(device) {
        let state = match error {
            DriverError::DeviceAbsent => DeviceState::Unsupported,
            DriverError::Deferred => DeviceState::Deferred,
            DriverError::Unsupported => DeviceState::Unsupported,
            DriverError::Busy => DeviceState::Busy,
            _ => DeviceState::Failed,
        };
        if let Err(transition_error) = device.transition(state) {
            trace_event(TraceOp::Probe, device, Some(driver.id), false);
            return Err(transition_error);
        }
        let _ = device.release_resources();
        trace_event(TraceOp::Probe, device, Some(driver.id), false);
        return Err(error);
    }
    if let Err(error) = device.bind_driver(driver.id) {
        let _ = device.transition(DeviceState::Failed);
        let _ = device.release_resources();
        trace_event(TraceOp::Probe, device, Some(driver.id), false);
        return Err(error);
    }
    let result = device.transition(DeviceState::Ready);
    trace_event(TraceOp::Probe, device, Some(driver.id), result.is_ok());
    result
}

pub fn publish(device: &mut Device) -> Result<(), DriverError> {
    let result = device.publish();
    trace_event(
        TraceOp::Publish,
        device,
        device.bound_driver(),
        result.is_ok(),
    );
    result
}

pub fn suspend(device: &mut Device, driver: Driver) -> Result<(), DriverError> {
    if device.bound_driver() != Some(driver.id) {
        return Err(DriverError::DriverNotBound);
    }
    if device.state != DeviceState::Ready {
        return Err(DriverError::InvalidState {
            expected: DeviceState::Ready,
            actual: device.state,
        });
    }
    let Some(ops) = driver.ops else {
        return Err(DriverError::Unsupported);
    };
    let Some(suspend) = ops.suspend else {
        return Err(DriverError::Unsupported);
    };
    if let Err(error) = suspend(device) {
        trace_event(TraceOp::Suspend, device, Some(driver.id), false);
        return Err(error);
    }
    let result = device.transition(DeviceState::Suspended);
    trace_event(TraceOp::Suspend, device, Some(driver.id), result.is_ok());
    result
}

pub fn resume(device: &mut Device, driver: Driver) -> Result<(), DriverError> {
    if device.bound_driver() != Some(driver.id) {
        return Err(DriverError::DriverNotBound);
    }
    if device.state != DeviceState::Suspended {
        return Err(DriverError::InvalidState {
            expected: DeviceState::Suspended,
            actual: device.state,
        });
    }
    let Some(ops) = driver.ops else {
        return Err(DriverError::Unsupported);
    };
    let Some(resume) = ops.resume else {
        return Err(DriverError::Unsupported);
    };
    if let Err(error) = resume(device) {
        trace_event(TraceOp::Resume, device, Some(driver.id), false);
        return Err(error);
    }
    let result = device.transition(DeviceState::Ready);
    trace_event(TraceOp::Resume, device, Some(driver.id), result.is_ok());
    result
}

pub fn quiesce(device: &mut Device, driver: Driver) -> Result<(), DriverError> {
    if device.bound_driver() != Some(driver.id) {
        return Err(DriverError::DriverNotBound);
    }
    if !matches!(device.state, DeviceState::Ready | DeviceState::Suspended) {
        return Err(DriverError::InvalidState {
            expected: DeviceState::Ready,
            actual: device.state,
        });
    }
    let Some(ops) = driver.ops else {
        return Err(DriverError::Unsupported);
    };
    if let Some(quiesce) = ops.quiesce {
        if let Err(error) = quiesce(device) {
            trace_event(TraceOp::Quiesce, device, Some(driver.id), false);
            return Err(error);
        }
    }
    let result = device.transition(DeviceState::Quiescing);
    trace_event(TraceOp::Quiesce, device, Some(driver.id), result.is_ok());
    result
}

pub fn remove(device: &mut Device, driver: Driver) -> Result<(), DriverError> {
    if device.bound_driver() != Some(driver.id) {
        return Err(DriverError::DriverNotBound);
    }
    if device.state != DeviceState::Quiescing {
        return Err(DriverError::InvalidState {
            expected: DeviceState::Quiescing,
            actual: device.state,
        });
    }
    let Some(ops) = driver.ops else {
        return Err(DriverError::Unsupported);
    };
    if let Err(error) = (ops.remove)(device) {
        let _ = device.transition(DeviceState::Failed);
        trace_event(TraceOp::Remove, device, Some(driver.id), false);
        return Err(error);
    }
    if device.is_published() {
        device.unpublish()?;
    }
    device.unbind_driver()?;
    device.transition(DeviceState::Removed)?;
    let result = device.release_resources();
    trace_event(TraceOp::Remove, device, Some(driver.id), result.is_ok());
    result
}

pub fn revoke(device: &mut Device, driver: Driver) -> Result<(), DriverError> {
    if device.bound_driver() != Some(driver.id) {
        return Err(DriverError::DriverNotBound);
    }
    if !matches!(device.state, DeviceState::Quiescing | DeviceState::Failed) {
        return Err(DriverError::InvalidState {
            expected: DeviceState::Quiescing,
            actual: device.state,
        });
    }
    if device.is_published() {
        device.unpublish()?;
    }
    device.unbind_driver()?;
    if device.state != DeviceState::Removed {
        device.transition(DeviceState::Removed)?;
    }
    let result = device.release_resources();
    trace_event(TraceOp::Revoke, device, Some(driver.id), result.is_ok());
    result
}

pub fn hot_unplug(device: &mut Device, driver: Driver) -> Result<(), DriverError> {
    quiesce(device, driver)?;
    let result = remove(device, driver);
    trace_event(TraceOp::HotUnplug, device, Some(driver.id), result.is_ok());
    result
}

pub fn rebind(
    device: &mut Device,
    old_driver: Driver,
    new_driver: Driver,
) -> Result<(), DriverError> {
    if device.bound_driver() != Some(old_driver.id) {
        return Err(DriverError::DriverNotBound);
    }
    quiesce(device, old_driver)?;
    remove(device, old_driver)?;
    let result = probe(device, new_driver);
    trace_event(TraceOp::Rebind, device, Some(new_driver.id), result.is_ok());
    result
}

fn contract_ok(_device: &mut Device) -> Result<(), DriverError> {
    Ok(())
}

fn contract_fail(_device: &mut Device) -> Result<(), DriverError> {
    Err(DriverError::ProbeFailed)
}

fn contract_unsupported(_device: &mut Device) -> Result<(), DriverError> {
    Err(DriverError::Unsupported)
}

fn contract_deferred(_device: &mut Device) -> Result<(), DriverError> {
    Err(DriverError::Deferred)
}

fn contract_busy(_device: &mut Device) -> Result<(), DriverError> {
    Err(DriverError::Busy)
}

fn contract_absent(_device: &mut Device) -> Result<(), DriverError> {
    Err(DriverError::DeviceAbsent)
}

fn contract_timeout(_device: &mut Device) -> Result<(), DriverError> {
    Err(DriverError::Timeout)
}

fn contract_malformed(_device: &mut Device) -> Result<(), DriverError> {
    Err(DriverError::MalformedDescriptor)
}

fn contract_dma_failure(_device: &mut Device) -> Result<(), DriverError> {
    Err(DriverError::DmaFailure)
}

fn contract_probe_status(
    bus: &mut Bus,
    id: DeviceId,
    name: &'static str,
    probe_callback: DeviceCallback,
    expected_state: DeviceState,
    expected_error: DriverError,
) {
    let mut device = Device::new(id, bus.id, bus.kind, name, Class::Serial);
    let driver = Driver::with_ops(
        id,
        name,
        Class::Serial,
        BusKind::Platform,
        DriverOps {
            probe: probe_callback,
            suspend: None,
            resume: None,
            quiesce: None,
            remove: contract_ok,
        },
    );
    assert!(discover(bus, &mut device).is_ok());
    assert!(device
        .add_resource(Resource::Pio {
            base: 0x3f8,
            size: 8,
            owner: device.id,
        })
        .is_ok());
    assert!(matches!(
        probe(&mut device, driver),
        Err(error) if error == expected_error
    ));
    assert!(device.state == expected_state);
    assert!(device.resources().is_empty());
}

#[derive(Clone, Copy)]
pub struct MatrixReport {
    pub scenarios: usize,
    pub passed: usize,
}

pub fn matrix_self_check() -> MatrixReport {
    let mut bus = Bus::new(90, "matrix", BusKind::Platform);
    let mut passed = 0;

    if contract_probe_status_result(
        &mut bus,
        90,
        "matrix-absent",
        contract_absent,
        DeviceState::Unsupported,
        DriverError::DeviceAbsent,
    ) {
        passed += 1;
    }
    if contract_probe_status_result(
        &mut bus,
        91,
        "matrix-timeout",
        contract_timeout,
        DeviceState::Failed,
        DriverError::Timeout,
    ) {
        passed += 1;
    }
    if contract_probe_status_result(
        &mut bus,
        92,
        "matrix-malformed",
        contract_malformed,
        DeviceState::Failed,
        DriverError::MalformedDescriptor,
    ) {
        passed += 1;
    }
    if contract_probe_status_result(
        &mut bus,
        93,
        "matrix-dma-failure",
        contract_dma_failure,
        DeviceState::Failed,
        DriverError::DmaFailure,
    ) {
        passed += 1;
    }

    let mut hot_unplug_device =
        Device::new(94, bus.id, bus.kind, "matrix-hot-unplug", Class::Serial);
    let hot_unplug_driver = Driver::with_ops(
        94,
        "matrix-hot-unplug",
        Class::Serial,
        BusKind::Platform,
        DriverOps {
            probe: contract_ok,
            suspend: None,
            resume: None,
            quiesce: None,
            remove: contract_ok,
        },
    );
    let hot_unplug_passed = discover(&mut bus, &mut hot_unplug_device).is_ok()
        && hot_unplug_device
            .add_resource(Resource::Pio {
                base: 0x3f8,
                size: 8,
                owner: hot_unplug_device.id,
            })
            .is_ok()
        && probe(&mut hot_unplug_device, hot_unplug_driver).is_ok()
        && publish(&mut hot_unplug_device).is_ok()
        && hot_unplug(&mut hot_unplug_device, hot_unplug_driver).is_ok()
        && hot_unplug_device.state == DeviceState::Removed
        && hot_unplug_device.resources().is_empty();
    if hot_unplug_passed {
        passed += 1;
    }

    if crate::irq::interrupt_storm_self_check() {
        passed += 1;
    }

    MatrixReport {
        scenarios: 6,
        passed,
    }
}

fn contract_probe_status_result(
    bus: &mut Bus,
    id: DeviceId,
    name: &'static str,
    probe_callback: DeviceCallback,
    expected_state: DeviceState,
    expected_error: DriverError,
) -> bool {
    let mut device = Device::new(id, bus.id, bus.kind, name, Class::Serial);
    let driver = Driver::with_ops(
        id,
        name,
        Class::Serial,
        BusKind::Platform,
        DriverOps {
            probe: probe_callback,
            suspend: None,
            resume: None,
            quiesce: None,
            remove: contract_ok,
        },
    );
    discover(bus, &mut device).is_ok()
        && device
            .add_resource(Resource::Pio {
                base: 0x3f8,
                size: 8,
                owner: device.id,
            })
            .is_ok()
        && matches!(probe(&mut device, driver), Err(error) if error == expected_error)
        && device.state == expected_state
        && device.resources().is_empty()
}

pub fn contract_self_check() {
    let physical = crate::address::PhysAddr::new(0x1000);
    let virtual_address = crate::address::VirtAddr::new(0xffff_8000_0000_1000);
    let _ = physical.value();
    let _ = virtual_address.value();
    let _ = crate::arch::physical_to_virtual(physical);
    let _ = crate::arch::virtual_to_physical(virtual_address);
    assert!(unsafe { crate::io::MmioRegion::new(usize::MAX, 2) }.is_none());
    #[cfg(target_arch = "x86_64")]
    {
        assert!(crate::io::PioRegion::new(0xffff, 2).is_none());
        assert!(crate::io::PioRegion::new(0x3f8, 8).is_some());
    }

    let _other_buses = [BusKind::Pci, BusKind::Virtio, BusKind::Usb, BusKind::Ps2];
    let _other_states = [
        DeviceState::Suspended,
        DeviceState::Deferred,
        DeviceState::Unsupported,
        DeviceState::Busy,
        DeviceState::Failed,
    ];
    let _irq_kinds = [IrqKind::Legacy, IrqKind::Msi, IrqKind::MsiX, IrqKind::Gic];
    let _dma_directions = [
        DmaDirection::ToDevice,
        DmaDirection::FromDevice,
        DmaDirection::Bidirectional,
    ];
    let _driver_errors = [
        DriverError::Unsupported,
        DriverError::Deferred,
        DriverError::Busy,
        DriverError::Timeout,
        DriverError::ProbeFailed,
        DriverError::RemoveFailed,
        DriverError::NotMatched,
        DriverError::NotPublished,
        DriverError::InvalidResourceRange,
        DriverError::InvalidAlignment,
        DriverError::InvalidAddressRange,
        DriverError::InvalidDmaDirection,
        DriverError::DeviceAbsent,
        DriverError::MalformedDescriptor,
        DriverError::DmaFailure,
        DriverError::InterruptStorm,
    ];

    let mut bus = Bus::new(1, "platform", BusKind::Platform);
    let mut device = Device::new(1, bus.id, bus.kind, "contract-test", Class::Serial);
    let driver = Driver::with_ops(
        1,
        "contract-test",
        Class::Serial,
        BusKind::Platform,
        DriverOps {
            probe: contract_ok,
            suspend: Some(contract_ok),
            resume: Some(contract_ok),
            quiesce: Some(contract_ok),
            remove: contract_ok,
        },
    );

    assert!(discover(&mut bus, &mut device).is_ok());
    assert_eq!(bus.name, "platform");
    assert_eq!(bus.devices(), &[Some(device.id)]);
    assert_eq!(device.bus, bus.id);
    assert_eq!(device.name, "contract-test");
    assert!(match_driver(driver, &device));
    assert!(device
        .add_resource(Resource::Mmio {
            base: 0x1000,
            size: 0x1000,
            owner: device.id,
        })
        .is_ok());
    assert!(device
        .add_resource(Resource::Pio {
            base: 0x3f8,
            size: 8,
            owner: device.id,
        })
        .is_ok());
    assert!(device
        .add_resource(Resource::Irq(Irq {
            kind: IrqKind::Legacy,
            line: 4,
            owner: device.id,
        }))
        .is_ok());
    assert!(device
        .add_resource(Resource::Dma(DmaBuffer {
            physical: crate::address::PhysAddr::new(0x1000),
            virtual_address: crate::address::VirtAddr::new(0xffff_8000_0000_1000),
            length: 4096,
            alignment: 4096,
            direction: DmaDirection::Bidirectional,
            owner: device.id,
        }))
        .is_ok());
    let from_device_dma = DmaBuffer {
        physical: crate::address::PhysAddr::new(0x2000),
        virtual_address: crate::address::VirtAddr::new(0xffff_8000_0000_2000),
        length: 4096,
        alignment: 4096,
        direction: DmaDirection::FromDevice,
        owner: device.id,
    };
    assert!(matches!(
        unsafe { from_device_dma.sync_for_device() },
        Err(DriverError::InvalidDmaDirection)
    ));
    assert_eq!(device.resources().len(), 4);
    let mut invalid_resources =
        Device::new(7, bus.id, bus.kind, "invalid-resources", Class::Serial);
    assert!(matches!(
        invalid_resources.add_resource(Resource::Mmio {
            base: u64::MAX,
            size: 2,
            owner: invalid_resources.id,
        }),
        Err(DriverError::InvalidResourceRange)
    ));
    assert!(matches!(
        invalid_resources.add_resource(Resource::Dma(DmaBuffer {
            physical: crate::address::PhysAddr::new(3),
            virtual_address: crate::address::VirtAddr::new(0x1003),
            length: 16,
            alignment: 4,
            direction: DmaDirection::ToDevice,
            owner: invalid_resources.id,
        })),
        Err(DriverError::InvalidAlignment)
    ));
    assert!(probe(&mut device, driver).is_ok());
    assert!(publish(&mut device).is_ok());
    assert!(device.is_published());
    assert!(suspend(&mut device, driver).is_ok());
    assert!(resume(&mut device, driver).is_ok());
    assert!(quiesce(&mut device, driver).is_ok());
    assert!(remove(&mut device, driver).is_ok());
    assert!(device.resources().is_empty());

    let new_driver = Driver::with_ops(
        2,
        "contract-test-rebind",
        Class::Serial,
        BusKind::Platform,
        DriverOps {
            probe: contract_ok,
            suspend: None,
            resume: None,
            quiesce: None,
            remove: contract_ok,
        },
    );
    let mut rebound = Device::new(2, bus.id, bus.kind, "rebind-test", Class::Serial);
    assert!(discover(&mut bus, &mut rebound).is_ok());
    assert!(probe(&mut rebound, driver).is_ok());
    assert!(publish(&mut rebound).is_ok());
    assert!(rebind(&mut rebound, driver, new_driver).is_ok());
    assert!(rebound.state == DeviceState::Ready);

    let failing_driver = Driver::with_ops(
        3,
        "contract-test-fail",
        Class::Serial,
        BusKind::Platform,
        DriverOps {
            probe: contract_fail,
            suspend: None,
            resume: None,
            quiesce: None,
            remove: contract_ok,
        },
    );
    let mut failed = Device::new(3, bus.id, bus.kind, "failed-test", Class::Serial);
    assert!(discover(&mut bus, &mut failed).is_ok());
    assert!(failed
        .add_resource(Resource::Pio {
            base: 0x3f8,
            size: 8,
            owner: failed.id,
        })
        .is_ok());
    assert!(matches!(
        probe(&mut failed, failing_driver),
        Err(DriverError::ProbeFailed)
    ));
    assert!(failed.state == DeviceState::Failed);
    assert!(failed.resources().is_empty());

    contract_probe_status(
        &mut bus,
        4,
        "unsupported-test",
        contract_unsupported,
        DeviceState::Unsupported,
        DriverError::Unsupported,
    );
    contract_probe_status(
        &mut bus,
        5,
        "deferred-test",
        contract_deferred,
        DeviceState::Deferred,
        DriverError::Deferred,
    );
    contract_probe_status(
        &mut bus,
        6,
        "busy-test",
        contract_busy,
        DeviceState::Busy,
        DriverError::Busy,
    );
}

#[derive(Clone, Copy)]
pub struct DriverStatus {
    pub driver: Driver,
    pub state: DeviceState,
}

static mut DRIVERS: [Option<DriverStatus>; MAX_DRIVERS] = [None; MAX_DRIVERS];
static mut LEN: usize = 0;

pub fn init() {
    unsafe {
        LEN = 0;
        DRIVERS = [None; MAX_DRIVERS];
        TRACE = [None; MAX_TRACE_EVENTS];
        TRACE_NEXT = 0;
        TRACE_LEN = 0;
    }
}

pub fn register(driver: Driver, state: DeviceState) -> bool {
    unsafe {
        if LEN == MAX_DRIVERS {
            return false;
        }
        DRIVERS[LEN] = Some(DriverStatus { driver, state });
        LEN += 1;
        true
    }
}

pub fn list(mut f: impl FnMut(DriverStatus)) {
    unsafe {
        let drivers = &raw const DRIVERS;
        for i in 0..LEN {
            if let Some(driver) = (*drivers)[i] {
                f(driver);
            }
        }
    }
}

pub fn class_name(class: Class) -> &'static str {
    match class {
        Class::Display => "display",
        Class::Serial => "serial",
        Class::Block => "block",
        Class::Input => "input",
        Class::UsbHost => "usb-host",
        Class::Audio => "audio",
        Class::Network => "network",
    }
}

pub fn bus_name(bus: BusKind) -> &'static str {
    match bus {
        BusKind::Platform => "platform",
        BusKind::Pci => "pci",
        BusKind::Virtio => "virtio",
        BusKind::Usb => "usb",
        BusKind::Ps2 => "ps2",
    }
}

pub fn stage_name(stage: DriverStage) -> &'static str {
    match stage {
        DriverStage::Early => "early",
        DriverStage::Runtime => "runtime",
    }
}

pub fn state_name(state: DeviceState) -> &'static str {
    match state {
        DeviceState::Discovered => "discovered",
        DeviceState::Probing => "probing",
        DeviceState::Ready => "ready",
        DeviceState::Suspended => "suspended",
        DeviceState::Deferred => "deferred",
        DeviceState::Unsupported => "unsupported",
        DeviceState::Busy => "busy",
        DeviceState::Quiescing => "quiescing",
        DeviceState::Removed => "removed",
        DeviceState::Failed => "failed",
    }
}
