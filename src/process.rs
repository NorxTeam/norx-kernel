#![allow(
    dead_code,
    reason = "Staged ABI and hardware hooks are not all enabled in the bootstrap profile"
)]

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::address::PhysAddr;
use crate::signal::{
    self, DefaultAction, MaskHow, SignalAction, SignalDelivery, SignalError, SignalReturn,
    SignalState, ThreadSignalState, UserSignalContext, SA_NOCLDSTOP, SA_RESETHAND, SIGCHLD,
    SIGCONT, SIGKILL, SIGSTOP,
};

const MAX_PROCESSES: usize = 32;
const MAX_THREADS: usize = 64;
const MAX_PROCESS_THREADS: usize = 4;
const MAX_FDS: usize = 32;
const RUNTIME_LOCK_SPIN_LIMIT: usize = 1_000_000;

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
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }

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
    Stopped,
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
    Stopped,
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadKind {
    Kernel,
    User,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserContext {
    pub registers: [u64; 35],
}

impl UserContext {
    pub const EMPTY: Self = Self { registers: [0; 35] };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextSwitch {
    pub from: Option<ThreadId>,
    pub to: ThreadId,
    pub to_kind: ThreadKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockCurrentResult {
    TimedOut,
    Blocked {
        token: crate::wait::WaitToken,
        switch: ContextSwitch,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct SpawnTransaction {
    pub parent: ProcessId,
    pub child: ProcessId,
    pub thread: ThreadId,
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

const NO_TARGET_UID: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionCapability {
    process: ProcessId,
    generation: u32,
}

impl SessionCapability {
    pub fn from_raw(raw: u64) -> Option<Self> {
        let process = ProcessId::from_raw(raw as u32);
        let generation = (raw >> 32) as u32;
        if generation == 0 || process.slot().is_none() {
            None
        } else {
            Some(Self {
                process,
                generation,
            })
        }
    }

    pub const fn raw(self) -> u64 {
        ((self.generation as u64) << 32) | self.process.get() as u64
    }
}

pub const MAX_SESSION_CWD: usize = 256;

const fn default_session_cwd() -> [u8; MAX_SESSION_CWD] {
    let mut cwd = [0; MAX_SESSION_CWD];
    cwd[0] = b'/';
    cwd
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionState {
    pub cwd: [u8; MAX_SESSION_CWD],
    pub cwd_length: u16,
    pub umask: u16,
    pub max_fds: u32,
    pub max_address_space_pages: u32,
    pub max_cpu_ticks: u64,
}

impl SessionState {
    pub const DEFAULT: Self = Self {
        cwd: default_session_cwd(),
        cwd_length: 1,
        umask: 0,
        max_fds: MAX_FDS as u32,
        max_address_space_pages: 16 * 1024,
        max_cpu_ticks: u64::MAX,
    };

    pub fn validate(self) -> Result<(), Error> {
        let length = self.cwd_length as usize;
        if self.cwd_length == 0
            || length > MAX_SESSION_CWD
            || self.cwd[0] != b'/'
            || self.cwd[..length].contains(&0)
            || self.cwd[length..].iter().any(|byte| *byte != 0)
            || self.umask & !0o777 != 0
            || self.max_fds < 3
            || self.max_fds > MAX_FDS as u32
            || self.max_address_space_pages == 0
            || self.max_cpu_ticks == 0
        {
            return Err(Error::InvalidSession);
        }
        Ok(())
    }
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
    SessionAdmin = 6,
    PrivilegeDelegation = 7,
    AccountAdmin = 8,
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
    pgid: ProcessId,
    sid: ProcessId,
    state: ProcessState,
    credentials: Credentials,
    session: SessionState,
    session_generation: u32,
    exit_status: Option<i32>,
    signals: SignalState,
    pending_events: u64,
    address_space_root: Option<PhysAddr>,
    threads: [Option<ThreadId>; MAX_PROCESS_THREADS],
    fds: [Option<FdEntry>; MAX_FDS],
}

impl ProcessRecord {
    const fn new(
        id: ProcessId,
        parent: Option<ProcessId>,
        pgid: ProcessId,
        credentials: Credentials,
    ) -> Self {
        Self {
            id,
            parent,
            pgid,
            sid: id,
            state: ProcessState::Creating,
            credentials,
            session: SessionState::DEFAULT,
            session_generation: 1,
            exit_status: None,
            signals: SignalState::new(),
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
    wake_at: u64,
    kind: ThreadKind,
    runtime_ticks: u64,
    preemptions: u64,
    context_switches: u64,
    user_context: UserContext,
    user_context_valid: bool,
    signals: ThreadSignalState,
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
    InvalidSession,
    SignalFrameCapacity,
    InvalidSignalFrame,
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
        self.processes[0] = Some(ProcessRecord::new(
            process,
            None,
            process,
            Credentials::BOOTSTRAP,
        ));
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
        self.spawn_child_with_state(parent, credentials, true)
    }

    pub fn spawn_child_staged(
        &mut self,
        parent: ProcessId,
        credentials: Credentials,
    ) -> Result<(ProcessId, ThreadId), Error> {
        self.spawn_child_with_state(parent, credentials, false)
    }

    fn spawn_child_with_state(
        &mut self,
        parent: ProcessId,
        credentials: Credentials,
        publish: bool,
    ) -> Result<(ProcessId, ThreadId), Error> {
        if self.process(parent)?.state != ProcessState::Running {
            return Err(Error::InvalidState);
        }
        let parent_group = self.process(parent)?.pgid;
        let parent_session_id = self.process(parent)?.sid;
        let parent_session = self.process(parent)?.session;
        let parent_signal_state = self.process(parent)?.signals.inherit();
        let parent_thread = self.process_thread(parent)?;
        let parent_signal_mask = self.thread(parent_thread)?.signals.inherit();
        let slot = self
            .processes
            .iter()
            .enumerate()
            .find_map(|(slot, process)| {
                (process.is_none() && self.process_generations[slot] != u16::MAX).then_some(slot)
            })
            .ok_or(Error::ProcessCapacity)?;
        let process = ProcessId::new(slot, self.process_generations[slot]);
        self.processes[slot] = Some(ProcessRecord::new(
            process,
            Some(parent),
            parent_group,
            credentials,
        ));
        self.process_mut(process)?.sid = parent_session_id;
        self.process_mut(process)?.session = parent_session;
        self.process_mut(process)?.signals = parent_signal_state;
        let thread = match self.allocate_thread(Some(process), ThreadKind::User) {
            Ok(thread) => thread,
            Err(error) => {
                self.processes[slot] = None;
                return Err(error);
            }
        };
        self.thread_mut(thread)?.signals = parent_signal_mask;
        if publish {
            self.publish_child(process, thread)?;
        }
        Ok((process, thread))
    }

    pub fn publish_child(&mut self, process: ProcessId, thread: ThreadId) -> Result<(), Error> {
        if self.process(process)?.state != ProcessState::Creating
            || self.thread_owner(thread)? != Some(process)
            || self.thread_state(thread)? != ThreadState::Created
        {
            return Err(Error::InvalidState);
        }
        self.process_mut(process)?.state = ProcessState::Running;
        self.thread_mut(thread)?.state = ThreadState::Ready;
        Ok(())
    }

    pub fn finalize_spawn(
        &mut self,
        transaction: SpawnTransaction,
        source_fds: [FileDescriptor; 3],
        inherit_open_fds: bool,
        process_group: Option<ProcessId>,
    ) -> Result<(), Error> {
        let SpawnTransaction {
            parent,
            child,
            thread,
        } = transaction;
        let result = if inherit_open_fds {
            self.inherit_open_fds(parent, child, source_fds)
        } else {
            self.inherit_standard_fds(parent, child, source_fds)
        }
        .and_then(|()| match process_group {
            Some(process_group) => self.set_process_group_for_child(parent, child, process_group),
            None => Ok(()),
        })
        .and_then(|()| self.publish_child(child, thread));
        if let Err(error) = result {
            return match self.abort_child(parent, child) {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(rollback_error),
            };
        }
        Ok(())
    }

    pub fn abort_child(&mut self, parent: ProcessId, child: ProcessId) -> Result<(), Error> {
        let record = *self.process(child)?;
        if record.parent != Some(parent) || record.state != ProcessState::Creating {
            return Err(Error::InvalidState);
        }
        let process_slot = child.slot().ok_or(Error::InvalidId)?;
        if self.process_generations[process_slot] == u16::MAX {
            return Err(Error::GenerationExhausted);
        }
        let mut thread_slots = [None; MAX_PROCESS_THREADS];
        for (index, thread) in record.threads.into_iter().enumerate() {
            let Some(thread) = thread else {
                continue;
            };
            let slot = thread.slot().ok_or(Error::InvalidId)?;
            if self.threads[slot].map(|entry| entry.id) != Some(thread)
                || self.thread_generations[slot] == u16::MAX
            {
                return Err(Error::InvalidState);
            }
            thread_slots[index] = Some(thread);
        }
        for entry in record.fds.into_iter().flatten() {
            release_open_file(entry.open_file);
        }
        for thread in thread_slots.into_iter().flatten() {
            let slot = thread.slot().ok_or(Error::InvalidId)?;
            self.threads[slot] = None;
            self.thread_generations[slot] += 1;
        }
        self.processes[process_slot] = None;
        self.process_generations[process_slot] += 1;
        Ok(())
    }

    pub fn spawn_kernel_thread(&mut self) -> Result<ThreadId, Error> {
        let thread = self.allocate_thread(None, ThreadKind::Kernel)?;
        self.thread_mut(thread)?.state = ThreadState::Ready;
        Ok(thread)
    }

    pub fn exit(&mut self, process: ProcessId, status: i32) -> Result<(), Error> {
        if process == ProcessId::INIT
            || !matches!(
                self.process(process)?.state,
                ProcessState::Running | ProcessState::Stopped
            )
        {
            return Err(Error::InvalidState);
        }
        self.finish_exit(process, status, None)
    }

    fn wait_candidate(
        &self,
        parent: ProcessId,
        child: Option<ProcessId>,
    ) -> Result<ProcessId, Error> {
        self.process(parent)?;
        if let Some(child) = child {
            let record = self.process(child)?;
            if record.parent != Some(parent) || record.state != ProcessState::Zombie {
                return Err(Error::NoChild);
            }
            Ok(child)
        } else {
            self.processes
                .iter()
                .flatten()
                .find(|record| {
                    record.parent == Some(parent) && record.state == ProcessState::Zombie
                })
                .map(|record| record.id)
                .ok_or(Error::NoChild)
        }
    }

    pub fn peek_wait(
        &self,
        parent: ProcessId,
        child: Option<ProcessId>,
    ) -> Result<(ProcessId, i32), Error> {
        let candidate = self.wait_candidate(parent, child)?;
        let record = *self.process(candidate)?;
        let status = record.exit_status.ok_or(Error::InvalidState)?;
        Ok((candidate, status))
    }

    pub fn wait(
        &mut self,
        parent: ProcessId,
        child: Option<ProcessId>,
    ) -> Result<(ProcessId, i32), Error> {
        let (candidate, status) = self.peek_wait(parent, child)?;
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
        self.open_fd_with_flags(process, open_file, false, readable, writable)
    }

    pub fn open_fd_with_flags(
        &mut self,
        process: ProcessId,
        open_file: u32,
        nonblocking: bool,
        readable: bool,
        writable: bool,
    ) -> Result<FileDescriptor, Error> {
        validate_open_file(open_file)?;
        if self.process(process)?.state != ProcessState::Running {
            return Err(Error::InvalidState);
        }
        let record = self.process_mut(process)?;
        if record.fds.iter().flatten().count() >= record.session.max_fds as usize {
            return Err(Error::FdCapacity);
        }
        let slot = record
            .fds
            .iter()
            .position(Option::is_none)
            .ok_or(Error::FdCapacity)?;
        record.fds[slot] = Some(FdEntry {
            open_file,
            close_on_exec: false,
            nonblocking,
            readable,
            writable,
        });
        Ok(FileDescriptor::new(slot))
    }

    pub fn close_fd(&mut self, process: ProcessId, fd: FileDescriptor) -> Result<(), Error> {
        let slot = fd.slot().ok_or(Error::InvalidFd)?;
        let record = self.process_mut(process)?;
        let entry = record.fds[slot].take().ok_or(Error::InvalidFd)?;
        release_open_file(entry.open_file);
        Ok(())
    }

    pub fn duplicate_fd(
        &mut self,
        process: ProcessId,
        old_fd: FileDescriptor,
        new_fd: FileDescriptor,
    ) -> Result<FileDescriptor, Error> {
        let old_slot = old_fd.slot().ok_or(Error::InvalidFd)?;
        let new_slot = new_fd.slot().ok_or(Error::InvalidFd)?;
        if new_slot >= self.process(process)?.session.max_fds as usize {
            return Err(Error::FdCapacity);
        }
        if old_slot == new_slot {
            self.process(process)?.fds[old_slot].ok_or(Error::InvalidFd)?;
            return Ok(new_fd);
        }
        let mut entry = self.process(process)?.fds[old_slot].ok_or(Error::InvalidFd)?;
        entry.close_on_exec = false;
        duplicate_open_file(entry.open_file)?;
        let replaced = {
            let record = self.process_mut(process)?;
            let replaced = record.fds[new_slot].take();
            record.fds[new_slot] = Some(entry);
            replaced
        };
        if let Some(replaced) = replaced {
            release_open_file(replaced.open_file);
        }
        Ok(new_fd)
    }

    pub fn inherit_standard_fds(
        &mut self,
        parent: ProcessId,
        child: ProcessId,
        source_fds: [FileDescriptor; 3],
    ) -> Result<(), Error> {
        self.inherit_fds(parent, child, source_fds, false)
    }

    pub fn inherit_open_fds(
        &mut self,
        parent: ProcessId,
        child: ProcessId,
        source_fds: [FileDescriptor; 3],
    ) -> Result<(), Error> {
        self.inherit_fds(parent, child, source_fds, true)
    }

    fn inherit_fds(
        &mut self,
        parent: ProcessId,
        child: ProcessId,
        source_fds: [FileDescriptor; 3],
        inherit_open_fds: bool,
    ) -> Result<(), Error> {
        let parent_record = *self.process(parent)?;
        let child_record = *self.process(child)?;
        if parent_record.state != ProcessState::Running
            || child_record.parent != Some(parent)
            || child_record.state != ProcessState::Creating
        {
            return Err(Error::InvalidState);
        }
        let mut sources = [None; 3];
        for (index, fd) in source_fds.into_iter().enumerate() {
            let slot = fd.slot().ok_or(Error::InvalidFd)?;
            sources[index] = Some(parent_record.fds[slot].ok_or(Error::InvalidFd)?);
        }
        let mut target = [None; MAX_FDS];
        if inherit_open_fds {
            target = parent_record.fds;
        }
        for (index, source) in sources.into_iter().enumerate() {
            target[index] = source;
        }
        let max_fds = usize::try_from(child_record.session.max_fds)
            .ok()
            .filter(|max_fds| (3..=MAX_FDS).contains(max_fds))
            .ok_or(Error::InvalidSession)?;
        if target.iter().flatten().count() > max_fds {
            return Err(Error::FdCapacity);
        }
        let mut duplicated = [None; MAX_FDS];
        for (index, source) in target.into_iter().enumerate() {
            let Some(source) = source else {
                continue;
            };
            if let Err(error) = duplicate_open_file(source.open_file) {
                for open_file in duplicated.into_iter().flatten() {
                    release_open_file(open_file);
                }
                return Err(error);
            }
            duplicated[index] = Some(source.open_file);
        }
        let old = self.process_mut(child)?.fds;
        for entry in old.into_iter().flatten() {
            release_open_file(entry.open_file);
        }
        self.process_mut(child)?.fds = target;
        Ok(())
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
                let open_file = entry.expect("checked Some").open_file;
                *entry = None;
                release_open_file(open_file);
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

    fn signal_error(error: SignalError) -> Error {
        match error {
            SignalError::FrameCapacity => Error::SignalFrameCapacity,
            SignalError::StaleFrame => Error::InvalidSignalFrame,
            SignalError::FrameUnavailable
            | SignalError::TokenExhausted
            | SignalError::InvalidSignal
            | SignalError::InvalidMask
            | SignalError::InvalidHandler
            | SignalError::Uncatchable
            | SignalError::UnsupportedFlags => Error::InvalidState,
        }
    }

    fn validate_signal(signal: u8) -> Result<(), Error> {
        if signal as usize >= signal::MAX_SIGNALS {
            return Err(Error::InvalidState);
        }
        Ok(())
    }

    fn live_process(&self, process: ProcessId) -> Result<(), Error> {
        if !matches!(
            self.process(process)?.state,
            ProcessState::Running | ProcessState::Stopped
        ) {
            return Err(Error::InvalidState);
        }
        Ok(())
    }

    pub fn signal_action(&self, process: ProcessId, signal: u8) -> Result<SignalAction, Error> {
        Self::validate_signal(signal)?;
        self.process(process)?
            .signals
            .action(signal)
            .map_err(Self::signal_error)
    }

    pub fn set_signal_action(
        &mut self,
        process: ProcessId,
        signal: u8,
        action: SignalAction,
    ) -> Result<SignalAction, Error> {
        Self::validate_signal(signal)?;
        self.live_process(process)?;
        let old = self
            .process_mut(process)?
            .signals
            .install(signal, action)
            .map_err(Self::signal_error)?;
        if action.is_ignored() {
            let bit = signal::signal_bit(signal);
            for thread in self.threads.iter_mut().flatten() {
                if thread.process == Some(process) {
                    thread.signals.clear_pending_mask(bit);
                }
            }
        }
        Ok(old)
    }

    pub fn signal_mask(&self, thread: ThreadId) -> Result<u64, Error> {
        Ok(self.thread(thread)?.signals.mask())
    }

    pub fn set_signal_mask(&mut self, thread: ThreadId, mask: u64) -> Result<u64, Error> {
        self.thread_mut(thread)?
            .signals
            .set_mask(mask)
            .map_err(Self::signal_error)
    }

    pub fn change_signal_mask(
        &mut self,
        thread: ThreadId,
        how: MaskHow,
        mask: u64,
    ) -> Result<u64, Error> {
        self.thread_mut(thread)?
            .signals
            .change_mask(how, mask)
            .map_err(Self::signal_error)
    }

    pub fn set_signal_restart(
        &mut self,
        thread: ThreadId,
        restart: Option<crate::signal::RestartState>,
    ) -> Result<(), Error> {
        self.thread_mut(thread)?.signals.set_restart(restart);
        Ok(())
    }

    pub fn signal_restart(
        &self,
        thread: ThreadId,
    ) -> Result<Option<crate::signal::RestartState>, Error> {
        Ok(self.thread(thread)?.signals.restart())
    }

    pub fn raise_signal(&mut self, process: ProcessId, signal: u8) -> Result<(), Error> {
        Self::validate_signal(signal)?;
        self.live_process(process)?;
        self.process_mut(process)?
            .signals
            .set_pending(signal)
            .map_err(Self::signal_error)?;
        self.wake_signal_waiter(process, signal);
        Ok(())
    }

    pub fn raise_signal_to_thread(&mut self, thread: ThreadId, signal: u8) -> Result<(), Error> {
        Self::validate_signal(signal)?;
        let process = self.thread(thread)?.process.ok_or(Error::InvalidState)?;
        self.live_process(process)?;
        self.thread_mut(thread)?
            .signals
            .set_pending(signal)
            .map_err(Self::signal_error)?;
        self.wake_signal_waiter(process, signal);
        Ok(())
    }

    pub fn signal_pending(&self, process: ProcessId, signal: u8) -> Result<bool, Error> {
        Self::validate_signal(signal)?;
        self.process(process)?
            .signals
            .pending(signal)
            .map_err(Self::signal_error)
    }

    pub fn thread_signal_pending(&self, thread: ThreadId, signal: u8) -> Result<bool, Error> {
        Self::validate_signal(signal)?;
        self.thread(thread)?
            .signals
            .pending(signal)
            .map_err(Self::signal_error)
    }

    fn wake_signal_waiter(&mut self, process: ProcessId, signal: u8) {
        let bit = signal::signal_bit(signal);
        for thread in self.threads.iter_mut().flatten() {
            if thread.process == Some(process)
                && matches!(thread.state, ThreadState::Blocked | ThreadState::Sleeping)
                && (thread.signals.mask() & bit == 0 || signal == SIGKILL || signal == SIGSTOP)
            {
                thread.state = ThreadState::Ready;
                thread.wake_at = 0;
            }
        }
    }

    fn pending_for_thread(
        &self,
        process: ProcessId,
        thread: ThreadId,
    ) -> Result<Option<(u8, bool)>, Error> {
        let process_record = self.process(process)?;
        let thread_record = self.thread(thread)?;
        let unmaskable = signal::signal_bit(SIGKILL) | signal::signal_bit(SIGSTOP);
        let pending = (process_record.signals.pending_mask()
            | thread_record.signals.pending_mask())
            & (!thread_record.signals.mask() | unmaskable)
            & signal::SIGNAL_MASK;
        if pending == 0 {
            return Ok(None);
        }
        let signal = pending.trailing_zeros() as u8;
        let from_thread = thread_record.signals.pending_mask() & signal::signal_bit(signal) != 0;
        Ok(Some((signal, from_thread)))
    }

    pub fn deliver_pending_signal(
        &mut self,
        thread: ThreadId,
        context: UserSignalContext,
    ) -> Result<SignalDelivery, Error> {
        let process = self.thread(thread)?.process.ok_or(Error::InvalidState)?;
        self.live_process(process)?;
        let state = self.thread(thread)?.state;
        let Some((signal, from_thread)) = self.pending_for_thread(process, thread)? else {
            return Ok(SignalDelivery::NoPending);
        };
        if state == ThreadState::Stopped && signal != SIGCONT && signal != SIGKILL {
            return Ok(SignalDelivery::NoPending);
        }
        let action = self.signal_action(process, signal)?;
        if action.is_default() {
            let default = signal::default_action(signal);
            if signal == SIGKILL && process == ProcessId::INIT {
                return Err(Error::PermissionDenied);
            }
            self.clear_pending_source(thread, process, signal, from_thread);
            match default {
                DefaultAction::Ignore => {}
                DefaultAction::Terminate => self.terminate(process, signal)?,
                DefaultAction::Stop => self.stop_process(process, signal)?,
                DefaultAction::Continue => self.continue_process(process, signal)?,
            }
            return Ok(SignalDelivery::Default {
                signal,
                action: default,
            });
        }
        if action.is_ignored() {
            self.clear_pending_source(thread, process, signal, from_thread);
            if signal == SIGCONT {
                self.continue_process(process, signal)?;
            }
            return Ok(SignalDelivery::Ignored { signal });
        }

        let delivery = self
            .thread_mut(thread)?
            .signals
            .begin_handler(signal, action, context)
            .map_err(Self::signal_error)?;
        self.clear_pending_source(thread, process, signal, from_thread);
        if action.flags & SA_RESETHAND != 0 {
            self.process_mut(process)?
                .signals
                .install(signal, SignalAction::DEFAULT)
                .map_err(Self::signal_error)?;
        }
        if signal == SIGCONT {
            self.continue_process(process, signal)?;
        }
        Ok(delivery)
    }

    fn clear_pending_source(
        &mut self,
        thread: ThreadId,
        process: ProcessId,
        signal: u8,
        from_thread: bool,
    ) {
        if from_thread {
            if let Ok(thread) = self.thread_mut(thread) {
                thread.signals.clear_pending(signal);
            }
        } else {
            if let Ok(process) = self.process_mut(process) {
                process.signals.clear_pending(signal);
            }
        }
    }

    pub fn finish_signal(&mut self, thread: ThreadId, token: u32) -> Result<SignalReturn, Error> {
        self.thread_mut(thread)?
            .signals
            .finish_handler(token)
            .map_err(Self::signal_error)
    }

    pub fn signal_frame_count(&self, thread: ThreadId) -> Result<usize, Error> {
        Ok(self.thread(thread)?.signals.frame_count())
    }

    pub fn get_process_group(&self, process: ProcessId) -> Result<ProcessId, Error> {
        Ok(self.process(process)?.pgid)
    }

    pub fn get_session_id(&self, process: ProcessId) -> Result<ProcessId, Error> {
        Ok(self.process(process)?.sid)
    }

    pub fn process_group_has_live_member(&self, group: ProcessId) -> bool {
        self.processes.iter().flatten().any(|record| {
            record.pgid == group
                && matches!(
                    record.state,
                    ProcessState::Creating | ProcessState::Running | ProcessState::Stopped
                )
        })
    }

    pub fn process_group_has_live_member_in_session(
        &self,
        group: ProcessId,
        session: ProcessId,
    ) -> bool {
        self.processes.iter().flatten().any(|record| {
            record.sid == session
                && record.pgid == group
                && matches!(
                    record.state,
                    ProcessState::Creating | ProcessState::Running | ProcessState::Stopped
                )
        })
    }

    pub fn set_process_group(&mut self, process: ProcessId, pgid: ProcessId) -> Result<(), Error> {
        if pgid.get() == 0 {
            return Err(Error::InvalidId);
        }
        let record = self.process(process)?;
        if record.state != ProcessState::Running {
            return Err(Error::InvalidState);
        }
        if pgid != process && !self.process_group_has_live_member_in_session(pgid, record.sid) {
            return Err(Error::InvalidId);
        }
        self.process_mut(process)?.pgid = pgid;
        Ok(())
    }

    pub fn set_process_group_for_child(
        &mut self,
        parent: ProcessId,
        child: ProcessId,
        pgid: ProcessId,
    ) -> Result<(), Error> {
        let parent_record = *self.process(parent)?;
        let child_record = *self.process(child)?;
        if parent_record.state != ProcessState::Running
            || child_record.parent != Some(parent)
            || child_record.state != ProcessState::Creating
            || child_record.sid != parent_record.sid
        {
            return Err(Error::InvalidState);
        }
        if pgid.get() == 0 || (pgid != child && pgid != parent_record.pgid) {
            return Err(Error::PermissionDenied);
        }
        self.process_mut(child)?.pgid = pgid;
        Ok(())
    }

    pub fn authorize_process_group_change(
        &self,
        caller: ProcessId,
        process: ProcessId,
        pgid: ProcessId,
    ) -> Result<(), Error> {
        let caller_record = self.process(caller)?;
        let target_record = self.process(process)?;
        if caller_record.state != ProcessState::Running
            || target_record.state != ProcessState::Running
            || caller_record.sid != target_record.sid
        {
            return Err(Error::InvalidState);
        }
        if process != caller && target_record.parent != Some(caller) {
            return Err(Error::PermissionDenied);
        }
        if pgid.get() == 0 || (pgid != process && pgid != caller_record.pgid) {
            return Err(Error::PermissionDenied);
        }
        Ok(())
    }

    pub fn authorize_process_group_signal(
        &self,
        caller: ProcessId,
        pgid: ProcessId,
    ) -> Result<(), Error> {
        let caller_record = self.process(caller)?;
        if caller_record.state != ProcessState::Running || pgid.get() == 0 {
            return Err(Error::InvalidState);
        }
        if !self.process_group_has_live_member_in_session(pgid, caller_record.sid) {
            return Err(Error::PermissionDenied);
        }
        if caller_record.pgid == pgid {
            return Ok(());
        }
        if self
            .processes
            .iter()
            .flatten()
            .any(|record| record.parent == Some(caller) && record.pgid == pgid)
        {
            return Ok(());
        }
        if self
            .credentials(caller)?
            .has_capability(Capability::SessionAdmin as u8)
        {
            return Ok(());
        }
        Err(Error::PermissionDenied)
    }

    pub fn raise_signal_to_group(&mut self, pgid: ProcessId, signal: u8) -> Result<(), Error> {
        if pgid.get() == 0 {
            return Err(Error::InvalidId);
        }
        Self::validate_signal(signal)?;
        let mut members = 0;
        for record in self.processes.iter_mut().flatten() {
            if record.pgid == pgid
                && matches!(record.state, ProcessState::Running | ProcessState::Stopped)
            {
                record
                    .signals
                    .set_pending(signal)
                    .map_err(Self::signal_error)?;
                members += 1;
            }
        }
        if members == 0 {
            return Err(Error::InvalidId);
        }
        Ok(())
    }

    pub fn send_signal(
        &mut self,
        caller: ProcessId,
        target: ProcessId,
        signal: u8,
    ) -> Result<(), Error> {
        Self::validate_signal(signal)?;
        self.live_process(caller)?;
        self.live_process(target)?;
        if !self.can_signal(caller, target) {
            return Err(Error::PermissionDenied);
        }
        self.raise_signal(target, signal)
    }

    pub fn send_signal_to_group(
        &mut self,
        caller: ProcessId,
        pgid: ProcessId,
        signal: u8,
    ) -> Result<usize, Error> {
        Self::validate_signal(signal)?;
        self.live_process(caller)?;
        if pgid.get() == 0 {
            return Err(Error::InvalidId);
        }
        let mut targets = [None; MAX_PROCESSES];
        let mut target_count = 0;
        for record in self.processes.iter().flatten() {
            if record.pgid == pgid
                && matches!(record.state, ProcessState::Running | ProcessState::Stopped)
            {
                targets[target_count] = Some(record.id);
                target_count += 1;
            }
        }
        if target_count == 0 {
            return Err(Error::InvalidId);
        }
        let mut sent = 0;
        for target in targets.into_iter().flatten() {
            if self.can_signal(caller, target) {
                self.raise_signal(target, signal)?;
                sent += 1;
            }
        }
        if sent == 0 {
            return Err(Error::PermissionDenied);
        }
        Ok(sent)
    }

    fn can_signal(&self, caller: ProcessId, target: ProcessId) -> bool {
        let Ok(caller) = self.process(caller) else {
            return false;
        };
        let Ok(target) = self.process(target) else {
            return false;
        };
        caller
            .credentials
            .has_capability(Capability::SessionAdmin as u8)
            || caller.credentials.effective_uid == target.credentials.real_uid
            || caller.credentials.effective_uid == target.credentials.effective_uid
            || caller.credentials.real_uid == target.credentials.real_uid
            || caller.credentials.real_uid == target.credentials.effective_uid
    }

    /// Kept as an internal compatibility name for callers that used the old
    /// immediate-termination probe.  POSIX `killpg` only queues a signal;
    /// default actions run when the target reaches a safe delivery boundary.
    pub fn terminate_signal_to_group(&mut self, pgid: ProcessId, signal: u8) -> Result<(), Error> {
        self.raise_signal_to_group(pgid, signal)
    }

    fn terminate(&mut self, process: ProcessId, signal: u8) -> Result<(), Error> {
        if process == ProcessId::INIT || self.process(process)?.state != ProcessState::Running {
            return Err(Error::InvalidState);
        }
        self.finish_exit(process, -(signal as i32), Some(signal))
    }

    fn finish_exit(
        &mut self,
        process: ProcessId,
        status: i32,
        _signal: Option<u8>,
    ) -> Result<(), Error> {
        let parent = self.process(process)?.parent;
        {
            let record = self.process_mut(process)?;
            record.state = ProcessState::Exiting;
            record.session_generation = 0;
            record.exit_status = Some(status);
            record.signals.clear_all();
            let fds = record.fds;
            record.fds = [None; MAX_FDS];
            for entry in fds.into_iter().flatten() {
                release_open_file(entry.open_file);
            }
        }
        for thread in &mut self.threads {
            if let Some(thread) = thread.as_mut() {
                if thread.process == Some(process) {
                    thread.state = ThreadState::Exited;
                    thread.signals.clear_for_teardown();
                }
            }
        }
        for record in &mut self.processes {
            if let Some(record) = record.as_mut() {
                if record.parent == Some(process) {
                    record.parent = Some(ProcessId::INIT);
                }
            }
        }
        self.process_mut(process)?.state = ProcessState::Zombie;
        if let Some(parent) = parent {
            self.notify_parent(parent, false);
        }
        Ok(())
    }

    fn notify_parent(&mut self, parent: ProcessId, stop_or_continue: bool) {
        let Ok(parent_record) = self.process(parent) else {
            return;
        };
        if !matches!(
            parent_record.state,
            ProcessState::Running | ProcessState::Stopped
        ) {
            return;
        }
        let action = parent_record
            .signals
            .action(SIGCHLD)
            .unwrap_or(SignalAction::DEFAULT);
        if stop_or_continue && action.flags & SA_NOCLDSTOP != 0 {
            return;
        }
        let _ = self.process_mut(parent).and_then(|record| {
            record
                .signals
                .set_pending(SIGCHLD)
                .map_err(Self::signal_error)
        });
        self.wake_signal_waiter(parent, SIGCHLD);
    }

    fn stop_process(&mut self, process: ProcessId, _signal: u8) -> Result<(), Error> {
        if process == ProcessId::INIT {
            return Err(Error::PermissionDenied);
        }
        if !matches!(
            self.process(process)?.state,
            ProcessState::Running | ProcessState::Stopped
        ) {
            return Err(Error::InvalidState);
        }
        self.process_mut(process)?.state = ProcessState::Stopped;
        for thread in self.threads.iter_mut().flatten() {
            if thread.process == Some(process) {
                thread.state = ThreadState::Stopped;
            }
        }
        if let Some(parent) = self.process(process)?.parent {
            self.notify_parent(parent, true);
        }
        Ok(())
    }

    fn continue_process(&mut self, process: ProcessId, _signal: u8) -> Result<(), Error> {
        if self.process(process)?.state != ProcessState::Stopped {
            return Ok(());
        }
        self.process_mut(process)?.state = ProcessState::Running;
        for thread in self.threads.iter_mut().flatten() {
            if thread.process == Some(process) && thread.state == ThreadState::Stopped {
                thread.state = ThreadState::Ready;
            }
        }
        if let Some(parent) = self.process(process)?.parent {
            self.notify_parent(parent, true);
        }
        Ok(())
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

    pub fn set_credentials(
        &mut self,
        process: ProcessId,
        credentials: Credentials,
    ) -> Result<(), Error> {
        self.rotate_session(process)?;
        self.process_mut(process)?.credentials = credentials;
        Ok(())
    }

    pub fn session(&self, process: ProcessId) -> Result<SessionState, Error> {
        Ok(self.process(process)?.session)
    }

    pub fn set_session(&mut self, process: ProcessId, session: SessionState) -> Result<(), Error> {
        session.validate()?;
        self.rotate_session(process)?;
        self.process_mut(process)?.session = session;
        Ok(())
    }

    pub fn set_credentials_and_session(
        &mut self,
        process: ProcessId,
        credentials: Credentials,
        session: SessionState,
    ) -> Result<(), Error> {
        session.validate()?;
        self.rotate_session(process)?;
        let record = self.process_mut(process)?;
        record.credentials = credentials;
        record.session = session;
        Ok(())
    }

    pub fn session_capability(&self, process: ProcessId) -> Result<SessionCapability, Error> {
        let record = self.process(process)?;
        if record.session_generation == 0 {
            return Err(Error::InvalidState);
        }
        Ok(SessionCapability {
            process,
            generation: record.session_generation,
        })
    }

    pub fn validate_session_capability(
        &self,
        capability: SessionCapability,
        required_capabilities: u64,
        target_uid: u32,
    ) -> Result<(), Error> {
        let record = self.process(capability.process)?;
        if record.session_generation == 0 || record.session_generation != capability.generation {
            return Err(Error::PermissionDenied);
        }
        if required_capabilities == 0 {
            if target_uid == NO_TARGET_UID || record.credentials.effective_uid != target_uid {
                return Err(Error::PermissionDenied);
            }
        } else if record.credentials.capabilities & required_capabilities != required_capabilities {
            return Err(Error::PermissionDenied);
        }
        Ok(())
    }

    fn rotate_session(&mut self, process: ProcessId) -> Result<(), Error> {
        let record = self.process_mut(process)?;
        record.session_generation = record
            .session_generation
            .checked_add(1)
            .ok_or(Error::GenerationExhausted)?;
        Ok(())
    }

    pub fn create_session(&mut self, process: ProcessId) -> Result<ProcessId, Error> {
        let record = *self.process(process)?;
        if record.state != ProcessState::Running || record.pgid == process {
            return Err(Error::PermissionDenied);
        }
        let old_session = record.sid;
        let record = self.process_mut(process)?;
        record.sid = process;
        record.pgid = process;
        Ok(old_session)
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
        if !matches!(record.state, ProcessState::Creating | ProcessState::Running) {
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

    fn revive_init_thread(&mut self, thread: ThreadId) -> Result<(), Error> {
        if self.thread_owner(thread)? != Some(ProcessId::INIT)
            || self.process(ProcessId::INIT)?.state != ProcessState::Running
        {
            return Err(Error::InvalidState);
        }
        let record = self.thread_mut(thread)?;
        if record.state != ThreadState::Exited {
            return Err(Error::InvalidState);
        }
        record.state = ThreadState::Ready;
        Ok(())
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
        record.wake_at = 0;
        Ok(())
    }

    pub fn block_thread_until(
        &mut self,
        thread: ThreadId,
        deadline: Option<u64>,
    ) -> Result<(), Error> {
        let record = self.thread_mut(thread)?;
        if !matches!(record.state, ThreadState::Ready | ThreadState::Running) {
            return Err(Error::InvalidState);
        }
        record.state = ThreadState::Blocked;
        record.wake_at = deadline.unwrap_or(0);
        Ok(())
    }

    pub fn sleep_thread_until(&mut self, thread: ThreadId, wake_at: u64) -> Result<(), Error> {
        let record = self.thread_mut(thread)?;
        if !matches!(record.state, ThreadState::Ready | ThreadState::Running) {
            return Err(Error::InvalidState);
        }
        record.state = ThreadState::Sleeping;
        record.wake_at = wake_at.max(1);
        Ok(())
    }

    pub fn wake_sleepers(&mut self, now: u64) {
        for record in self.threads.iter_mut().flatten() {
            if record.state == ThreadState::Sleeping && record.wake_at != 0 && record.wake_at <= now
            {
                record.state = ThreadState::Ready;
                record.wake_at = 0;
            }
        }
    }

    pub fn wake_thread(&mut self, thread: ThreadId) -> Result<(), Error> {
        let record = self.thread_mut(thread)?;
        if !matches!(record.state, ThreadState::Blocked | ThreadState::Sleeping) {
            return Err(Error::InvalidState);
        }
        record.state = ThreadState::Ready;
        record.wake_at = 0;
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

    pub fn pick_ready_user_with_init_policy(
        &self,
        excluded_process: Option<ProcessId>,
        exclude_init: bool,
    ) -> Option<ThreadId> {
        self.threads
            .iter()
            .flatten()
            .find(|thread| {
                thread.state == ThreadState::Ready
                    && thread.kind == ThreadKind::User
                    && (excluded_process.is_none() || thread.process != excluded_process)
                    && (!exclude_init || thread.process != Some(ProcessId::INIT))
            })
            .map(|thread| thread.id)
    }

    pub fn thread_process(&self, thread: ThreadId) -> Result<Option<ProcessId>, Error> {
        Ok(self.thread(thread)?.process)
    }

    fn allocate_thread(
        &mut self,
        process: Option<ProcessId>,
        kind: ThreadKind,
    ) -> Result<ThreadId, Error> {
        let slot = self
            .threads
            .iter()
            .enumerate()
            .find_map(|(slot, thread)| {
                (thread.is_none() && self.thread_generations[slot] != u16::MAX).then_some(slot)
            })
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
            wake_at: 0,
            kind,
            runtime_ticks: 0,
            preemptions: 0,
            context_switches: 0,
            user_context: UserContext::EMPTY,
            user_context_valid: false,
            signals: ThreadSignalState::new(),
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

    fn set_user_context(&mut self, id: ThreadId, context: UserContext) -> Result<(), Error> {
        let record = self.thread_mut(id)?;
        if record.kind != ThreadKind::User {
            return Err(Error::InvalidState);
        }
        record.user_context = context;
        record.user_context_valid = true;
        Ok(())
    }

    fn save_user_context(&mut self, id: ThreadId, context: UserContext) -> Result<(), Error> {
        let record = self.thread_mut(id)?;
        if record.kind != ThreadKind::User || !record.user_context_valid {
            return Err(Error::InvalidState);
        }
        record.user_context = context;
        Ok(())
    }

    fn has_user_context(&self, id: ThreadId) -> Result<bool, Error> {
        Ok(self.thread(id)?.user_context_valid)
    }

    fn revoke_user_context(&mut self, id: ThreadId) -> Result<(), Error> {
        let record = self.thread_mut(id)?;
        record.user_context = UserContext::EMPTY;
        record.user_context_valid = false;
        Ok(())
    }

    fn user_context_pointer(&self, id: ThreadId) -> Result<*const UserContext, Error> {
        let record = self.thread(id)?;
        if record.kind != ThreadKind::User || !record.user_context_valid {
            return Err(Error::InvalidState);
        }
        Ok(core::ptr::addr_of!(record.user_context))
    }
}

struct Runtime {
    table: ProcessTable,
    init_thread: Option<ThreadId>,
    init_exit_status: Option<i32>,
    current_process: Option<ProcessId>,
    current_thread: Option<ThreadId>,
}

impl Runtime {
    const fn new() -> Self {
        Self {
            table: ProcessTable::new(),
            init_thread: None,
            init_exit_status: None,
            current_process: None,
            current_thread: None,
        }
    }

    fn set_current(&mut self, thread: ThreadId) -> Result<(), Error> {
        self.current_thread = Some(thread);
        self.current_process = self.table.thread_owner(thread)?;
        Ok(())
    }

    fn exclude_init_from_user_schedule(&self, current_process: Option<ProcessId>) -> bool {
        current_process != Some(ProcessId::INIT)
            && !self
                .init_thread
                .is_some_and(|thread| self.table.has_user_context(thread).unwrap_or(false))
    }
}

struct RuntimeLock {
    owner: AtomicUsize,
    depth: AtomicUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeLockError {
    Contended,
    InvalidToken,
    InvalidState,
    DepthExhausted,
}

impl RuntimeLock {
    const fn new() -> Self {
        Self {
            owner: AtomicUsize::new(0),
            depth: AtomicUsize::new(0),
        }
    }

    fn try_lock_for(&self, token: usize) -> Option<RuntimeLockGuard<'_>> {
        if token == 0 {
            return None;
        }
        if self.owner.load(Ordering::Acquire) == token {
            let mut depth = self.depth.load(Ordering::Relaxed);
            if depth == 0 {
                return None;
            }
            loop {
                let next = depth.checked_add(1)?;
                match self.depth.compare_exchange_weak(
                    depth,
                    next,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => return Some(RuntimeLockGuard { lock: self, token }),
                    Err(observed) => depth = observed,
                }
            }
        }
        if self
            .owner
            .compare_exchange(0, token, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            self.depth.store(1, Ordering::Relaxed);
            return Some(RuntimeLockGuard { lock: self, token });
        }
        None
    }

    fn lock_for(
        &self,
        token: usize,
        spin_limit: usize,
    ) -> Result<RuntimeLockGuard<'_>, RuntimeLockError> {
        if token == 0 {
            return Err(RuntimeLockError::InvalidToken);
        }
        let mut spins = 0;
        loop {
            if let Some(guard) = self.try_lock_for(token) {
                return Ok(guard);
            }
            if self.owner.load(Ordering::Acquire) == token {
                return Err(match self.depth.load(Ordering::Relaxed) {
                    0 => RuntimeLockError::InvalidState,
                    usize::MAX => RuntimeLockError::DepthExhausted,
                    _ => RuntimeLockError::Contended,
                });
            }
            if spins >= spin_limit {
                return Err(RuntimeLockError::Contended);
            }
            spins += 1;
            core::hint::spin_loop();
        }
    }

    fn lock(&self) -> RuntimeLockGuard<'_> {
        let token = crate::arch::cpu_id().saturating_add(1).max(1);
        self.lock_for(token, RUNTIME_LOCK_SPIN_LIMIT)
            .unwrap_or_else(|error| panic!("process runtime lock failure: {error:?}"))
    }
}

struct RuntimeLockGuard<'a> {
    lock: &'a RuntimeLock,
    token: usize,
}

impl Drop for RuntimeLockGuard<'_> {
    fn drop(&mut self) {
        assert_eq!(
            self.lock.owner.load(Ordering::Acquire),
            self.token,
            "process runtime lock released by a non-owner"
        );
        let depth = self.lock.depth.load(Ordering::Relaxed);
        assert!(depth != 0, "process runtime lock depth underflow");
        if self.lock.depth.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.lock.owner.store(0, Ordering::Release);
        }
    }
}

// ponytail: one global lock is the bounded throughput ceiling; split ownership
// only after SMP profiling proves this table is a contention hotspot.
struct RuntimeCell {
    lock: RuntimeLock,
    runtime: UnsafeCell<Runtime>,
}

impl RuntimeCell {
    const fn new() -> Self {
        Self {
            lock: RuntimeLock::new(),
            runtime: UnsafeCell::new(Runtime::new()),
        }
    }
}

unsafe impl Sync for RuntimeCell {}

static RUNTIME: RuntimeCell = RuntimeCell::new();

fn with_runtime<R>(f: impl FnOnce(&mut Runtime) -> R) -> R {
    // Mask local IRQs before taking the SMP lock: an IRQ on this CPU cannot
    // re-enter the runtime while the guard is held. Re-entry from the same
    // CPU is counted; foreign contenders spin only up to the fixed budget.
    crate::arch::without_interrupts(|| {
        let _guard = RUNTIME.lock.lock();
        unsafe { f(&mut *RUNTIME.runtime.get()) }
    })
}

pub fn init_runtime() -> bool {
    with_runtime(|runtime| {
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
    with_runtime(|runtime| f(&mut runtime.table))
}

#[allow(dead_code)]
pub fn spawn_child_current(credentials: Credentials) -> Result<(ProcessId, ThreadId), Error> {
    with_runtime(|runtime| {
        let parent = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime.table.spawn_child(parent, credentials)
    })
}

pub fn spawn_child_current_staged(credentials: Credentials) -> Result<SpawnTransaction, Error> {
    with_runtime(|runtime| {
        let parent = runtime.current_process.ok_or(Error::InvalidState)?;
        let (child, thread) = runtime.table.spawn_child_staged(parent, credentials)?;
        Ok(SpawnTransaction {
            parent,
            child,
            thread,
        })
    })
}

pub fn current_credentials() -> Result<Credentials, Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime.table.credentials(process)
    })
}

pub fn current_session_capability() -> Result<SessionCapability, Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime.table.session_capability(process)
    })
}

pub fn validate_session_capability(
    capability: SessionCapability,
    required_capabilities: u64,
    target_uid: u32,
) -> Result<(), Error> {
    with_runtime(|runtime| {
        runtime
            .table
            .validate_session_capability(capability, required_capabilities, target_uid)
    })
}

pub fn set_current_credentials(credentials: Credentials) -> Result<(), Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        let current = runtime.table.credentials(process)?;
        let privileged = current.has_capability(Capability::SessionAdmin as u8);
        if !privileged
            && (credentials.real_uid != current.real_uid
                || credentials.effective_uid != current.effective_uid
                || credentials.saved_uid != current.saved_uid
                || credentials.real_gid != current.real_gid
                || credentials.effective_gid != current.effective_gid
                || credentials.saved_gid != current.saved_gid
                || credentials.capabilities & !current.capabilities != 0)
        {
            return Err(Error::PermissionDenied);
        }
        runtime.table.set_credentials(process, credentials)
    })
}

pub fn current_session() -> Result<SessionState, Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime.table.session(process)
    })
}

