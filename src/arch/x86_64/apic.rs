use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::io::MmioRegion;

const APIC_BASE_MSR: u32 = 0x1b;
const APIC_BASE_X2APIC: u64 = 1 << 10;
const APIC_BASE_ENABLE: u64 = 1 << 11;
const X2APIC_ID: u32 = 0x802;
const X2APIC_VERSION: u32 = 0x803;
const X2APIC_EOI: u32 = 0x80b;
const X2APIC_SVR: u32 = 0x80f;
const APIC_SVR: usize = 0x0f0;
const APIC_ID: usize = 0x020;
const APIC_VERSION: usize = 0x030;
const APIC_LVT_TIMER: usize = 0x320;
const APIC_TIMER_INITIAL: usize = 0x380;
const APIC_TIMER_CURRENT: usize = 0x390;
const APIC_TIMER_DIVIDE: usize = 0x3e0;
const APIC_EOI: usize = 0x0b0;
const TIMER_VECTOR: u32 = 32;
const TIMER_MASKED: u32 = 1 << 16;
const TIMER_PERIODIC: u32 = 1 << 17;
const TIMER_DIVIDE_BY_16: u32 = 0x3;
const HPET_GENERAL_CAPABILITIES: usize = 0x000;
const HPET_GENERAL_CONFIGURATION: usize = 0x010;
const HPET_MAIN_COUNTER: usize = 0x0f0;
const HPET_ENABLE: u64 = 1;
const HPET_CALIBRATION_TICKS_DIVISOR: u64 = 100;
const HPET_CALIBRATION_SPINS: u32 = 10_000_000;

static TIMER_READY: AtomicBool = AtomicBool::new(false);
static TIMER_ENABLED: AtomicBool = AtomicBool::new(false);
static APIC_TIMER_HZ: AtomicU64 = AtomicU64::new(0);
static HPET_HZ: AtomicU64 = AtomicU64::new(0);
static TIMER_INITIAL_COUNT: AtomicU64 = AtomicU64::new(0);
static APIC_MMIO_BASE: AtomicU64 = AtomicU64::new(0);
static X2APIC_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub struct Status {
    pub present: bool,
    pub x2apic: bool,
    pub x2apic_enabled: bool,
    pub enabled: bool,
    pub software_enabled: bool,
    pub base: u64,
    pub id: u32,
    pub version: u32,
}

#[derive(Clone, Copy)]
pub struct TimerCalibration {
    pub hpet_hz: u64,
    pub apic_timer_hz: u64,
    pub initial_count: u32,
}

pub fn init() -> Status {
    let mut current = status();
    if current.present && (!current.enabled || (current.x2apic && !current.x2apic_enabled)) {
        unsafe {
            let mut msr = rdmsr(APIC_BASE_MSR) | APIC_BASE_ENABLE;
            if current.x2apic {
                msr |= APIC_BASE_X2APIC;
            }
            wrmsr(APIC_BASE_MSR, msr);
        }
        current = status();
    }
    if current.enabled && !current.software_enabled {
        let value = read_register(APIC_SVR).unwrap_or(0) | (1 << 8) | 0xff;
        if write_register(APIC_SVR, value) {
            current = status();
        }
    }
    X2APIC_ACTIVE.store(current.x2apic_enabled, Ordering::Release);
    current
}

pub fn current_id() -> usize {
    // Owner tokens must be per-CPU; never cache this in a shared static.
    if X2APIC_ACTIVE.load(Ordering::Acquire) {
        unsafe { rdmsr(X2APIC_ID) as usize }
    } else {
        status().id as usize
    }
}

