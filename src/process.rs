#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SpawnError {
    NoLoader,
    NotFound,
}

const MAX_PROCESSES: usize = 16;
const PATH_MAX: usize = 48;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    Exited,
    Failed,
    Killed,
}

impl State {
    pub const fn name(self) -> &'static str {
        match self {
            State::Running => "running",
            State::Exited => "exited",
            State::Failed => "failed",
            State::Killed => "killed",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Record {
    pub pid: u64,
    pub path: [u8; PATH_MAX],
    pub path_len: usize,
    pub state: State,
    pub exit: u64,
    pub owner: crate::abi::syscall::IoTarget,
    pub capabilities: crate::capability::Set,
    pub code_len: u16,
    pub started: u64,
    pub ended: u64,
}

impl Record {
    const fn empty() -> Self {
        Self {
            pid: 0,
            path: [0; PATH_MAX],
            path_len: 0,
            state: State::Exited,
            exit: 0,
            owner: crate::abi::syscall::IoTarget::Kernel,
            capabilities: crate::capability::Set::NONE,
            code_len: 0,
            started: 0,
            ended: 0,
        }
    }

    pub fn path(&self) -> &str {
        core::str::from_utf8(&self.path[..self.path_len]).unwrap_or("")
    }

    pub fn runtime(&self) -> u64 {
        self.ended.saturating_sub(self.started)
    }
}

static mut TABLE: [Record; MAX_PROCESSES] = [Record::empty(); MAX_PROCESSES];
static mut NEXT_PID: u64 = 1;
static mut NEXT_SLOT: usize = 0;

pub struct ProcessRequest<'a> {
    pub path: &'a str,
    pub args: &'a [&'a str],
}

pub struct ProcessOutput {
    pub code: i32,
    pub capabilities: crate::capability::Set,
    pub user_code_loaded: bool,
    pub user_code_len: u16,
}

pub struct UserCodeOutput {
    pub pid: u64,
    pub value: u64,
    pub capabilities: crate::capability::Set,
    pub code_len: u16,
}

type Program = fn(&[&str]) -> i32;

pub const NORX_EXEC_MAGIC: u32 = 0x5845_4f42;
pub const NORX_EXEC_ABI: u16 = 1;

#[derive(Clone, Copy)]
pub struct NorxExecutable {
    pub magic: u32,
    pub abi: u16,
    pub entry: u16,
    pub flags: u32,
    pub capabilities: crate::capability::Set,
    pub code_len: u16,
}

struct Executable {
    path: &'static str,
    entry_id: u16,
    entry: Program,
}

const EXECUTABLES: [Executable; 2] = [
    Executable {
        path: "/bin/hello",
        entry_id: 1,
        entry: hello,
    },
    Executable {
        path: "/bin/args",
        entry_id: 2,
        entry: args,
    },
];

pub fn spawn(request: ProcessRequest) -> Result<ProcessOutput, SpawnError> {
    if !crate::vfs::exists(request.path) {
        return Err(SpawnError::NotFound);
    }
    let Some((object, code)) = load_object_with_code(request.path) else {
        return Err(SpawnError::NoLoader);
    };
    let Some(executable) = EXECUTABLES.iter().find(|exe| exe.entry_id == object.entry) else {
        return Err(SpawnError::NoLoader);
    };
    let user_code_loaded = if object.code_len == 0 {
        false
    } else {
        crate::arch::user::load_code(code)
    };
    let code = (executable.entry)(request.args);
    Ok(ProcessOutput {
        code,
        capabilities: object.capabilities,
        user_code_loaded,
        user_code_len: object.code_len,
    })
}

pub fn run_user_code(
    path: &str,
    args: &[&str],
    io: crate::abi::syscall::ProcessIo,
) -> Result<UserCodeOutput, SpawnError> {
    if !crate::vfs::exists(path) {
        return Err(SpawnError::NotFound);
    }
    let Some((object, code)) = load_object_with_code(path) else {
        return Err(SpawnError::NoLoader);
    };
    if object.code_len == 0 || !crate::arch::user::load_code(code) {
        return Err(SpawnError::NoLoader);
    }
    if !crate::arch::user::load_argv(args) {
        return Err(SpawnError::NoLoader);
    }
    let pid = begin(path, object.capabilities, object.code_len, io.stdout);
    let _io = crate::abi::syscall::enter_io(io);
    let Some(value) = crate::arch::user::probe() else {
        finish(pid, State::Failed, 38);
        return Err(SpawnError::NoLoader);
    };
    finish(pid, State::Exited, value);
    Ok(UserCodeOutput {
        pid,
        value,
        capabilities: object.capabilities,
        code_len: object.code_len,
    })
}