pub fn set_current_session(credentials: Credentials, session: SessionState) -> Result<(), Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        session.validate()?;
        let current_credentials = runtime.table.credentials(process)?;
        let current_session = runtime.table.session(process)?;
        let privileged = current_credentials.has_capability(Capability::SessionAdmin as u8);
        if !privileged
            && (credentials.real_uid != current_credentials.real_uid
                || credentials.effective_uid != current_credentials.effective_uid
                || credentials.saved_uid != current_credentials.saved_uid
                || credentials.real_gid != current_credentials.real_gid
                || credentials.effective_gid != current_credentials.effective_gid
                || credentials.saved_gid != current_credentials.saved_gid
                || credentials.capabilities & !current_credentials.capabilities != 0
                || session.max_fds > current_session.max_fds
                || session.max_address_space_pages > current_session.max_address_space_pages
                || session.max_cpu_ticks > current_session.max_cpu_ticks)
        {
            return Err(Error::PermissionDenied);
        }
        runtime
            .table
            .set_credentials_and_session(process, credentials, session)
    })
}

pub fn current_process_id() -> Option<ProcessId> {
    with_runtime(|runtime| runtime.current_process)
}

// ABI wrappers stay local until syscall dispatch is implemented in the next pass.
#[allow(dead_code)]
pub fn current_process_group_id() -> Option<ProcessId> {
    with_runtime(|runtime| {
        runtime
            .current_process
            .and_then(|process| runtime.table.get_process_group(process).ok())
    })
}

