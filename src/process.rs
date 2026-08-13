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
    state: ProcessState,
    credentials: Credentials,
    session: SessionState,
    exit_status: Option<i32>,
    pending_signals: u64,
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
            state: ProcessState::Creating,
            credentials,
            session: SessionState::DEFAULT,
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
    wake_at: u64,
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
    InvalidSession,
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
        if self.process(parent)?.state != ProcessState::Running {
            return Err(Error::InvalidState);
        }
        let parent_group = self.process(parent)?.pgid;
        let parent_session = self.process(parent)?.session;
        let slot = self
            .processes
            .iter()
            .position(Option::is_none)
            .ok_or(Error::ProcessCapacity)?;
        let process = ProcessId::new(slot, self.process_generations[slot]);
        self.processes[slot] = Some(ProcessRecord::new(
            process,
            Some(parent),
            parent_group,
            credentials,
        ));
        self.process_mut(process)?.session = parent_session;
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
            let fds = record.fds;
            record.fds = [None; MAX_FDS];
            for entry in fds.into_iter().flatten() {
                release_open_file(entry.open_file);
            }
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
        if old_slot == new_slot {
            self.process(process)?.fds[old_slot].ok_or(Error::InvalidFd)?;
            return Ok(new_fd);
        }
        let entry = self.process(process)?.fds[old_slot].ok_or(Error::InvalidFd)?;
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
        self.process(parent)?;
        self.process(child)?;
        let mut sources = [None; 3];
        for (index, fd) in source_fds.into_iter().enumerate() {
            let slot = fd.slot().ok_or(Error::InvalidFd)?;
            sources[index] = Some(self.process(parent)?.fds[slot].ok_or(Error::InvalidFd)?);
        }
        let mut duplicated = [None; 3];
        for (index, source) in sources.into_iter().flatten().enumerate() {
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
        let record = self.process_mut(child)?;
        for (index, source) in sources.into_iter().flatten().enumerate() {
            record.fds[index] = Some(FdEntry {
                open_file: duplicated[index].ok_or(Error::InvalidState)?,
                close_on_exec: source.close_on_exec,
                nonblocking: source.nonblocking,
                readable: source.readable,
                writable: source.writable,
            });
        }
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

    pub fn get_process_group(&self, process: ProcessId) -> Result<ProcessId, Error> {
        Ok(self.process(process)?.pgid)
    }

    pub fn process_group_has_live_member(&self, group: ProcessId) -> bool {
        self.processes.iter().flatten().any(|record| {
            record.pgid == group
                && matches!(record.state, ProcessState::Creating | ProcessState::Running)
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
        if pgid != process
            && !self
                .processes
                .iter()
                .flatten()
                .any(|member| member.pgid == pgid)
        {
            return Err(Error::InvalidId);
        }
        self.process_mut(process)?.pgid = pgid;
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
        if signal >= 64 {
            return Err(Error::InvalidState);
        }
        let bit = 1u64 << signal;
        let mut members = 0;
        for record in self.processes.iter_mut().flatten() {
            if record.pgid == pgid && record.state == ProcessState::Running {
                record.pending_signals |= bit;
                members += 1;
            }
        }
        if members == 0 {
            return Err(Error::InvalidId);
        }
        Ok(())
    }

    pub fn terminate_signal_to_group(&mut self, pgid: ProcessId, signal: u8) -> Result<(), Error> {
        if pgid.get() == 0 || signal >= 64 {
            return Err(Error::InvalidState);
        }
        let members: [Option<ProcessId>; MAX_PROCESSES] = {
            let mut members = [None; MAX_PROCESSES];
            let mut count = 0;
            for record in self.processes.iter().flatten() {
                if record.pgid == pgid && record.state == ProcessState::Running {
                    if count == MAX_PROCESSES {
                        return Err(Error::ProcessCapacity);
                    }
                    members[count] = Some(record.id);
                    count += 1;
                }
            }
            members
        };
        let mut terminated = 0;
        for process in members.into_iter().flatten() {
            self.terminate(process, signal)?;
            terminated += 1;
        }
        if terminated == 0 {
            return Err(Error::InvalidId);
        }
        Ok(())
    }

    fn terminate(&mut self, process: ProcessId, signal: u8) -> Result<(), Error> {
        if process == ProcessId::INIT || self.process(process)?.state != ProcessState::Running {
            return Err(Error::InvalidState);
        }
        {
            let record = self.process_mut(process)?;
            record.state = ProcessState::Exiting;
            record.exit_status = Some(-(signal as i32));
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
        self.process_mut(process)?.credentials = credentials;
        Ok(())
    }

    pub fn session(&self, process: ProcessId) -> Result<SessionState, Error> {
        Ok(self.process(process)?.session)
    }

    pub fn set_session(&mut self, process: ProcessId, session: SessionState) -> Result<(), Error> {
        session.validate()?;
        self.process_mut(process)?.session = session;
        Ok(())
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
            wake_at: 0,
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

pub fn current_credentials() -> Result<Credentials, Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &*RUNTIME.0.get();
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime.table.credentials(process)
    })
}

pub fn set_current_credentials(credentials: Credentials) -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
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
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &*RUNTIME.0.get();
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime.table.session(process)
    })
}

