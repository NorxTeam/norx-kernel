use core::sync::atomic::{AtomicU64, Ordering};

static TIMER: AtomicU64 = AtomicU64::new(0);
static KEYBOARD: AtomicU64 = AtomicU64::new(0);
static SPURIOUS: AtomicU64 = AtomicU64::new(0);
static EXCEPTIONS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub struct Stats {
    pub timer: u64,
    pub keyboard: u64,
    pub spurious: u64,
    pub exceptions: u64,
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn timer() {
    TIMER.fetch_add(1, Ordering::Relaxed);
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn keyboard() {
    KEYBOARD.fetch_add(1, Ordering::Relaxed);
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
        keyboard: KEYBOARD.load(Ordering::Relaxed),
        spurious: SPURIOUS.load(Ordering::Relaxed),
        exceptions: EXCEPTIONS.load(Ordering::Relaxed),
    }
}
