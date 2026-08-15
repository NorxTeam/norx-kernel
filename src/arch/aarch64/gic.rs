use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::boot::Aarch64InterruptInfo;
use crate::io::MmioRegion;

const GICD_CTLR: usize = 0x000;
const GICD_TYPER: usize = 0x004;
const GICD_ICENABLER: usize = 0x180;
const GICD_ISENABLER: usize = 0x100;
const GICD_IPRIORITYR: usize = 0x400;
const GICD_ICFGR: usize = 0xc00;
const GICD_ENABLE_GROUP0: u32 = 1;
const GICD_ENABLE_GROUP1: u32 = 2;

const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004;
const GICC_BPR: usize = 0x008;
const GICC_IAR: usize = 0x00c;
const GICC_EOIR: usize = 0x010;

const GICR_WAKER: usize = 0x0014;
const GICR_IGROUPR0: usize = 0x0080;
const GICR_ICENABLER0: usize = 0x0180;
const GICR_ISENABLER0: usize = 0x0100;
const GICR_IPRIORITYR: usize = 0x0400;
const GICR_ICFGR: usize = 0x0c00;
const GICR_SGI_BASE: usize = 0x1_0000;

const MAX_INTERRUPTS: u32 = 1024;
const SPURIOUS_INTID: u32 = 1020;

static READY: AtomicBool = AtomicBool::new(false);
static TIMER_ENABLED: AtomicBool = AtomicBool::new(false);
static VERSION: AtomicU64 = AtomicU64::new(0);
static DISTRIBUTOR: AtomicU64 = AtomicU64::new(0);
static CPU_INTERFACE: AtomicU64 = AtomicU64::new(0);
static REDISTRIBUTOR: AtomicU64 = AtomicU64::new(0);
static TIMER_INTID: AtomicU64 = AtomicU64::new(0);
static ACK_COUNT: AtomicU64 = AtomicU64::new(0);
static LAST_ACK: AtomicU64 = AtomicU64::new(SPURIOUS_INTID as u64);

#[derive(Clone, Copy)]
pub struct Status {
    pub version: u8,
    pub distributor: u64,
    pub cpu_interface: u64,
    pub redistributor: u64,
    pub timer_intid: u32,
    pub ack_count: u64,
    pub last_ack: u32,
    pub ready: bool,
    pub timer_enabled: bool,
}

pub fn contract_self_check() {
    assert!(init(Aarch64InterruptInfo::empty()).is_none());
}

pub fn init(info: Aarch64InterruptInfo) -> Option<Status> {
    READY.store(false, Ordering::Release);
    TIMER_ENABLED.store(false, Ordering::Release);
    ACK_COUNT.store(0, Ordering::Relaxed);
    LAST_ACK.store(SPURIOUS_INTID as u64, Ordering::Relaxed);
    if !(info.gic_version == 2 || info.gic_version == 3)
        || info.gic_distributor == 0
        || !(16..MAX_INTERRUPTS).contains(&info.timer_irq)
    {
        return None;
    }
    if info.gic_version == 2 {
        if info.gic_cpu_interface == 0 || !init_v2(info) {
            return None;
        }
    } else if info.gic_redistributor == 0 || !init_v3(info) {
        return None;
    }
    VERSION.store(info.gic_version as u64, Ordering::Release);
    DISTRIBUTOR.store(info.gic_distributor, Ordering::Release);
    CPU_INTERFACE.store(info.gic_cpu_interface, Ordering::Release);
    REDISTRIBUTOR.store(info.gic_redistributor, Ordering::Release);
    TIMER_INTID.store(info.timer_irq as u64, Ordering::Release);
    READY.store(true, Ordering::Release);
    Some(status())
}

pub fn status() -> Status {
    Status {
        version: VERSION.load(Ordering::Acquire) as u8,
        distributor: DISTRIBUTOR.load(Ordering::Acquire),
        cpu_interface: CPU_INTERFACE.load(Ordering::Acquire),
        redistributor: REDISTRIBUTOR.load(Ordering::Acquire),
        timer_intid: TIMER_INTID.load(Ordering::Acquire) as u32,
        ack_count: ACK_COUNT.load(Ordering::Acquire),
        last_ack: LAST_ACK.load(Ordering::Acquire) as u32,
        ready: READY.load(Ordering::Acquire),
        timer_enabled: TIMER_ENABLED.load(Ordering::Acquire),
    }
}

