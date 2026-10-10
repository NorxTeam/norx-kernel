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
const HPET_COUNTER_64: u64 = 1 << 13;
const HPET_MMIO_SIZE: usize = 0x400;
const ACPI_HEADER_SIZE: usize = 36;
const ACPI_MAX_TABLE_SIZE: usize = 1024 * 1024;
const RSDP_MAX_SIZE: usize = 4096;
const HPET_CALIBRATION_TICKS_DIVISOR: u64 = 100;
const HPET_CALIBRATION_SPINS: u32 = 10_000_000;

static TIMER_READY: AtomicBool = AtomicBool::new(false);
static TIMER_ENABLED: AtomicBool = AtomicBool::new(false);
static APIC_TIMER_HZ: AtomicU64 = AtomicU64::new(0);
static HPET_HZ: AtomicU64 = AtomicU64::new(0);
static TIMER_INITIAL_COUNT: AtomicU64 = AtomicU64::new(0);
static APIC_MMIO_BASE: AtomicU64 = AtomicU64::new(0);
static HPET_MMIO_BASE: AtomicU64 = AtomicU64::new(0);
static HPET_COUNTER_MASK: AtomicU64 = AtomicU64::new(0);
static HPET_CLOCK_READY: AtomicBool = AtomicBool::new(false);
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
    if current.enabled && !current.x2apic_enabled {
        let _ = ensure_runtime_mmio();
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
        let Some(mmio) = runtime_mmio() else {
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
    assert_eq!(hpet_frequency(100_000_000), Some(10_000_000));
    assert_eq!(hpet_frequency(0), None);
    assert_eq!(hpet_frequency(u64::MAX), None);
}

pub fn prepare_timer(rsdp: u64, scheduler_hz: u64) -> Option<TimerCalibration> {
    TIMER_READY.store(false, Ordering::Release);
    TIMER_ENABLED.store(false, Ordering::Release);
    APIC_TIMER_HZ.store(0, Ordering::Relaxed);
    HPET_HZ.store(0, Ordering::Relaxed);
    TIMER_INITIAL_COUNT.store(0, Ordering::Relaxed);
    HPET_CLOCK_READY.store(false, Ordering::Release);
    HPET_MMIO_BASE.store(0, Ordering::Relaxed);
    HPET_COUNTER_MASK.store(0, Ordering::Relaxed);

    let current = status();
    if !current.enabled || !current.software_enabled {
        return None;
    }
    let (hpet, hpet_base, hpet_hz, counter_mask) = discover_hpet(rsdp)?;
    HPET_MMIO_BASE.store(hpet_base as u64, Ordering::Release);
    HPET_COUNTER_MASK.store(counter_mask, Ordering::Release);
    HPET_HZ.store(hpet_hz, Ordering::Release);
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
        let _ = hpet.write_u64_le(HPET_GENERAL_CONFIGURATION, hpet_config);
        return None;
    }

    let Some(hpet_start) = hpet.read_u64_le(HPET_MAIN_COUNTER) else {
        let _ = hpet.write_u64_le(HPET_GENERAL_CONFIGURATION, hpet_config);
        let _ = write_register(APIC_TIMER_INITIAL, 0);
        return None;
    };
    let Some(apic_start) = read_register(APIC_TIMER_CURRENT) else {
        let _ = hpet.write_u64_le(HPET_GENERAL_CONFIGURATION, hpet_config);
        let _ = write_register(APIC_TIMER_INITIAL, 0);
        return None;
    };
    HPET_CLOCK_READY.store(true, Ordering::Release);
    let mut hpet_end = hpet_start;
    let mut spins = 0;
    while hpet_end.wrapping_sub(hpet_start) < period && spins < HPET_CALIBRATION_SPINS {
        let Some(next) = hpet.read_u64_le(HPET_MAIN_COUNTER) else {
            HPET_CLOCK_READY.store(false, Ordering::Release);
            let _ = hpet.write_u64_le(HPET_GENERAL_CONFIGURATION, hpet_config);
            let _ = write_register(APIC_TIMER_INITIAL, 0);
            return None;
        };
        hpet_end = next;
        spins += 1;
    }
    let Some(apic_end) = read_register(APIC_TIMER_CURRENT) else {
        HPET_CLOCK_READY.store(false, Ordering::Release);
        let _ = hpet.write_u64_le(HPET_GENERAL_CONFIGURATION, hpet_config);
        let _ = write_register(APIC_TIMER_INITIAL, 0);
        return None;
    };
    let hpet_elapsed = hpet_end.wrapping_sub(hpet_start) & counter_mask;
    let apic_elapsed = apic_start.wrapping_sub(apic_end);
    if hpet_elapsed < period || apic_elapsed == 0 {
        let _ = write_register(APIC_TIMER_INITIAL, 0);
        return None;
    }
    let Some(apic_timer_hz) = u64::from(apic_elapsed)
        .checked_mul(hpet_hz)
        .and_then(|value| value.checked_div(hpet_elapsed))
        .filter(|frequency| *frequency != 0)
    else {
        let _ = write_register(APIC_TIMER_INITIAL, 0);
        return None;
    };
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

pub fn disable_timer() {
    TIMER_ENABLED.store(false, Ordering::Release);
    let _ = write_register(APIC_LVT_TIMER, TIMER_VECTOR | TIMER_MASKED);
    let _ = write_register(APIC_TIMER_INITIAL, 0);
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

pub fn clock_frequency_hz() -> Option<u64> {
    let frequency = HPET_HZ.load(Ordering::Acquire);
    (HPET_CLOCK_READY.load(Ordering::Acquire) && frequency != 0).then_some(frequency)
}

pub fn clock_ticks() -> Option<u64> {
    clock_frequency_hz()?;
    let base = HPET_MMIO_BASE.load(Ordering::Acquire);
    if base == 0 {
        return None;
    }
    let mmio = unsafe { MmioRegion::new(base as usize, HPET_MMIO_SIZE) }?;
    Some(mmio.read_u64_le(HPET_MAIN_COUNTER)? & HPET_COUNTER_MASK.load(Ordering::Acquire))
}

pub fn end_of_interrupt() {
    if X2APIC_ACTIVE.load(Ordering::Acquire) {
        unsafe { wrmsr(X2APIC_EOI, 0) };
    } else if let Some(mmio) = runtime_mmio() {
        let _ = mmio.write_u32_le(APIC_EOI, 0);
    }
}

fn ensure_runtime_mmio() -> bool {
    let msr = unsafe { rdmsr(APIC_BASE_MSR) };
    if msr & APIC_BASE_X2APIC != 0 {
        return true;
    }
    if APIC_MMIO_BASE.load(Ordering::Acquire) != 0 {
        return true;
    }
    let base = msr & 0x000f_ffff_ffff_f000;
    if base == 0 {
        return false;
    }
    let Some(mapped) = crate::arch::x86_64::paging::map_device(base, HPET_MMIO_SIZE) else {
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
    let mmio = runtime_mmio()?;
    mmio.read_u32_le(register)
}

fn write_register(register: usize, value: u32) -> bool {
    let current = status();
    if current.x2apic_enabled {
        unsafe { wrmsr(0x800 + (register / 0x10) as u32, value as u64) };
        return true;
    }
    let Some(mmio) = runtime_mmio() else {
        return false;
    };
    mmio.write_u32_le(register, value)
}

fn discover_hpet(rsdp: u64) -> Option<(MmioRegion, usize, u64, u64)> {
    let rsdp = usize::try_from(rsdp).ok()?;
    if rsdp == 0 {
        return None;
    }
    let rsdp_header = map_firmware_range(rsdp as u64, ACPI_HEADER_SIZE)?;
    if !has_signature(rsdp_header, b"RSD PTR ") || !checksum(rsdp_header, 20)? {
        return None;
    }

    let revision = rsdp_header.read_u8(15)?;
    let (root, root_signature, entry_size) = if revision >= 2 {
        let length = usize::try_from(rsdp_header.read_u32_le(20)?).ok()?;
        if !(ACPI_HEADER_SIZE..=RSDP_MAX_SIZE).contains(&length) {
            return None;
        }
        let rsdp_extended = map_firmware_range(rsdp as u64, length)?;
        if !checksum(rsdp_extended, length)? {
            return None;
        }
        let xsdt = rsdp_extended.read_u64_le(24)?;
        let rsdt = rsdp_extended.read_u32_le(16)? as u64;
        if xsdt != 0 {
            (xsdt, b"XSDT".as_slice(), 8usize)
        } else {
            (rsdt, b"RSDT".as_slice(), 4usize)
        }
    } else {
        (
            rsdp_header.read_u32_le(16)? as u64,
            b"RSDT".as_slice(),
            4usize,
        )
    };

    let (root, root_length) = map_acpi_table(root, root_signature)?;
    let entries = (root_length - ACPI_HEADER_SIZE) / entry_size;
    for index in 0..entries {
        let offset = ACPI_HEADER_SIZE.checked_add(index.checked_mul(entry_size)?)?;
        let address = if entry_size == 8 {
            root.read_u64_le(offset)?
        } else {
            root.read_u32_le(offset)? as u64
        };
        let Some((header, _)) = map_acpi_header(address) else {
            continue;
        };
        if !has_signature(header, b"HPET") {
            continue;
        }
        let Some((hpet_table, length)) = map_acpi_table(address, b"HPET") else {
            continue;
        };
        if length < 56 || hpet_table.read_u8(40)? != 0 {
            continue;
        }
        let base = hpet_table.read_u64_le(44)?;
        let (mmio, mapped) = map_firmware_range_with_base(base, HPET_MMIO_SIZE)?;
        let capabilities = mmio.read_u64_le(HPET_GENERAL_CAPABILITIES)?;
        let frequency = hpet_frequency(capabilities >> 32)?;
        let counter_mask = if capabilities & HPET_COUNTER_64 != 0 {
            u64::MAX
        } else {
            u32::MAX as u64
        };
        return Some((mmio, mapped, frequency, counter_mask));
    }
    None
}

fn map_acpi_header(address: u64) -> Option<(MmioRegion, usize)> {
    let header = map_firmware_range(address, ACPI_HEADER_SIZE)?;
    let length = usize::try_from(header.read_u32_le(4)?).ok()?;
    (ACPI_HEADER_SIZE..=ACPI_MAX_TABLE_SIZE)
        .contains(&length)
        .then_some((header, length))
}

fn map_acpi_table(address: u64, expected: &[u8]) -> Option<(MmioRegion, usize)> {
    let (_, length) = map_acpi_header(address)?;
    let table = map_firmware_range(address, length)?;
    if !has_signature(table, expected) || !checksum(table, length)? {
        return None;
    }
    Some((table, length))
}

fn map_firmware_range(physical: u64, size: usize) -> Option<MmioRegion> {
    map_firmware_range_with_base(physical, size).map(|(region, _)| region)
}

fn map_firmware_range_with_base(physical: u64, size: usize) -> Option<(MmioRegion, usize)> {
    if physical == 0 || size == 0 {
        return None;
    }
    let end = physical.checked_add(u64::try_from(size.checked_sub(1)?).ok()?)?;
    if let (Some(start), Some(_)) = (
        crate::arch::x86_64::paging::direct_map_ptr(physical),
        crate::arch::x86_64::paging::direct_map_ptr(end),
    ) {
        let base = start as usize;
        return Some((unsafe { MmioRegion::new(base, size) }?, base));
    }
    let base = crate::arch::x86_64::paging::map_device(physical, size)?;
    Some((unsafe { MmioRegion::new(base, size) }?, base))
}

fn has_signature(region: MmioRegion, expected: &[u8]) -> bool {
    expected
        .iter()
        .enumerate()
        .all(|(offset, byte)| region.read_u8(offset) == Some(*byte))
}

fn checksum(region: MmioRegion, length: usize) -> Option<bool> {
    if length == 0 {
        return None;
    }
    let mut sum = 0u8;
    for offset in 0..length {
        sum = sum.wrapping_add(region.read_u8(offset)?);
    }
    Some(sum == 0)
}

fn hpet_frequency(period_fs: u64) -> Option<u64> {
    if period_fs == 0 {
        return None;
    }
    let frequency = 1_000_000_000_000_000u64.checked_div(period_fs)?;
    (frequency != 0).then_some(frequency)
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
