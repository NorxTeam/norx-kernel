use crate::drivers::framework::{
    self, Bus, BusKind, Class, Device, DeviceState, DmaBuffer, DmaDirection, Driver, DriverError,
    DriverOps, Resource,
};
use crate::process::{
    Credentials, ProcessId, ProcessState, ProcessTable, ThreadId, ThreadKind, ThreadState,
};

const MAX_SERVICES: usize = 8;
const MAX_RESTARTS: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState {
    Registered,
    Running,
    Quiescing,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Capacity,
    InvalidHandle,
    GenerationExhausted,
    RestartLimit,
    AlreadyRegistered,
    InvalidState,
    DeviceMismatch,
    NotUserThread,
    Process(crate::process::Error),
    Driver(DriverError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceHandle {
    slot: u8,
    generation: u16,
}

#[derive(Clone, Copy)]
struct ServiceRecord {
    device: DeviceId,
    driver: Driver,
    process: ProcessId,
    thread: ThreadId,
    state: ServiceState,
    endpoint_active: bool,
    restarts: u8,
}

type DeviceId = framework::DeviceId;

pub struct Supervisor {
    records: [Option<ServiceRecord>; MAX_SERVICES],
    generations: [u16; MAX_SERVICES],
}

#[cfg(target_arch = "x86_64")]
struct RiskyDriverRuntime {
    bus: Bus,
    device: Device,
    driver: Driver,
    supervisor: Supervisor,
    process: crate::user_runtime::NativeRuntime,
}

#[cfg(target_arch = "x86_64")]
static mut ACTIVE_RISKY_DRIVER: Option<RiskyDriverRuntime> = None;

impl Supervisor {
    pub const fn new() -> Self {
        Self {
            records: [None; MAX_SERVICES],
            generations: [0; MAX_SERVICES],
        }
    }

    pub fn register(
        &mut self,
        device: &Device,
        driver: Driver,
        processes: &ProcessTable,
        process: ProcessId,
        thread: ThreadId,
    ) -> Result<ServiceHandle, Error> {
        if device.state != DeviceState::Ready
            || !device.is_published()
            || device.bound_driver() != Some(driver.id)
        {
            return Err(Error::InvalidState);
        }
        self.validate_process(processes, process, thread)?;
        if self
            .records
            .iter()
            .flatten()
            .any(|record| record.device == device.id)
        {
            return Err(Error::AlreadyRegistered);
        }
        let slot = self
            .records
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let generation = self.generations[slot];
        self.records[slot] = Some(ServiceRecord {
            device: device.id,
            driver,
            process,
            thread,
            state: ServiceState::Registered,
            endpoint_active: false,
            restarts: 0,
        });
        Ok(ServiceHandle {
            slot: slot as u8,
            generation,
        })
    }

    pub fn start(
        &mut self,
        handle: ServiceHandle,
        device: &Device,
        driver: Driver,
        processes: &ProcessTable,
    ) -> Result<(), Error> {
        let (slot, mut record) = self.record(handle)?;
        self.validate_device(&record, device, driver)?;
        if record.state != ServiceState::Registered {
            return Err(Error::InvalidState);
        }
        self.validate_process(processes, record.process, record.thread)?;
        record.state = ServiceState::Running;
        record.endpoint_active = true;
        self.records[slot] = Some(record);
        Ok(())
    }

    pub fn quiesce(
        &mut self,
        handle: ServiceHandle,
        device: &mut Device,
        driver: Driver,
    ) -> Result<(), Error> {
        let (slot, mut record) = self.record(handle)?;
        self.validate_device(&record, device, driver)?;
        if record.state != ServiceState::Running {
            return Err(Error::InvalidState);
        }
        if let Err(error) = framework::quiesce(device, driver) {
            record.state = ServiceState::Failed;
            record.endpoint_active = false;
            self.records[slot] = Some(record);
            return Err(Error::Driver(error));
        }
        record.state = ServiceState::Quiescing;
        record.endpoint_active = false;
        self.records[slot] = Some(record);
        Ok(())
    }

    pub fn stop(
        &mut self,
        handle: ServiceHandle,
        device: &mut Device,
        driver: Driver,
        processes: &mut ProcessTable,
    ) -> Result<(), Error> {
        let (slot, mut record) = self.record(handle)?;
        self.validate_device(&record, device, driver)?;
        if record.state != ServiceState::Quiescing {
            return Err(Error::InvalidState);
        }

        let mut failure = None;
        if let Err(error) = framework::remove(device, driver) {
            failure = Some(Error::Driver(error));
            if let Err(revoke_error) = framework::revoke(device, driver) {
                failure = Some(Error::Driver(revoke_error));
            }
        }
        if device.state != DeviceState::Removed || !device.resources().is_empty() {
            failure.get_or_insert(Error::InvalidState);
        }
        match processes.process_state(record.process) {
            Ok(ProcessState::Running) => {
                if let Err(error) = processes.exit(record.process, 0) {
                    failure.get_or_insert(Error::Process(error));
                }
            }
            Ok(ProcessState::Exiting | ProcessState::Zombie | ProcessState::Reaped) => {}
            Ok(ProcessState::Creating) => {
                failure.get_or_insert(Error::InvalidState);
            }
            Err(crate::process::Error::InvalidId) => {}
            Err(error) => {
                failure.get_or_insert(Error::Process(error));
            }
        }
        record.endpoint_active = false;
        record.state = if failure.is_some() {
            ServiceState::Failed
        } else {
            ServiceState::Stopped
        };
        self.records[slot] = Some(record);
        failure.map_or(Ok(()), Err)
    }

    pub fn restart(
        &mut self,
        handle: ServiceHandle,
        device: &mut Device,
        driver: Driver,
        processes: &ProcessTable,
        process: ProcessId,
        thread: ThreadId,
    ) -> Result<(), Error> {
        let (slot, mut record) = self.record(handle)?;
        self.validate_device_id(&record, device, driver)?;
        if !matches!(record.state, ServiceState::Stopped | ServiceState::Failed) {
            return Err(Error::InvalidState);
        }
        if record.restarts >= MAX_RESTARTS {
            return Err(Error::RestartLimit);
        }
        self.validate_process(processes, process, thread)?;
        framework::probe(device, driver).map_err(Error::Driver)?;
        framework::publish(device).map_err(Error::Driver)?;
        record.driver = driver;
        record.process = process;
        record.thread = thread;
        record.state = ServiceState::Registered;
        record.endpoint_active = false;
        record.restarts = record.restarts.saturating_add(1);
        self.records[slot] = Some(record);
        Ok(())
    }

    pub fn recover_crash(
        &mut self,
        handle: ServiceHandle,
        device: &mut Device,
        driver: Driver,
        processes: &mut ProcessTable,
    ) -> Result<bool, Error> {
        let (slot, record) = self.record(handle)?;
        self.validate_device(&record, device, driver)?;
        if record.state != ServiceState::Running {
            return Err(Error::InvalidState);
        }
        if processes
            .process_state(record.process)
            .map_err(Error::Process)?
            == ProcessState::Running
        {
            return Ok(false);
        }
        self.quiesce(handle, device, driver)?;
        self.stop(handle, device, driver, processes)?;
        let (_, mut record) = self.record(handle)?;
        record.state = ServiceState::Failed;
        record.endpoint_active = false;
        self.records[slot] = Some(record);
        Ok(true)
    }

    pub fn unregister(&mut self, handle: ServiceHandle) -> Result<(), Error> {
        let (slot, record) = self.record(handle)?;
        if matches!(
            record.state,
            ServiceState::Running | ServiceState::Quiescing
        ) {
            return Err(Error::InvalidState);
        }
        self.records[slot] = None;
        self.generations[slot] = self.generations[slot]
            .checked_add(1)
            .ok_or(Error::GenerationExhausted)?;
        Ok(())
    }

    pub fn state(&self, handle: ServiceHandle) -> Result<ServiceState, Error> {
        Ok(self.record(handle)?.1.state)
    }

    pub fn endpoint_active(&self, handle: ServiceHandle) -> Result<bool, Error> {
        Ok(self.record(handle)?.1.endpoint_active)
    }

    pub fn restart_count(&self, handle: ServiceHandle) -> Result<u8, Error> {
        Ok(self.record(handle)?.1.restarts)
    }

    fn validate_process(
        &self,
        processes: &ProcessTable,
        process: ProcessId,
        thread: ThreadId,
    ) -> Result<(), Error> {
        if process == ProcessId::INIT
            || processes.process_state(process).map_err(Error::Process)? != ProcessState::Running
            || processes.thread_owner(thread).map_err(Error::Process)? != Some(process)
            || processes.thread_kind(thread).map_err(Error::Process)? != ThreadKind::User
            || !matches!(
                processes.thread_state(thread).map_err(Error::Process)?,
                ThreadState::Ready | ThreadState::Running
            )
        {
            return Err(Error::NotUserThread);
        }
        Ok(())
    }

    fn validate_device(
        &self,
        record: &ServiceRecord,
        device: &Device,
        driver: Driver,
    ) -> Result<(), Error> {
        self.validate_device_id(record, device, driver)?;
        if device.bound_driver() != Some(driver.id) {
            return Err(Error::DeviceMismatch);
        }
        Ok(())
    }

    fn validate_device_id(
        &self,
        record: &ServiceRecord,
        device: &Device,
        driver: Driver,
    ) -> Result<(), Error> {
        if record.device != device.id || record.driver.id != driver.id {
            return Err(Error::DeviceMismatch);
        }
        Ok(())
    }

    fn record(&self, handle: ServiceHandle) -> Result<(usize, ServiceRecord), Error> {
        let slot = handle.slot as usize;
        if slot >= MAX_SERVICES || self.generations[slot] != handle.generation {
            return Err(Error::InvalidHandle);
        }
        self.records[slot]
            .map(|record| (slot, record))
            .ok_or(Error::InvalidHandle)
    }
}

fn service_probe(device: &mut Device) -> Result<(), DriverError> {
    device.add_resource(Resource::Mmio {
        base: 0x1000,
        size: 0x1000,
        owner: device.id,
    })?;
    device.add_resource(Resource::Dma(DmaBuffer {
        physical: crate::address::PhysAddr::new(0x2000),
        virtual_address: crate::address::VirtAddr::new(0xffff_8000_0000_2000),
        length: 4096,
        alignment: 4096,
        direction: DmaDirection::Bidirectional,
        owner: device.id,
    }))
}

fn service_remove_ok(_device: &mut Device) -> Result<(), DriverError> {
    Ok(())
}

fn service_remove_fail(_device: &mut Device) -> Result<(), DriverError> {
    Err(DriverError::RemoveFailed)
}

pub fn contract_self_check() {
    let mut processes = ProcessTable::new();
    let (init, _) = processes.create_init().unwrap();
    let service_credentials = Credentials {
        capabilities: 0,
        ..Credentials::BOOTSTRAP
    };
    let (service_process, service_thread) =
        processes.spawn_child(init, service_credentials).unwrap();
    let mut bus = Bus::new(120, "service-contract", BusKind::Platform);
    let mut device = Device::new(120, bus.id, bus.kind, "service-contract", Class::Network);
    let driver = Driver::with_ops(
        120,
        "service-contract",
        Class::Network,
        BusKind::Platform,
        DriverOps {
            probe: service_probe,
            suspend: None,
            resume: None,
            quiesce: None,
            remove: service_remove_ok,
        },
    );
    assert!(framework::discover(&mut bus, &mut device).is_ok());
    assert!(framework::probe(&mut device, driver).is_ok());
    assert!(framework::publish(&mut device).is_ok());

    let mut supervisor = Supervisor::new();
    let handle = supervisor
        .register(&device, driver, &processes, service_process, service_thread)
        .unwrap();
    assert_eq!(supervisor.state(handle), Ok(ServiceState::Registered));
    assert!(!supervisor.endpoint_active(handle).unwrap());
    supervisor
        .start(handle, &device, driver, &processes)
        .unwrap();
    assert!(supervisor.endpoint_active(handle).unwrap());
    supervisor.quiesce(handle, &mut device, driver).unwrap();
    assert!(!supervisor.endpoint_active(handle).unwrap());
    supervisor
        .stop(handle, &mut device, driver, &mut processes)
        .unwrap();
    assert_eq!(supervisor.state(handle), Ok(ServiceState::Stopped));
    assert_eq!(supervisor.restart_count(handle), Ok(0));
    assert_eq!(device.state, DeviceState::Removed);
    assert!(device.resources().is_empty());
    assert_eq!(
        processes.process_state(service_process),
        Ok(ProcessState::Zombie)
    );

    let (replacement_process, replacement_thread) =
        processes.spawn_child(init, service_credentials).unwrap();
    supervisor
        .restart(
            handle,
            &mut device,
            driver,
            &processes,
            replacement_process,
            replacement_thread,
        )
        .unwrap();
    supervisor
        .start(handle, &device, driver, &processes)
        .unwrap();
    assert_eq!(
        processes.authorize(replacement_process, crate::process::Capability::DeviceAdmin),
        Err(crate::process::Error::PermissionDenied)
    );
    processes.exit(replacement_process, -9).unwrap();
    assert!(supervisor
        .recover_crash(handle, &mut device, driver, &mut processes)
        .unwrap());
    assert_eq!(supervisor.state(handle), Ok(ServiceState::Failed));
    assert_eq!(device.state, DeviceState::Removed);
    assert!(device.resources().is_empty());

    let (failed_process, failed_thread) = processes.spawn_child(init, service_credentials).unwrap();
    let failing_driver = Driver::with_ops(
        driver.id,
        driver.name,
        driver.class,
        driver.bus,
        DriverOps {
            probe: service_probe,
            suspend: None,
            resume: None,
            quiesce: None,
            remove: service_remove_fail,
        },
    );
    supervisor
        .restart(
            handle,
            &mut device,
            failing_driver,
            &processes,
            failed_process,
            failed_thread,
        )
        .unwrap();
    supervisor
        .start(handle, &device, failing_driver, &processes)
        .unwrap();
    supervisor
        .quiesce(handle, &mut device, failing_driver)
        .unwrap();
    assert!(matches!(
        supervisor.stop(handle, &mut device, failing_driver, &mut processes),
        Err(Error::Driver(DriverError::RemoveFailed))
    ));
    assert_eq!(supervisor.state(handle), Ok(ServiceState::Failed));
    assert_eq!(device.state, DeviceState::Removed);
    assert!(device.resources().is_empty());
    assert!(!supervisor.endpoint_active(handle).unwrap());
    supervisor.unregister(handle).unwrap();
    assert_eq!(supervisor.state(handle), Err(Error::InvalidHandle));
}

#[cfg(target_arch = "x86_64")]
pub fn risky_driver_handoff() -> bool {
    if unsafe { (*core::ptr::addr_of!(ACTIVE_RISKY_DRIVER)).is_some() } {
        return true;
    }
    let Some(_) = crate::drivers::usb::xhci::status() else {
        crate::bootlog::warn("xHCI service handoff deferred: controller unavailable");
        return true;
    };
    let driver = crate::drivers::usb::xhci::service_driver();
    let mut bus = Bus::new(7, "pci-xhci-service", BusKind::Pci);
    let mut device = Device::new(
        crate::drivers::usb::xhci::SERVICE_DEVICE_ID,
        bus.id,
        bus.kind,
        "xhci-service",
        Class::UsbHost,
    );
    if framework::discover(&mut bus, &mut device)
        .and_then(|_| framework::probe(&mut device, driver))
        .and_then(|_| framework::publish(&mut device))
        .is_err()
    {
        crate::bootlog::warn("xHCI service handoff failed while binding resources");
        return false;
    }

    let credentials = Credentials {
        capabilities: 0,
        ..Credentials::BOOTSTRAP
    };
    let (process, thread) = match crate::process::spawn_child_current(credentials) {
        Ok(ids) => ids,
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!(
                "xHCI service process creation failed: {:?}",
                error
            ));
            return false;
        }
    };
    let image = crate::elf::service_image(crate::elf::Machine::current());
    let load_bias = crate::elf::load_bias_for_image(&image, USER_SERVICE_BASE, 53);
    let plan = match crate::elf::parse(&image, crate::elf::Machine::current(), load_bias) {
        Ok(plan) => plan,
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!("xHCI service image parse failed: {:?}", error));
            return false;
        }
    };
    let arguments = [b"xhci-service".as_slice()];
    let environment: [&[u8]; 0] = [];
    let mut runtime = match crate::user_runtime::NativeRuntime::prepare_image(
        process,
        &image,
        &plan,
        crate::address_space::AslrHook::new(53),
        &arguments,
        &environment,
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!(
                "xHCI service address-space preparation failed: {:?}",
                error
            ));
            return false;
        }
    };

    let mut supervisor = Supervisor::new();
    let handle = match crate::process::with_process_table(|processes| {
        supervisor.register(&device, driver, processes, process, thread)
    }) {
        Ok(handle) => handle,
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!(
                "xHCI service registration failed: {:?}",
                error
            ));
            return false;
        }
    };
    crate::bootlog::ok("xHCI risky driver service registered with MMIO and DMA ownership");
    if let Err(error) = crate::process::with_process_table(|processes| {
        supervisor.start(handle, &device, driver, processes)
    }) {
        crate::bootlog::warn_fmt(format_args!("xHCI service start failed: {:?}", error));
        return false;
    }
    crate::bootlog::ok("xHCI risky driver service started; endpoint active");

    if crate::process::switch_to_user(process, thread).is_err()
        || runtime.start().is_err()
        || runtime.enter_user().is_err()
    {
        crate::bootlog::warn("xHCI service user entry failed");
        return false;
    }
    if crate::process::restore_init().is_err()
        || crate::process::wait_current(Some(process.get())) != Ok((process.get(), 42))
    {
        crate::bootlog::warn("xHCI service exit was not reaped by init");
        return false;
    }

    if let Err(error) = supervisor.quiesce(handle, &mut device, driver) {
        crate::bootlog::warn_fmt(format_args!("xHCI service quiesce failed: {:?}", error));
        return false;
    }
    crate::bootlog::ok("xHCI risky driver service quiesced; endpoint revoked");
    if let Err(error) = crate::process::with_process_table(|processes| {
        supervisor.stop(handle, &mut device, driver, processes)
    }) {
        crate::bootlog::warn_fmt(format_args!("xHCI service stop failed: {:?}", error));
        return false;
    }
    crate::bootlog::ok("xHCI risky driver service stopped; resources revoked");

    let (replacement_process, replacement_thread) =
        match crate::process::spawn_child_current(credentials) {
            Ok(ids) => ids,
            Err(error) => {
                crate::bootlog::warn_fmt(format_args!(
                    "xHCI service replacement process creation failed: {:?}",
                    error
                ));
                return false;
            }
        };
    if let Err(error) = crate::process::with_process_table(|processes| {
        supervisor.restart(
            handle,
            &mut device,
            driver,
            processes,
            replacement_process,
            replacement_thread,
        )
    }) {
        crate::bootlog::warn_fmt(format_args!("xHCI service restart failed: {:?}", error));
        return false;
    }
    if let Err(error) = crate::process::with_process_table(|processes| {
        supervisor.start(handle, &device, driver, processes)
    }) {
        crate::bootlog::warn_fmt(format_args!(
            "xHCI replacement service start failed: {:?}",
            error
        ));
        return false;
    }
    crate::bootlog::ok("xHCI risky driver service restarted after clean stop");

    if crate::process::with_process_table(|processes| processes.exit(replacement_process, -9))
        .is_err()
    {
        crate::bootlog::warn("xHCI service crash injection failed");
        return false;
    }
    if crate::process::with_process_table(|processes| {
        supervisor.recover_crash(handle, &mut device, driver, processes)
    }) != Ok(true)
    {
        crate::bootlog::warn("xHCI service crash recovery failed");
        return false;
    }
    crate::bootlog::ok("xHCI risky driver crash recovered; endpoint and DMA resources revoked");

    let (final_process, final_thread) = match crate::process::spawn_child_current(credentials) {
        Ok(ids) => ids,
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!(
                "xHCI final service process creation failed: {:?}",
                error
            ));
            return false;
        }
    };
    if crate::process::with_process_table(|processes| {
        supervisor.restart(
            handle,
            &mut device,
            driver,
            processes,
            final_process,
            final_thread,
        )
    })
    .is_err()
        || crate::process::with_process_table(|processes| {
            supervisor.start(handle, &device, driver, processes)
        })
        .is_err()
    {
        crate::bootlog::warn("xHCI service final restart failed");
        return false;
    }
    let final_runtime = match crate::user_runtime::NativeRuntime::prepare_image(
        final_process,
        &image,
        &plan,
        crate::address_space::AslrHook::new(59),
        &arguments,
        &environment,
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!(
                "xHCI final service address-space preparation failed: {:?}",
                error
            ));
            return false;
        }
    };
    unsafe {
        core::ptr::addr_of_mut!(ACTIVE_RISKY_DRIVER).write(Some(RiskyDriverRuntime {
            bus,
            device,
            driver,
            supervisor,
            process: final_runtime,
        }));
        let active = (*core::ptr::addr_of!(ACTIVE_RISKY_DRIVER))
            .as_ref()
            .unwrap();
        crate::bootlog::ok_fmt(format_args!(
            "xHCI service runtime retained bus={} device={} driver={} state={:?} entry=0x{:x}",
            active.bus.id,
            active.device.id,
            active.driver.id,
            active.supervisor.state(handle),
            active.process.entry(),
        ));
    }
    crate::bootlog::ok("xHCI risky driver is running under the service supervisor");
    true
}