pub fn current_session_id() -> Option<ProcessId> {
    with_runtime(|runtime| {
        runtime
            .current_process
            .and_then(|process| runtime.table.get_session_id(process).ok())
    })
}

pub fn process_group_in_session(group: ProcessId, session: ProcessId) -> bool {
    with_runtime(|runtime| {
        runtime
            .table
            .process_group_has_live_member_in_session(group, session)
    })
}

pub fn raise_signal_to_group(group: ProcessId, signal: u8) -> Result<(), Error> {
    with_runtime(|runtime| runtime.table.raise_signal_to_group(group, signal))
}

#[allow(dead_code)]
pub fn set_current_process_group(pgid: ProcessId) -> Result<(), Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime.table.set_process_group(process, pgid)
    })
}

#[allow(dead_code)]
pub fn raise_signal_to_current_group(signal: u8) -> Result<(), Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        let pgid = runtime.table.get_process_group(process)?;
        runtime.table.raise_signal_to_group(pgid, signal)
    })
}

pub fn attach_address_space(process: ProcessId, root: PhysAddr) -> Result<(), Error> {
    with_runtime(|runtime| runtime.table.attach_address_space(process, root))
}

pub fn address_space_root(process: ProcessId) -> Result<Option<PhysAddr>, Error> {
    with_runtime(|runtime| runtime.table.address_space_root(process))
}

