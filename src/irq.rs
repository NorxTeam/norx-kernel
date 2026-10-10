use core::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use crate::drivers::framework::{DeviceId, IrqKind};

const MAX_HANDLERS: usize = 64;
const MAX_CPUS: usize = 64;
const SLOT_FREE: u8 = 0;
const SLOT_ACTIVE: u8 = 1;
const SLOT_QUIESCING: u8 = 2;
const QUIESCE_SPIN_LIMIT: usize = 1_000_000;

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
    CallbackInFlight,
    GenerationExhausted,
    StaleRegistration,
}

struct IrqSlot {
    state: AtomicU8,
    generation: AtomicU32,
    owner: AtomicU32,
    kind: AtomicU8,
    line: AtomicU32,
    vector: AtomicU32,
    hard: AtomicUsize,
    deferred: AtomicUsize,
    readers: AtomicUsize,
}

impl IrqSlot {
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(SLOT_FREE),
            generation: AtomicU32::new(0),
            owner: AtomicU32::new(0),
            kind: AtomicU8::new(0),
            line: AtomicU32::new(0),
            vector: AtomicU32::new(0),
            hard: AtomicUsize::new(0),
            deferred: AtomicUsize::new(0),
            readers: AtomicUsize::new(0),
        }
    }
}

static TIMER: AtomicU64 = AtomicU64::new(0);
static TIMER_PENDING: AtomicU64 = AtomicU64::new(0);
static TIMER_REGISTRATION: AtomicU64 = AtomicU64::new(u64::MAX);
static SPURIOUS: AtomicU64 = AtomicU64::new(0);
static EXCEPTIONS: AtomicU64 = AtomicU64::new(0);
static UNHANDLED: AtomicU64 = AtomicU64::new(0);
static DEFERRED: AtomicU64 = AtomicU64::new(0);
static HARD_CONTEXT_VIOLATIONS: AtomicU64 = AtomicU64::new(0);
static IN_DEFERRED: AtomicU8 = AtomicU8::new(0);
static PENDING: AtomicU64 = AtomicU64::new(0);
static REGISTRY_LOCK: crate::sync::SmpLock<()> = crate::sync::SmpLock::new(());
static SLOTS: [IrqSlot; MAX_HANDLERS] = [const { IrqSlot::new() }; MAX_HANDLERS];
static HARD_DEPTH: [AtomicU8; MAX_CPUS] = [const { AtomicU8::new(0) }; MAX_CPUS];

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
    IN_DEFERRED.store(0, Ordering::Relaxed);
    for depth in &HARD_DEPTH {
        depth.store(0, Ordering::Relaxed);
    }
    REGISTRY_LOCK.with(|_| {
        for slot in &SLOTS {
            assert_eq!(slot.readers.load(Ordering::Acquire), 0);
            slot.state.store(SLOT_FREE, Ordering::Release);
            slot.hard.store(0, Ordering::Relaxed);
            slot.deferred.store(0, Ordering::Relaxed);
            let generation = slot.generation.load(Ordering::Relaxed);
            if generation != u32::MAX {
                slot.generation
                    .store(generation.saturating_add(1), Ordering::Relaxed);
            }
        }
    });
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
    assert!(quiesce(id).is_ok());
    assert_eq!(run_deferred(), 1);
    assert!(unregister_owned(id, IrqOwner::Device(0x7fff)).is_ok());
    assert!(matches!(unregister(id), Err(IrqError::StaleRegistration)));
    let replacement = register_system(IrqKind::Msi, 1, 200, contract_hard, Some(contract_deferred))
        .expect("slot should be reusable");
    assert!(matches!(unregister(id), Err(IrqError::StaleRegistration)));
    assert!(unregister(replacement).is_ok());
    timer_registration_self_check();
    hard_context_mutation_self_check();
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

fn hard_rejects_registration() -> bool {
    matches!(
        register_system(IrqKind::Msi, 0xface, 204, contract_hard, None),
        Err(IrqError::HardContext)
    )
}