pub fn status() -> Status {
    let features = cpuid(1, 0);
    let present = features.edx & (1 << 9) != 0;
    let x2apic = features.ecx & (1 << 21) != 0;
    let msr = if present {
        unsafe { rdmsr(APIC_BASE_MSR) }
    } else {
        0
    };
    let enabled = present && msr & APIC_BASE_ENABLE != 0;
    let x2apic_enabled = enabled && x2apic && msr & APIC_BASE_X2APIC != 0;
    let base = msr & 0x000f_ffff_ffff_f000;
    let (software_enabled, id, version) = if x2apic_enabled {
        unsafe {
            (
                rdmsr(X2APIC_SVR) & (1 << 8) != 0,
                rdmsr(X2APIC_ID) as u32,
                rdmsr(X2APIC_VERSION) as u32 & 0xff,
            )
        }
    } else if enabled {
        let Some(mmio) =
            runtime_mmio().or_else(|| unsafe { MmioRegion::new(base as usize, 0x400) })
        else {
            return Status {
                present,
                x2apic,
                x2apic_enabled,
                enabled,
                software_enabled: false,
                base,
                id: 0,
                version: 0,
            };
        };
        let Some(spurious) = mmio.read_u32_le(APIC_SVR) else {
            return Status {
                present,
                x2apic,
                x2apic_enabled,
                enabled,
                software_enabled: false,
                base,
                id: 0,
                version: 0,
            };
        };
        let Some(id_value) = mmio.read_u32_le(APIC_ID) else {
            return Status {
                present,
                x2apic,
                x2apic_enabled,
                enabled,
                software_enabled: false,
                base,
                id: 0,
                version: 0,
            };
        };
        let Some(version_value) = mmio.read_u32_le(APIC_VERSION) else {
            return Status {
                present,
                x2apic,
                x2apic_enabled,
                enabled,
                software_enabled: false,
                base,
                id: 0,
                version: 0,
            };
        };
        (
            spurious & (1 << 8) != 0,
            id_value >> 24,
            version_value & 0xff,
        )
    } else {
        (false, 0, 0)
    };

    Status {
        present,
        x2apic,
        x2apic_enabled,
        enabled,
        software_enabled,
        base,
        id,
        version,
    }
}

pub fn contract_self_check() {
    assert_eq!(timer_initial_count(1_000_000, 100), 10_000);
    assert_eq!(timer_initial_count(99, 100), 1);
    assert_eq!(timer_initial_count(u64::MAX, 1), u32::MAX);
}

pub fn prepare_timer(rsdp: u64, scheduler_hz: u64) -> Option<TimerCalibration> {
    TIMER_READY.store(false, Ordering::Release);
    TIMER_ENABLED.store(false, Ordering::Release);
    APIC_TIMER_HZ.store(0, Ordering::Relaxed);
    HPET_HZ.store(0, Ordering::Relaxed);
    TIMER_INITIAL_COUNT.store(0, Ordering::Relaxed);

    let current = status();
    if !current.enabled || !current.software_enabled {
        return None;
    }
    let (hpet, hpet_hz) = discover_hpet(rsdp)?;
    let period = (hpet_hz / HPET_CALIBRATION_TICKS_DIVISOR).max(1);
    let hpet_config = hpet.read_u64_le(HPET_GENERAL_CONFIGURATION)?;
    if !hpet.write_u64_le(HPET_GENERAL_CONFIGURATION, hpet_config & !HPET_ENABLE)
        || !hpet.write_u64_le(HPET_MAIN_COUNTER, 0)
    {
        return None;
    }

    if !write_register(APIC_TIMER_DIVIDE, TIMER_DIVIDE_BY_16)
        || !write_register(APIC_LVT_TIMER, TIMER_VECTOR | TIMER_MASKED)
        || !write_register(APIC_TIMER_INITIAL, u32::MAX)
        || !hpet.write_u64_le(HPET_GENERAL_CONFIGURATION, hpet_config | HPET_ENABLE)
    {
        return None;
    }

    let hpet_start = hpet.read_u64_le(HPET_MAIN_COUNTER)?;
    let apic_start = read_register(APIC_TIMER_CURRENT)? as u64;
    let mut hpet_end = hpet_start;
    let mut spins = 0;
    while hpet_end.wrapping_sub(hpet_start) < period && spins < HPET_CALIBRATION_SPINS {
        hpet_end = hpet.read_u64_le(HPET_MAIN_COUNTER)?;
        spins += 1;
    }
    let apic_end = read_register(APIC_TIMER_CURRENT)? as u64;
    let _ = hpet.write_u64_le(HPET_GENERAL_CONFIGURATION, hpet_config & !HPET_ENABLE);
    let hpet_elapsed = hpet_end.wrapping_sub(hpet_start);
    let apic_elapsed = apic_start.wrapping_sub(apic_end);
    if hpet_elapsed < period || apic_elapsed == 0 {
        let _ = write_register(APIC_TIMER_INITIAL, 0);
        return None;
    }
    let apic_timer_hz = apic_elapsed.checked_mul(hpet_hz)? / hpet_elapsed;
    let initial_count = timer_initial_count(apic_timer_hz, scheduler_hz);
    if !write_register(APIC_TIMER_INITIAL, 0) {
        return None;
    }

    let calibration = TimerCalibration {
        hpet_hz,
        apic_timer_hz,
        initial_count,
    };
    APIC_TIMER_HZ.store(apic_timer_hz, Ordering::Release);
    HPET_HZ.store(hpet_hz, Ordering::Release);
    TIMER_INITIAL_COUNT.store(initial_count as u64, Ordering::Release);
    TIMER_READY.store(true, Ordering::Release);
    Some(calibration)
}

