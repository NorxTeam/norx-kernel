mod arch {
    pub fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
        f()
    }

    pub fn ticks() -> u64 {
        0
    }

    pub fn timer_frequency_hz() -> Option<u64> {
        None
    }
}

mod process {
    pub fn wake_sleepers(_now: u64) {}
}

mod irq {
    #[derive(Clone, Copy)]
    pub struct Stats {
        pub timer: u64,
        pub timer_pending: u64,
    }

    pub fn stats() -> Stats {
        Stats {
            timer: 0,
            timer_pending: 0,
        }
    }

    pub fn run_deferred() -> usize {
        0
    }
}

#[path = "src/time.rs"]
mod time;
#[path = "src/sched.rs"]
mod sched;

#[test]
fn s067_contracts() {
    time::contract_self_check();
    sched::self_check();
}