pub fn enable_timer() -> bool {
    if !READY.load(Ordering::Acquire) {
        return false;
    }
    let intid = TIMER_INTID.load(Ordering::Acquire) as u32;
    let enabled = if VERSION.load(Ordering::Acquire) == 2 {
        let Some(distributor) = distributor() else {
            return false;
        };
        let Some(typer) = distributor.read_u32_le(GICD_TYPER) else {
            return false;
        };
        let count = ((typer & 0x1f) + 1) * 32;
        count > intid
            && distributor.write_u32_le(
                GICD_ISENABLER + (intid as usize / 32) * 4,
                1 << (intid % 32),
            )
    } else {
        let Some(redistributor) = redistributor_sgi() else {
            return false;
        };
        redistributor.write_u32_le(GICR_ISENABLER0, 1 << (intid % 32))
    };
    if enabled {
        TIMER_ENABLED.store(true, Ordering::Release);
    }
    enabled
}

pub fn timer_enabled() -> bool {
    TIMER_ENABLED.load(Ordering::Acquire)
}

pub fn acknowledge() -> Option<u32> {
    if !READY.load(Ordering::Acquire) {
        return None;
    }
    let raw = if VERSION.load(Ordering::Acquire) == 2 {
        cpu_interface()?.read_u32_le(GICC_IAR)?
    } else {
        unsafe { read_icc_iar1() }
    };
    let intid = raw & 0x3ff;
    ACK_COUNT.fetch_add(1, Ordering::Relaxed);
    LAST_ACK.store(intid as u64, Ordering::Release);
    (intid < SPURIOUS_INTID).then_some(intid)
}

pub fn end_of_interrupt(intid: u32) {
    if VERSION.load(Ordering::Acquire) == 2 {
        if let Some(cpu) = cpu_interface() {
            let _ = cpu.write_u32_le(GICC_EOIR, intid);
        }
    } else {
        unsafe { write_icc_eoir1(intid) };
    }
}

fn init_v2(info: Aarch64InterruptInfo) -> bool {
    let Some(distributor) = (unsafe { MmioRegion::new(info.gic_distributor as usize, 0x1000) })
    else {
        return false;
    };
    let Some(cpu) = (unsafe { MmioRegion::new(info.gic_cpu_interface as usize, 0x1000) }) else {
        return false;
    };
    let Some(typer) = distributor.read_u32_le(GICD_TYPER) else {
        return false;
    };
    let count = ((typer & 0x1f) + 1) * 32;
    let intid = info.timer_irq;
    if count > MAX_INTERRUPTS || intid >= count {
        return false;
    }
    if !distributor.write_u32_le(GICD_CTLR, 0) {
        return false;
    }
    for index in 0..((count as usize + 31) / 32) {
        if !distributor.write_u32_le(GICD_ICENABLER + index * 4, u32::MAX) {
            return false;
        }
    }
    let config_offset = GICD_ICFGR + (intid as usize / 16) * 4;
    let config_shift = (intid as usize % 16) * 2;
    let Some(mut config) = distributor.read_u32_le(config_offset) else {
        return false;
    };
    config &= !(0b11 << config_shift);
    if !distributor.write_u32_le(config_offset, config)
        || !distributor.write_u8(GICD_IPRIORITYR + intid as usize, 0x80)
        || !distributor.write_u32_le(GICD_CTLR, GICD_ENABLE_GROUP0)
    {
        return false;
    }
    cpu.write_u32_le(GICC_CTLR, 0)
        && cpu.write_u8(GICC_PMR, 0xff)
        && cpu.write_u8(GICC_BPR, 0)
        && cpu.write_u32_le(GICC_CTLR, GICD_ENABLE_GROUP0)
}

