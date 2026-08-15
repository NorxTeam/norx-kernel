use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::drivers::framework::{DeviceId, IrqKind};

const MAX_HANDLERS: usize = 64;

/// A bounded, non-blocking handler that only acknowledges device state and
/// publishes bounded work for the normal-context pass.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub type HardHandler = fn() -> bool;
/// A bounded callback that runs outside hard-interrupt context.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub type DeferredHandler = fn();
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegistrationId {
    slot: u8,
    generation: u32,
    owner: IrqOwner,
}

impl RegistrationId {
    #[allow(dead_code)]
    pub fn slot(self) -> usize {
        self.slot as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrqOwner {
    Kernel,
    Device(DeviceId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub enum IrqError {
    Capacity,
    Duplicate,
    InvalidRegistration,
    OwnerMismatch,
    HardContext,
    DeferredPending,
    StaleRegistration,
}

#[derive(Clone, Copy)]
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
struct Registration {
    owner: IrqOwner,
    generation: u32,
    kind: IrqKind,
    line: u32,
    vector: u32,
    hard: HardHandler,
    deferred: Option<DeferredHandler>,
}

static TIMER: AtomicU64 = AtomicU64::new(0);
static TIMER_PENDING: AtomicU64 = AtomicU64::new(0);
static TIMER_REGISTRATION: AtomicU64 = AtomicU64::new(u64::MAX);
static SPURIOUS: AtomicU64 = AtomicU64::new(0);
static EXCEPTIONS: AtomicU64 = AtomicU64::new(0);
static UNHANDLED: AtomicU64 = AtomicU64::new(0);
static DEFERRED: AtomicU64 = AtomicU64::new(0);
static HARD_CONTEXT_VIOLATIONS: AtomicU64 = AtomicU64::new(0);
static IN_HARD_CONTEXT: AtomicBool = AtomicBool::new(false);
static IN_DEFERRED: AtomicBool = AtomicBool::new(false);
static PENDING: AtomicU64 = AtomicU64::new(0);
static mut GENERATIONS: [u32; MAX_HANDLERS] = [0; MAX_HANDLERS];
static mut HANDLERS: [Option<Registration>; MAX_HANDLERS] = [None; MAX_HANDLERS];

#[derive(Clone, Copy)]
pub struct Stats {
    pub timer: u64,
    pub timer_pending: u64,
    pub spurious: u64,
    pub exceptions: u64,
    pub unhandled: u64,
    pub deferred: u64,
    pub hard_context_violations: u64,
}

pub fn init() {
    PENDING.store(0, Ordering::Release);
    TIMER.store(0, Ordering::Relaxed);
    TIMER_PENDING.store(0, Ordering::Relaxed);
    TIMER_REGISTRATION.store(u64::MAX, Ordering::Relaxed);
    SPURIOUS.store(0, Ordering::Relaxed);
    EXCEPTIONS.store(0, Ordering::Relaxed);
    UNHANDLED.store(0, Ordering::Relaxed);
    DEFERRED.store(0, Ordering::Relaxed);
    HARD_CONTEXT_VIOLATIONS.store(0, Ordering::Relaxed);
    IN_HARD_CONTEXT.store(false, Ordering::Relaxed);
    IN_DEFERRED.store(false, Ordering::Relaxed);
    unsafe {
        GENERATIONS = [0; MAX_HANDLERS];
        HANDLERS = [None; MAX_HANDLERS];
    }
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn contract_hard() -> bool {
    assert!(in_hard_context());
    true
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn contract_deferred() {
    assert!(!in_hard_context());
}

pub fn contract_self_check() {
    assert!(matches!(
        unregister(RegistrationId {
            slot: u8::MAX,
            generation: 0,
            owner: IrqOwner::Kernel,
        }),
        Err(IrqError::InvalidRegistration)
    ));
    let registration = register_owned(
        0x7fff,
        IrqKind::Msi,
        1,
        200,
        contract_hard,
        Some(contract_deferred),
    );
    assert!(registration.is_ok());
    let id = match registration {
        Ok(id) => id,
        Err(_) => return,
    };
    assert_eq!(registration_owner(id), Some(IrqOwner::Device(0x7fff)));
    assert!(matches!(
        unregister_owned(id, IrqOwner::Kernel),
        Err(IrqError::OwnerMismatch)
    ));
    assert!(dispatch(200));
    assert!(matches!(unregister(id), Err(IrqError::DeferredPending)));
    assert_eq!(run_deferred(), 1);
    assert!(unregister_owned(id, IrqOwner::Device(0x7fff)).is_ok());
    assert!(matches!(unregister(id), Err(IrqError::StaleRegistration)));
    let replacement = register_system(IrqKind::Msi, 1, 200, contract_hard, Some(contract_deferred))
        .expect("slot should be reusable");
    assert!(matches!(unregister(id), Err(IrqError::StaleRegistration)));
    assert!(unregister(replacement).is_ok());
    timer_contract_self_check();
    assert!(interrupt_storm_self_check());
}

fn storm_hard() -> bool {
    assert!(in_hard_context());
    true
}

fn storm_deferred() {
    assert!(!in_hard_context());
}

pub fn interrupt_storm_self_check() -> bool {
    let Ok(id) = register_owned(
        0x7ffe,
        IrqKind::Gic,
        0xfeed,
        201,
        storm_hard,
        Some(storm_deferred),
    ) else {
        return false;
    };
    let mut accepted = 0;
    for _ in 0..4096 {
        if dispatch(201) {
            accepted += 1;
        }
    }
    let deferred = run_deferred();
    let unregistered = unregister(id).is_ok();
    accepted == 4096 && deferred == 1 && unregistered
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn register_with_owner(
    owner: IrqOwner,
    kind: IrqKind,
    line: u32,
    vector: u32,
    hard: HardHandler,
    deferred: Option<DeferredHandler>,
) -> Result<RegistrationId, IrqError> {
    if in_hard_context() {
        HARD_CONTEXT_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        return Err(IrqError::HardContext);
    }
    crate::arch::without_interrupts(|| unsafe {
        let handlers = core::ptr::addr_of!(HANDLERS);
        let mut index = 0;
        while index < MAX_HANDLERS {
            if let Some(registration) = (*handlers)[index] {
                if (registration.kind == kind && registration.line == line)
                    || registration.vector == vector
                {
                    return Err(IrqError::Duplicate);
                }
            }
            index += 1;
        }
        let handlers = core::ptr::addr_of_mut!(HANDLERS);
        let mut slot = 0;
        while slot < MAX_HANDLERS && (*handlers)[slot].is_some() {
            slot += 1;
        }
        if slot == MAX_HANDLERS {
            return Err(IrqError::Capacity);
        }
        let generations = core::ptr::addr_of_mut!(GENERATIONS);
        let generation = (*generations)[slot].wrapping_add(1).max(1);
        (*generations)[slot] = generation;
        (*handlers)[slot] = Some(Registration {
            owner,
            generation,
            kind,
            line,
            vector,
            hard,
            deferred,
        });
        Ok(RegistrationId {
            slot: slot as u8,
            generation,
            owner,
        })
    })
}

pub fn register_system(
    kind: IrqKind,
    line: u32,
    vector: u32,
    hard: HardHandler,
    deferred: Option<DeferredHandler>,
) -> Result<RegistrationId, IrqError> {
    register_with_owner(IrqOwner::Kernel, kind, line, vector, hard, deferred)
}

pub fn register_owned(
    owner: DeviceId,
    kind: IrqKind,
    line: u32,
    vector: u32,
    hard: HardHandler,
    deferred: Option<DeferredHandler>,
) -> Result<RegistrationId, IrqError> {
    register_with_owner(IrqOwner::Device(owner), kind, line, vector, hard, deferred)
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn unregister(id: RegistrationId) -> Result<(), IrqError> {
    unregister_owned(id, id.owner)
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn unregister_owned(id: RegistrationId, owner: IrqOwner) -> Result<(), IrqError> {
    if id.owner != owner {
        return Err(IrqError::OwnerMismatch);
    }
    let slot = id.slot as usize;
    if slot >= MAX_HANDLERS {
        return Err(IrqError::InvalidRegistration);
    }
    if in_hard_context() {
        HARD_CONTEXT_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        return Err(IrqError::HardContext);
    }
    crate::arch::without_interrupts(|| unsafe {
        let handlers = core::ptr::addr_of_mut!(HANDLERS);
        let Some(registration) = (*handlers)[slot] else {
            return Err(IrqError::StaleRegistration);
        };
        if registration.owner != owner || registration.generation != id.generation {
            return Err(IrqError::StaleRegistration);
        }
        if PENDING.load(Ordering::Acquire) & (1u64 << slot) != 0 {
            return Err(IrqError::DeferredPending);
        }
        (*handlers)[slot] = None;
        Ok(())
    })
}

pub fn registration_owner(id: RegistrationId) -> Option<IrqOwner> {
    let slot = id.slot as usize;
    if slot >= MAX_HANDLERS {
        return None;
    }
    unsafe {
        let handlers = core::ptr::addr_of!(HANDLERS);
        (*handlers)[slot].and_then(|registration| {
            (registration.owner == id.owner && registration.generation == id.generation)
                .then_some(registration.owner)
        })
    }
}

pub fn registration_pending(id: RegistrationId) -> bool {
    registration_owner(id).is_some() && PENDING.load(Ordering::Acquire) & (1u64 << id.slot) != 0
}

/// Dispatches one vector. The hard handler must be bounded and non-blocking;
/// deferred work is only marked here and runs through `run_deferred` later.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn dispatch(vector: u32) -> bool {
    if IN_HARD_CONTEXT.swap(true, Ordering::AcqRel) {
        HARD_CONTEXT_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    let result = dispatch_inner(vector);
    IN_HARD_CONTEXT.store(false, Ordering::Release);
    result
}

fn dispatch_inner(vector: u32) -> bool {
    let registration = unsafe {
        let handlers = core::ptr::addr_of!(HANDLERS);
        let mut index = 0;
        let mut found = None;
        while index < MAX_HANDLERS {
            if let Some(registration) = (*handlers)[index] {
                if registration.vector == vector {
                    found = Some((index, registration));
                    break;
                }
            }
            index += 1;
        }
        found
    };
    let Some((slot, registration)) = registration else {
        SPURIOUS.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    if !(registration.hard)() {
        UNHANDLED.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    if registration.deferred.is_some() {
        PENDING.fetch_or(1u64 << slot, Ordering::Release);
    }
    true
}

/// Runs one bounded pass over pending callbacks in normal context.
pub fn run_deferred() -> usize {
    if in_hard_context() {
        HARD_CONTEXT_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        return 0;
    }
    if IN_DEFERRED.swap(true, Ordering::AcqRel) {
        HARD_CONTEXT_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        return 0;
    }
    let result = run_deferred_inner();
    IN_DEFERRED.store(false, Ordering::Release);
    result
}

fn run_deferred_inner() -> usize {
    let pending = PENDING.swap(0, Ordering::AcqRel);
    let mut ran = 0;
    let handlers = core::ptr::addr_of!(HANDLERS);
    let mut slot = 0;
    while slot < MAX_HANDLERS {
        if pending & (1u64 << slot) == 0 {
            slot += 1;
            continue;
        }
        let deferred = unsafe { (*handlers)[slot].and_then(|entry| entry.deferred) };
        if let Some(deferred) = deferred {
            deferred();
            ran += 1;
            DEFERRED.fetch_add(1, Ordering::Relaxed);
        }
        slot += 1;
    }
    ran
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn timer() {
    TIMER.fetch_add(1, Ordering::Relaxed);
    TIMER_PENDING.fetch_add(1, Ordering::Release);
}

pub fn register_timer(id: RegistrationId) {
    let encoded = (id.generation as u64) << 8 | id.slot as u64;
    TIMER_REGISTRATION.store(encoded, Ordering::Release);
}

pub fn timer_pending() -> bool {
    TIMER_PENDING.load(Ordering::Acquire) != 0
}

pub fn requeue_timer() {
    let registration = TIMER_REGISTRATION.load(Ordering::Acquire);
    if registration != u64::MAX {
        let slot = registration & 0xff;
        if slot < 64 {
            PENDING.fetch_or(1u64 << slot, Ordering::Release);
        }
    }
}

pub fn take_timer_ticks(limit: u64) -> u64 {
    if limit == 0 {
        return 0;
    }
    loop {
        let pending = TIMER_PENDING.load(Ordering::Acquire);
        if pending == 0 {
            return 0;
        }
        let taken = pending.min(limit);
        if TIMER_PENDING
            .compare_exchange(
                pending,
                pending - taken,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            return taken;
        }
    }
}

pub fn timer_contract_self_check() {
    for _ in 0..5 {
        timer();
    }
    assert_eq!(take_timer_ticks(2), 2);
    assert_eq!(take_timer_ticks(8), 3);
    assert!(!timer_pending());
    TIMER.store(0, Ordering::Relaxed);
}

pub fn in_hard_context() -> bool {
    IN_HARD_CONTEXT.load(Ordering::Acquire)
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn spurious() {
    SPURIOUS.fetch_add(1, Ordering::Relaxed);
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn exception() {
    EXCEPTIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn stats() -> Stats {
    Stats {
        timer: TIMER.load(Ordering::Relaxed),
        timer_pending: TIMER_PENDING.load(Ordering::Acquire),
        spurious: SPURIOUS.load(Ordering::Relaxed),
        exceptions: EXCEPTIONS.load(Ordering::Relaxed),
        unhandled: UNHANDLED.load(Ordering::Relaxed),
        deferred: DEFERRED.load(Ordering::Relaxed),
        hard_context_violations: HARD_CONTEXT_VIOLATIONS.load(Ordering::Relaxed),
    }
}
