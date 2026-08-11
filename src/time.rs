static mut BOOT_TICKS: u64 = 0;
static mut LAST_SCHEDULER_TICK: u64 = 0;

pub const SCHEDULER_HZ: u64 = 100;
const FALLBACK_TICKS_PER_SCHEDULER_TICK: u64 = 50_000_000;

pub fn init() {
    unsafe {
        BOOT_TICKS = crate::arch::ticks();
        LAST_SCHEDULER_TICK = BOOT_TICKS;
    }
}

pub fn boot_time() -> Option<u64> {
    unsafe { Some(BOOT_TICKS) }
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