pub fn handle_user_fault(fault: crate::vm::FaultInfo) -> crate::vm::FaultResult {
    let Some(process) = current_process_id() else {
        return crate::vm::FaultResult::KernelFatal;
    };
    crate::user_runtime::handle_fault(process, fault).unwrap_or({
        if fault.user {
            crate::vm::FaultResult::UserFault(crate::vm::FaultReason::InvalidAddress)
        } else {
            crate::vm::FaultResult::KernelFatal
        }
    })
}

pub fn clear_address_space(process: ProcessId) -> Result<(), Error> {
    with_runtime(|runtime| runtime.table.clear_address_space(process))
}

pub fn restore_process(process: ProcessId) -> Result<ThreadId, Error> {
    with_runtime(|runtime| {
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
    with_runtime(|runtime| {
        if runtime.table.process_state(child)? == ProcessState::Creating {
            runtime.table.abort_child(parent, child)?;
        } else {
            runtime.table.exit(child, -127)?;
            let _ = runtime.table.wait(parent, Some(child))?;
        }
        Ok(())
    })
}

pub fn rollback_spawn(transaction: SpawnTransaction) -> Result<(), Error> {
    let SpawnTransaction { parent, child, .. } = transaction;
    with_runtime(|runtime| runtime.table.abort_child(parent, child))
}

#[allow(dead_code)]
pub fn switch_to_user(process: ProcessId, thread: ThreadId) -> Result<(), Error> {
    with_runtime(|runtime| {
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
    with_runtime(|runtime| {
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

pub fn restore_init_after_user() -> Result<(), Error> {
    with_runtime(|runtime| {
        if runtime.current_thread.is_some() || runtime.current_process.is_some() {
            return Err(Error::InvalidState);
        }
        let init_thread = runtime.init_thread.ok_or(Error::InvalidState)?;
        runtime.table.revive_init_thread(init_thread)?;
        runtime.table.switch_to(None, init_thread)?;
        runtime.current_thread = Some(init_thread);
        runtime.current_process = Some(ProcessId::INIT);
        Ok(())
    })
}

pub fn current_ids() -> Option<(u32, u32)> {
    with_runtime(|runtime| {
        Some((
            runtime.current_process?.get(),
            runtime.current_thread?.get(),
        ))
    })
}

pub fn exit_current(status: i32) -> Result<Option<ContextSwitch>, Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        let thread = runtime.current_thread.ok_or(Error::InvalidState)?;
        if process == ProcessId::INIT {
            runtime.init_exit_status = Some(status);
            runtime.table.thread_mut(thread)?.state = ThreadState::Exited;
        } else {
            if let Err(error) = runtime.table.exit(process, status) {
                if runtime.table.process_state(process) != Ok(ProcessState::Zombie) {
                    return Err(error);
                }
            }
        }
        if !runtime.table.has_user_context(thread)? {
            runtime.current_process = None;
            runtime.current_thread = None;
            return Ok(None);
        }
        let Some(next) = runtime.table.pick_ready_user_with_init_policy(
            runtime.current_process,
            runtime.exclude_init_from_user_schedule(runtime.current_process),
        ) else {
            runtime.current_process = None;
            runtime.current_thread = None;
            return Ok(None);
        };
        let switch = runtime.table.switch_to(None, next)?;
        crate::arch::request_user_switch(thread.get(), next.get());
        Ok(Some(switch))
    })
}

pub fn init_exit_status() -> Option<i32> {
    with_runtime(|runtime| runtime.init_exit_status)
}

pub fn wait_current(child: Option<u32>) -> Result<(u32, i32), Error> {
    with_runtime(|runtime| {
        let parent = runtime.current_process.ok_or(Error::InvalidState)?;
        let child = child.map(ProcessId::from_raw);
        let (process, status) = runtime.table.wait(parent, child)?;
        let _ = crate::user_runtime::discard(ProcessId::from_raw(process.get()));
        Ok((process.get(), status))
    })
}

pub fn peek_wait_current(child: Option<u32>) -> Result<(u32, i32), Error> {
    with_runtime(|runtime| {
        let parent = runtime.current_process.ok_or(Error::InvalidState)?;
        let child = child.map(ProcessId::from_raw);
        let (process, status) = runtime.table.peek_wait(parent, child)?;
        Ok((process.get(), status))
    })
}

pub fn close_current(fd: u32) -> Result<(), Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime
            .table
            .close_fd(process, FileDescriptor::from_raw(fd))
    })
}

pub fn open_current_fd(
    open_file: u32,
    nonblocking: bool,
    readable: bool,
    writable: bool,
) -> Result<u32, Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        Ok(runtime
            .table
            .open_fd_with_flags(process, open_file, nonblocking, readable, writable)?
            .get())
    })
}