#[cfg(target_arch = "x86_64")]
const USER_SERVICE_BASE: usize = 0x4000_0000_0000;
#[cfg(target_arch = "aarch64")]
const USER_SERVICE_BASE: usize = 0x0000_0100_0000_0000;

pub fn user_entry_self_check() -> bool {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        crate::bootlog::quickinit_overlay_begin(crate::boot::info().framebuffer);
        let quickinit_ok = run_init_fixture(
            crate::elf::quickinit_image(),
            "quickinit",
            option_env!("QUICKINIT_FIXTURE") == Some("external"),
        );
        if !quickinit_ok {
            crate::bootlog::quickinit_overlay_crash("quickinit PID 1 hand-off failed");
            crate::bootlog::fail(
                "quickinit PID 1 hand-off failed; deterministic recovery mode remains active",
            );
            return false;
        }
        crate::bootlog::quickinit_overlay_finish(true);
        let fixtures_ok = run_user_fixture(
            crate::elf::representative_image(),
            "rust-smoke",
            option_env!("RUST_FIXTURE") == Some("external"),
        ) && run_user_fixture(
            crate::elf::representative_c_image(),
            "c-runtime",
            option_env!("C_FIXTURE") == Some("external"),
        ) && run_user_fixture(
            crate::elf::representative_cxx_image(),
            "cxx-runtime",
            option_env!("CXX_FIXTURE") == Some("external"),
        );
        let login_ok = if option_env!("LOGIN_SMOKE") == Some("enabled") {
            run_user_fixture(
                crate::elf::login_image(),
                "login-smoke",
                option_env!("LOGIN_FIXTURE") == Some("external"),
            )
        } else {
            true
        };
        let passwd_ok = if option_env!("PASSWD_SMOKE") == Some("enabled") {
            run_user_fixture(
                crate::elf::passwd_image(),
                "passwd-smoke",
                option_env!("PASSWD_FIXTURE") == Some("external"),
            )
        } else {
            true
        };
        let userctl_ok = if option_env!("USERCTL_SMOKE") == Some("enabled") {
            run_user_fixture(
                crate::elf::userctl_image(),
                "userctl-smoke",
                option_env!("USERCTL_FIXTURE") == Some("external"),
            )
        } else {
            true
        };
        let sudo_ok = if option_env!("SUDO_SMOKE") == Some("enabled") {
            run_user_fixture(
                crate::elf::sudo_image(),
                "sudo-smoke",
                option_env!("SUDO_FIXTURE") == Some("external"),
            )
        } else {
            true
        };
        let shell_ok = if option_env!("NSH_SMOKE") == Some("enabled") {
            run_user_fixture(
                crate::elf::nsh_image(),
                "nsh",
                option_env!("NSH_FIXTURE") == Some("external"),
            )
        } else {
            crate::bootlog::warn(
                "nsh interactive smoke deferred; build with RUN_NSH_SMOKE=1 to launch the target shell",
            );
            true
        };
        quickinit_ok && fixtures_ok && shell_ok && login_ok && passwd_ok && userctl_ok && sudo_ok
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn run_init_fixture(image: &[u8], label: &'static str, external: bool) -> bool {
    crate::bootlog::quickinit_overlay_stage("creating PID 1", 12);
    if !external {
        crate::bootlog::warn_fmt(format_args!(
            "{label}: external ELF is required for the PID 1 hand-off"
        ));
        return false;
    }

    let load_bias = crate::elf::load_bias_for_image(image, USER_SERVICE_BASE, 47);
    crate::bootlog::quickinit_overlay_stage("validating ELF", 28);
    let plan = match crate::elf::parse(image, crate::elf::Machine::current(), load_bias) {
        Ok(plan) => plan,
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!("{label}: ELF validation failed: {:?}", error));
            return false;
        }
    };
    let arguments = [label.as_bytes()];
    let environment: [&[u8]; 0] = [];
    let mut runtime = match crate::user_runtime::NativeRuntime::prepare_image(
        ProcessId::INIT,
        image,
        &plan,
        crate::address_space::AslrHook::new(47),
        &arguments,
        &environment,
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!(
                "{label}: user image preparation failed: {:?}",
                error
            ));
            return false;
        }
    };
    let Some(root) = runtime.root_frame() else {
        let _ = runtime.discard();
        crate::bootlog::warn("quickinit: PID 1 address space has no root frame");
        return false;
    };
    if crate::process::attach_address_space(ProcessId::INIT, root).is_err() {
        let _ = runtime.discard();
        crate::bootlog::warn("quickinit: PID 1 address-space attachment failed");
        return false;
    }
    crate::bootlog::quickinit_overlay_stage("preparing PID 1 address space", 48);
    if runtime.start().is_err() {
        let _ = runtime.discard();
        let _ = crate::process::clear_address_space(ProcessId::INIT);
        crate::bootlog::warn("quickinit: PID 1 start failed");
        return false;
    }
    crate::bootlog::quickinit_overlay_stage("entering PID 1", 68);
    if runtime.enter_user().is_err() || !runtime.is_exited() {
        let _ = runtime.discard();
        let _ = crate::process::clear_address_space(ProcessId::INIT);
        crate::bootlog::warn("quickinit: PID 1 returned without a clean exit");
        return false;
    }

    let status = crate::process::init_exit_status().unwrap_or(-1);
    let _ = crate::process::clear_address_space(ProcessId::INIT);
    if crate::process::restore_init_after_user().is_err() {
        crate::bootlog::warn("quickinit: kernel context restore after PID 1 failed");
        return false;
    }
    crate::bootlog::quickinit_overlay_stage("reaping PID 1 children", 88);
    if status != 0 {
        crate::bootlog::warn_fmt(format_args!(
            "quickinit: PID 1 exited with status={status}; recovery required"
        ));
        return false;
    }
    crate::bootlog::ok("quickinit: PID 1 hand-off complete; child reaped");
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnError {
    InvalidPath,
    NotFound,
    Capacity,
    InvalidState,
    Elf,
    Runtime,
}

