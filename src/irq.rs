use core::sync::atomic::{AtomicU64, Ordering};

use crate::drivers::framework::IrqKind;

const MAX_HANDLERS: usize = 64;

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub type HardHandler = fn() -> bool;
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub type DeferredHandler = fn();
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub type RegistrationId = usize;

#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub enum IrqError {
    Capacity,
    Duplicate,
    InvalidRegistration,
    NotRegistered,
}

#[derive(Clone, Copy)]
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
struct Registration {
    kind: IrqKind,
    line: u32,
    vector: u8,
    hard: HardHandler,
    deferred: Option<DeferredHandler>,
}

static TIMER: AtomicU64 = AtomicU64::new(0);
static SPURIOUS: AtomicU64 = AtomicU64::new(0);
static EXCEPTIONS: AtomicU64 = AtomicU64::new(0);
static UNHANDLED: AtomicU64 = AtomicU64::new(0);
static DEFERRED: AtomicU64 = AtomicU64::new(0);
static PENDING: AtomicU64 = AtomicU64::new(0);
static mut HANDLERS: [Option<Registration>; MAX_HANDLERS] = [None; MAX_HANDLERS];

#[derive(Clone, Copy)]
pub struct Stats {
    pub timer: u64,
    pub spurious: u64,
    pub exceptions: u64,
    pub unhandled: u64,
    pub deferred: u64,
}

pub fn init() {
    PENDING.store(0, Ordering::Release);
    TIMER.store(0, Ordering::Relaxed);
    SPURIOUS.store(0, Ordering::Relaxed);
    EXCEPTIONS.store(0, Ordering::Relaxed);
    UNHANDLED.store(0, Ordering::Relaxed);
    DEFERRED.store(0, Ordering::Relaxed);
    unsafe {
        HANDLERS = [None; MAX_HANDLERS];
    }
}

#[cfg(target_arch = "x86_64")]
fn contract_hard() -> bool {
    true
}

#[cfg(target_arch = "x86_64")]
fn contract_deferred() {}

pub fn contract_self_check() {
    #[cfg(target_arch = "x86_64")]
    {
        assert!(matches!(
            unregister(usize::MAX),
            Err(IrqError::InvalidRegistration)
        ));
        let registration = register(IrqKind::Msi, 1, 200, contract_hard, Some(contract_deferred));
        assert!(registration.is_ok());
        let id = match registration {
            Ok(id) => id,
            Err(_) => return,
        };
        assert!(dispatch(200));
        assert_eq!(run_deferred(), 1);
        assert!(unregister(id).is_ok());
        assert!(matches!(unregister(id), Err(IrqError::NotRegistered)));
    }
    assert!(interrupt_storm_self_check());
}

fn storm_hard() -> bool {
    true
}

fn storm_deferred() {}

pub fn interrupt_storm_self_check() -> bool {
    let Ok(id) = register(IrqKind::Gic, 0xfeed, 201, storm_hard, Some(storm_deferred)) else {
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
pub fn register(
    kind: IrqKind,
    line: u32,
    vector: u8,
    hard: HardHandler,
    deferred: Option<DeferredHandler>,
) -> Result<RegistrationId, IrqError> {
    unsafe {
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
        (*handlers)[slot] = Some(Registration {
            kind,
            line,
            vector,
            hard,
            deferred,
        });
        Ok(slot)
    }
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn unregister(id: RegistrationId) -> Result<(), IrqError> {
    if id >= MAX_HANDLERS {
        return Err(IrqError::InvalidRegistration);
    }
    PENDING.fetch_and(!(1u64 << id), Ordering::AcqRel);
    unsafe {
        let handlers = core::ptr::addr_of_mut!(HANDLERS);
        if (*handlers)[id].take().is_none() {
            return Err(IrqError::NotRegistered);
        }
    }
    Ok(())
}

/// Dispatches one vector. The hard handler must be bounded and non-blocking;
/// deferred work is only marked here and runs through `run_deferred` later.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn dispatch(vector: u8) -> bool {
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
        spurious: SPURIOUS.load(Ordering::Relaxed),
        exceptions: EXCEPTIONS.load(Ordering::Relaxed),
        unhandled: UNHANDLED.load(Ordering::Relaxed),
        deferred: DEFERRED.load(Ordering::Relaxed),
    }
}