pub fn current_fd_info(fd: u32) -> Result<(u32, bool, bool, bool, bool), Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime.table.fd_info(process, FileDescriptor::from_raw(fd))
    })
}

pub fn set_close_on_exec_current(fd: u32, enabled: bool) -> Result<(), Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime
            .table
            .set_close_on_exec(process, FileDescriptor::from_raw(fd), enabled)
    })
}

pub fn duplicate_fd_current(old_fd: u32, new_fd: u32) -> Result<u32, Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime.table.duplicate_fd(
            process,
            FileDescriptor::from_raw(old_fd),
            FileDescriptor::from_raw(new_fd),
        )?;
        Ok(new_fd)
    })
}

fn duplicate_open_file(open_file: u32) -> Result<(), Error> {
    if open_file <= 2 {
        return Ok(());
    }
    if crate::vfs::FileHandle::from_raw(open_file).is_some() {
        crate::vfs::duplicate_raw(open_file).map_err(|error| match error {
            crate::vfs::Error::NoSpace => Error::FdCapacity,
            _ => Error::InvalidFd,
        })?;
    } else if crate::pipe::is_pipe_raw(open_file) {
        crate::pipe::duplicate_raw(open_file).map_err(|error| match error {
            crate::pipe::Error::NoSpace => Error::FdCapacity,
            _ => Error::InvalidFd,
        })?;
    } else {
        return Err(Error::InvalidFd);
    }
    Ok(())
}

