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

pub fn scheduler_ticks_for_frequency(frequency_hz: Option<u64>) -> u64 {
    frequency_hz
        .map(|hz| (hz / SCHEDULER_HZ).max(1))
        .unwrap_or(FALLBACK_TICKS_PER_SCHEDULER_TICK)
}

pub fn scheduler_ticks() -> u64 {
    scheduler_ticks_for_frequency(crate::arch::timer_frequency_hz())
}

pub const fn scheduler_tick_due(now: u64, last: u64, interval: u64) -> bool {
    interval != 0 && now.wrapping_sub(last) >= interval
}

pub fn poll_scheduler_tick() -> bool {
    let now = ticks();
    let interval = scheduler_ticks();
    unsafe {
        if !scheduler_tick_due(now, LAST_SCHEDULER_TICK, interval) {
            return false;
        }
        LAST_SCHEDULER_TICK = now;
    }
    true
}

pub fn contract_self_check() {
    assert_eq!(scheduler_hz(), 100);
    assert_eq!(scheduler_ticks_for_frequency(Some(1_000_000)), 10_000);
    assert_eq!(scheduler_ticks_for_frequency(Some(99)), 1);
    assert_eq!(
        scheduler_ticks_for_frequency(None),
        FALLBACK_TICKS_PER_SCHEDULER_TICK
    );
    assert!(!scheduler_tick_due(99, 0, 100));
    assert!(scheduler_tick_due(100, 0, 100));
    assert!(scheduler_tick_due(10, u64::MAX - 20, 31));
}
