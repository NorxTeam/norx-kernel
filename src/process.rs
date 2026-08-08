use core::cell::UnsafeCell;

use crate::address::PhysAddr;

const MAX_PROCESSES: usize = 32;
const MAX_THREADS: usize = 64;
const MAX_PROCESS_THREADS: usize = 4;
const MAX_FDS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessId(u32);

impl ProcessId {
    pub const INIT: Self = Self(1);

    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    fn new(slot: usize, generation: u16) -> Self {
        Self((generation as u32) << 16 | (slot as u32 + 1))
    }

    fn slot(self) -> Option<usize> {
        let slot = (self.0 & 0xffff) as usize;
        (slot != 0 && slot <= MAX_PROCESSES).then_some(slot - 1)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadId(u32);

impl ThreadId {
    fn new(slot: usize, generation: u16) -> Self {
        Self((generation as u32) << 16 | (slot as u32 + 1))
    }

    fn slot(self) -> Option<usize> {
        let slot = (self.0 & 0xffff) as usize;
        (slot != 0 && slot <= MAX_THREADS).then_some(slot - 1)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileDescriptor(u32);

impl FileDescriptor {
    fn new(slot: usize) -> Self {
        Self(slot as u32)
    }

    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    fn slot(self) -> Option<usize> {
        (self.0 < MAX_FDS as u32).then_some(self.0 as usize)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    Creating,
    Running,
    Exiting,
    Zombie,
    Reaped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadState {
    Created,
    Ready,
    Running,
    Blocked,
    Sleeping,
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadKind {
    Kernel,
    User,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextSwitch {
    pub from: Option<ThreadId>,
    pub to: ThreadId,
    pub to_kind: ThreadKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub real_uid: u32,
    pub effective_uid: u32,
    pub saved_uid: u32,
    pub real_gid: u32,
    pub effective_gid: u32,
    pub saved_gid: u32,
    pub capabilities: u64,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Mount = 0,
    RawIo = 1,
    NetAdmin = 2,
    NetRaw = 3,
    MemoryMap = 4,
    DeviceAdmin = 5,
}

impl Credentials {
    pub const BOOTSTRAP: Self = Self {
        real_uid: 0,
        effective_uid: 0,
        saved_uid: 0,
        real_gid: 0,
        effective_gid: 0,
        saved_gid: 0,
        capabilities: u64::MAX,
    };

    pub const fn has_capability(self, capability: u8) -> bool {
        capability < 64 && self.capabilities & (1u64 << capability) != 0
    }

    pub const fn authorize(self, capability: Capability) -> Result<(), Error> {
        if self.has_capability(capability as u8) {
            Ok(())
        } else {
            Err(Error::PermissionDenied)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FdEntry {
    open_file: u32,
    close_on_exec: bool,
    nonblocking: bool,
    readable: bool,
    writable: bool,
}

const fn standard_fds() -> [Option<FdEntry>; MAX_FDS] {
    let mut fds = [None; MAX_FDS];
    fds[0] = Some(FdEntry {
        open_file: 0,
        close_on_exec: false,
        nonblocking: true,
        readable: true,
        writable: false,
    });
    fds[1] = Some(FdEntry {
        open_file: 1,
        close_on_exec: false,
        nonblocking: false,
        readable: false,
        writable: true,
    });
    fds[2] = Some(FdEntry {
        open_file: 2,
        close_on_exec: false,
        nonblocking: false,
        readable: false,
        writable: true,
    });
    fds
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProcessRecord {
    id: ProcessId,
    parent: Option<ProcessId>,
    state: ProcessState,
    credentials: Credentials,
    exit_status: Option<i32>,
    pending_signals: u64,
    pending_events: u64,
    address_space_root: Option<PhysAddr>,
    threads: [Option<ThreadId>; MAX_PROCESS_THREADS],
    fds: [Option<FdEntry>; MAX_FDS],
}

impl ProcessRecord {
    const fn new(id: ProcessId, parent: Option<ProcessId>, credentials: Credentials) -> Self {
        Self {
            id,
            parent,
            state: ProcessState::Creating,
            credentials,
            exit_status: None,
            pending_signals: 0,
            pending_events: 0,
            address_space_root: None,
            threads: [None; MAX_PROCESS_THREADS],
            fds: standard_fds(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ThreadRecord {
    id: ThreadId,
    process: Option<ProcessId>,
    state: ThreadState,
    kind: ThreadKind,
    runtime_ticks: u64,
    preemptions: u64,
    context_switches: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidId,
    InvalidState,
    ProcessCapacity,
    ThreadCapacity,
    FdCapacity,
    InvalidFd,
    NoChild,
    GenerationExhausted,
    PermissionDenied,
}

pub struct ProcessTable {
    processes: [Option<ProcessRecord>; MAX_PROCESSES],
    process_generations: [u16; MAX_PROCESSES],
    threads: [Option<ThreadRecord>; MAX_THREADS],
    thread_generations: [u16; MAX_THREADS],
}

impl ProcessTable {
    pub const fn new() -> Self {
        Self {
            processes: [None; MAX_PROCESSES],
            process_generations: [0; MAX_PROCESSES],
            threads: [None; MAX_THREADS],
            thread_generations: [0; MAX_THREADS],
        }
    }

    pub fn create_init(&mut self) -> Result<(ProcessId, ThreadId), Error> {
        if self.processes[0].is_some() {
            return Err(Error::InvalidState);
        }
        let process = ProcessId::new(0, self.process_generations[0]);
        self.processes[0] = Some(ProcessRecord::new(process, None, Credentials::BOOTSTRAP));
        let thread = match self.allocate_thread(Some(process), ThreadKind::User) {
            Ok(thread) => thread,
            Err(error) => {
                self.processes[0] = None;
                return Err(error);
            }
        };
        self.process_mut(process)?.state = ProcessState::Running;
        self.thread_mut(thread)?.state = ThreadState::Ready;
        Ok((process, thread))
    }

    pub fn spawn_child(
        &mut self,
        parent: ProcessId,
        credentials: Credentials,
    ) -> Result<(ProcessId, ThreadId), Error> {
        if self.process(parent)?.state != ProcessState::Running {
            return Err(Error::InvalidState);
        }
        let slot = self
            .processes
            .iter()
            .position(Option::is_none)
            .ok_or(Error::ProcessCapacity)?;
        let process = ProcessId::new(slot, self.process_generations[slot]);
        self.processes[slot] = Some(ProcessRecord::new(process, Some(parent), credentials));
        let thread = match self.allocate_thread(Some(process), ThreadKind::User) {
            Ok(thread) => thread,
            Err(error) => {
                self.processes[slot] = None;
                return Err(error);
            }
        };
        self.process_mut(process)?.state = ProcessState::Running;
        self.thread_mut(thread)?.state = ThreadState::Ready;
        Ok((process, thread))
    }

    pub fn spawn_kernel_thread(&mut self) -> Result<ThreadId, Error> {
        let thread = self.allocate_thread(None, ThreadKind::Kernel)?;
        self.thread_mut(thread)?.state = ThreadState::Ready;
        Ok(thread)
    }

    pub fn exit(&mut self, process: ProcessId, status: i32) -> Result<(), Error> {
        if process == ProcessId::INIT || self.process(process)?.state != ProcessState::Running {
            return Err(Error::InvalidState);
        }
        {
            let record = self.process_mut(process)?;
            record.state = ProcessState::Exiting;
            record.exit_status = Some(status);
            record.fds = [None; MAX_FDS];
        }
        for thread in &mut self.threads {
            let Some(thread) = thread.as_mut() else {
                continue;
            };
            if thread.process == Some(process) {
                thread.state = ThreadState::Exited;
            }
        }
        for record in &mut self.processes {
            let Some(record) = record.as_mut() else {
                continue;
            };
            if record.parent == Some(process) {
                record.parent = Some(ProcessId::INIT);
            }
        }
        self.process_mut(process)?.state = ProcessState::Zombie;
        Ok(())
    }

    pub fn wait(
        &mut self,
        parent: ProcessId,
        child: Option<ProcessId>,
    ) -> Result<(ProcessId, i32), Error> {
        self.process(parent)?;
        let candidate = if let Some(child) = child {
            let record = self.process(child)?;
            if record.parent != Some(parent) || record.state != ProcessState::Zombie {
                return Err(Error::NoChild);
            }
            child
        } else {
            self.processes
                .iter()
                .flatten()
                .find(|record| {
                    record.parent == Some(parent) && record.state == ProcessState::Zombie
                })
                .map(|record| record.id)
                .ok_or(Error::NoChild)?
        };
        let record = *self.process(candidate)?;
        let status = record.exit_status.ok_or(Error::InvalidState)?;
        self.reap(candidate)?;
        Ok((candidate, status))
    }

    pub fn open_fd(
        &mut self,
        process: ProcessId,
        open_file: u32,
        readable: bool,
        writable: bool,
    ) -> Result<FileDescriptor, Error> {
        if self.process(process)?.state != ProcessState::Running {
            return Err(Error::InvalidState);
        }
        let record = self.process_mut(process)?;
        let slot = record
            .fds
            .iter()
            .position(Option::is_none)
            .ok_or(Error::FdCapacity)?;
        record.fds[slot] = Some(FdEntry {
            open_file,
            close_on_exec: false,
            nonblocking: false,
            readable,
            writable,
        });
        Ok(FileDescriptor::new(slot))
    }

    pub fn close_fd(&mut self, process: ProcessId, fd: FileDescriptor) -> Result<(), Error> {
        let slot = fd.slot().ok_or(Error::InvalidFd)?;
        let record = self.process_mut(process)?;
        record.fds[slot].take().ok_or(Error::InvalidFd).map(|_| ())
    }

    pub fn set_close_on_exec(
        &mut self,
        process: ProcessId,
        fd: FileDescriptor,
        enabled: bool,
    ) -> Result<(), Error> {
        let slot = fd.slot().ok_or(Error::InvalidFd)?;
        let entry = self.process_mut(process)?.fds[slot]
            .as_mut()
            .ok_or(Error::InvalidFd)?;
        entry.close_on_exec = enabled;
        Ok(())
    }

    pub fn close_on_exec(&mut self, process: ProcessId) -> Result<usize, Error> {
        let record = self.process_mut(process)?;
        let mut closed = 0;
        for entry in &mut record.fds {
            if entry.is_some_and(|entry| entry.close_on_exec) {
                *entry = None;
                closed += 1;
            }
        }
        Ok(closed)
    }

    pub fn fd_info(
        &self,
        process: ProcessId,
        fd: FileDescriptor,
    ) -> Result<(u32, bool, bool, bool, bool), Error> {
        let slot = fd.slot().ok_or(Error::InvalidFd)?;
        let entry = self.process(process)?.fds[slot].ok_or(Error::InvalidFd)?;
        Ok((
            entry.open_file,
            entry.close_on_exec,
            entry.nonblocking,
            entry.readable,
            entry.writable,
        ))
    }

    pub fn fd_access(
        &self,
        process: ProcessId,
        fd: FileDescriptor,
    ) -> Result<(bool, bool, bool), Error> {
        let slot = fd.slot().ok_or(Error::InvalidFd)?;
        let entry = self.process(process)?.fds[slot].ok_or(Error::InvalidFd)?;
        Ok((entry.nonblocking, entry.readable, entry.writable))
    }

    pub fn raise_signal(&mut self, process: ProcessId, signal: u8) -> Result<(), Error> {
        if signal >= 64 {
            return Err(Error::InvalidState);
        }
        self.process_mut(process)?.pending_signals |= 1u64 << signal;
        Ok(())
    }

    pub fn signal_pending(&self, process: ProcessId, signal: u8) -> Result<bool, Error> {
        if signal >= 64 {
            return Err(Error::InvalidState);
        }
        Ok(self.process(process)?.pending_signals & (1u64 << signal) != 0)
    }

    pub fn raise_event(&mut self, process: ProcessId, event: u8) -> Result<(), Error> {
        if event >= 64 {
            return Err(Error::InvalidState);
        }
        self.process_mut(process)?.pending_events |= 1u64 << event;
        Ok(())
    }

    pub fn credentials(&self, process: ProcessId) -> Result<Credentials, Error> {
        Ok(self.process(process)?.credentials)
    }

    pub fn process_state(&self, process: ProcessId) -> Result<ProcessState, Error> {
        Ok(self.process(process)?.state)
    }

    pub fn attach_address_space(
        &mut self,
        process: ProcessId,
        root: PhysAddr,
    ) -> Result<(), Error> {
        let record = self.process_mut(process)?;
        if record.state != ProcessState::Running {
            return Err(Error::InvalidState);
        }
        record.address_space_root = Some(root);
        Ok(())
    }

    pub fn address_space_root(&self, process: ProcessId) -> Result<Option<PhysAddr>, Error> {
        Ok(self.process(process)?.address_space_root)
    }

    pub fn clear_address_space(&mut self, process: ProcessId) -> Result<(), Error> {
        self.process_mut(process)?.address_space_root = None;
        Ok(())
    }

    fn process_thread(&self, process: ProcessId) -> Result<ThreadId, Error> {
        self.process(process)?
            .threads
            .into_iter()
            .flatten()
            .next()
            .ok_or(Error::InvalidState)
    }

    pub fn authorize(&self, process: ProcessId, capability: Capability) -> Result<(), Error> {
        self.credentials(process)?.authorize(capability)
    }

    pub fn thread_state(&self, thread: ThreadId) -> Result<ThreadState, Error> {
        Ok(self.thread(thread)?.state)
    }

    pub fn thread_kind(&self, thread: ThreadId) -> Result<ThreadKind, Error> {
        Ok(self.thread(thread)?.kind)
    }

    pub fn thread_owner(&self, thread: ThreadId) -> Result<Option<ProcessId>, Error> {
        Ok(self.thread(thread)?.process)
    }

    pub fn account_thread(&mut self, thread: ThreadId, ticks: u64) -> Result<(), Error> {
        let record = self.thread_mut(thread)?;
        if record.state != ThreadState::Running {
            return Err(Error::InvalidState);
        }
        record.runtime_ticks = record.runtime_ticks.saturating_add(ticks);
        Ok(())
    }

    pub fn block_thread(&mut self, thread: ThreadId, sleeping: bool) -> Result<(), Error> {
        let record = self.thread_mut(thread)?;
        if !matches!(record.state, ThreadState::Ready | ThreadState::Running) {
            return Err(Error::InvalidState);
        }
        record.state = if sleeping {
            ThreadState::Sleeping
        } else {
            ThreadState::Blocked
        };
        Ok(())
    }

    pub fn wake_thread(&mut self, thread: ThreadId) -> Result<(), Error> {
        let record = self.thread_mut(thread)?;
        if !matches!(record.state, ThreadState::Blocked | ThreadState::Sleeping) {
            return Err(Error::InvalidState);
        }
        record.state = ThreadState::Ready;
        Ok(())
    }

    pub fn preempt_thread(&mut self, thread: ThreadId) -> Result<(), Error> {
        let record = self.thread_mut(thread)?;
        if record.state != ThreadState::Running {
            return Err(Error::InvalidState);
        }
        record.state = ThreadState::Ready;
        record.preemptions = record.preemptions.saturating_add(1);
        Ok(())
    }

    pub fn switch_to(
        &mut self,
        from: Option<ThreadId>,
        to: ThreadId,
    ) -> Result<ContextSwitch, Error> {
        if from == Some(to) {
            return Err(Error::InvalidState);
        }
        if let Some(from) = from {
            if self.thread(from)?.state != ThreadState::Running {
                return Err(Error::InvalidState);
            }
        }
        let to_kind = self.thread(to)?.kind;
        if self.thread(to)?.state != ThreadState::Ready {
            return Err(Error::InvalidState);
        }
        if let Some(from) = from {
            self.thread_mut(from)?.state = ThreadState::Ready;
        }
        let next = self.thread_mut(to)?;
        next.state = ThreadState::Running;
        next.context_switches = next.context_switches.saturating_add(1);
        Ok(ContextSwitch { from, to, to_kind })
    }

    pub fn thread_accounting(&self, thread: ThreadId) -> Result<(u64, u64, u64), Error> {
        let record = self.thread(thread)?;
        Ok((
            record.runtime_ticks,
            record.preemptions,
            record.context_switches,
        ))
    }

    pub fn pick_ready(&self) -> Option<ThreadId> {
        self.threads
            .iter()
            .flatten()
            .find(|thread| thread.state == ThreadState::Ready)
            .map(|thread| thread.id)
    }

    fn allocate_thread(
        &mut self,
        process: Option<ProcessId>,
        kind: ThreadKind,
    ) -> Result<ThreadId, Error> {
        let slot = self
            .threads
            .iter()
            .position(Option::is_none)
            .ok_or(Error::ThreadCapacity)?;
        let thread_index = if let Some(process) = process {
            let process_slot = self.process(process)?.threads;
            Some(
                process_slot
                    .iter()
                    .position(Option::is_none)
                    .ok_or(Error::ThreadCapacity)?,
            )
        } else {
            None
        };
        let id = ThreadId::new(slot, self.thread_generations[slot]);
        self.threads[slot] = Some(ThreadRecord {
            id,
            process,
            state: ThreadState::Created,
            kind,
            runtime_ticks: 0,
            preemptions: 0,
            context_switches: 0,
        });
        if let (Some(process), Some(thread_index)) = (process, thread_index) {
            self.process_mut(process)?.threads[thread_index] = Some(id);
        }
        Ok(id)
    }

    fn reap(&mut self, process: ProcessId) -> Result<(), Error> {
        let slot = process.slot().ok_or(Error::InvalidId)?;
        let record = *self.process(process)?;
        if record.state != ProcessState::Zombie {
            return Err(Error::InvalidState);
        }
        for thread in record.threads.into_iter().flatten() {
            let thread_slot = thread.slot().ok_or(Error::InvalidId)?;
            if self.threads[thread_slot].map(|entry| entry.id) != Some(thread) {
                return Err(Error::InvalidId);
            }
            self.threads[thread_slot] = None;
            self.thread_generations[thread_slot] = self.thread_generations[thread_slot]
                .checked_add(1)
                .ok_or(Error::GenerationExhausted)?;
        }
        self.processes[slot] = None;
        self.process_generations[slot] = self.process_generations[slot]
            .checked_add(1)
            .ok_or(Error::GenerationExhausted)?;
        Ok(())
    }

    fn process(&self, id: ProcessId) -> Result<&ProcessRecord, Error> {
        let slot = id.slot().ok_or(Error::InvalidId)?;
        if self.process_generations[slot] != (id.0 >> 16) as u16 {
            return Err(Error::InvalidId);
        }
        let record = self.processes[slot].as_ref().ok_or(Error::InvalidId)?;
        (record.id == id).then_some(record).ok_or(Error::InvalidId)
    }

    fn process_mut(&mut self, id: ProcessId) -> Result<&mut ProcessRecord, Error> {
        let slot = id.slot().ok_or(Error::InvalidId)?;
        if self.process_generations[slot] != (id.0 >> 16) as u16 {
            return Err(Error::InvalidId);
        }
        let record = self.processes[slot].as_mut().ok_or(Error::InvalidId)?;
        (record.id == id).then_some(record).ok_or(Error::InvalidId)
    }

    fn thread(&self, id: ThreadId) -> Result<&ThreadRecord, Error> {
        let slot = id.slot().ok_or(Error::InvalidId)?;
        if self.thread_generations[slot] != (id.0 >> 16) as u16 {
            return Err(Error::InvalidId);
        }
        let record = self.threads[slot].as_ref().ok_or(Error::InvalidId)?;
        (record.id == id).then_some(record).ok_or(Error::InvalidId)
    }

    fn thread_mut(&mut self, id: ThreadId) -> Result<&mut ThreadRecord, Error> {
        let slot = id.slot().ok_or(Error::InvalidId)?;
        if self.thread_generations[slot] != (id.0 >> 16) as u16 {
            return Err(Error::InvalidId);
        }
        let record = self.threads[slot].as_mut().ok_or(Error::InvalidId)?;
        (record.id == id).then_some(record).ok_or(Error::InvalidId)
    }
}

struct Runtime {
    table: ProcessTable,
    init_thread: Option<ThreadId>,
    current_process: Option<ProcessId>,
    current_thread: Option<ThreadId>,
}

impl Runtime {
    const fn new() -> Self {
        Self {
            table: ProcessTable::new(),
            init_thread: None,
            current_process: None,
            current_thread: None,
        }
    }

    fn set_current(&mut self, thread: ThreadId) -> Result<(), Error> {
        self.current_thread = Some(thread);
        self.current_process = self.table.thread_owner(thread)?;
        Ok(())
    }
}

struct RuntimeCell(UnsafeCell<Runtime>);

unsafe impl Sync for RuntimeCell {}

static RUNTIME: RuntimeCell = RuntimeCell(UnsafeCell::new(Runtime::new()));

pub fn init_runtime() -> bool {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        *runtime = Runtime::new();
        let Ok((process, thread)) = runtime.table.create_init() else {
            return false;
        };
        if runtime.table.switch_to(None, thread).is_err() {
            return false;
        }
        runtime.current_process = Some(process);
        runtime.current_thread = Some(thread);
        runtime.init_thread = Some(thread);
        true
    })
}

#[allow(dead_code)]
pub fn with_process_table<R>(f: impl FnOnce(&mut ProcessTable) -> R) -> R {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        f(&mut runtime.table)
    })
}

#[allow(dead_code)]
pub fn spawn_child_current(credentials: Credentials) -> Result<(ProcessId, ThreadId), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let parent = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime.table.spawn_child(parent, credentials)
    })
}

pub fn current_process_id() -> Option<ProcessId> {
    crate::arch::without_interrupts(|| unsafe { (&*RUNTIME.0.get()).current_process })
}

pub fn attach_address_space(process: ProcessId, root: PhysAddr) -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        (&mut *RUNTIME.0.get())
            .table
            .attach_address_space(process, root)
    })
}

pub fn address_space_root(process: ProcessId) -> Result<Option<PhysAddr>, Error> {
    crate::arch::without_interrupts(|| unsafe {
        (&*RUNTIME.0.get()).table.address_space_root(process)
    })
}

pub fn clear_address_space(process: ProcessId) -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        (&mut *RUNTIME.0.get()).table.clear_address_space(process)
    })
}

pub fn restore_process(process: ProcessId) -> Result<ThreadId, Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        if runtime.current_thread.is_some() || runtime.current_process.is_some() {
            return Err(Error::InvalidState);
        }
        let thread = runtime.table.process_thread(process)?;
        runtime.table.switch_to(None, thread)?;
        runtime.current_thread = Some(thread);
        runtime.current_process = Some(process);
        Ok(thread)
    })
}