fn validate_open_file(open_file: u32) -> Result<(), Error> {
    if open_file <= 2 {
        return Ok(());
    }
    if crate::vfs::FileHandle::from_raw(open_file).is_some() {
        let duplicate = crate::vfs::duplicate_raw(open_file).map_err(|_| Error::InvalidFd)?;
        crate::vfs::close_raw(duplicate).map_err(|_| Error::InvalidFd)?;
        return Ok(());
    }
    if crate::pipe::is_pipe_raw(open_file) {
        let duplicate = crate::pipe::duplicate_raw(open_file).map_err(|_| Error::InvalidFd)?;
        crate::pipe::close_raw(duplicate).map_err(|_| Error::InvalidFd)?;
        return Ok(());
    }
    Err(Error::InvalidFd)
}

fn release_open_file(open_file: u32) {
    if crate::vfs::FileHandle::from_raw(open_file).is_some() {
        let _ = crate::vfs::close_raw(open_file);
    } else if crate::pipe::is_pipe_raw(open_file) {
        let _ = crate::pipe::close_raw(open_file);
    }
}

pub fn current_fd_access(fd: u32) -> Result<(bool, bool, bool), Error> {
    with_runtime(|runtime| {
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime
            .table
            .fd_access(process, FileDescriptor::from_raw(fd))
    })
}

pub fn yield_current() -> Result<Option<ContextSwitch>, Error> {
    with_runtime(|runtime| {
        let current = runtime.current_thread.ok_or(Error::InvalidState)?;
        if !runtime.table.has_user_context(current)? {
            return Ok(None);
        }
        runtime.table.wake_sleepers(crate::time::ticks());
        let Some(next) = runtime.table.pick_ready_user_with_init_policy(
            runtime.current_process,
            runtime.exclude_init_from_user_schedule(runtime.current_process),
        ) else {
            return Ok(None);
        };
        let switch = runtime.table.switch_to(Some(current), next)?;
        crate::arch::request_user_switch(current.get(), next.get());
        Ok(Some(switch))
    })
}

pub fn block_current_on(
    queue: &mut crate::wait::WaitQueue,
    owner: u64,
    deadline: Option<u64>,
) -> Result<BlockCurrentResult, crate::wait::Error> {
    with_runtime(|runtime| {
        let current = runtime
            .current_thread
            .ok_or(crate::wait::Error::ThreadState)?;
        if !runtime.table.has_user_context(current).unwrap_or(false) {
            return Err(crate::wait::Error::ThreadState);
        }
        let action = queue.park(
            &mut runtime.table,
            current,
            owner,
            deadline,
            crate::time::ticks(),
        )?;
        let crate::wait::WaitAction::Blocked(token) = action else {
            return Ok(BlockCurrentResult::TimedOut);
        };
        let Some(next) = runtime.table.pick_ready_user_with_init_policy(
            runtime.current_process,
            runtime.exclude_init_from_user_schedule(runtime.current_process),
        ) else {
            queue
                .cancel_with_table(&mut runtime.table, token, current)
                .map_err(|_| crate::wait::Error::ThreadState)?;
            return Err(crate::wait::Error::ThreadState);
        };
        let switch = runtime
            .table
            .switch_to(None, next)
            .map_err(|_| crate::wait::Error::ThreadState)?;
        crate::arch::request_user_switch(current.get(), next.get());
        Ok(BlockCurrentResult::Blocked { token, switch })
    })
}

pub fn wake_waiter_one(
    queue: &mut crate::wait::WaitQueue,
    owner: u64,
) -> Result<Option<crate::wait::Wake>, crate::wait::Error> {
    with_runtime(|runtime| queue.wake_one_with_table(&mut runtime.table, owner))
}

pub fn wake_waiters_all(
    queue: &mut crate::wait::WaitQueue,
    owner: u64,
) -> Result<crate::wait::WakeBatch, crate::wait::Error> {
    with_runtime(|runtime| queue.wake_all_with_table(&mut runtime.table, owner))
}

pub fn expire_waiters(
    queue: &mut crate::wait::WaitQueue,
    owner: u64,
    now: u64,
) -> Result<crate::wait::WakeBatch, crate::wait::Error> {
    with_runtime(|runtime| queue.expire_with_table(&mut runtime.table, owner, now))
}

pub fn commit_user_switch(from: u32, to: u32) -> Result<PhysAddr, Error> {
    with_runtime(|runtime| {
        let from = ThreadId::from_raw(from);
        let to = ThreadId::from_raw(to);
        if runtime.current_thread != Some(from) || runtime.current_process.is_none() {
            return Err(Error::InvalidState);
        }
        let process = runtime
            .table
            .thread_process(to)?
            .ok_or(Error::InvalidState)?;
        if !runtime.table.has_user_context(to)? {
            return Err(Error::InvalidState);
        }
        let root = runtime
            .table
            .address_space_root(process)?
            .ok_or(Error::InvalidState)?;
        runtime.current_thread = Some(to);
        runtime.current_process = Some(process);
        Ok(root)
    })
}

#[allow(dead_code)]
pub fn user_thread_root(thread: u32) -> Result<PhysAddr, Error> {
    with_runtime(|runtime| {
        let process = runtime
            .table
            .thread_process(ThreadId::from_raw(thread))?
            .ok_or(Error::InvalidState)?;
        runtime
            .table
            .address_space_root(process)?
            .ok_or(Error::InvalidState)
    })
}

pub fn install_user_context(
    thread: u32,
    registers: crate::elf::InitialRegisters,
) -> Result<(), Error> {
    if crate::arch::install_user_context(thread, registers) {
        Ok(())
    } else {
        Err(Error::InvalidState)
    }
}

pub fn set_user_context(thread: u32, context: UserContext) -> Result<(), Error> {
    with_runtime(|runtime| {
        runtime
            .table
            .set_user_context(ThreadId::from_raw(thread), context)
    })
}

pub fn save_user_context(thread: u32, context: UserContext) -> Result<(), Error> {
    with_runtime(|runtime| {
        runtime
            .table
            .save_user_context(ThreadId::from_raw(thread), context)
    })
}

pub fn user_context_pointer(thread: u32) -> Result<*const UserContext, Error> {
    with_runtime(|runtime| {
        runtime
            .table
            .user_context_pointer(ThreadId::from_raw(thread))
    })
}

pub fn has_user_context(thread: u32) -> bool {
    with_runtime(|runtime| {
        runtime
            .table
            .has_user_context(ThreadId::from_raw(thread))
            .unwrap_or(false)
    })
}

pub fn revoke_user_context(thread: u32) {
    with_runtime(|runtime| {
        let _ = runtime
            .table
            .revoke_user_context(ThreadId::from_raw(thread));
    });
}

pub fn inherit_spawn_standard_fds(
    transaction: SpawnTransaction,
    source_fds: [u32; 3],
) -> Result<(), Error> {
    inherit_spawn_fds(transaction, source_fds, false)
}

pub fn inherit_spawn_fds(
    transaction: SpawnTransaction,
    source_fds: [u32; 3],
    inherit_open_fds: bool,
) -> Result<(), Error> {
    with_runtime(|runtime| {
        let inherit = if inherit_open_fds {
            ProcessTable::inherit_open_fds
        } else {
            ProcessTable::inherit_standard_fds
        };
        inherit(
            &mut runtime.table,
            transaction.parent,
            transaction.child,
            source_fds.map(FileDescriptor::from_raw),
        )
    })
}

pub fn finalize_spawn(
    transaction: SpawnTransaction,
    source_fds: [u32; 3],
    inherit_open_fds: bool,
    process_group: Option<ProcessId>,
) -> Result<(), Error> {
    with_runtime(|runtime| {
        ProcessTable::finalize_spawn(
            &mut runtime.table,
            transaction,
            source_fds.map(FileDescriptor::from_raw),
            inherit_open_fds,
            process_group,
        )
    })
}

pub fn publish_spawn(transaction: SpawnTransaction) -> Result<(), Error> {
    with_runtime(|runtime| {
        runtime
            .table
            .publish_child(transaction.child, transaction.thread)
    })
}

pub fn set_process_group_for_spawn(
    transaction: SpawnTransaction,
    pgid: ProcessId,
) -> Result<(), Error> {
    with_runtime(|runtime| {
        runtime
            .table
            .set_process_group_for_child(transaction.parent, transaction.child, pgid)
    })
}

pub fn sleep_current(duration: u64) -> Result<Option<ContextSwitch>, Error> {
    sleep_current_ticks(
        crate::time::scheduler_ticks().saturating_mul(duration),
        true,
    )
}

pub fn sleep_current_ticks(
    duration: u64,
    saturating_deadline: bool,
) -> Result<Option<ContextSwitch>, Error> {
    with_runtime(|runtime| {
        let current = runtime.current_thread.ok_or(Error::InvalidState)?;
        if duration == 0 || !runtime.table.has_user_context(current)? {
            return Ok(None);
        }
        let now = crate::time::ticks();
        let wake_at = if saturating_deadline {
            now.saturating_add(duration)
        } else {
            now.checked_add(duration).ok_or(Error::InvalidState)?
        };
        runtime.table.sleep_thread_until(current, wake_at)?;
        let Some(next) = runtime.table.pick_ready_user_with_init_policy(
            runtime.current_process,
            runtime.exclude_init_from_user_schedule(runtime.current_process),
        ) else {
            runtime.table.wake_thread(current)?;
            runtime.table.switch_to(None, current)?;
            return Ok(None);
        };
        let switch = runtime.table.switch_to(None, next)?;
        crate::arch::request_user_switch(current.get(), next.get());
        Ok(Some(switch))
    })
}

pub fn wake_sleepers(now: u64) {
    with_runtime(|runtime| {
        runtime.table.wake_sleepers(now);
    });
}