pub fn list(mut f: impl FnMut(&'static str, NorxExecutable, crate::capability::Set)) {
    for executable in EXECUTABLES {
        if let Some(object) = load_object(executable.path) {
            f(executable.path, object, object.capabilities);
        }
    }
}

pub fn list_records(mut f: impl FnMut(Record)) {
    crate::arch::without_interrupts(|| unsafe {
        for record in TABLE {
            if record.pid != 0 {
                f(record);
            }
        }
    });
}

pub fn find(pid: u64) -> Option<Record> {
    crate::arch::without_interrupts(|| unsafe {
        core::slice::from_raw_parts(core::ptr::addr_of!(TABLE).cast::<Record>(), MAX_PROCESSES)
            .iter()
            .copied()
            .find(|record| record.pid == pid)
    })
}

pub fn kill(pid: u64) -> Result<Record, KillError> {
    crate::arch::without_interrupts(|| unsafe {
        let table = core::ptr::addr_of_mut!(TABLE) as *mut Record;
        for i in 0..MAX_PROCESSES {
            let record = table.add(i);
            if (*record).pid != pid {
                continue;
            }
            if (*record).state != State::Running {
                return Err(KillError::NotRunning((*record).state));
            }
            (*record).state = State::Killed;
            (*record).exit = 130;
            (*record).ended = crate::time::ticks();
            return Ok(*record);
        }
        Err(KillError::NotFound)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KillError {
    NotFound,
    NotRunning(State),
}

fn begin(
    path: &str,
    capabilities: crate::capability::Set,
    code_len: u16,
    owner: crate::abi::syscall::IoTarget,
) -> u64 {
    crate::arch::without_interrupts(|| unsafe {
        let pid = NEXT_PID;
        NEXT_PID = NEXT_PID.saturating_add(1);
        let slot = NEXT_SLOT % MAX_PROCESSES;
        NEXT_SLOT = (NEXT_SLOT + 1) % MAX_PROCESSES;
        let mut record = Record::empty();
        record.pid = pid;
        record.state = State::Running;
        record.owner = owner;
        record.capabilities = capabilities;
        record.code_len = code_len;
        record.started = crate::time::ticks();
        record.path_len = path.len().min(PATH_MAX);
        record.path[..record.path_len].copy_from_slice(&path.as_bytes()[..record.path_len]);
        TABLE[slot] = record;
        pid
    })
}

fn finish(pid: u64, state: State, exit: u64) {
    crate::arch::without_interrupts(|| unsafe {
        let table = core::ptr::addr_of_mut!(TABLE) as *mut Record;
        for i in 0..MAX_PROCESSES {
            let record = table.add(i);
            if (*record).pid == pid {
                (*record).state = state;
                (*record).exit = exit;
                (*record).ended = crate::time::ticks();
                return;
            }
        }
    });
}

fn valid_object(object: NorxExecutable) -> bool {
    object.magic == NORX_EXEC_MAGIC
        && object.abi == NORX_EXEC_ABI
        && object.entry != 0
        && object.code_len <= 491
}

fn load_object(path: &str) -> Option<NorxExecutable> {
    Some(load_object_with_code(path)?.0)
}

fn load_object_with_code(path: &str) -> Option<(NorxExecutable, &'static [u8])> {
    let mut bytes = [0u8; 511];
    let len = crate::vfs::read(path, &mut bytes)?;
    if len < 20 {
        return None;
    }
    let object = NorxExecutable {
        magic: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        abi: u16::from_le_bytes([bytes[4], bytes[5]]),
        entry: u16::from_le_bytes([bytes[6], bytes[7]]),
        flags: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        capabilities: crate::capability::Set::from_bits(u32::from_le_bytes([
            bytes[12], bytes[13], bytes[14], bytes[15],
        ]) as u64),
        code_len: u16::from_le_bytes([bytes[16], bytes[17]]),
    };
    let code_start = 20;
    let code_end = code_start + object.code_len as usize;
    if !valid_object(object) || code_end > len {
        return None;
    }
    let mut code = [0u8; 491];
    code[..object.code_len as usize].copy_from_slice(&bytes[code_start..code_end]);
    Some((object, leak_code(code, object.code_len as usize)))
}

fn leak_code(code: [u8; 491], len: usize) -> &'static [u8] {
    // TODO(process): replace the fixed loader scratch with process-owned memory
    // once processes stop being synchronous shell calls.
    static mut SCRATCH: [u8; 491] = [0; 491];
    unsafe {
        SCRATCH[..len].copy_from_slice(&code[..len]);
        &SCRATCH[..len]
    }
}

fn hello(args: &[&str]) -> i32 {
    crate::kprint!("hello from Norx process");
    for arg in args {
        crate::kprint!(" {}", arg);
    }
    crate::kprintln!();
    0
}

fn args(args: &[&str]) -> i32 {
    crate::kprintln!("argc={}", args.len());
    for (i, arg) in args.iter().enumerate() {
        crate::kprintln!("argv[{}]={}", i, arg);
    }
    0
}