pub fn discard_child(parent: ProcessId, child: ProcessId) -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        runtime.table.exit(child, -127)?;
        let _ = runtime.table.wait(parent, Some(child))?;
        Ok(())
    })
}

#[allow(dead_code)]
pub fn switch_to_user(process: ProcessId, thread: ThreadId) -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let current = runtime.current_thread.ok_or(Error::InvalidState)?;
        if runtime.table.thread_owner(thread)? != Some(process)
            || runtime.table.thread_kind(thread)? != ThreadKind::User
        {
            return Err(Error::InvalidState);
        }
        runtime.table.switch_to(Some(current), thread)?;
        runtime.set_current(thread)
    })
}

#[allow(dead_code)]
pub fn restore_init() -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        if runtime.current_thread.is_some() {
            return Err(Error::InvalidState);
        }
        let init_thread = runtime.init_thread.ok_or(Error::InvalidState)?;
        runtime.table.switch_to(None, init_thread)?;
        runtime.current_thread = Some(init_thread);
        runtime.current_process = Some(ProcessId::INIT);
        Ok(())
    })
}

pub fn current_ids() -> Option<(u32, u32)> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &*RUNTIME.0.get();
        Some((
            runtime.current_process?.get(),
            runtime.current_thread?.get(),
        ))
    })
}