const SPAWN_INHERIT_CREDENTIALS: u64 = 1 << 2;

fn image_for_user_path<'a>(path: &'a str) -> Result<(&'static [u8], &'a str), SpawnError> {
    let image = match path {
        "/bin/rust-smoke" | "/bin/userspace-smoke" => {
            return Ok((crate::elf::representative_image(), "rust-smoke"));
        }
        "/bin/c-runtime" => return Ok((crate::elf::representative_c_image(), "c-runtime")),
        "/bin/cxx-runtime" => {
            return Ok((crate::elf::representative_cxx_image(), "cxx-runtime"));
        }
        "/bin/nsh" => return Ok((crate::elf::nsh_image(), "nsh")),
        "/bin/cat" | "/bin/echo" | "/bin/env" | "/bin/ls" | "/bin/pwd" | "/bin/mkdir"
        | "/bin/rmdir" | "/bin/cp" | "/bin/mv" | "/bin/rm" | "/bin/touch" | "/bin/ln"
        | "/bin/stat" | "/bin/find" | "/bin/grep" | "/bin/head" | "/bin/tail" | "/bin/sort"
        | "/bin/wc" | "/bin/true" | "/bin/false" | "/bin/sleep" => crate::elf::coreutils_image(),
        "/bin/userdb-smoke" => crate::elf::userdb_image(),
        "/bin/getty-smoke" => crate::elf::getty_image(),
        "/bin/login-smoke" => crate::elf::login_image(),
        "/bin/passwd-smoke" => crate::elf::passwd_image(),
        "/bin/userctl-smoke" => crate::elf::userctl_image(),
        "/bin/sudo-smoke" => crate::elf::sudo_image(),
        _ => return Err(SpawnError::NotFound),
    };
    Ok((image, path.strip_prefix("/bin/").unwrap_or(path)))
}