pub fn contract_self_check() {
    let mut context_table = ProcessTable::new();
    let (context_init, _) = context_table.create_init().unwrap();
    let (context_child, context_thread) = context_table
        .spawn_child(context_init, Credentials::BOOTSTRAP)
        .unwrap();
    let context_registers = crate::elf::InitialRegisters {
        instruction_pointer: 1,
        stack_pointer: 2,
        flags: 0x202,
        arg0: 0,
        arg1: 0,
        arg2: 0,
    };
    let mut context = UserContext::EMPTY;
    context.registers[0] = context_registers.instruction_pointer as u64;
    context.registers[1] = context_registers.stack_pointer as u64;
    context.registers[2] = context_registers.flags;
    context_table
        .set_user_context(context_thread, context)
        .unwrap();
    assert!(context_table.has_user_context(context_thread).unwrap());
    context_table.exit(context_child, 0).unwrap();
    assert_eq!(
        context_table.wait(context_init, Some(context_child)),
        Ok((context_child, 0))
    );
    assert!(!context_table
        .has_user_context(context_thread)
        .unwrap_or(false));
    let (_recycled_child, recycled_thread) = context_table
        .spawn_child(context_init, Credentials::BOOTSTRAP)
        .unwrap();
    assert_ne!(recycled_thread.get(), context_thread.get());
    assert!(!context_table.has_user_context(recycled_thread).unwrap());
    context_table
        .set_user_context(recycled_thread, context)
        .unwrap();
    assert!(context_table.has_user_context(recycled_thread).unwrap());
    assert!(!context_table
        .has_user_context(ThreadId::from_raw(context_thread.get()))
        .unwrap_or(false));
    context_table
        .thread_mut(recycled_thread)
        .unwrap()
        .user_context_valid = false;
    assert!(!context_table.has_user_context(recycled_thread).unwrap());
    let (staged_child, staged_thread) = context_table
        .spawn_child_staged(context_init, Credentials::BOOTSTRAP)
        .unwrap();
    context_table
        .set_user_context(staged_thread, context)
        .unwrap();
    context_table
        .abort_child(context_init, staged_child)
        .unwrap();
    assert!(!context_table
        .has_user_context(staged_thread)
        .unwrap_or(false));

    let lock = RuntimeLock::new();
    assert!(lock.try_lock_for(0).is_none());
    assert!(matches!(
        lock.lock_for(0, 0),
        Err(RuntimeLockError::InvalidToken)
    ));
    let first = lock
        .try_lock_for(1)
        .unwrap_or_else(|| panic!("runtime lock acquire"));
    assert_eq!(lock.owner.load(Ordering::Relaxed), 1);
    assert_eq!(lock.depth.load(Ordering::Relaxed), 1);
    let nested = lock
        .try_lock_for(1)
        .unwrap_or_else(|| panic!("runtime lock re-entry"));
    assert_eq!(lock.depth.load(Ordering::Relaxed), 2);
    assert!(lock.try_lock_for(2).is_none());
    drop(nested);
    assert_eq!(lock.depth.load(Ordering::Relaxed), 1);
    assert!(lock.try_lock_for(2).is_none());
    assert!(matches!(
        lock.lock_for(2, 0),
        Err(RuntimeLockError::Contended)
    ));
    assert_eq!(lock.owner.load(Ordering::Relaxed), 1);
    assert_eq!(lock.depth.load(Ordering::Relaxed), 1);
    lock.depth.store(0, Ordering::Relaxed);
    assert!(matches!(
        lock.lock_for(1, 0),
        Err(RuntimeLockError::InvalidState)
    ));
    lock.depth.store(1, Ordering::Relaxed);
    lock.depth.store(usize::MAX, Ordering::Relaxed);
    assert!(matches!(
        lock.lock_for(1, 0),
        Err(RuntimeLockError::DepthExhausted)
    ));
    lock.depth.store(1, Ordering::Relaxed);
    drop(first);
    assert_eq!(lock.owner.load(Ordering::Relaxed), 0);
    assert_eq!(lock.depth.load(Ordering::Relaxed), 0);
    let foreign = lock
        .try_lock_for(2)
        .unwrap_or_else(|| panic!("runtime lock release"));
    drop(foreign);
    assert_eq!(with_runtime(|_| current_process_id()), None);

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
        Capability::SessionAdmin,
        Capability::AccountAdmin,
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
    assert_eq!(table.get_process_group(init).unwrap(), init);
    assert_eq!(table.get_process_group(child).unwrap(), init);
    table.set_process_group(child, child).unwrap();
    assert_eq!(table.get_process_group(child).unwrap(), child);
    assert_eq!(
        table.authorize_process_group_change(init, child, child),
        Ok(())
    );
    assert_eq!(
        table.authorize_process_group_change(child, init, init),
        Err(Error::PermissionDenied)
    );
    assert_eq!(
        table.set_process_group(child, ProcessId::from_raw(99)),
        Err(Error::InvalidId)
    );
    table.raise_signal_to_group(init, 4).unwrap();
    assert!(table.signal_pending(init, 4).unwrap());
    assert!(!table.signal_pending(child, 4).unwrap());
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
    assert_eq!(table.open_fd(child, 7, true, false), Err(Error::InvalidFd));
    let fd = table.open_fd(child, 1, true, false).unwrap();
    assert_eq!(fd.get(), 3);
    table.set_close_on_exec(child, fd, true).unwrap();
    assert_eq!(
        table.fd_info(child, fd).unwrap(),
        (1, true, false, true, false)
    );
    let duplicate = table
        .duplicate_fd(child, fd, FileDescriptor::from_raw(4))
        .unwrap();
    assert_eq!(duplicate.get(), 4);
    assert_eq!(table.fd_info(child, duplicate).unwrap().0, 1);
    assert!(!table.fd_info(child, duplicate).unwrap().1);
    table.set_close_on_exec(child, fd, false).unwrap();
    let extra_fd = table.open_fd(child, 2, false, true).unwrap();
    assert_eq!(extra_fd.get(), 5);
    table.set_close_on_exec(child, extra_fd, true).unwrap();
    let (all_fds_child, all_fds_thread) = table
        .spawn_child_staged(child, Credentials::BOOTSTRAP)
        .unwrap();
    table
        .inherit_open_fds(child, all_fds_child, [fd, duplicate, fd])
        .unwrap();
    assert_eq!(
        table
            .fd_info(all_fds_child, FileDescriptor::from_raw(3))
            .unwrap()
            .0,
        1
    );
    assert_eq!(
        table
            .fd_info(all_fds_child, FileDescriptor::from_raw(4))
            .unwrap()
            .0,
        1
    );
    assert_eq!(
        table
            .fd_info(all_fds_child, FileDescriptor::from_raw(5))
            .unwrap(),
        (2, true, false, false, true)
    );
    assert!(
        !table
            .fd_info(all_fds_child, FileDescriptor::from_raw(3))
            .unwrap()
            .1
    );
    assert!(
        table
            .fd_info(all_fds_child, FileDescriptor::from_raw(5))
            .unwrap()
            .1
    );
    assert_eq!(table.close_on_exec(all_fds_child), Ok(1));
    assert_eq!(
        table.fd_info(all_fds_child, FileDescriptor::from_raw(5)),
        Err(Error::InvalidFd)
    );
    table.publish_child(all_fds_child, all_fds_thread).unwrap();
    table.exit(all_fds_child, 0).unwrap();
    assert_eq!(
        table.wait(child, Some(all_fds_child)).unwrap(),
        (all_fds_child, 0)
    );

    let (capacity_child, capacity_thread) = table
        .spawn_child_staged(child, Credentials::BOOTSTRAP)
        .unwrap();
    table.process_mut(capacity_child).unwrap().session.max_fds = 5;
    assert_eq!(
        table.inherit_open_fds(child, capacity_child, [fd, duplicate, fd]),
        Err(Error::FdCapacity)
    );
    assert_eq!(
        table
            .fd_info(capacity_child, FileDescriptor::from_raw(0))
            .unwrap()
            .0,
        0
    );
    table.abort_child(child, capacity_child).unwrap();
    let _ = capacity_thread;

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
    table.sleep_thread_until(child_thread, 10).unwrap();
    assert_eq!(
        table.thread_state(child_thread).unwrap(),
        ThreadState::Sleeping
    );
    table.wake_sleepers(9);
    assert_eq!(
        table.thread_state(child_thread).unwrap(),
        ThreadState::Sleeping
    );
    table.wake_sleepers(10);
    assert_eq!(
        table.thread_state(child_thread).unwrap(),
        ThreadState::Ready
    );
    let (inherited_child, inherited_thread) = table
        .spawn_child_staged(child, Credentials::BOOTSTRAP)
        .unwrap();
    let transaction = SpawnTransaction {
        parent: child,
        child: inherited_child,
        thread: inherited_thread,
    };
    assert_eq!(
        table
            .fd_info(inherited_child, FileDescriptor::from_raw(3))
            .unwrap_err(),
        Error::InvalidFd
    );
    assert_eq!(table.get_process_group(inherited_child), Ok(child));
    assert_eq!(
        table.set_process_group_for_child(init, transaction.child, transaction.child),
        Err(Error::InvalidState)
    );
    table
        .finalize_spawn(
            transaction,
            [fd, fd, duplicate],
            false,
            Some(inherited_child),
        )
        .unwrap();
    for target in 0..=2 {
        assert_eq!(
            table
                .fd_info(inherited_child, FileDescriptor::from_raw(target))
                .unwrap()
                .0,
            1
        );
    }
    assert_eq!(
        table.get_process_group(inherited_child),
        Ok(inherited_child)
    );
    table.exit(inherited_child, 0).unwrap();
    assert_eq!(
        table.wait(child, Some(inherited_child)).unwrap(),
        (inherited_child, 0)
    );
    table.switch_to(None, kernel_thread).unwrap();
    table.account_thread(kernel_thread, 2).unwrap();
    table.preempt_thread(kernel_thread).unwrap();
    assert_eq!(table.thread_accounting(kernel_thread).unwrap(), (2, 1, 1));
    table.close_fd(child, fd).unwrap();
    table.close_fd(child, duplicate).unwrap();
    table.close_fd(child, extra_fd).unwrap();
    table.exit(child, 23).unwrap();
    assert_eq!(table.wait(init, Some(child)).unwrap(), (child, 23));
    assert_eq!(table.thread_state(child_thread), Err(Error::InvalidId));
    assert_eq!(table.wait(init, None), Err(Error::NoChild));
    table.thread_mut(init_thread).unwrap().state = ThreadState::Exited;
    table.revive_init_thread(init_thread).unwrap();
    assert_eq!(table.thread_state(init_thread).unwrap(), ThreadState::Ready);
    table.switch_to(None, kernel_thread).unwrap();
    assert_eq!(
        table
            .switch_to(Some(kernel_thread), init_thread)
            .unwrap()
            .to_kind,
        ThreadKind::User
    );

    let mut resource_table = ProcessTable::new();
    let (resource_init, _) = resource_table.create_init().unwrap();
    let (resource_parent, _) = resource_table
        .spawn_child(resource_init, Credentials::BOOTSTRAP)
        .unwrap();
    let file = crate::vfs::open("/hello.txt", crate::vfs::OpenOptions::read()).unwrap();
    let file_raw = file.raw();
    let file_fd = resource_table
        .open_fd(resource_parent, file_raw, true, false)
        .unwrap();
    resource_table
        .set_close_on_exec(resource_parent, file_fd, true)
        .unwrap();
    let (pipe_read_raw, pipe_write_raw) = crate::pipe::create().unwrap();
    let pipe_read_fd = resource_table
        .open_fd(resource_parent, pipe_read_raw, true, false)
        .unwrap();
    let pipe_write_fd = resource_table
        .open_fd(resource_parent, pipe_write_raw, false, true)
        .unwrap();
    let (resource_child, resource_child_thread) = resource_table
        .spawn_child_staged(resource_parent, Credentials::BOOTSTRAP)
        .unwrap();
    resource_table
        .inherit_open_fds(
            resource_parent,
            resource_child,
            [
                FileDescriptor::from_raw(0),
                FileDescriptor::from_raw(1),
                FileDescriptor::from_raw(2),
            ],
        )
        .unwrap();
    assert_eq!(
        resource_table
            .fd_info(resource_child, FileDescriptor::from_raw(3))
            .unwrap()
            .0,
        file_raw
    );
    assert!(
        resource_table
            .fd_info(resource_child, FileDescriptor::from_raw(3))
            .unwrap()
            .1
    );
    assert_eq!(
        resource_table
            .fd_info(resource_child, FileDescriptor::from_raw(4))
            .unwrap()
            .0,
        pipe_read_raw
    );
    assert_eq!(
        resource_table
            .fd_info(resource_child, FileDescriptor::from_raw(5))
            .unwrap()
            .0,
        pipe_write_raw
    );
    resource_table.close_fd(resource_parent, file_fd).unwrap();
    assert_eq!(crate::vfs::read_raw(file_raw, &mut [0; 1]), Ok(1));
    resource_table
        .close_fd(resource_child, FileDescriptor::from_raw(3))
        .unwrap();
    assert_eq!(
        crate::vfs::duplicate_raw(file_raw),
        Err(crate::vfs::Error::InvalidHandle)
    );
    resource_table
        .close_fd(resource_parent, pipe_read_fd)
        .unwrap();
    assert_eq!(crate::pipe::write_raw(pipe_write_raw, &[0x5a]), Ok(1));
    resource_table
        .close_fd(resource_child, FileDescriptor::from_raw(4))
        .unwrap();
    assert_eq!(
        crate::pipe::write_raw(pipe_write_raw, &[0x5a]),
        Err(crate::pipe::Error::BrokenPipe)
    );
    resource_table
        .close_fd(resource_child, FileDescriptor::from_raw(5))
        .unwrap();
    resource_table
        .close_fd(resource_parent, pipe_write_fd)
        .unwrap();
    resource_table
        .publish_child(resource_child, resource_child_thread)
        .unwrap();
    resource_table.exit(resource_child, 0).unwrap();
    assert_eq!(
        resource_table.wait(resource_parent, Some(resource_child)),
        Ok((resource_child, 0))
    );

    let (rollback_read_raw, rollback_write_raw) = crate::pipe::create().unwrap();
    let rollback_read_fd = resource_table
        .open_fd(resource_parent, rollback_read_raw, true, false)
        .unwrap();
    let rollback_write_fd = resource_table
        .open_fd(resource_parent, rollback_write_raw, false, true)
        .unwrap();
    let (rollback_child, _rollback_thread) = resource_table
        .spawn_child_staged(resource_parent, Credentials::BOOTSTRAP)
        .unwrap();
    resource_table
        .process_mut(rollback_child)
        .unwrap()
        .session
        .max_fds = 4;
    assert_eq!(
        resource_table.inherit_open_fds(
            resource_parent,
            rollback_child,
            [
                FileDescriptor::from_raw(0),
                FileDescriptor::from_raw(1),
                FileDescriptor::from_raw(2),
            ],
        ),
        Err(Error::FdCapacity)
    );
    resource_table
        .abort_child(resource_parent, rollback_child)
        .unwrap();
    resource_table
        .close_fd(resource_parent, rollback_read_fd)
        .unwrap();
    assert_eq!(
        crate::pipe::write_raw(rollback_write_raw, &[0x5a]),
        Err(crate::pipe::Error::BrokenPipe)
    );
    resource_table
        .close_fd(resource_parent, rollback_write_fd)
        .unwrap();

    // Keep two staged transactions alive together: each child must hold its
    // own reference, and a failed third transaction must not leak or disturb
    // either successful inheritance.
    let mut concurrent_table = ProcessTable::new();
    let (concurrent_init, _) = concurrent_table.create_init().unwrap();
    let (concurrent_parent, _) = concurrent_table
        .spawn_child(concurrent_init, Credentials::BOOTSTRAP)
        .unwrap();
    let concurrent_file = crate::vfs::open("/hello.txt", crate::vfs::OpenOptions::read()).unwrap();
    let concurrent_file_raw = concurrent_file.raw();
    let concurrent_file_fd = concurrent_table
        .open_fd(concurrent_parent, concurrent_file_raw, true, false)
        .unwrap();
    let (concurrent_pipe_read_raw, concurrent_pipe_write_raw) = crate::pipe::create().unwrap();
    let concurrent_pipe_read_fd = concurrent_table
        .open_fd(concurrent_parent, concurrent_pipe_read_raw, true, false)
        .unwrap();
    let concurrent_pipe_write_fd = concurrent_table
        .open_fd(concurrent_parent, concurrent_pipe_write_raw, false, true)
        .unwrap();
    let (first_concurrent_child, first_concurrent_thread) = concurrent_table
        .spawn_child_staged(concurrent_parent, Credentials::BOOTSTRAP)
        .unwrap();
    let (second_concurrent_child, second_concurrent_thread) = concurrent_table
        .spawn_child_staged(concurrent_parent, Credentials::BOOTSTRAP)
        .unwrap();
    let concurrent_sources = [
        FileDescriptor::from_raw(0),
        FileDescriptor::from_raw(1),
        FileDescriptor::from_raw(2),
    ];
    concurrent_table
        .inherit_open_fds(
            concurrent_parent,
            first_concurrent_child,
            concurrent_sources,
        )
        .unwrap();
    concurrent_table
        .inherit_open_fds(
            concurrent_parent,
            second_concurrent_child,
            concurrent_sources,
        )
        .unwrap();
    for child in [first_concurrent_child, second_concurrent_child] {
        assert_eq!(
            concurrent_table
                .fd_info(child, FileDescriptor::from_raw(3))
                .unwrap()
                .0,
            concurrent_file_raw
        );
        assert_eq!(
            concurrent_table
                .fd_info(child, FileDescriptor::from_raw(4))
                .unwrap()
                .0,
            concurrent_pipe_read_raw
        );
        assert_eq!(
            concurrent_table
                .fd_info(child, FileDescriptor::from_raw(5))
                .unwrap()
                .0,
            concurrent_pipe_write_raw
        );
    }

    let (invalid_concurrent_child, invalid_concurrent_thread) = concurrent_table
        .spawn_child_staged(concurrent_parent, Credentials::BOOTSTRAP)
        .unwrap();
    assert_eq!(
        concurrent_table.finalize_spawn(
            SpawnTransaction {
                parent: concurrent_parent,
                child: invalid_concurrent_child,
                thread: invalid_concurrent_thread,
            },
            [
                FileDescriptor::from_raw(0),
                FileDescriptor::from_raw(1),
                FileDescriptor::from_raw(31),
            ],
            true,
            None,
        ),
        Err(Error::InvalidFd)
    );
    assert_eq!(
        concurrent_table.fd_info(invalid_concurrent_child, FileDescriptor::from_raw(3)),
        Err(Error::InvalidId)
    );
    assert_eq!(
        concurrent_table.thread_state(invalid_concurrent_thread),
        Err(Error::InvalidId)
    );

    let (capacity_concurrent_child, capacity_concurrent_thread) = concurrent_table
        .spawn_child_staged(concurrent_parent, Credentials::BOOTSTRAP)
        .unwrap();
    concurrent_table
        .process_mut(capacity_concurrent_child)
        .unwrap()
        .session
        .max_fds = 5;
    assert_eq!(
        concurrent_table.finalize_spawn(
            SpawnTransaction {
                parent: concurrent_parent,
                child: capacity_concurrent_child,
                thread: capacity_concurrent_thread,
            },
            concurrent_sources,
            true,
            None,
        ),
        Err(Error::FdCapacity)
    );
    assert_eq!(
        concurrent_table.fd_info(capacity_concurrent_child, FileDescriptor::from_raw(3)),
        Err(Error::InvalidId)
    );
    assert_eq!(
        concurrent_table.thread_state(capacity_concurrent_thread),
        Err(Error::InvalidId)
    );

    concurrent_table
        .close_fd(concurrent_parent, concurrent_file_fd)
        .unwrap();
    concurrent_table
        .close_fd(concurrent_parent, concurrent_pipe_read_fd)
        .unwrap();
    assert_eq!(
        crate::vfs::read_raw(concurrent_file_raw, &mut [0; 1]),
        Ok(1)
    );
    assert_eq!(
        crate::pipe::write_raw(concurrent_pipe_write_raw, &[0x5a]),
        Ok(1)
    );

    for fd in [3, 4, 5] {
        concurrent_table
            .close_fd(first_concurrent_child, FileDescriptor::from_raw(fd))
            .unwrap();
    }
    assert_eq!(
        crate::vfs::read_raw(concurrent_file_raw, &mut [0; 1]),
        Ok(1)
    );
    assert_eq!(
        crate::pipe::write_raw(concurrent_pipe_write_raw, &[0x5a]),
        Ok(1)
    );

    for fd in [3, 4, 5] {
        concurrent_table
            .close_fd(second_concurrent_child, FileDescriptor::from_raw(fd))
            .unwrap();
    }
    assert_eq!(
        crate::vfs::duplicate_raw(concurrent_file_raw),
        Err(crate::vfs::Error::InvalidHandle)
    );
    assert_eq!(
        crate::pipe::write_raw(concurrent_pipe_write_raw, &[0x5a]),
        Err(crate::pipe::Error::BrokenPipe)
    );
    concurrent_table
        .close_fd(concurrent_parent, concurrent_pipe_write_fd)
        .unwrap();
    assert_eq!(
        crate::pipe::close_raw(concurrent_pipe_write_raw),
        Err(crate::pipe::Error::InvalidHandle)
    );
    concurrent_table
        .abort_child(concurrent_parent, first_concurrent_child)
        .unwrap();
    concurrent_table
        .abort_child(concurrent_parent, second_concurrent_child)
        .unwrap();
    assert_eq!(
        concurrent_table.thread_state(first_concurrent_thread),
        Err(Error::InvalidId)
    );
    assert_eq!(
        concurrent_table.thread_state(second_concurrent_thread),
        Err(Error::InvalidId)
    );
}