pub fn exit_current(status: i32) -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        let thread = runtime.current_thread.ok_or(Error::InvalidState)?;
        if process == ProcessId::INIT {
            runtime.table.thread_mut(thread)?.state = ThreadState::Exited;
        } else {
            if let Err(error) = runtime.table.exit(process, status) {
                if runtime.table.process_state(process) != Ok(ProcessState::Zombie) {
                    return Err(error);
                }
            }
        }
        runtime.current_process = None;
        runtime.current_thread = None;
        Ok(())
    })
}

pub fn wait_current(child: Option<u32>) -> Result<(u32, i32), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let parent = runtime.current_process.ok_or(Error::InvalidState)?;
        let child = child.map(ProcessId::from_raw);
        let (process, status) = runtime.table.wait(parent, child)?;
        Ok((process.get(), status))
    })
}

pub fn close_current(fd: u32) -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime
            .table
            .close_fd(process, FileDescriptor::from_raw(fd))
    })
}

pub fn current_fd_access(fd: u32) -> Result<(bool, bool, bool), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &*RUNTIME.0.get();
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime
            .table
            .fd_access(process, FileDescriptor::from_raw(fd))
    })
}

pub fn yield_current() -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        if runtime.current_process != Some(ProcessId::INIT) {
            // Native EL0 smoke runs synchronously. There is no scheduler
            // continuation to resume while the syscall returns to that frame.
            return Ok(());
        }
        let current = runtime.current_thread.ok_or(Error::InvalidState)?;
        let Some(next) = runtime.table.pick_ready() else {
            return Ok(());
        };
        runtime.table.switch_to(Some(current), next)?;
        runtime.set_current(next)
    })
}