pub fn spawn_user_path(path: &str) -> Result<u32, SpawnError> {
    let (image, label) = image_for_user_path(path)?;
    if path.as_bytes().contains(&0) {
        return Err(SpawnError::InvalidPath);
    }

    let parent = crate::process::current_process_id().ok_or(SpawnError::InvalidState)?;
    let parent_root = crate::process::address_space_root(parent)
        .map_err(|_| SpawnError::InvalidState)?
        .ok_or(SpawnError::InvalidState)?;
    crate::arch::restore_kernel_address_space();
    let credentials = match label {
        "sudo-smoke" => Credentials {
            real_uid: 1000,
            effective_uid: 1000,
            saved_uid: 1000,
            real_gid: 1000,
            effective_gid: 1000,
            saved_gid: 1000,
            capabilities: (1u64 << (crate::process::Capability::Mount as u8))
                | (1u64 << (crate::process::Capability::PrivilegeDelegation as u8)),
        },
        _ => Credentials {
            capabilities: match label {
                "login-smoke" => 1u64 << (crate::process::Capability::SessionAdmin as u8),
                "userctl-smoke" => 1u64 << (crate::process::Capability::AccountAdmin as u8),
                _ => 0,
            },
            ..Credentials::BOOTSTRAP
        },
    };
    let (child, thread) =
        crate::process::spawn_child_current(credentials).map_err(|error| match error {
            crate::process::Error::ProcessCapacity | crate::process::Error::ThreadCapacity => {
                SpawnError::Capacity
            }
            _ => SpawnError::InvalidState,
        })?;
    let load_bias = crate::elf::load_bias_for_image(image, USER_SERVICE_BASE, 53);
    let plan = match crate::elf::parse(image, crate::elf::Machine::current(), load_bias) {
        Ok(plan) => plan,
        Err(_) => {
            let _ = crate::process::discard_child(parent, child);
            return Err(SpawnError::Elf);
        }
    };
    let arguments = [label.as_bytes()];
    let environment: [&[u8]; 0] = [];
    let mut runtime = match crate::user_runtime::NativeRuntime::prepare_image(
        child,
        image,
        &plan,
        crate::address_space::AslrHook::new(53),
        &arguments,
        &environment,
    ) {
        Ok(runtime) => runtime,
        Err(_) => {
            let _ = crate::process::discard_child(parent, child);
            return Err(SpawnError::Runtime);
        }
    };
    let Some(root) = runtime.root_frame() else {
        let _ = runtime.discard();
        let _ = crate::process::discard_child(parent, child);
        return Err(SpawnError::Runtime);
    };
    if crate::process::attach_address_space(child, root).is_err()
        || crate::process::switch_to_user(child, thread).is_err()
    {
        let _ = runtime.discard();
        let _ = crate::process::discard_child(parent, child);
        return Err(SpawnError::Runtime);
    }
    let start_failed = runtime.start().is_err();
    let enter_failed = !start_failed && runtime.enter_user_quiet().is_err();
    if start_failed || enter_failed || !runtime.is_exited() {
        let _ = crate::process::clear_address_space(child);
        let _ = crate::process::discard_child(parent, child);
        let _ = crate::process::restore_process(parent);
        let _ = crate::arch::switch_to_user(parent_root);
        return Err(SpawnError::Runtime);
    }
    let _ = crate::process::clear_address_space(child);
    if crate::process::restore_process(parent).is_err() || !crate::arch::switch_to_user(parent_root)
    {
        return Err(SpawnError::InvalidState);
    }
    Ok(child.get())
}

