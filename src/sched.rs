use core::cell::UnsafeCell;

const MAX_TASKS: usize = 16;
const BASE_SLICE: u64 = 4;
const INTERACTIVE_SLICE: u64 = 2;
const HARDWARE_PROBE_SPINS: u32 = 10_000_000;
const POLLING_PROBE_SPINS: u32 = 100_000_000;
const PROBE_BATCH_SPINS: u32 = 4096;
const RUNTIME_PROBE_TICKS: u64 = 8;

struct RuntimeCell(UnsafeCell<Scheduler>);

unsafe impl Sync for RuntimeCell {}

static RUNTIME: RuntimeCell = RuntimeCell(UnsafeCell::new(Scheduler::new()));

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Sleeping,
    Done,
}

#[derive(Clone, Copy)]
pub struct Task {
    pub id: usize,
    pub weight: u32,
    pub latency: u8,
    pub vruntime: u64,
    pub deadline: u64,
    pub burst: u64,
    pub state: TaskState,
}

impl Task {
    pub const fn new(id: usize, weight: u32, latency: u8) -> Self {
        Self {
            id,
            weight,
            latency,
            vruntime: 0,
            deadline: 0,
            burst: 0,
            state: TaskState::Ready,
        }
    }
}

pub struct Scheduler {
    tasks: [Option<Task>; MAX_TASKS],
    len: usize,
    clock: u64,
    current: Option<usize>,
    timer_ticks: u64,
    switches: u64,
}

#[derive(Clone, Copy)]
pub struct Status {
    pub current: Option<usize>,
    pub next: Option<usize>,
    pub clock: u64,
    pub timer_ticks: u64,
    pub tasks: usize,
    pub switches: u64,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            tasks: [None; MAX_TASKS],
            len: 0,
            clock: 0,
            current: None,
            timer_ticks: 0,
            switches: 0,
        }
    }

    pub fn add(&mut self, mut task: Task) -> bool {
        if self.len == MAX_TASKS
            || task.weight == 0
            || self.tasks[..self.len]
                .iter()
                .flatten()
                .any(|existing| existing.id == task.id)
        {
            return false;
        }
        let Some(deadline) = self.clock.checked_add(slice_for(task)) else {
            return false;
        };
        task.deadline = deadline;
        self.tasks[self.len] = Some(task);
        self.len += 1;
        true
    }

    pub fn pick(&self) -> Option<usize> {
        let mut best = None;
        let mut best_key = u64::MAX;

        for slot in &self.tasks[..self.len] {
            let Some(task) = slot.as_ref() else {
                continue;
            };
            if task.state != TaskState::Ready {
                continue;
            }
            let key = task.deadline.saturating_add(task.burst / 2);
            if key < best_key {
                best_key = key;
                best = Some(task.id);
            }
        }

        best
    }

    pub fn clock(&self) -> u64 {
        self.clock
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn charge(&mut self, id: usize, runtime: u64, slept: bool) -> bool {
        for slot in &mut self.tasks[..self.len] {
            let Some(task) = slot.as_mut() else { continue };
            if task.id != id {
                continue;
            }

            let weighted = runtime.saturating_mul(1024) / task.weight as u64;
            task.vruntime = task.vruntime.saturating_add(weighted);
            task.burst = if slept {
                task.burst / 2
            } else {
                task.burst.saturating_add(runtime)
            };
            self.clock = self.clock.saturating_add(runtime);
            task.deadline = task
                .vruntime
                .saturating_add(slice_for(*task))
                .saturating_add(task.burst / 4);
            return true;
        }
        false
    }

    fn select_initial(&mut self) {
        self.current = self.pick();
    }

    fn select_next(&mut self) {
        let next = self.pick();
        if next != self.current {
            self.switches = self.switches.saturating_add(1);
        }
        self.current = next;
    }
}

fn slice_for(task: Task) -> u64 {
    let latency_bonus = (task.latency as u64).min(3);
    BASE_SLICE
        .saturating_sub(latency_bonus)
        .max(INTERACTIVE_SLICE)
}