pub fn set_current_session(credentials: Credentials, session: SessionState) -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
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
        runtime.table.set_credentials(process, credentials)?;
        runtime.table.set_session(process, session)
    })
}

pub fn current_process_id() -> Option<ProcessId> {
    crate::arch::without_interrupts(|| unsafe { (&*RUNTIME.0.get()).current_process })
}

// ABI wrappers stay local until syscall dispatch is implemented in the next pass.
#[allow(dead_code)]
pub fn current_process_group_id() -> Option<ProcessId> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &*RUNTIME.0.get();
        runtime
            .current_process
            .and_then(|process| runtime.table.get_process_group(process).ok())
    })
}

#[allow(dead_code)]
pub fn set_current_process_group(pgid: ProcessId) -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime.table.set_process_group(process, pgid)
    })
}

#[allow(dead_code)]
pub fn raise_signal_to_current_group(signal: u8) -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        let pgid = runtime.table.get_process_group(process)?;
        runtime.table.raise_signal_to_group(pgid, signal)
    })
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

pub fn handle_user_fault(fault: crate::vm::FaultInfo) -> crate::vm::FaultResult {
    let Some(process) = current_process_id() else {
        return crate::vm::FaultResult::KernelFatal;
    };
    crate::user_runtime::handle_fault(process, fault).unwrap_or_else(|| {
        if fault.user {
            crate::vm::FaultResult::UserFault(crate::vm::FaultReason::InvalidAddress)
        } else {
            crate::vm::FaultResult::KernelFatal
        }
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

pub fn restore_init_after_user() -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
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
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &*RUNTIME.0.get();
        Some((
            runtime.current_process?.get(),
            runtime.current_thread?.get(),
        ))
    })
}

pub fn exit_current(status: i32) -> Result<Option<ContextSwitch>, Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
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
        if !crate::arch::has_user_context(thread.get()) {
            runtime.current_process = None;
            runtime.current_thread = None;
            return Ok(None);
        }
        let Some(next) = runtime
            .table
            .pick_ready_user_with_init_policy(runtime.current_process, process != ProcessId::INIT)
        else {
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
    crate::arch::without_interrupts(|| unsafe { (&*RUNTIME.0.get()).init_exit_status })
}

pub fn wait_current(child: Option<u32>) -> Result<(u32, i32), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let parent = runtime.current_process.ok_or(Error::InvalidState)?;
        let child = child.map(ProcessId::from_raw);
        let (process, status) = runtime.table.wait(parent, child)?;
        let _ = crate::user_runtime::discard(ProcessId::from_raw(process.get()));
        Ok((process.get(), status))
    })
}

pub fn peek_wait_current(child: Option<u32>) -> Result<(u32, i32), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &*RUNTIME.0.get();
        let parent = runtime.current_process.ok_or(Error::InvalidState)?;
        let child = child.map(ProcessId::from_raw);
        let (process, status) = runtime.table.peek_wait(parent, child)?;
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

pub fn open_current_fd(
    open_file: u32,
    nonblocking: bool,
    readable: bool,
    writable: bool,
) -> Result<u32, Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        Ok(runtime
            .table
            .open_fd_with_flags(process, open_file, nonblocking, readable, writable)?
            .get())
    })
}

pub fn current_fd_info(fd: u32) -> Result<(u32, bool, bool, bool, bool), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &*RUNTIME.0.get();
        let process = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime.table.fd_info(process, FileDescriptor::from_raw(fd))
    })
}