pub fn sleep_current() -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let current = runtime.current_thread.ok_or(Error::InvalidState)?;
        runtime.table.block_thread(current, true)?;
        let Some(next) = runtime.table.pick_ready() else {
            runtime.table.wake_thread(current)?;
            runtime.table.switch_to(None, current)?;
            return Ok(());
        };
        runtime.current_thread = None;
        runtime.current_process = None;
        runtime.table.switch_to(None, next)?;
        runtime.set_current(next)
    })
}

pub fn contract_self_check() {
    let _ = (
        ProcessState::Reaped,
        ThreadState::Blocked,
        ThreadState::Sleeping,
    );
    let _capabilities = [
        Capability::Mount,
        Capability::RawIo,
        Capability::NetAdmin,
        Capability::NetRaw,
        Capability::MemoryMap,
        Capability::DeviceAdmin,
    ];
    let mut table = ProcessTable::new();
    let (init, init_thread) = table.create_init().unwrap();
    assert_eq!(init, ProcessId::INIT);
    assert_eq!(init.get(), 1);
    assert_eq!(table.thread_state(init_thread).unwrap(), ThreadState::Ready);
    assert_eq!(table.thread_kind(init_thread).unwrap(), ThreadKind::User);
    assert_eq!(table.thread_owner(init_thread).unwrap(), Some(init));
    assert!(table.credentials(init).unwrap().has_capability(63));
    let kernel_thread = table.spawn_kernel_thread().unwrap();
    assert_eq!(
        table.thread_kind(kernel_thread).unwrap(),
        ThreadKind::Kernel
    );
    assert_eq!(table.thread_owner(kernel_thread).unwrap(), None);

    let credentials = Credentials {
        real_uid: 1000,
        effective_uid: 1000,
        saved_uid: 1000,
        real_gid: 1000,
        effective_gid: 1000,
        saved_gid: 1000,
        capabilities: 1,
    };
    let (child, child_thread) = table.spawn_child(init, credentials).unwrap();
    assert!(child_thread.get() != 0);
    assert_eq!(
        table.switch_to(None, init_thread).unwrap().to_kind,
        ThreadKind::User
    );
    table.account_thread(init_thread, 1).unwrap();
    assert_eq!(
        table
            .switch_to(Some(init_thread), child_thread)
            .unwrap()
            .to_kind,
        ThreadKind::User
    );
    assert_eq!(table.credentials(child).unwrap().effective_uid, 1000);
    assert!(table.authorize(child, Capability::Mount).is_ok());
    assert_eq!(
        table.authorize(child, Capability::RawIo),
        Err(Error::PermissionDenied)
    );
    let uid_zero_without_capabilities = Credentials {
        capabilities: 0,
        ..Credentials::BOOTSTRAP
    };
    assert_eq!(
        uid_zero_without_capabilities.authorize(Capability::Mount),
        Err(Error::PermissionDenied)
    );
    let fd = table.open_fd(child, 7, true, false).unwrap();
    assert_eq!(fd.get(), 3);
    assert_eq!(
        table.fd_info(child, fd).unwrap(),
        (7, false, false, true, false)
    );
    table.raise_signal(child, 2).unwrap();
    table.raise_event(child, 3).unwrap();
    assert!(table.signal_pending(child, 2).unwrap());
    table.account_thread(child_thread, 4).unwrap();
    table.preempt_thread(child_thread).unwrap();
    table.block_thread(child_thread, false).unwrap();
    assert_eq!(
        table.thread_state(child_thread).unwrap(),
        ThreadState::Blocked
    );
    table.wake_thread(child_thread).unwrap();
    assert_eq!(
        table.thread_state(child_thread).unwrap(),
        ThreadState::Ready
    );
    table.switch_to(None, kernel_thread).unwrap();
    table.account_thread(kernel_thread, 2).unwrap();
    table.preempt_thread(kernel_thread).unwrap();
    assert_eq!(table.thread_accounting(kernel_thread).unwrap(), (2, 1, 1));
    table.close_fd(child, fd).unwrap();
    table.exit(child, 23).unwrap();
    assert_eq!(table.wait(init, Some(child)).unwrap(), (child, 23));
    assert_eq!(table.thread_state(child_thread), Err(Error::InvalidId));
    assert_eq!(table.wait(init, None), Err(Error::NoChild));
}
