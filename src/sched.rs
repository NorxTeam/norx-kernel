use core::cell::UnsafeCell;

const MAX_TASKS: usize = 16;
const BASE_SLICE: u64 = 4;
const INTERACTIVE_SLICE: u64 = 2;

struct RuntimeCell(UnsafeCell<Scheduler>);

unsafe impl Sync for RuntimeCell {}

static RUNTIME: RuntimeCell = RuntimeCell(UnsafeCell::new(Scheduler::new()));
static mut CURRENT: Option<usize> = None;
static mut TIMER_TICKS: u64 = 0;

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
}

#[derive(Clone, Copy)]
pub struct Status {
    pub current: Option<usize>,
    pub next: Option<usize>,
    pub clock: u64,
    pub timer_ticks: u64,
    pub tasks: usize,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            tasks: [None; MAX_TASKS],
            len: 0,
            clock: 0,
        }
    }

    pub fn add(&mut self, mut task: Task) -> bool {
        if self.len == MAX_TASKS || task.weight == 0 {
            return false;
        }
        task.deadline = self.clock + slice_for(task);
        self.tasks[self.len] = Some(task);
        self.len += 1;
        true
    }

    pub fn pick(&self) -> Option<usize> {
        let mut best = None;
        let mut best_key = u64::MAX;

        for slot in &self.tasks[..self.len] {
            let task = slot.as_ref()?;
            if task.state != TaskState::Ready {
                continue;
            }
            let key = task.deadline + task.burst / 2;
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

    pub fn charge(&mut self, id: usize, runtime: u64, slept: bool) {
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
            task.deadline = task.vruntime + slice_for(*task) + task.burst / 4;
            return;
        }
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
    let _ = TaskState::Sleeping;
    let _ = TaskState::Done;
    assert_eq!(sched.pick(), Some(1));
    sched.charge(1, 12, false);
    assert_eq!(sched.pick(), Some(2));
    sched.charge(2, 4, true);
    assert_eq!(sched.pick(), Some(2));
}

pub fn init_runtime() {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        *runtime = Scheduler::new();
        let _ = runtime.add(Task::new(1, 1024, 3));
        let _ = runtime.add(Task::new(2, 1024, 1));
        CURRENT = runtime.pick();
        TIMER_TICKS = 0;
    });
}

pub fn on_timer_tick() {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        TIMER_TICKS = TIMER_TICKS.saturating_add(1);
        if let Some(id) = CURRENT {
            runtime.charge(id, 1, false);
        }
        CURRENT = runtime.pick();
    });
}

pub fn status() -> Status {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &*RUNTIME.0.get();
        Status {
            current: CURRENT,
            next: runtime.pick(),
            clock: runtime.clock(),
            timer_ticks: TIMER_TICKS,
            tasks: runtime.len(),
        }
    })
}