pub fn duplicate_fd_current(old_fd: u32, new_fd: u32) -> Result<u32, Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
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
    if crate::vfs::FileHandle::from_raw(open_file).is_some() {
        crate::vfs::duplicate_raw(open_file).map_err(|_| Error::InvalidFd)?;
    } else if crate::pipe::is_pipe_raw(open_file) {
        crate::pipe::duplicate_raw(open_file).map_err(|_| Error::InvalidFd)?;
    }
    Ok(())
}

fn release_open_file(open_file: u32) {
    if crate::vfs::FileHandle::from_raw(open_file).is_some() {
        let _ = crate::vfs::close_raw(open_file);
    } else if crate::pipe::is_pipe_raw(open_file) {
        let _ = crate::pipe::close_raw(open_file);
    }
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

pub fn yield_current() -> Result<Option<ContextSwitch>, Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let current = runtime.current_thread.ok_or(Error::InvalidState)?;
        if !crate::arch::has_user_context(current.get()) {
            return Ok(None);
        }
        runtime.table.wake_sleepers(crate::time::ticks());
        let Some(next) = runtime.table.pick_ready_user_with_init_policy(
            runtime.current_process,
            runtime.current_process != Some(ProcessId::INIT),
        ) else {
            return Ok(None);
        };
        let switch = runtime.table.switch_to(Some(current), next)?;
        crate::arch::request_user_switch(current.get(), next.get());
        Ok(Some(switch))
    })
}

pub fn commit_user_switch(from: u32, to: u32) -> Result<PhysAddr, Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let from = ThreadId::from_raw(from);
        let to = ThreadId::from_raw(to);
        if runtime.current_thread != Some(from) || runtime.current_process.is_none() {
            return Err(Error::InvalidState);
        }
        let process = runtime
            .table
            .thread_process(to)?
            .ok_or(Error::InvalidState)?;
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
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &*RUNTIME.0.get();
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

pub fn inherit_current_standard_fds(child: ProcessId, source_fds: [u32; 3]) -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let parent = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime
            .table
            .inherit_standard_fds(parent, child, source_fds.map(FileDescriptor::from_raw))
    })
}

pub fn set_process_group_for_child(process: ProcessId, pgid: ProcessId) -> Result<(), Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let caller = runtime.current_process.ok_or(Error::InvalidState)?;
        runtime
            .table
            .authorize_process_group_change(caller, process, pgid)?;
        runtime.table.set_process_group(process, pgid)
    })
}

pub fn sleep_current(duration: u64) -> Result<Option<ContextSwitch>, Error> {
    crate::arch::without_interrupts(|| unsafe {
        let runtime = &mut *RUNTIME.0.get();
        let current = runtime.current_thread.ok_or(Error::InvalidState)?;
        if duration == 0 || !crate::arch::has_user_context(current.get()) {
            return Ok(None);
        }
        let wake_at = crate::time::ticks()
            .saturating_add(crate::time::scheduler_ticks().saturating_mul(duration));
        runtime.table.sleep_thread_until(current, wake_at)?;
        let Some(next) = runtime.table.pick_ready_user_with_init_policy(
            runtime.current_process,
            runtime.current_process != Some(ProcessId::INIT),
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
    crate::arch::without_interrupts(|| unsafe {
        (&mut *RUNTIME.0.get()).table.wake_sleepers(now);
    });
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
    let fd = table.open_fd(child, 7, true, false).unwrap();
    assert_eq!(fd.get(), 3);
    assert_eq!(
        table.fd_info(child, fd).unwrap(),
        (7, false, false, true, false)
    );
    let duplicate = table
        .duplicate_fd(child, fd, FileDescriptor::from_raw(4))
        .unwrap();
    assert_eq!(duplicate.get(), 4);
    assert_eq!(table.fd_info(child, duplicate).unwrap().0, 7);
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
    table.switch_to(None, kernel_thread).unwrap();
    table.account_thread(kernel_thread, 2).unwrap();
    table.preempt_thread(kernel_thread).unwrap();
    assert_eq!(table.thread_accounting(kernel_thread).unwrap(), (2, 1, 1));
    table.close_fd(child, fd).unwrap();
    table.close_fd(child, duplicate).unwrap();
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