#[cfg(test)]
mod tests {
    use super::{Credentials, Error, ProcessId, ProcessTable};

    fn table_with_children() -> (ProcessTable, ProcessId, ProcessId, ProcessId) {
        let mut table = ProcessTable::new();
        let (init, _) = table.create_init().unwrap();
        let (first, _) = table.spawn_child(init, Credentials::BOOTSTRAP).unwrap();
        let (second, _) = table.spawn_child(init, Credentials::BOOTSTRAP).unwrap();
        (table, init, first, second)
    }

    #[test]
    fn process_groups_inherit_and_can_be_reassigned_to_a_live_group() {
        let (mut table, init, first, second) = table_with_children();
        assert_eq!(table.get_process_group(first), Ok(init));
        assert_eq!(table.get_process_group(second), Ok(init));

        table.set_process_group(first, first).unwrap();
        assert_eq!(table.get_process_group(first), Ok(first));
        assert_eq!(table.set_process_group(second, first), Ok(()));
        assert_eq!(table.get_process_group(second), Ok(first));
        assert_eq!(
            table.set_process_group(second, ProcessId::from_raw(0)),
            Err(Error::InvalidId)
        );
        assert_eq!(
            table.set_process_group(second, ProcessId::from_raw(99)),
            Err(Error::InvalidId)
        );
    }

    #[test]
    fn group_signal_marks_only_running_members() {
        let (mut table, init, first, second) = table_with_children();
        table.set_process_group(first, first).unwrap();
        table.raise_signal_to_group(init, 7).unwrap();
        assert!(table.signal_pending(init, 7).unwrap());
        assert!(!table.signal_pending(first, 7).unwrap());
        assert!(table.signal_pending(second, 7).unwrap());

        assert_eq!(
            table.raise_signal_to_group(ProcessId::from_raw(99), 7),
            Err(Error::InvalidId)
        );
        assert_eq!(
            table.raise_signal_to_group(init, 64),
            Err(Error::InvalidState)
        );
        assert_eq!(
            table.raise_signal_to_group(ProcessId::from_raw(0), 7),
            Err(Error::InvalidId)
        );
    }

    #[test]
    fn peeking_wait_status_does_not_reap_the_child() {
        let (mut table, init, child, _) = table_with_children();
        table.exit(child, 37).unwrap();

        assert_eq!(table.peek_wait(init, Some(child)), Ok((child, 37)));
        assert_eq!(table.process_state(child), Ok(super::ProcessState::Zombie));
        assert_eq!(table.wait(init, Some(child)), Ok((child, 37)));
        assert_eq!(table.process_state(child), Err(Error::InvalidId));
    }
}
