use core::sync::atomic::{AtomicBool, Ordering};

static HARDWARE_TICKS: AtomicBool = AtomicBool::new(false);

pub fn init() -> bool {
    let ok = crate::arch::init_timer_interrupts();
    HARDWARE_TICKS.store(ok, Ordering::Relaxed);
    ok
}

pub fn hardware_ticks() -> bool {
    HARDWARE_TICKS.load(Ordering::Relaxed)
}