fn hard_context_mutation_self_check() {
    let violations = HARD_CONTEXT_VIOLATIONS.load(Ordering::Acquire);
    let id = register_system(IrqKind::Msi, 0xfacd, 203, hard_rejects_registration, None)
        .expect("hard-context contract registration");
    assert!(dispatch(203));
    assert_eq!(
        HARD_CONTEXT_VIOLATIONS.load(Ordering::Acquire),
        violations + 1
    );
    // Remove only the deliberately injected violation from production telemetry.
    HARD_CONTEXT_VIOLATIONS.fetch_sub(1, Ordering::AcqRel);
    assert!(unregister(id).is_ok());
}

fn timer_registration_self_check() {
    let timer_id = register_system(
        IrqKind::Msi,
        0xfa11,
        202,
        contract_hard,
        Some(contract_deferred),
    )
    .expect("timer generation contract registration");
    assert!(register_timer(timer_id));
    assert!(unregister(timer_id).is_ok());

    let replacement = register_system(
        IrqKind::Msi,
        0xfa11,
        202,
        contract_hard,
        Some(contract_deferred),
    )
    .expect("timer replacement registration");
    timer();
    requeue_timer();
    assert!(!registration_pending(replacement));
    assert!(unregister(replacement).is_ok());
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
    REGISTRY_LOCK.with(|_| {
        for slot in &SLOTS {
            if slot.state.load(Ordering::Acquire) != SLOT_FREE
                && ((decode_kind(slot.kind.load(Ordering::Acquire)) == Some(kind)
                    && slot.line.load(Ordering::Acquire) == line)
                    || slot.vector.load(Ordering::Acquire) == vector)
            {
                return Err(IrqError::Duplicate);
            }
        }
        let Some((index, slot)) = SLOTS
            .iter()
            .enumerate()
            .find(|(_, slot)| slot.state.load(Ordering::Acquire) == SLOT_FREE)
        else {
            return Err(IrqError::Capacity);
        };
        let previous = slot.generation.load(Ordering::Acquire);
        let generation = previous
            .checked_add(1)
            .filter(|generation| *generation != 0)
            .ok_or(IrqError::GenerationExhausted)?;
        slot.generation.store(generation, Ordering::Relaxed);
        slot.owner.store(owner_code(owner), Ordering::Relaxed);
        slot.kind.store(kind_code(kind), Ordering::Relaxed);
        slot.line.store(line, Ordering::Relaxed);
        slot.vector.store(vector, Ordering::Relaxed);
        slot.hard.store(hard as usize, Ordering::Relaxed);
        slot.deferred.store(
            deferred.map_or(0, |callback| callback as usize),
            Ordering::Relaxed,
        );
        slot.state.store(SLOT_ACTIVE, Ordering::Release);
        Ok(RegistrationId {
            slot: index as u8,
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
    REGISTRY_LOCK.with(|_| {
        let entry = &SLOTS[slot];
        if !matches_registration(entry, id, false) {
            return Err(IrqError::StaleRegistration);
        }
        if entry.state.load(Ordering::Acquire) == SLOT_ACTIVE {
            entry.state.store(SLOT_QUIESCING, Ordering::Release);
        }
        if !wait_for_readers(entry) {
            return Err(IrqError::CallbackInFlight);
        }
        if PENDING.load(Ordering::Acquire) & (1u64 << slot) != 0 {
            return Err(IrqError::DeferredPending);
        }
        let encoded = encode_registration(id);
        let _ = TIMER_REGISTRATION.compare_exchange(
            encoded,
            u64::MAX,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        entry.hard.store(0, Ordering::Release);
        entry.deferred.store(0, Ordering::Release);
        entry.state.store(SLOT_FREE, Ordering::Release);
        Ok(())
    })
}

/// Stop new hard callbacks for a registration while leaving one pending
/// deferred callback drainable. The owner must drain it before unregistering.
pub fn quiesce(id: RegistrationId) -> Result<(), IrqError> {
    if in_hard_context() {
        HARD_CONTEXT_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        return Err(IrqError::HardContext);
    }
    let slot = id.slot as usize;
    if slot >= MAX_HANDLERS {
        return Err(IrqError::InvalidRegistration);
    }
    REGISTRY_LOCK.with(|_| {
        let entry = &SLOTS[slot];
        if !matches_registration(entry, id, false) {
            return Err(IrqError::StaleRegistration);
        }
        entry.state.store(SLOT_QUIESCING, Ordering::Release);
        if wait_for_readers(entry) {
            Ok(())
        } else {
            Err(IrqError::CallbackInFlight)
        }
    })
}

pub fn registration_owner(id: RegistrationId) -> Option<IrqOwner> {
    let slot = id.slot as usize;
    if slot >= MAX_HANDLERS {
        return None;
    }
    let entry = &SLOTS[slot];
    matches_registration(entry, id, false).then_some(id.owner)
}

pub fn registration_pending(id: RegistrationId) -> bool {
    registration_owner(id).is_some() && PENDING.load(Ordering::Acquire) & (1u64 << id.slot) != 0
}

/// Dispatches one vector. The hard handler must be bounded and non-blocking;
/// deferred work is only marked here and runs through `run_deferred` later.
#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn dispatch(vector: u32) -> bool {
    let Some(cpu) = current_cpu_slot() else {
        HARD_CONTEXT_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    if HARD_DEPTH[cpu]
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        HARD_CONTEXT_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    let result = dispatch_inner(vector);
    HARD_DEPTH[cpu].store(0, Ordering::Release);
    result
}

fn dispatch_inner(vector: u32) -> bool {
    let Some((slot, hard, deferred)) = acquire_dispatch(vector) else {
        SPURIOUS.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    if !(hard)() {
        release_reader(&SLOTS[slot]);
        UNHANDLED.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    if deferred.is_some() {
        PENDING.fetch_or(1u64 << slot, Ordering::Release);
    }
    release_reader(&SLOTS[slot]);
    true
}

/// Runs one bounded pass over pending callbacks in normal context.
pub fn run_deferred() -> usize {
    if in_hard_context() {
        HARD_CONTEXT_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        return 0;
    }
    if IN_DEFERRED
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        HARD_CONTEXT_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        return 0;
    }
    let result = run_deferred_inner();
    IN_DEFERRED.store(0, Ordering::Release);
    result
}

fn run_deferred_inner() -> usize {
    let mut ran = 0;
    let mut slot = 0;
    while slot < MAX_HANDLERS {
        let entry = &SLOTS[slot];
        entry.readers.fetch_add(1, Ordering::Acquire);
        if PENDING.fetch_and(!(1u64 << slot), Ordering::AcqRel) & (1u64 << slot) == 0 {
            release_reader(entry);
            slot += 1;
            continue;
        }
        let deferred = if entry.state.load(Ordering::Acquire) != SLOT_FREE {
            load_deferred(entry)
        } else {
            None
        };
        if let Some(deferred) = deferred {
            deferred();
            ran += 1;
            DEFERRED.fetch_add(1, Ordering::Relaxed);
        }
        release_reader(entry);
        slot += 1;
    }
    ran
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn timer() {
    TIMER.fetch_add(1, Ordering::Relaxed);
    TIMER_PENDING.fetch_add(1, Ordering::Release);
}

pub fn register_timer(id: RegistrationId) -> bool {
    if id.owner != IrqOwner::Kernel || id.slot as usize >= MAX_HANDLERS {
        return false;
    }
    let entry = &SLOTS[id.slot as usize];
    entry.readers.fetch_add(1, Ordering::Acquire);
    let valid = matches_registration(entry, id, true);
    let result = if valid {
        let encoded = encode_registration(id);
        let current = TIMER_REGISTRATION.load(Ordering::Acquire);
        if current == encoded {
            true
        } else if current == u64::MAX {
            TIMER_REGISTRATION
                .compare_exchange(u64::MAX, encoded, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        } else {
            false
        }
    } else {
        false
    };
    release_reader(entry);
    result
}

pub fn timer_pending() -> bool {
    TIMER_PENDING.load(Ordering::Acquire) != 0
}

pub fn requeue_timer() {
    let registration = TIMER_REGISTRATION.load(Ordering::Acquire);
    if registration != u64::MAX {
        let slot = registration & 0xff;
        let generation = registration >> 8;
        if slot < MAX_HANDLERS as u64 {
            let entry = &SLOTS[slot as usize];
            entry.readers.fetch_add(1, Ordering::Acquire);
            if entry.state.load(Ordering::Acquire) == SLOT_ACTIVE
                && entry.generation.load(Ordering::Acquire) == generation as u32
                && entry.owner.load(Ordering::Acquire) == owner_code(IrqOwner::Kernel)
            {
                PENDING.fetch_or(1u64 << slot, Ordering::Release);
            }
            release_reader(entry);
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
    crate::arch::without_interrupts(|| {
        let pending = TIMER_PENDING.swap(0, Ordering::AcqRel);
        let count = TIMER.load(Ordering::Acquire);
        for _ in 0..5 {
            timer();
        }
        assert_eq!(take_timer_ticks(2), 2);
        assert_eq!(take_timer_ticks(8), 3);
        assert!(!timer_pending());
        TIMER_PENDING.store(pending, Ordering::Release);
        TIMER.store(count, Ordering::Release);
    });
}

pub fn in_hard_context() -> bool {
    current_cpu_slot().is_some_and(|cpu| HARD_DEPTH[cpu].load(Ordering::Acquire) != 0)
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

fn current_cpu_slot() -> Option<usize> {
    #[cfg(test)]
    {
        Some(0)
    }
    #[cfg(not(test))]
    {
        let cpu = crate::arch::cpu_id();
        (cpu < MAX_CPUS).then_some(cpu)
    }
}

fn owner_code(owner: IrqOwner) -> u32 {
    match owner {
        IrqOwner::Kernel => 0,
        IrqOwner::Device(device) => device as u32 + 1,
    }
}

fn kind_code(kind: IrqKind) -> u8 {
    match kind {
        IrqKind::Legacy => 1,
        IrqKind::Msi => 2,
        IrqKind::MsiX => 3,
        IrqKind::Gic => 4,
        IrqKind::Local => 5,
    }
}

fn decode_kind(value: u8) -> Option<IrqKind> {
    match value {
        1 => Some(IrqKind::Legacy),
        2 => Some(IrqKind::Msi),
        3 => Some(IrqKind::MsiX),
        4 => Some(IrqKind::Gic),
        5 => Some(IrqKind::Local),
        _ => None,
    }
}

fn encode_registration(id: RegistrationId) -> u64 {
    (id.generation as u64) << 8 | id.slot as u64
}

fn matches_registration(entry: &IrqSlot, id: RegistrationId, active: bool) -> bool {
    let state = entry.state.load(Ordering::Acquire);
    (!active || state == SLOT_ACTIVE)
        && state != SLOT_FREE
        && entry.generation.load(Ordering::Acquire) == id.generation
        && entry.owner.load(Ordering::Acquire) == owner_code(id.owner)
}

fn wait_for_readers(entry: &IrqSlot) -> bool {
    let mut spins = 0;
    while entry.readers.load(Ordering::Acquire) != 0 {
        if spins == QUIESCE_SPIN_LIMIT {
            return false;
        }
        spins += 1;
        core::hint::spin_loop();
    }
    true
}

fn acquire_dispatch(vector: u32) -> Option<(usize, HardHandler, Option<DeferredHandler>)> {
    for (index, entry) in SLOTS.iter().enumerate() {
        entry.readers.fetch_add(1, Ordering::Acquire);
        if entry.state.load(Ordering::Acquire) != SLOT_ACTIVE
            || entry.vector.load(Ordering::Acquire) != vector
        {
            release_reader(entry);
            continue;
        }
        let hard = load_hard(entry)?;
        let deferred = load_deferred(entry);
        return Some((index, hard, deferred));
    }
    None
}

fn load_hard(entry: &IrqSlot) -> Option<HardHandler> {
    let address = entry.hard.load(Ordering::Acquire);
    (address != 0).then(|| unsafe { core::mem::transmute::<usize, HardHandler>(address) })
}

fn load_deferred(entry: &IrqSlot) -> Option<DeferredHandler> {
    let address = entry.deferred.load(Ordering::Acquire);
    (address != 0).then(|| unsafe { core::mem::transmute::<usize, DeferredHandler>(address) })
}

fn release_reader(entry: &IrqSlot) {
    entry.readers.fetch_sub(1, Ordering::Release);
}