pub fn enable_timer() -> bool {
    if !TIMER_READY.load(Ordering::Acquire) {
        return false;
    }
    if !ensure_runtime_mmio() {
        return false;
    }
    let initial_count = TIMER_INITIAL_COUNT.load(Ordering::Acquire) as u32;
    if initial_count == 0
        || !write_register(APIC_TIMER_DIVIDE, TIMER_DIVIDE_BY_16)
        || !write_register(APIC_TIMER_INITIAL, initial_count)
        || !write_register(APIC_LVT_TIMER, TIMER_VECTOR | TIMER_PERIODIC)
    {
        return false;
    }
    TIMER_ENABLED.store(true, Ordering::Release);
    true
}

pub fn timer_enabled() -> bool {
    TIMER_ENABLED.load(Ordering::Acquire)
}

pub fn timer_calibration() -> Option<TimerCalibration> {
    if !TIMER_READY.load(Ordering::Acquire) {
        return None;
    }
    Some(TimerCalibration {
        hpet_hz: HPET_HZ.load(Ordering::Acquire),
        apic_timer_hz: APIC_TIMER_HZ.load(Ordering::Acquire),
        initial_count: TIMER_INITIAL_COUNT.load(Ordering::Acquire) as u32,
    })
}

pub fn end_of_interrupt() {
    if X2APIC_ACTIVE.load(Ordering::Acquire) {
        unsafe { wrmsr(X2APIC_EOI, 0) };
    } else if let Some(mmio) = runtime_mmio() {
        let _ = mmio.write_u32_le(APIC_EOI, 0);
    }
}

fn ensure_runtime_mmio() -> bool {
    if X2APIC_ACTIVE.load(Ordering::Acquire) {
        return true;
    }
    if APIC_MMIO_BASE.load(Ordering::Acquire) != 0 {
        return true;
    }
    let base = unsafe { rdmsr(APIC_BASE_MSR) & 0x000f_ffff_ffff_f000 };
    let Some(mapped) = crate::arch::x86_64::paging::map_device(base, 0x400) else {
        return false;
    };
    APIC_MMIO_BASE.store(mapped as u64, Ordering::Release);
    true
}

fn runtime_mmio() -> Option<MmioRegion> {
    let base = APIC_MMIO_BASE.load(Ordering::Acquire);
    if base == 0 {
        return None;
    }
    unsafe { MmioRegion::new(base as usize, 0x400) }
}

fn timer_initial_count(apic_timer_hz: u64, scheduler_hz: u64) -> u32 {
    let count = apic_timer_hz / scheduler_hz.max(1);
    count.clamp(1, u32::MAX as u64) as u32
}

fn read_register(register: usize) -> Option<u32> {
    let current = status();
    if current.x2apic_enabled {
        return Some(unsafe { rdmsr((0x800 + register / 0x10) as u32) as u32 });
    }
    let mmio =
        runtime_mmio().or_else(|| unsafe { MmioRegion::new(current.base as usize, 0x400) })?;
    mmio.read_u32_le(register)
}

fn write_register(register: usize, value: u32) -> bool {
    let current = status();
    if current.x2apic_enabled {
        unsafe { wrmsr(0x800 + (register / 0x10) as u32, value as u64) };
        return true;
    }
    let Some(mmio) =
        runtime_mmio().or_else(|| unsafe { MmioRegion::new(current.base as usize, 0x400) })
    else {
        return false;
    };
    mmio.write_u32_le(register, value)
}