pub fn spawn_user_path_resumable(path: &str) -> Result<u32, SpawnError> {
    let (_, label) = image_for_user_path(path)?;
    let arguments = [label.as_bytes()];
    spawn_user_path_resumable_with_args(path, &arguments, &[])
}

pub fn spawn_user_path_resumable_with_args(
    path: &str,
    arguments: &[&[u8]],
    environment: &[&[u8]],
) -> Result<u32, SpawnError> {
    spawn_user_path_resumable_with_args_and_flags(path, arguments, environment, 0)
}

pub fn spawn_user_path_resumable_with_args_and_flags(
    path: &str,
    arguments: &[&[u8]],
    environment: &[&[u8]],
    flags: u64,
) -> Result<u32, SpawnError> {
    spawn_user_path_resumable_with_args_and_credentials(path, arguments, environment, flags, None)
}

pub fn spawn_delegated_user_path_resumable_with_args(
    path: &str,
    arguments: &[&[u8]],
    environment: &[&[u8]],
    flags: u64,
    credentials: Credentials,
) -> Result<u32, SpawnError> {
    spawn_user_path_resumable_with_args_and_credentials(
        path,
        arguments,
        environment,
        flags,
        Some(credentials),
    )
}

fn spawn_user_path_resumable_with_args_and_credentials(
    path: &str,
    arguments: &[&[u8]],
    environment: &[&[u8]],
    flags: u64,
    credentials_override: Option<Credentials>,
) -> Result<u32, SpawnError> {
    let (image, _label) = image_for_user_path(path)?;
    if path.as_bytes().contains(&0) {
        return Err(SpawnError::InvalidPath);
    }
    let parent = crate::process::current_process_id().ok_or(SpawnError::InvalidState)?;
    let credentials = match credentials_override {
        Some(credentials) => credentials,
        None if flags & SPAWN_INHERIT_CREDENTIALS != 0 => {
            crate::process::current_credentials().map_err(|_| SpawnError::InvalidState)?
        }
        None => Credentials {
            capabilities: 0,
            ..Credentials::BOOTSTRAP
        },
    };
    let (child, thread) =
        crate::process::spawn_child_current(credentials).map_err(|error| match error {
            crate::process::Error::ProcessCapacity | crate::process::Error::ThreadCapacity => {
                SpawnError::Capacity
            }
            _ => SpawnError::InvalidState,
        })?;
    let load_bias = crate::elf::load_bias_for_image(image, USER_SERVICE_BASE, 53);
    let plan = match crate::elf::parse(image, crate::elf::Machine::current(), load_bias) {
        Ok(plan) => plan,
        Err(_) => {
            let _ = crate::process::discard_child(parent, child);
            return Err(SpawnError::Elf);
        }
    };
    let mut runtime = match crate::user_runtime::NativeRuntime::prepare_image(
        child,
        image,
        &plan,
        crate::address_space::AslrHook::new(53),
        &arguments,
        &environment,
    ) {
        Ok(runtime) => runtime,
        Err(_) => {
            let _ = crate::process::discard_child(parent, child);
            return Err(SpawnError::Runtime);
        }
    };
    let Some(root) = runtime.root_frame() else {
        let mut runtime = runtime;
        let _ = runtime.discard();
        let _ = crate::process::discard_child(parent, child);
        return Err(SpawnError::Runtime);
    };
    if runtime.activate_for_resumable().is_err() {
        let mut runtime = runtime;
        let _ = runtime.discard();
        let _ = crate::process::discard_child(parent, child);
        return Err(SpawnError::Runtime);
    }
    if crate::process::attach_address_space(child, root).is_err()
        || crate::user_runtime::install(child, runtime).is_err()
    {
        let _ = crate::user_runtime::discard(child);
        let _ = crate::process::clear_address_space(child);
        let _ = crate::process::discard_child(parent, child);
        return Err(SpawnError::Runtime);
    }
    let registers = match crate::user_runtime::start(child) {
        Ok(registers) => registers,
        Err(_) => {
            let _ = crate::user_runtime::discard(child);
            let _ = crate::process::clear_address_space(child);
            let _ = crate::process::discard_child(parent, child);
            return Err(SpawnError::Runtime);
        }
    };
    if crate::process::install_user_context(thread.get(), registers).is_err() {
        let _ = crate::user_runtime::discard(child);
        let _ = crate::process::clear_address_space(child);
        let _ = crate::process::discard_child(parent, child);
        return Err(SpawnError::Runtime);
    }
    Ok(child.get())
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn run_user_fixture(image: &[u8], label: &'static str, external: bool) -> bool {
    let quickinit = label == "quickinit-bootstrap";
    if quickinit {
        crate::bootlog::quickinit_overlay_stage("creating process", 12);
    }
    let credentials = match label {
        "sudo-smoke" => Credentials {
            real_uid: 1000,
            effective_uid: 1000,
            saved_uid: 1000,
            real_gid: 1000,
            effective_gid: 1000,
            saved_gid: 1000,
            capabilities: (1 << (crate::process::Capability::Mount as u8))
                | (1 << (crate::process::Capability::PrivilegeDelegation as u8)),
        },
        _ => Credentials {
            capabilities: match label {
                "login-smoke" => 1 << (crate::process::Capability::SessionAdmin as u8),
                "userctl-smoke" => 1 << (crate::process::Capability::AccountAdmin as u8),
                _ => 0,
            },
            ..Credentials::BOOTSTRAP
        },
    };
    let (process, thread) = match crate::process::spawn_child_current(credentials) {
        Ok(ids) => ids,
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!(
                "{label}: process creation failed: {:?}",
                error
            ));
            return false;
        }
    };
    let load_bias = crate::elf::load_bias_for_image(image, USER_SERVICE_BASE, 47);
    if quickinit {
        crate::bootlog::quickinit_overlay_stage("validating ELF", 28);
    }
    let plan = match crate::elf::parse(image, crate::elf::Machine::current(), load_bias) {
        Ok(plan) => plan,
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!("{label}: ELF validation failed: {:?}", error));
            return false;
        }
    };
    let arguments = [label.as_bytes()];
    let environment: [&[u8]; 0] = [];
    let mut runtime = match crate::user_runtime::NativeRuntime::prepare_image(
        process,
        image,
        &plan,
        crate::address_space::AslrHook::new(47),
        &arguments,
        &environment,
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            crate::bootlog::warn_fmt(format_args!(
                "{label}: user image preparation failed: {:?}",
                error
            ));
            return false;
        }
    };
    if let Some(root) = runtime.root_frame() {
        crate::process::attach_address_space(process, root).unwrap();
    } else {
        return false;
    }
    if quickinit {
        crate::bootlog::quickinit_overlay_stage("preparing address space", 48);
    }
    let resumable = external;
    if resumable {
        let registers = runtime.start().unwrap();
        crate::user_runtime::install(process, runtime).unwrap();
        crate::process::install_user_context(thread.get(), registers).unwrap();
        crate::process::switch_to_user(process, thread).unwrap();
        if quickinit {
            crate::bootlog::quickinit_overlay_stage("entering userspace", 68);
        }
        crate::user_runtime::enter_resumable(process, true).unwrap();
        let mut runtime = crate::user_runtime::take(process).unwrap();
        let _ = runtime.discard();
    } else {
        crate::process::switch_to_user(process, thread).unwrap();
        if quickinit {
            crate::bootlog::quickinit_overlay_stage("entering userspace", 68);
        }
        let registers = runtime.start().unwrap();
        if matches!(label, "login-smoke" | "sudo-smoke") {
            crate::process::install_user_context(thread.get(), registers).unwrap();
        }
        runtime.enter_user().unwrap();
        assert!(runtime.is_exited());
    }
    assert_eq!(crate::process::current_ids(), None);
    crate::process::clear_address_space(process).unwrap();
    crate::process::restore_init().unwrap();
    if quickinit {
        crate::bootlog::quickinit_overlay_stage("reaping child", 88);
    }
    let expected_status = if external { 0 } else { 42 };
    let result = crate::process::wait_current(Some(process.get())).unwrap();
    if result != (process.get(), expected_status) {
        crate::bootlog::warn_fmt(format_args!(
            "{label}: unexpected exit status {}, expected {}",
            result.1, expected_status
        ));
        return false;
    }
    if external {
        crate::bootlog::ok_fmt(format_args!(
            "{label}: external userspace ELF exited cleanly"
        ));
    } else {
        crate::bootlog::warn_fmt(format_args!(
            "{label}: external ELF unavailable; bounded fallback image executed"
        ));
    }
    true
}