fn init_v3(info: Aarch64InterruptInfo) -> bool {
    let Some(distributor) = (unsafe { MmioRegion::new(info.gic_distributor as usize, 0x1000) })
    else {
        return false;
    };
    let Some(redistributor) =
        (unsafe { MmioRegion::new(info.gic_redistributor as usize + GICR_SGI_BASE, 0x1000) })
    else {
        return false;
    };
    let Some(typer) = distributor.read_u32_le(GICD_TYPER) else {
        return false;
    };
    let count = ((typer & 0x1f) + 1) * 32;
    let intid = info.timer_irq;
    if count > MAX_INTERRUPTS || intid >= 32 {
        return false;
    }
    let Some(waker) = (unsafe { MmioRegion::new(info.gic_redistributor as usize, GICR_SGI_BASE) })
    else {
        return false;
    };
    let Some(mut value) = waker.read_u32_le(GICR_WAKER) else {
        return false;
    };
    value &= !(1 << 1);
    if !waker.write_u32_le(GICR_WAKER, value) {
        return false;
    }
    let mut spins = 0;
    while spins < 1_000_000 {
        if waker
            .read_u32_le(GICR_WAKER)
            .is_some_and(|state| state & (1 << 2) == 0)
        {
            break;
        }
        spins += 1;
    }
    if spins == 1_000_000
        || !distributor.write_u32_le(GICD_CTLR, 0)
        || !redistributor.write_u32_le(GICR_IGROUPR0, u32::MAX)
        || !redistributor.write_u32_le(GICR_ICENABLER0, u32::MAX)
        || !redistributor.write_u8(GICR_IPRIORITYR + intid as usize, 0x80)
    {
        return false;
    }
    let config_offset = GICR_ICFGR + (intid as usize / 16) * 4;
    let config_shift = (intid as usize % 16) * 2;
    let Some(mut config) = redistributor.read_u32_le(config_offset) else {
        return false;
    };
    config &= !(0b11 << config_shift);
    if !redistributor.write_u32_le(config_offset, config)
        || !distributor.write_u32_le(GICD_CTLR, GICD_ENABLE_GROUP1)
        || !enable_system_registers()
    {
        return false;
    }
    true
}

fn distributor() -> Option<MmioRegion> {
    unsafe { MmioRegion::new(DISTRIBUTOR.load(Ordering::Acquire) as usize, 0x1000) }
}

fn cpu_interface() -> Option<MmioRegion> {
    unsafe { MmioRegion::new(CPU_INTERFACE.load(Ordering::Acquire) as usize, 0x1000) }
}

fn redistributor_sgi() -> Option<MmioRegion> {
    unsafe {
        MmioRegion::new(
            REDISTRIBUTOR.load(Ordering::Acquire) as usize + GICR_SGI_BASE,
            0x1000,
        )
    }
}

fn enable_system_registers() -> bool {
    unsafe {
        let mut sre: u64;
        asm!("mrs {}, ICC_SRE_EL1", out(reg) sre, options(nomem, nostack, preserves_flags));
        sre |= 1;
        asm!("msr ICC_SRE_EL1, {}", in(reg) sre, options(nomem, nostack, preserves_flags));
        asm!("isb", options(nomem, nostack, preserves_flags));
        asm!("msr ICC_PMR_EL1, {}", in(reg) 0xffu64, options(nomem, nostack, preserves_flags));
        asm!("msr ICC_BPR1_EL1, {}", in(reg) 0u64, options(nomem, nostack, preserves_flags));
        asm!("msr ICC_IGRPEN1_EL1, {}", in(reg) 1u64, options(nomem, nostack, preserves_flags));
        asm!("isb", options(nomem, nostack, preserves_flags));
    }
    true
}

unsafe fn read_icc_iar1() -> u32 {
    let value: u64;
    asm!("mrs {}, ICC_IAR1_EL1", out(reg) value, options(nomem, nostack, preserves_flags));
    value as u32
}

unsafe fn write_icc_eoir1(intid: u32) {
    asm!("msr ICC_EOIR1_EL1, {}", in(reg) intid as u64, options(nomem, nostack, preserves_flags));
    asm!("isb", options(nomem, nostack, preserves_flags));
}