fn discover_hpet(rsdp: u64) -> Option<(MmioRegion, u64)> {
    let rsdp = usize::try_from(rsdp).ok()?;
    let base = unsafe { hpet_base_from_rsdp(rsdp)? };
    let mmio = unsafe { MmioRegion::new(usize::try_from(base).ok()?, 0x400) }?;
    let capabilities = mmio.read_u64_le(HPET_GENERAL_CAPABILITIES)?;
    let period_fs = capabilities >> 32;
    if period_fs == 0 {
        return None;
    }
    let frequency = 1_000_000_000_000_000u64 / period_fs;
    (frequency != 0).then_some((mmio, frequency))
}

unsafe fn hpet_base_from_rsdp(rsdp: usize) -> Option<u64> {
    if !signature(rsdp, b"RSD PTR ") || !checksum(rsdp, 20)? {
        return None;
    }
    let revision = raw_u8(rsdp.checked_add(15)?);
    let (root, entry_size) =
        if revision >= 2 && raw_u32(rsdp.checked_add(20)?) >= 36 && checksum(rsdp, 36)? {
            (raw_u64(rsdp.checked_add(24)?), 8usize)
        } else {
            (raw_u32(rsdp.checked_add(16)?) as u64, 4usize)
        };
    let root = usize::try_from(root).ok()?;
    if root == 0 || !checksum_table(root, if entry_size == 8 { b"XSDT" } else { b"RSDT" })? {
        return None;
    }
    let length = raw_u32(root.checked_add(4)?) as usize;
    if !(36..=1024 * 1024).contains(&length) {
        return None;
    }
    let entries = (length - 36) / entry_size;
    for index in 0..entries {
        let entry = root
            .checked_add(36)?
            .checked_add(index.checked_mul(entry_size)?)?;
        let address = if entry_size == 8 {
            raw_u64(entry)
        } else {
            raw_u32(entry) as u64
        };
        let address = usize::try_from(address).ok()?;
        if checksum_table(address, b"HPET")? {
            let length = raw_u32(address.checked_add(4)?) as usize;
            if length >= 56 && raw_u8(address.checked_add(40)?) == 0 {
                let base = raw_u64(address.checked_add(44)?);
                if base != 0 {
                    return Some(base);
                }
            }
        }
    }
    None
}

unsafe fn checksum_table(address: usize, expected: &[u8]) -> Option<bool> {
    if !signature(address, expected) {
        return Some(false);
    }
    let length = raw_u32(address.checked_add(4)?) as usize;
    if !(36..=1024 * 1024).contains(&length) {
        return Some(false);
    }
    checksum(address, length)
}

unsafe fn signature(address: usize, expected: &[u8]) -> bool {
    expected
        .iter()
        .enumerate()
        .all(|(index, byte)| raw_u8(address.saturating_add(index)) == *byte)
}

unsafe fn checksum(address: usize, length: usize) -> Option<bool> {
    let mut sum = 0u8;
    for index in 0..length {
        sum = sum.wrapping_add(raw_u8(address.checked_add(index)?));
    }
    Some(sum == 0)
}

unsafe fn raw_u8(address: usize) -> u8 {
    (address as *const u8).read_unaligned()
}

unsafe fn raw_u32(address: usize) -> u32 {
    (address as *const u32).read_unaligned()
}

unsafe fn raw_u64(address: usize) -> u64 {
    (address as *const u64).read_unaligned()
}

#[derive(Clone, Copy)]
struct Cpuid {
    ecx: u32,
    edx: u32,
}

fn cpuid(leaf: u32, subleaf: u32) -> Cpuid {
    let ecx: u32;
    let edx: u32;
    unsafe {
        asm!(
            "push rbx",
            "cpuid",
            "pop rbx",
            inout("eax") leaf => _,
            inout("ecx") subleaf => ecx,
            out("edx") edx,
            options(preserves_flags)
        );
    }
    Cpuid { ecx, edx }
}

unsafe fn rdmsr(msr: u32) -> u64 {
    let high: u32;
    let low: u32;
    asm!(
        "rdmsr",
        in("ecx") msr,
        out("edx") high,
        out("eax") low,
        options(nomem, nostack, preserves_flags)
    );
    ((high as u64) << 32) | low as u64
}

unsafe fn wrmsr(msr: u32, value: u64) {
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("edx") (value >> 32) as u32,
        in("eax") value as u32,
        options(nomem, nostack, preserves_flags)
    );
}