pub fn self_check() {
    let mut sched = Scheduler::new();
    assert!(sched.add(Task::new(1, 1024, 3)));
    assert!(sched.add(Task::new(2, 1024, 0)));
    assert!(!sched.add(Task::new(1, 1024, 0)));
    assert!(!sched.add(Task::new(3, 0, 0)));
    let _ = TaskState::Sleeping;
    let _ = TaskState::Done;
    assert_eq!(sched.pick(), Some(1));
    assert!(sched.charge(1, 12, false));
    assert_eq!(sched.pick(), Some(2));
    assert!(sched.charge(2, 4, true));
    assert_eq!(sched.pick(), Some(2));
    assert!(!sched.charge(99, 1, false));

    let mut capacity = Scheduler::new();
    for id in 1..=MAX_TASKS {
        assert!(capacity.add(Task::new(id, 1024, 0)));
    }
    assert!(!capacity.add(Task::new(MAX_TASKS + 1, 1024, 0)));

    let mut accounting = Scheduler::new();
    assert!(accounting.add(Task::new(7, 1024, 0)));
    assert!(accounting.add(Task::new(8, 1024, 0)));
    let mut seen = 0u8;
    for _ in 0..8 {
        let current = accounting.pick().unwrap();
        seen |= if current == 7 { 1 } else { 2 };
        assert!(accounting.charge(current, 1, false));
    }
    assert_eq!(seen, 3);
    assert_eq!(accounting.clock(), 8);

    let mut saturated = Scheduler::new();
    assert!(saturated.add(Task::new(1, 1024, 0)));
    assert!(saturated.charge(1, u64::MAX, false));
    assert_eq!(saturated.clock(), u64::MAX);
    assert_eq!(saturated.pick(), Some(1));
    assert!(!saturated.add(Task::new(2, 1024, 0)));
}

pub fn init_runtime() {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        *runtime = Scheduler::new();
        let _ = runtime.add(Task::new(1, 1024, 3));
        let _ = runtime.add(Task::new(2, 1024, 1));
        runtime.select_initial();
    });
}

pub fn on_timer_tick() {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        runtime.timer_ticks = runtime.timer_ticks.saturating_add(1);
        crate::process::wake_sleepers(crate::time::ticks());
        if let Some(id) = runtime.current {
            let _ = runtime.charge(id, 1, false);
        }
        runtime.select_next();
    });
}

pub fn runtime_self_check(hardware_ticks: bool) -> bool {
    let (before, before_irq) = snapshot();
    if crate::time::scheduler_hz() != 100
        || before.tasks < 2
        || before.current.is_none()
        || before.next.is_none()
    {
        return false;
    }
    let probe_limit = if hardware_ticks {
        HARDWARE_PROBE_SPINS
    } else {
        POLLING_PROBE_SPINS
    };
    let mut spins = 0;
    while spins < probe_limit {
        if hardware_ticks {
            let _ = crate::irq::run_deferred();
            spins = spins.saturating_add(1);
        } else {
            for _ in 0..PROBE_BATCH_SPINS {
                core::hint::spin_loop();
            }
            if crate::time::poll_scheduler_tick() {
                on_timer_tick();
            }
            spins = spins.saturating_add(PROBE_BATCH_SPINS);
        }
        if status().timer_ticks.saturating_sub(before.timer_ticks) >= RUNTIME_PROBE_TICKS {
            break;
        }
    }
    let (after, after_irq) = snapshot();
    let advanced = after.timer_ticks.saturating_sub(before.timer_ticks);
    let irq_delta = after_irq.timer.saturating_sub(before_irq.timer);
    let timer_conservation = before_irq.timer_pending.saturating_add(irq_delta)
        == after_irq.timer_pending.saturating_add(advanced);
    advanced >= RUNTIME_PROBE_TICKS
        && after.clock.saturating_sub(before.clock) == advanced
        && after.switches > before.switches
        && after.current.is_some()
        && after.next.is_some()
        && (!hardware_ticks || (irq_delta >= advanced && timer_conservation))
}

fn snapshot() -> (Status, crate::irq::Stats) {
    crate::arch::without_interrupts(|| (status(), crate::irq::stats()))
}

pub fn status() -> Status {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &*RUNTIME.0.get();
        Status {
            current: runtime.current,
            next: runtime.pick(),
            clock: runtime.clock(),
            timer_ticks: runtime.timer_ticks,
            tasks: runtime.len(),
            switches: runtime.switches,
        }
    })
}
