use crate::uefi::{Status, SystemTable, Time};

static mut BOOT_TIME: Option<Time> = None;
static mut LAST_SCHEDULER_TICK: u64 = 0;

const SCHEDULER_HZ: u64 = 100;
const FALLBACK_TICKS_PER_SCHEDULER_TICK: u64 = 50_000_000;

pub fn init(system_table: *mut SystemTable) {
    unsafe {
        BOOT_TIME = crate::uefi::get_time(system_table);
        LAST_SCHEDULER_TICK = crate::arch::ticks();
    }
}

pub fn boot_time() -> Option<Time> {
    unsafe { BOOT_TIME }
}

pub fn ticks() -> u64 {
    crate::arch::ticks()
}

pub fn scheduler_hz() -> u64 {
    SCHEDULER_HZ
}

pub fn scheduler_ticks() -> u64 {
    crate::arch::timer_frequency_hz()
        .map(|hz| (hz / SCHEDULER_HZ).max(1))
        .unwrap_or(FALLBACK_TICKS_PER_SCHEDULER_TICK)
}

pub fn poll_scheduler_tick() -> bool {
    let now = ticks();
    let interval = scheduler_ticks();
    unsafe {
        if now.wrapping_sub(LAST_SCHEDULER_TICK) < interval {
            return false;
        }
        LAST_SCHEDULER_TICK = now;
    }
    true
}

pub fn ok(status: Status) -> bool {
    status == 0
}
