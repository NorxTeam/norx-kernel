pub const ABI_VERSION: u16 = 2;
pub const MAX_ARGS: usize = 6;
const MAX_IO: usize = 1024;
const MAX_PATH: usize = 256;
const MAX_SPAWN_ARGUMENTS: usize = 16;
const MAX_SPAWN_ENVIRONMENT: usize = 16;
const MAX_SPAWN_STRING: usize = 256;

pub type UserWord = u64;
pub type UserPointer = u64;
// Negative errno values occupy the top 4095 words.  Keep scheduler control
// returns below that range so, for example, -ENOENT cannot be mistaken for a
// request to leave the user entry path.
pub const EXIT_TO_KERNEL: UserWord = UserWord::MAX - 4096;
pub const SWITCH_TO_USER: UserWord = UserWord::MAX - 4097;
pub const PAGE_FAULT_SWITCH: UserWord = UserWord::MAX - 4098;
pub const PAGE_FAULT_EXIT: UserWord = UserWord::MAX - 4099;

#[repr(u64)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Number {
    Read = 0,
    Write = 1,
    Close = 3,
    Wait = 61,
    Exit = 60,
    GetPid = 39,
    GetTid = 186,
    Yield = 24,
    Sleep = 35,
    Spawn = 400,
    Open = 2,
    Pipe = 22,
    Dup2 = 33,
    WaitStatus = 402,
    Spawn2 = 401,
    SetProcessGroup = 403,
    GetProcessGroup = 404,
    KillProcessGroup = 405,
    TtyGetForeground = 406,
    TtySetForeground = 407,
    TtyGetInfo = 408,
    TtySetWindow = 409,
    SetSession = 410,
    GetCredentials = 411,
    SpawnDelegated = 412,
    Mkdir = 420,
    Rmdir = 421,
    Unlink = 422,
    Rename = 423,
    Link = 424,
    Stat = 425,
    ReadDir = 426,
    Fsync = 427,
    SyncPath = 428,
    Seek = 429,
    Fstat = 430,
    Fchmod = 431,
    Fcntl = 432,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    Never,
    Restartable,
}

#[derive(Clone, Copy)]
pub struct Metadata {
    pub number: Number,
    pub name: &'static str,
    pub arguments: u8,
    pub restart: RestartPolicy,
}

pub const TABLE: [Metadata; 38] = [
    Metadata {
        number: Number::Read,
        name: "read",
        arguments: 3,
        restart: RestartPolicy::Restartable,
    },
    Metadata {
        number: Number::Write,
        name: "write",
        arguments: 3,
        restart: RestartPolicy::Restartable,
    },
    Metadata {
        number: Number::Close,
        name: "close",
        arguments: 1,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Wait,
        name: "wait",
        arguments: 1,
        restart: RestartPolicy::Restartable,
    },
    Metadata {
        number: Number::Exit,
        name: "exit",
        arguments: 1,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::GetPid,
        name: "getpid",
        arguments: 0,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::GetTid,
        name: "gettid",
        arguments: 0,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Yield,
        name: "yield",
        arguments: 0,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Sleep,
        name: "sleep",
        arguments: 1,
        restart: RestartPolicy::Restartable,
    },
    Metadata {
        number: Number::Spawn,
        name: "spawn",
        arguments: 2,
        restart: RestartPolicy::Restartable,
    },
    Metadata {
        number: Number::Open,
        name: "open",
        arguments: 4,
        restart: RestartPolicy::Restartable,
    },
    Metadata {
        number: Number::Pipe,
        name: "pipe",
        arguments: 2,
        restart: RestartPolicy::Restartable,
    },
    Metadata {
        number: Number::Dup2,
        name: "dup2",
        arguments: 2,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::WaitStatus,
        name: "wait_status",
        arguments: 3,
        restart: RestartPolicy::Restartable,
    },
    Metadata {
        number: Number::Spawn2,
        name: "spawn2",
        arguments: 1,
        restart: RestartPolicy::Restartable,
    },
    Metadata {
        number: Number::SetProcessGroup,
        name: "setpgid",
        arguments: 2,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::GetProcessGroup,
        name: "getpgid",
        arguments: 1,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::KillProcessGroup,
        name: "killpg",
        arguments: 2,
        restart: RestartPolicy::Restartable,
    },
    Metadata {
        number: Number::TtyGetForeground,
        name: "tty_get_foreground",
        arguments: 1,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::TtySetForeground,
        name: "tty_set_foreground",
        arguments: 2,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::TtyGetInfo,
        name: "tty_get_info",
        arguments: 2,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::TtySetWindow,
        name: "tty_set_window",
        arguments: 3,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::SetSession,
        name: "set_session",
        arguments: 1,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::GetCredentials,
        name: "get_credentials",
        arguments: 1,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::SpawnDelegated,
        name: "spawn_delegated",
        arguments: 1,
        restart: RestartPolicy::Restartable,
    },
    Metadata {
        number: Number::Mkdir,
        name: "mkdir",
        arguments: 3,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Rmdir,
        name: "rmdir",
        arguments: 2,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Unlink,
        name: "unlink",
        arguments: 2,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Rename,
        name: "rename",
        arguments: 4,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Link,
        name: "link",
        arguments: 4,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Stat,
        name: "stat",
        arguments: 3,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::ReadDir,
        name: "read_dir",
        arguments: 4,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Fsync,
        name: "fsync",
        arguments: 1,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::SyncPath,
        name: "sync_path",
        arguments: 2,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Seek,
        name: "lseek",
        arguments: 3,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Fstat,
        name: "fstat",
        arguments: 2,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Fchmod,
        name: "fchmod",
        arguments: 2,
        restart: RestartPolicy::Never,
    },
    Metadata {
        number: Number::Fcntl,
        name: "fcntl",
        arguments: 3,
        restart: RestartPolicy::Never,
    },
];

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Args {
    pub values: [UserWord; MAX_ARGS],
}

impl Args {
    pub const fn empty() -> Self {
        Self {
            values: [0; MAX_ARGS],
        }
    }
}

#[repr(u64)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Errno {
    Eperm = 1,
    Enoent = 2,
    Enomem = 12,
    Enotty = 25,
    Epipe = 32,
    Ebadf = 9,
    Echild = 10,
    Eintr = 4,
    Eagain = 11,
    Efault = 14,
    Einval = 22,
    Enosys = 38,
    Eoverflow = 75,
    Eexist = 17,
    Enotdir = 20,
    Eisdir = 21,
    Enospc = 28,
    Enotempty = 39,
    Enotsup = 95,
    Enametoolong = 36,
    Exdev = 18,
    Erofs = 30,
    E2big = 7,
}

impl Errno {
    pub const fn return_value(self) -> UserWord {
        0u64.wrapping_sub(self as UserWord)
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Timespec {
    pub seconds: i64,
    pub nanoseconds: i64,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PipeFds {
    pub read: UserWord,
    pub write: UserWord,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct WaitStatus {
    pub kind: u32,
    pub code: i32,
    pub signal: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SpawnSpec {
    pub path: UserPointer,
    pub path_length: UserWord,
    pub argv: UserPointer,
    pub argc: UserWord,
    pub environment: UserPointer,
    pub environment_count: UserWord,
    pub stdin_fd: UserWord,
    pub stdout_fd: UserWord,
    pub stderr_fd: UserWord,
    pub process_group: UserWord,
    pub flags: UserWord,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DelegatedSpawnSpec {
    pub path: UserPointer,
    pub path_length: UserWord,
    pub argv: UserPointer,
    pub argc: UserWord,
    pub environment: UserPointer,
    pub environment_count: UserWord,
    pub stdin_fd: UserWord,
    pub stdout_fd: UserWord,
    pub stderr_fd: UserWord,
    pub process_group: UserWord,
    pub flags: UserWord,
    pub target_uid: u32,
    pub target_gid: u32,
    pub reserved: u32,
    pub capabilities: u64,
}

pub const SPAWN_INHERIT_CREDENTIALS: UserWord = 1 << 2;
pub const CAP_PRIVILEGE_DELEGATION: u64 = 1 << 7;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Credentials {
    pub real_uid: u32,
    pub effective_uid: u32,
    pub saved_uid: u32,
    pub real_gid: u32,
    pub effective_gid: u32,
    pub saved_gid: u32,
    pub capabilities: u64,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SessionSpec {
    pub credentials: Credentials,
    pub cwd: UserPointer,
    pub cwd_length: UserWord,
    pub umask: u32,
    pub reserved: u32,
    pub max_fds: u32,
    pub max_address_space_pages: u32,
    pub max_cpu_ticks: u64,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub kind: u32,
    pub mode: u32,
    pub size: u64,
    pub links: u32,
    pub inode: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct TtyInfo {
    pub flags: u32,
    pub controlling_process: u32,
    pub foreground_group: u32,
    pub columns: u16,
    pub rows: u16,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DirEntry {
    pub kind: u32,
    pub mode: u32,
    pub size: u64,
    pub links: u32,
    pub name_length: u32,
    pub name: [u8; 32],
}

pub const OPEN_READ: UserWord = 1 << 0;
pub const OPEN_WRITE: UserWord = 1 << 1;
pub const OPEN_CREATE: UserWord = 1 << 2;
pub const OPEN_TRUNCATE: UserWord = 1 << 3;
pub const OPEN_APPEND: UserWord = 1 << 4;
pub const OPEN_EXCLUSIVE: UserWord = 1 << 5;
pub const PIPE_NONBLOCK: UserWord = 1 << 0;
pub const F_GETFD: UserWord = 1;
pub const F_SETFD: UserWord = 2;
pub const FD_CLOEXEC: UserWord = 1 << 0;
pub const SPAWN_NEW_PROCESS_GROUP: UserWord = 1 << 0;
pub const SPAWN_FOREGROUND: UserWord = 1 << 1;
pub const WAIT_NONBLOCK: UserWord = 1 << 0;
pub const WAIT_EXITED: u32 = 1;
pub const WAIT_SIGNALED: u32 = 2;
pub const WAIT_STOPPED: u32 = 3;
pub const WAIT_CONTINUED: u32 = 4;
pub const STAT_REGULAR: u32 = 1;
pub const STAT_DIRECTORY: u32 = 2;
pub const MAX_DIR_ENTRIES: usize = 16;
pub const TTY_FLAG_AVAILABLE: u32 = 1 << 0;
pub const TTY_FLAG_SERIAL: u32 = 1 << 1;

pub fn dispatch(number: UserWord, args: Args) -> UserWord {
    match number {
        value if value == Number::Read as UserWord => {
            let fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            let address = args.values[1];
            let length = match usize::try_from(args.values[2]) {
                Ok(length) => length,
                Err(_) => return Errno::Eoverflow.return_value(),
            };
            if length > MAX_IO {
                return Errno::Eoverflow.return_value();
            }
            let (open_file, _, readable, _) = match current_fd_info(fd) {
                Ok(info) => info,
                Err(errno) => return errno.return_value(),
            };
            if !readable {
                return Errno::Ebadf.return_value();
            }
            if let Err(errno) = require_fd(fd, true) {
                return errno.return_value();
            }
            if length == 0 {
                return 0;
            }
            if crate::usercopy::validate(address, length).is_err() {
                return Errno::Efault.return_value();
            }
            let mut buffer = [0u8; MAX_IO];
            if crate::vfs::FileHandle::from_raw(open_file).is_some() {
                return match crate::vfs::read_raw(open_file, &mut buffer[..length]) {
                    Ok(count) => match crate::usercopy::copy_to_user(address, &buffer[..count]) {
                        Ok(copied) => copied as UserWord,
                        Err(_) => Errno::Efault.return_value(),
                    },
                    Err(error) => vfs_errno(error).return_value(),
                };
            }
            if crate::pipe::is_pipe_raw(open_file) {
                return match crate::pipe::read_raw(open_file, &mut buffer[..length]) {
                    Ok(count) => match crate::usercopy::copy_to_user(address, &buffer[..count]) {
                        Ok(copied) => copied as UserWord,
                        Err(_) => Errno::Efault.return_value(),
                    },
                    Err(error) => pipe_errno(error).return_value(),
                };
            }
            let mut count = 0;
            while count < length {
                let Some(byte) = crate::drivers::serial::read() else {
                    break;
                };
                buffer[count] = byte;
                count += 1;
            }
            if count == 0 {
                return Errno::Eagain.return_value();
            }
            match crate::usercopy::copy_to_user(address, &buffer[..count]) {
                Ok(copied) => copied as UserWord,
                Err(_) => Errno::Efault.return_value(),
            }
        }
        value if value == Number::Write as UserWord => {
            let fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            let address = args.values[1];
            let length = match usize::try_from(args.values[2]) {
                Ok(length) => length,
                Err(_) => return Errno::Eoverflow.return_value(),
            };
            if length > MAX_IO {
                return Errno::Eoverflow.return_value();
            }
            let open_file = match current_fd_info(fd) {
                Ok((open_file, _, _, _)) => open_file,
                Err(errno) => return errno.return_value(),
            };
            if let Err(errno) = require_fd(fd, false) {
                return errno.return_value();
            }
            if length == 0 {
                return 0;
            }
            let mut buffer = [0u8; MAX_IO];
            if crate::usercopy::copy_from_user(address, &mut buffer[..length]).is_err() {
                return Errno::Efault.return_value();
            }
            if crate::vfs::FileHandle::from_raw(open_file).is_some() {
                return match crate::vfs::write_raw(open_file, &buffer[..length]) {
                    Ok(count) => count as UserWord,
                    Err(error) => vfs_errno(error).return_value(),
                };
            }
            if crate::pipe::is_pipe_raw(open_file) {
                return match crate::pipe::write_raw(open_file, &buffer[..length]) {
                    Ok(count) => count as UserWord,
                    Err(error) => pipe_errno(error).return_value(),
                };
            }
            crate::log::write_bytes(&buffer[..length]);
            length as UserWord
        }
        value if value == Number::GetPid as UserWord => crate::process::current_ids()
            .map(|(process, _)| process as UserWord)
            .unwrap_or_else(|| Errno::Enosys.return_value()),
        value if value == Number::GetTid as UserWord => crate::process::current_ids()
            .map(|(_, thread)| thread as UserWord)
            .unwrap_or_else(|| Errno::Enosys.return_value()),
        value if value == Number::Exit as UserWord => {
            crate::bootlog::warn_fmt(format_args!(
                "sys_exit pid={:?} status={}",
                crate::process::current_ids(),
                args.values[0]
            ));
            match crate::process::exit_current(args.values[0] as i32) {
                Ok(Some(_)) => SWITCH_TO_USER,
                Ok(None) => {
                    crate::arch::restore_kernel_address_space();
                    EXIT_TO_KERNEL
                }
                Err(_) => Errno::Einval.return_value(),
            }
        }
        value if value == Number::Wait as UserWord => {
            let child = (args.values[0] != 0).then_some(args.values[0] as u32);
            match crate::process::wait_current(child) {
                Ok((process, _)) => process as UserWord,
                Err(crate::process::Error::NoChild) => Errno::Echild.return_value(),
                Err(_) => Errno::Einval.return_value(),
            }
        }
        value if value == Number::WaitStatus as UserWord => {
            let child = (args.values[0] != 0).then_some(args.values[0] as u32);
            if args.values[2] & !WAIT_NONBLOCK != 0 {
                return Errno::Einval.return_value();
            }
            if crate::usercopy::validate(args.values[1], core::mem::size_of::<WaitStatus>())
                .is_err()
            {
                return Errno::Efault.return_value();
            }
            let (process, code) = match crate::process::peek_wait_current(child) {
                Ok(status) => status,
                Err(crate::process::Error::NoChild) if args.values[2] & WAIT_NONBLOCK != 0 => {
                    return Errno::Eagain.return_value()
                }
                Err(crate::process::Error::NoChild) => return Errno::Echild.return_value(),
                Err(_) => return Errno::Einval.return_value(),
            };
            let status = if code < 0 {
                WaitStatus {
                    kind: WAIT_SIGNALED,
                    code: 0,
                    signal: code.unsigned_abs(),
                    reserved: 0,
                }
            } else {
                WaitStatus {
                    kind: WAIT_EXITED,
                    code,
                    signal: 0,
                    reserved: 0,
                }
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&status as *const WaitStatus).cast::<u8>(),
                    core::mem::size_of::<WaitStatus>(),
                )
            };
            if crate::usercopy::copy_to_user(args.values[1], bytes).is_err() {
                return Errno::Efault.return_value();
            }
            match crate::process::wait_current(Some(process)) {
                Ok((reaped, _)) if reaped == process => process as UserWord,
                _ => Errno::Einval.return_value(),
            }
        }
        value if value == Number::Close as UserWord => {
            let fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            if current_fd_info(fd).is_err() {
                return Errno::Ebadf.return_value();
            }
            match crate::process::close_current(fd) {
                Ok(()) => 0,
                Err(_) => Errno::Ebadf.return_value(),
            }
        }
        value if value == Number::Yield as UserWord => match crate::process::yield_current() {
            Ok(Some(_)) => SWITCH_TO_USER,
            Ok(None) => 0,
            Err(_) => Errno::Einval.return_value(),
        },
        value if value == Number::Sleep as UserWord => {
            match crate::process::sleep_current(args.values[0]) {
                Ok(Some(_)) => SWITCH_TO_USER,
                Ok(None) => 0,
                Err(_) => Errno::Einval.return_value(),
            }
        }
        value if value == Number::Spawn as UserWord => {
            let address = args.values[0];
            let length = match usize::try_from(args.values[1]) {
                Ok(length) if length != 0 && length <= MAX_PATH => length,
                _ => return Errno::Einval.return_value(),
            };
            let mut path = [0u8; MAX_PATH];
            if crate::usercopy::copy_from_user(address, &mut path[..length]).is_err() {
                return Errno::Efault.return_value();
            }
            let path = match core::str::from_utf8(&path[..length]) {
                Ok(path) if !path.as_bytes().contains(&0) => path,
                _ => return Errno::Einval.return_value(),
            };
            match crate::service::spawn_user_path(path) {
                Ok(process) => process as UserWord,
                Err(crate::service::SpawnError::NotFound) => Errno::Enoent.return_value(),
                Err(crate::service::SpawnError::Capacity) => Errno::Eagain.return_value(),
                Err(_) => Errno::Einval.return_value(),
            }
        }
        value if value == Number::Spawn2 as UserWord => {
            if crate::usercopy::validate(args.values[0], core::mem::size_of::<SpawnSpec>()).is_err()
            {
                return Errno::Efault.return_value();
            }
            let mut spec_bytes = [0u8; core::mem::size_of::<SpawnSpec>()];
            if crate::usercopy::copy_from_user(args.values[0], &mut spec_bytes).is_err() {
                return Errno::Efault.return_value();
            }
            let spec =
                unsafe { core::ptr::read_unaligned(spec_bytes.as_ptr().cast::<SpawnSpec>()) };
            let path_length = match usize::try_from(spec.path_length) {
                Ok(length) if length != 0 && length <= MAX_PATH => length,
                _ => return Errno::Einval.return_value(),
            };
            let mut argument_storage = [[0u8; MAX_SPAWN_STRING]; MAX_SPAWN_ARGUMENTS];
            let mut arguments = [&[][..]; MAX_SPAWN_ARGUMENTS];
            let argument_count = match copy_user_string_vector(
                spec.argv,
                spec.argc,
                &mut argument_storage,
                &mut arguments,
            ) {
                Ok(count) => count,
                Err(errno) => return errno.return_value(),
            };
            let mut environment_storage = [[0u8; MAX_SPAWN_STRING]; MAX_SPAWN_ENVIRONMENT];
            let mut environment = [&[][..]; MAX_SPAWN_ENVIRONMENT];
            let environment_count = match copy_user_string_vector(
                spec.environment,
                spec.environment_count,
                &mut environment_storage,
                &mut environment,
            ) {
                Ok(count) => count,
                Err(errno) => return errno.return_value(),
            };
            if spec.flags
                & !(SPAWN_NEW_PROCESS_GROUP | SPAWN_FOREGROUND | SPAWN_INHERIT_CREDENTIALS)
                != 0
            {
                return Errno::Einval.return_value();
            }
            let mut path_bytes = [0u8; MAX_PATH];
            if crate::usercopy::copy_from_user(spec.path, &mut path_bytes[..path_length]).is_err() {
                return Errno::Efault.return_value();
            }
            let path = match core::str::from_utf8(&path_bytes[..path_length]) {
                Ok(path) if !path.as_bytes().contains(&0) => path,
                _ => return Errno::Einval.return_value(),
            };
            let transaction =
                match crate::service::spawn_user_path_resumable_with_args_and_flags_transaction(
                    path,
                    &arguments[..argument_count],
                    &environment[..environment_count],
                    spec.flags,
                ) {
                    Ok(child) => child,
                    Err(crate::service::SpawnError::NotFound) => {
                        crate::bootlog::warn_fmt(format_args!("spawn2 path not found: {path}"));
                        return Errno::Enoent.return_value();
                    }
                    Err(crate::service::SpawnError::Capacity) => {
                        return Errno::Eagain.return_value()
                    }
                    Err(_) => return Errno::Einval.return_value(),
                };
            let child_id = transaction.child;
            let source_fds = [spec.stdin_fd, spec.stdout_fd, spec.stderr_fd];
            let source_fds = match (
                u32::try_from(source_fds[0]),
                u32::try_from(source_fds[1]),
                u32::try_from(source_fds[2]),
            ) {
                (Ok(stdin), Ok(stdout), Ok(stderr)) => [stdin, stdout, stderr],
                _ => {
                    discard_spawn(transaction);
                    return Errno::Ebadf.return_value();
                }
            };
            if crate::process::inherit_spawn_standard_fds(transaction, source_fds).is_err() {
                discard_spawn(transaction);
                return Errno::Ebadf.return_value();
            }
            let requested_group = if spec.flags & SPAWN_NEW_PROCESS_GROUP != 0 {
                child_id
            } else {
                crate::process::ProcessId::from_raw(spec.process_group as u32)
            };
            if (spec.flags & SPAWN_NEW_PROCESS_GROUP != 0 || spec.process_group != 0)
                && crate::process::set_process_group_for_spawn(transaction, requested_group)
                    .is_err()
            {
                discard_spawn(transaction);
                return Errno::Eperm.return_value();
            }
            if crate::process::publish_spawn(transaction).is_err() {
                discard_spawn(transaction);
                return Errno::Eagain.return_value();
            }
            child_id.get() as UserWord
        }
        value if value == Number::SpawnDelegated as UserWord => {
            if crate::usercopy::validate(args.values[0], core::mem::size_of::<DelegatedSpawnSpec>())
                .is_err()
            {
                return Errno::Efault.return_value();
            }
            let mut spec_bytes = [0u8; core::mem::size_of::<DelegatedSpawnSpec>()];
            if crate::usercopy::copy_from_user(args.values[0], &mut spec_bytes).is_err() {
                return Errno::Efault.return_value();
            }
            let spec = unsafe {
                core::ptr::read_unaligned(spec_bytes.as_ptr().cast::<DelegatedSpawnSpec>())
            };
            let caller = match crate::process::current_credentials() {
                Ok(credentials) => credentials,
                Err(_) => return Errno::Eperm.return_value(),
            };
            if !caller.has_capability(crate::process::Capability::PrivilegeDelegation as u8)
                || spec.capabilities & !caller.capabilities != 0
                || spec.capabilities
                    & ((1u64 << crate::process::Capability::SessionAdmin as u8)
                        | (1u64 << crate::process::Capability::PrivilegeDelegation as u8))
                    != 0
            {
                return Errno::Eperm.return_value();
            }
            if spec.flags & !(SPAWN_NEW_PROCESS_GROUP | SPAWN_FOREGROUND) != 0 {
                return Errno::Einval.return_value();
            }
            let path_length = match usize::try_from(spec.path_length) {
                Ok(length) if length != 0 && length <= MAX_PATH => length,
                _ => return Errno::Einval.return_value(),
            };
            let mut path_bytes = [0u8; MAX_PATH];
            if crate::usercopy::copy_from_user(spec.path, &mut path_bytes[..path_length]).is_err() {
                return Errno::Efault.return_value();
            }
            let path = match core::str::from_utf8(&path_bytes[..path_length]) {
                Ok(path) if !path.as_bytes().contains(&0) => path,
                _ => return Errno::Einval.return_value(),
            };
            let mut argument_storage = [[0u8; MAX_SPAWN_STRING]; MAX_SPAWN_ARGUMENTS];
            let mut arguments = [&[][..]; MAX_SPAWN_ARGUMENTS];
            let argument_count = match copy_user_string_vector(
                spec.argv,
                spec.argc,
                &mut argument_storage,
                &mut arguments,
            ) {
                Ok(count) => count,
                Err(errno) => return errno.return_value(),
            };
            let mut environment_storage = [[0u8; MAX_SPAWN_STRING]; MAX_SPAWN_ENVIRONMENT];
            let mut environment = [&[][..]; MAX_SPAWN_ENVIRONMENT];
            let environment_count = match copy_user_string_vector(
                spec.environment,
                spec.environment_count,
                &mut environment_storage,
                &mut environment,
            ) {
                Ok(count) => count,
                Err(errno) => return errno.return_value(),
            };
            let credentials = crate::process::Credentials {
                real_uid: spec.target_uid,
                effective_uid: spec.target_uid,
                saved_uid: spec.target_uid,
                real_gid: spec.target_gid,
                effective_gid: spec.target_gid,
                saved_gid: spec.target_gid,
                capabilities: spec.capabilities,
            };
            let transaction =
                match crate::service::spawn_delegated_user_path_resumable_with_args_transaction(
                    path,
                    &arguments[..argument_count],
                    &environment[..environment_count],
                    spec.flags,
                    credentials,
                ) {
                    Ok(child) => child,
                    Err(crate::service::SpawnError::NotFound) => {
                        return Errno::Enoent.return_value()
                    }
                    Err(crate::service::SpawnError::Capacity) => {
                        return Errno::Eagain.return_value()
                    }
                    Err(_) => return Errno::Einval.return_value(),
                };
            if finalize_spawn(
                transaction,
                spec.stdin_fd,
                spec.stdout_fd,
                spec.stderr_fd,
                spec.process_group,
                spec.flags,
            )
            .is_err()
            {
                return Errno::Eperm.return_value();
            }
            transaction.child.get() as UserWord
        }
        value if value == Number::SetSession as UserWord => {
            if crate::usercopy::validate(args.values[0], core::mem::size_of::<SessionSpec>())
                .is_err()
            {
                return Errno::Efault.return_value();
            }
            let mut spec_bytes = [0u8; core::mem::size_of::<SessionSpec>()];
            if crate::usercopy::copy_from_user(args.values[0], &mut spec_bytes).is_err() {
                return Errno::Efault.return_value();
            }
            let spec =
                unsafe { core::ptr::read_unaligned(spec_bytes.as_ptr().cast::<SessionSpec>()) };
            let cwd_length = match usize::try_from(spec.cwd_length) {
                Ok(length) if length != 0 && length <= crate::process::MAX_SESSION_CWD => length,
                _ => return Errno::Einval.return_value(),
            };
            let mut cwd = [0u8; crate::process::MAX_SESSION_CWD];
            if crate::usercopy::copy_from_user(spec.cwd, &mut cwd[..cwd_length]).is_err() {
                return Errno::Efault.return_value();
            }
            let credentials = crate::process::Credentials {
                real_uid: spec.credentials.real_uid,
                effective_uid: spec.credentials.effective_uid,
                saved_uid: spec.credentials.saved_uid,
                real_gid: spec.credentials.real_gid,
                effective_gid: spec.credentials.effective_gid,
                saved_gid: spec.credentials.saved_gid,
                capabilities: spec.credentials.capabilities,
            };
            let session = crate::process::SessionState {
                cwd,
                cwd_length: cwd_length as u16,
                umask: match u16::try_from(spec.umask) {
                    Ok(value) => value,
                    Err(_) => return Errno::Einval.return_value(),
                },
                max_fds: spec.max_fds,
                max_address_space_pages: spec.max_address_space_pages,
                max_cpu_ticks: spec.max_cpu_ticks,
            };
            match crate::process::set_current_session(credentials, session) {
                Ok(()) => 0,
                Err(crate::process::Error::PermissionDenied) => Errno::Eperm.return_value(),
                Err(crate::process::Error::InvalidSession) => Errno::Einval.return_value(),
                Err(_) => Errno::Einval.return_value(),
            }
        }
        value if value == Number::GetCredentials as UserWord => {
            if crate::usercopy::validate(args.values[0], core::mem::size_of::<Credentials>())
                .is_err()
            {
                return Errno::Efault.return_value();
            }
            let credentials = match crate::process::current_credentials() {
                Ok(value) => value,
                Err(_) => return Errno::Einval.return_value(),
            };
            let output = Credentials {
                real_uid: credentials.real_uid,
                effective_uid: credentials.effective_uid,
                saved_uid: credentials.saved_uid,
                real_gid: credentials.real_gid,
                effective_gid: credentials.effective_gid,
                saved_gid: credentials.saved_gid,
                capabilities: credentials.capabilities,
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&output as *const Credentials).cast::<u8>(),
                    core::mem::size_of::<Credentials>(),
                )
            };
            match crate::usercopy::copy_to_user(args.values[0], bytes) {
                Ok(_) => 0,
                Err(_) => Errno::Efault.return_value(),
            }
        }
        value if value == Number::Open as UserWord => {
            let address = args.values[0];
            let length = match usize::try_from(args.values[1]) {
                Ok(length) if length != 0 && length <= MAX_PATH => length,
                _ => return Errno::Einval.return_value(),
            };
            let flags = args.values[2];
            let allowed =
                OPEN_READ | OPEN_WRITE | OPEN_CREATE | OPEN_TRUNCATE | OPEN_APPEND | OPEN_EXCLUSIVE;
            if flags & !allowed != 0 || flags & (OPEN_READ | OPEN_WRITE) == 0 {
                return Errno::Einval.return_value();
            }
            if flags & (OPEN_TRUNCATE | OPEN_APPEND) != 0 && flags & OPEN_WRITE == 0 {
                return Errno::Einval.return_value();
            }
            if crate::usercopy::validate(address, length).is_err() {
                return Errno::Efault.return_value();
            }
            let mut path = [0u8; MAX_PATH];
            if crate::usercopy::copy_from_user(address, &mut path[..length]).is_err() {
                return Errno::Efault.return_value();
            }
            let path = match core::str::from_utf8(&path[..length]) {
                Ok(path) if !path.as_bytes().contains(&0) => path,
                _ => return Errno::Einval.return_value(),
            };
            let requested_mode = match u16::try_from(args.values[3]) {
                Ok(mode) => mode & 0o777,
                Err(_) => return Errno::Einval.return_value(),
            };
            let umask = match crate::process::current_session() {
                Ok(session) => session.umask,
                Err(_) => return Errno::Einval.return_value(),
            };
            if flags & OPEN_EXCLUSIVE != 0 && flags & OPEN_CREATE == 0 {
                return Errno::Einval.return_value();
            }
            let options = crate::vfs::OpenOptions {
                read: flags & OPEN_READ != 0,
                write: flags & OPEN_WRITE != 0,
                create: flags & OPEN_CREATE != 0,
                truncate: flags & OPEN_TRUNCATE != 0,
                append: flags & OPEN_APPEND != 0,
                exclusive: flags & OPEN_EXCLUSIVE != 0,
                mode: requested_mode & !umask,
            };
            let handle = match crate::vfs::open(path, options) {
                Ok(handle) => handle,
                Err(error) => return vfs_errno(error).return_value(),
            };
            match crate::process::open_current_fd(handle.raw(), false, options.read, options.write)
            {
                Ok(fd) => fd as UserWord,
                Err(_) => {
                    let _ = crate::vfs::close(handle);
                    Errno::Enomem.return_value()
                }
            }
        }
        value if value == Number::Mkdir as UserWord => {
            let mut path_bytes = [0u8; MAX_PATH];
            let path = match copy_user_path(args.values[0], args.values[1], &mut path_bytes) {
                Ok(path) => path,
                Err(errno) => return errno.return_value(),
            };
            let mode = match u16::try_from(args.values[2]) {
                Ok(mode) => mode & 0o777,
                Err(_) => return Errno::Einval.return_value(),
            };
            let umask = match crate::process::current_session() {
                Ok(session) => session.umask,
                Err(_) => return Errno::Einval.return_value(),
            };
            match crate::vfs::mkdir_with_mode(path, mode & !umask) {
                Ok(()) => 0,
                Err(error) => vfs_errno(error).return_value(),
            }
        }
        value if value == Number::Rmdir as UserWord => {
            let mut path_bytes = [0u8; MAX_PATH];
            let path = match copy_user_path(args.values[0], args.values[1], &mut path_bytes) {
                Ok(path) => path,
                Err(errno) => return errno.return_value(),
            };
            match crate::vfs::remove_dir(path) {
                Ok(()) => 0,
                Err(error) => vfs_errno(error).return_value(),
            }
        }
        value if value == Number::Unlink as UserWord => {
            let mut path_bytes = [0u8; MAX_PATH];
            let path = match copy_user_path(args.values[0], args.values[1], &mut path_bytes) {
                Ok(path) => path,
                Err(errno) => return errno.return_value(),
            };
            match crate::vfs::unlink(path) {
                Ok(()) => 0,
                Err(error) => vfs_errno(error).return_value(),
            }
        }
        value if value == Number::Rename as UserWord || value == Number::Link as UserWord => {
            let mut old_bytes = [0u8; MAX_PATH];
            let mut new_bytes = [0u8; MAX_PATH];
            let old = match copy_user_path(args.values[0], args.values[1], &mut old_bytes) {
                Ok(path) => path,
                Err(errno) => return errno.return_value(),
            };
            let new = match copy_user_path(args.values[2], args.values[3], &mut new_bytes) {
                Ok(path) => path,
                Err(errno) => return errno.return_value(),
            };
            let result = if value == Number::Rename as UserWord {
                crate::vfs::rename(old, new)
            } else {
                crate::vfs::link(old, new)
            };
            match result {
                Ok(()) => 0,
                Err(error) => vfs_errno(error).return_value(),
            }
        }
        value if value == Number::Stat as UserWord => {
            if crate::usercopy::validate(args.values[2], core::mem::size_of::<Stat>()).is_err() {
                return Errno::Efault.return_value();
            }
            let mut path_bytes = [0u8; MAX_PATH];
            let path = match copy_user_path(args.values[0], args.values[1], &mut path_bytes) {
                Ok(path) => path,
                Err(errno) => return errno.return_value(),
            };
            let value = match crate::vfs::stat(path) {
                Ok(value) => value,
                Err(error) => return vfs_errno(error).return_value(),
            };
            let stat = Stat {
                inode: value.inode,
                kind: match value.kind {
                    crate::vfs::NodeType::Regular => STAT_REGULAR,
                    crate::vfs::NodeType::Directory => STAT_DIRECTORY,
                },
                mode: value.mode as u32,
                size: value.size as u64,
                links: value.links,
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&stat as *const Stat).cast::<u8>(),
                    core::mem::size_of::<Stat>(),
                )
            };
            match crate::usercopy::copy_to_user(args.values[2], bytes) {
                Ok(_) => 0,
                Err(_) => Errno::Efault.return_value(),
            }
        }
        value if value == Number::ReadDir as UserWord => {
            let capacity = match usize::try_from(args.values[3]) {
                Ok(capacity) if capacity <= MAX_DIR_ENTRIES => capacity,
                _ => return Errno::Eoverflow.return_value(),
            };
            let byte_length = match capacity.checked_mul(core::mem::size_of::<DirEntry>()) {
                Some(length) => length,
                None => return Errno::Eoverflow.return_value(),
            };
            if crate::usercopy::validate(args.values[2], byte_length).is_err() {
                return Errno::Efault.return_value();
            }
            let mut path_bytes = [0u8; MAX_PATH];
            let path = match copy_user_path(args.values[0], args.values[1], &mut path_bytes) {
                Ok(path) => path,
                Err(errno) => return errno.return_value(),
            };
            let empty = crate::vfs::DirectoryEntry {
                kind: crate::vfs::NodeType::Regular,
                mode: 0,
                size: 0,
                links: 0,
                name: [0; 31],
                name_length: 0,
            };
            let mut entries = [empty; MAX_DIR_ENTRIES];
            let count = match crate::vfs::read_dir(path, &mut entries[..capacity]) {
                Ok(count) => count,
                Err(error) => return vfs_errno(error).return_value(),
            };
            for (index, entry) in entries.iter().take(count).enumerate() {
                let mut name = [0u8; 32];
                name[..entry.name_length].copy_from_slice(&entry.name[..entry.name_length]);
                let output = DirEntry {
                    kind: match entry.kind {
                        crate::vfs::NodeType::Regular => STAT_REGULAR,
                        crate::vfs::NodeType::Directory => STAT_DIRECTORY,
                    },
                    mode: entry.mode as u32,
                    size: entry.size as u64,
                    links: entry.links,
                    name_length: entry.name_length as u32,
                    name,
                };
                let address = match args.values[2]
                    .checked_add((index * core::mem::size_of::<DirEntry>()) as UserWord)
                {
                    Some(address) => address,
                    None => return Errno::Eoverflow.return_value(),
                };
                let bytes = unsafe {
                    core::slice::from_raw_parts(
                        (&output as *const DirEntry).cast::<u8>(),
                        core::mem::size_of::<DirEntry>(),
                    )
                };
                if crate::usercopy::copy_to_user(address, bytes).is_err() {
                    return Errno::Efault.return_value();
                }
            }
            count as UserWord
        }
        value if value == Number::Fsync as UserWord => {
            let fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            let (_, _, _, writable) = match current_fd_info(fd) {
                Ok(info) => info,
                Err(errno) => return errno.return_value(),
            };
            if !writable {
                return Errno::Ebadf.return_value();
            }
            Errno::Enotsup.return_value()
        }
        value if value == Number::SyncPath as UserWord => {
            let mut path_bytes = [0u8; MAX_PATH];
            let path = match copy_user_path(args.values[0], args.values[1], &mut path_bytes) {
                Ok(path) => path,
                Err(errno) => return errno.return_value(),
            };
            match crate::vfs::sync_path(path) {
                Ok(()) => Errno::Enotsup.return_value(),
                Err(error) => vfs_errno(error).return_value(),
            }
        }
        value if value == Number::Seek as UserWord => {
            let fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            let (open_file, _, _, _) = match current_fd_info(fd) {
                Ok(info) => info,
                Err(errno) => return errno.return_value(),
            };
            let Some(handle) = crate::vfs::FileHandle::from_raw(open_file) else {
                return Errno::Einval.return_value();
            };
            let whence = match u32::try_from(args.values[2]) {
                Ok(whence) => whence,
                Err(_) => return Errno::Einval.return_value(),
            };
            match crate::vfs::seek_from(handle, args.values[1] as i64, whence) {
                Ok(offset) => offset as UserWord,
                Err(error) => vfs_errno(error).return_value(),
            }
        }
        value if value == Number::Fstat as UserWord => {
            let fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            if crate::usercopy::validate(args.values[1], core::mem::size_of::<Stat>()).is_err() {
                return Errno::Efault.return_value();
            }
            let (open_file, _, _, _) = match current_fd_info(fd) {
                Ok(info) => info,
                Err(errno) => return errno.return_value(),
            };
            let Some(handle) = crate::vfs::FileHandle::from_raw(open_file) else {
                return Errno::Ebadf.return_value();
            };
            let value = match crate::vfs::stat_handle(handle) {
                Ok(value) => value,
                Err(error) => return vfs_errno(error).return_value(),
            };
            let stat = Stat {
                inode: value.inode,
                kind: match value.kind {
                    crate::vfs::NodeType::Regular => STAT_REGULAR,
                    crate::vfs::NodeType::Directory => STAT_DIRECTORY,
                },
                mode: value.mode as u32,
                size: value.size as u64,
                links: value.links,
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&stat as *const Stat).cast::<u8>(),
                    core::mem::size_of::<Stat>(),
                )
            };
            match crate::usercopy::copy_to_user(args.values[1], bytes) {
                Ok(_) => 0,
                Err(_) => Errno::Efault.return_value(),
            }
        }
        value if value == Number::Fchmod as UserWord => {
            let fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            let mode = match u16::try_from(args.values[1]) {
                Ok(mode) => mode,
                Err(_) => return Errno::Einval.return_value(),
            };
            let (open_file, _, _, _) = match current_fd_info(fd) {
                Ok(info) => info,
                Err(errno) => return errno.return_value(),
            };
            let Some(handle) = crate::vfs::FileHandle::from_raw(open_file) else {
                return Errno::Ebadf.return_value();
            };
            match crate::vfs::fchmod(handle, mode) {
                Ok(()) => 0,
                Err(error) => vfs_errno(error).return_value(),
            }
        }
        value if value == Number::Fcntl as UserWord => {
            let fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            let command = args.values[1];
            match command {
                F_GETFD => match crate::process::current_fd_info(fd) {
                    Ok((_, close_on_exec, _, _, _)) => {
                        if close_on_exec {
                            FD_CLOEXEC
                        } else {
                            0
                        }
                    }
                    Err(_) => Errno::Ebadf.return_value(),
                },
                F_SETFD => {
                    let flags = args.values[2];
                    if flags & !FD_CLOEXEC != 0 {
                        return Errno::Einval.return_value();
                    }
                    match crate::process::set_close_on_exec_current(fd, flags & FD_CLOEXEC != 0) {
                        Ok(()) => 0,
                        Err(
                            crate::process::Error::InvalidFd | crate::process::Error::InvalidId,
                        ) => Errno::Ebadf.return_value(),
                        Err(_) => Errno::Einval.return_value(),
                    }
                }
                _ => Errno::Einval.return_value(),
            }
        }
        value if value == Number::Pipe as UserWord => {
            let address = args.values[0];
            if args.values[1] & !PIPE_NONBLOCK != 0 {
                return Errno::Einval.return_value();
            }
            if crate::usercopy::validate(address, core::mem::size_of::<PipeFds>()).is_err() {
                return Errno::Efault.return_value();
            }
            let (read_raw, write_raw) = match crate::pipe::create() {
                Ok(handles) => handles,
                Err(error) => return pipe_errno(error).return_value(),
            };
            let nonblocking = args.values[1] & PIPE_NONBLOCK != 0;
            let read_fd = match crate::process::open_current_fd(read_raw, nonblocking, true, false)
            {
                Ok(fd) => fd,
                Err(_) => {
                    let _ = crate::pipe::close_raw(read_raw);
                    let _ = crate::pipe::close_raw(write_raw);
                    return Errno::Enomem.return_value();
                }
            };
            let write_fd =
                match crate::process::open_current_fd(write_raw, nonblocking, false, true) {
                    Ok(fd) => fd,
                    Err(_) => {
                        let _ = crate::process::close_current(read_fd);
                        let _ = crate::pipe::close_raw(write_raw);
                        return Errno::Enomem.return_value();
                    }
                };
            let fds = PipeFds {
                read: read_fd as UserWord,
                write: write_fd as UserWord,
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&fds as *const PipeFds).cast::<u8>(),
                    core::mem::size_of::<PipeFds>(),
                )
            };
            if crate::usercopy::copy_to_user(address, bytes).is_err() {
                let _ = crate::process::close_current(read_fd);
                let _ = crate::process::close_current(write_fd);
                return Errno::Efault.return_value();
            }
            0
        }
        value if value == Number::SetProcessGroup as UserWord => {
            let caller = match crate::process::current_process_id() {
                Some(process) => process,
                None => return Errno::Einval.return_value(),
            };
            let process = if args.values[0] == 0 {
                caller
            } else {
                crate::process::ProcessId::from_raw(args.values[0] as u32)
            };
            let group = args.values[1];
            let group = if group == 0 {
                process.get() as UserWord
            } else {
                group
            };
            let result = crate::process::with_process_table(|table| {
                table.authorize_process_group_change(
                    caller,
                    process,
                    crate::process::ProcessId::from_raw(group as u32),
                )?;
                table.set_process_group(process, crate::process::ProcessId::from_raw(group as u32))
            });
            match result {
                Ok(()) => 0,
                Err(crate::process::Error::PermissionDenied) => Errno::Eperm.return_value(),
                Err(_) => Errno::Einval.return_value(),
            }
        }
        value if value == Number::GetProcessGroup as UserWord => {
            if args.values[0] == 0 {
                crate::process::current_process_group_id()
                    .map(|group| group.get() as UserWord)
                    .unwrap_or_else(|| Errno::Einval.return_value())
            } else {
                crate::process::with_process_table(|table| {
                    table
                        .get_process_group(crate::process::ProcessId::from_raw(
                            args.values[0] as u32,
                        ))
                        .map(|group| group.get() as UserWord)
                        .unwrap_or_else(|_| Errno::Einval.return_value())
                })
            }
        }
        value if value == Number::KillProcessGroup as UserWord => {
            let group = args.values[0];
            let signal = match u8::try_from(args.values[1]) {
                Ok(signal) => signal,
                Err(_) => return Errno::Einval.return_value(),
            };
            let result = if group == 0 {
                crate::process::raise_signal_to_current_group(signal)
            } else {
                crate::process::with_process_table(|table| {
                    let caller = crate::process::current_process_id()
                        .ok_or(crate::process::Error::InvalidState)?;
                    let group = crate::process::ProcessId::from_raw(group as u32);
                    table.authorize_process_group_signal(caller, group)?;
                    table.terminate_signal_to_group(group, signal)
                })
            };
            match result {
                Ok(()) => 0,
                Err(crate::process::Error::PermissionDenied) => Errno::Eperm.return_value(),
                Err(_) => Errno::Einval.return_value(),
            }
        }
        value if value == Number::TtyGetForeground as UserWord => {
            let fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            if current_fd_info(fd).is_err() {
                return Errno::Ebadf.return_value();
            }
            match crate::tty::get_foreground(fd) {
                Ok(group) => group as UserWord,
                Err(crate::tty::Error::NotTty) => Errno::Enotty.return_value(),
                Err(crate::tty::Error::PermissionDenied) => Errno::Eperm.return_value(),
                Err(crate::tty::Error::InvalidGroup) => Errno::Einval.return_value(),
                Err(crate::tty::Error::InvalidWindow) => Errno::Einval.return_value(),
            }
        }
        value if value == Number::TtySetForeground as UserWord => {
            let fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            if current_fd_info(fd).is_err() {
                return Errno::Ebadf.return_value();
            }
            match crate::tty::set_foreground(
                fd,
                crate::process::ProcessId::from_raw(args.values[1] as u32),
            ) {
                Ok(()) => 0,
                Err(crate::tty::Error::NotTty) => Errno::Enotty.return_value(),
                Err(crate::tty::Error::PermissionDenied) => Errno::Eperm.return_value(),
                Err(crate::tty::Error::InvalidGroup) => Errno::Einval.return_value(),
                Err(crate::tty::Error::InvalidWindow) => Errno::Einval.return_value(),
            }
        }
        value if value == Number::TtyGetInfo as UserWord => {
            let fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            let address = args.values[1];
            if current_fd_info(fd).is_err() {
                return Errno::Ebadf.return_value();
            }
            if crate::usercopy::validate(address, core::mem::size_of::<TtyInfo>()).is_err() {
                return Errno::Efault.return_value();
            }
            let tty = match crate::tty::get_info(fd) {
                Ok(info) => TtyInfo {
                    flags: info.flags,
                    controlling_process: info.controlling_process,
                    foreground_group: info.foreground_group,
                    columns: info.columns,
                    rows: info.rows,
                    reserved: info.reserved,
                },
                Err(crate::tty::Error::NotTty) => return Errno::Enotty.return_value(),
                Err(crate::tty::Error::PermissionDenied) => return Errno::Eperm.return_value(),
                Err(crate::tty::Error::InvalidGroup | crate::tty::Error::InvalidWindow) => {
                    return Errno::Einval.return_value()
                }
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&tty as *const TtyInfo).cast::<u8>(),
                    core::mem::size_of::<TtyInfo>(),
                )
            };
            if crate::usercopy::copy_to_user(address, bytes).is_err() {
                return Errno::Efault.return_value();
            }
            0
        }
        value if value == Number::TtySetWindow as UserWord => {
            let fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            let columns = match u16::try_from(args.values[1]) {
                Ok(value) => value,
                Err(_) => return Errno::Einval.return_value(),
            };
            let rows = match u16::try_from(args.values[2]) {
                Ok(value) => value,
                Err(_) => return Errno::Einval.return_value(),
            };
            if current_fd_info(fd).is_err() {
                return Errno::Ebadf.return_value();
            }
            match crate::tty::set_window(fd, columns, rows) {
                Ok(()) => 0,
                Err(crate::tty::Error::NotTty) => Errno::Enotty.return_value(),
                Err(crate::tty::Error::PermissionDenied) => Errno::Eperm.return_value(),
                Err(crate::tty::Error::InvalidGroup | crate::tty::Error::InvalidWindow) => {
                    Errno::Einval.return_value()
                }
            }
        }
        value if value == Number::Dup2 as UserWord => {
            let old_fd = match u32::try_from(args.values[0]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            let new_fd = match u32::try_from(args.values[1]) {
                Ok(fd) => fd,
                Err(_) => return Errno::Ebadf.return_value(),
            };
            match crate::process::duplicate_fd_current(old_fd, new_fd) {
                Ok(fd) => fd as UserWord,
                Err(_) => Errno::Ebadf.return_value(),
            }
        }
        _ => Errno::Enosys.return_value(),
    }
}

fn require_fd(fd: u32, read: bool) -> Result<(), Errno> {
    let (_, readable, writable) =
        crate::process::current_fd_access(fd).map_err(|error| match error {
            crate::process::Error::InvalidFd | crate::process::Error::InvalidId => Errno::Ebadf,
            _ => Errno::Einval,
        })?;
    if (read && readable) || (!read && writable) {
        Ok(())
    } else {
        Err(Errno::Ebadf)
    }
}

fn copy_user_path<'a>(
    address: UserPointer,
    length: UserWord,
    output: &'a mut [u8; MAX_PATH],
) -> Result<&'a str, Errno> {
    let length = usize::try_from(length).map_err(|_| Errno::Eoverflow)?;
    if length == 0 || length > MAX_PATH {
        return Err(Errno::Einval);
    }
    if crate::usercopy::validate(address, length).is_err()
        || crate::usercopy::copy_from_user(address, &mut output[..length]).is_err()
    {
        return Err(Errno::Efault);
    }
    let path = core::str::from_utf8(&output[..length]).map_err(|_| Errno::Einval)?;
    if path.as_bytes().contains(&0) {
        return Err(Errno::Einval);
    }
    Ok(path)
}

fn copy_user_string_vector<'a>(
    address: UserPointer,
    count: UserWord,
    storage: &'a mut [[u8; MAX_SPAWN_STRING]],
    output: &mut [&'a [u8]],
) -> Result<usize, Errno> {
    let count = usize::try_from(count).map_err(|_| Errno::Eoverflow)?;
    if count > storage.len() || count > output.len() {
        return Err(Errno::E2big);
    }
    if count == 0 {
        return Ok(0);
    }
    let pointer_bytes = count
        .checked_mul(core::mem::size_of::<UserPointer>())
        .ok_or(Errno::Eoverflow)?;
    if crate::usercopy::validate(address, pointer_bytes).is_err() {
        return Err(Errno::Efault);
    }
    for index in 0..count {
        let pointer_address = address
            .checked_add((index * core::mem::size_of::<UserPointer>()) as UserPointer)
            .ok_or(Errno::Eoverflow)?;
        let mut pointer_bytes = [0u8; core::mem::size_of::<UserPointer>()];
        if crate::usercopy::copy_from_user(pointer_address, &mut pointer_bytes).is_err() {
            return Err(Errno::Efault);
        }
        let pointer = UserPointer::from_ne_bytes(pointer_bytes);
        if pointer == 0 {
            return Err(Errno::Einval);
        }
        let mut length = 0;
        while length < MAX_SPAWN_STRING {
            let byte_address = pointer
                .checked_add(length as UserPointer)
                .ok_or(Errno::Eoverflow)?;
            let mut byte = [0u8; 1];
            if crate::usercopy::copy_from_user(byte_address, &mut byte).is_err() {
                return Err(Errno::Efault);
            }
            if byte[0] == 0 {
                break;
            }
            storage[index][length] = byte[0];
            length += 1;
        }
        if length == MAX_SPAWN_STRING {
            return Err(Errno::E2big);
        }
        let bytes = unsafe { core::slice::from_raw_parts(storage[index].as_ptr(), length) };
        core::str::from_utf8(bytes).map_err(|_| Errno::Einval)?;
        output[index] = bytes;
    }
    Ok(count)
}

fn finalize_spawn(
    transaction: crate::process::SpawnTransaction,
    stdin_fd: UserWord,
    stdout_fd: UserWord,
    stderr_fd: UserWord,
    process_group: UserWord,
    flags: UserWord,
) -> Result<(), ()> {
    let source_fds = [stdin_fd, stdout_fd, stderr_fd];
    let source_fds = match (
        u32::try_from(source_fds[0]),
        u32::try_from(source_fds[1]),
        u32::try_from(source_fds[2]),
    ) {
        (Ok(stdin), Ok(stdout), Ok(stderr)) => [stdin, stdout, stderr],
        _ => {
            discard_spawn(transaction);
            return Err(());
        }
    };
    if crate::process::inherit_spawn_standard_fds(transaction, source_fds).is_err() {
        discard_spawn(transaction);
        return Err(());
    }
    let requested_group = if flags & SPAWN_NEW_PROCESS_GROUP != 0 {
        transaction.child
    } else {
        crate::process::ProcessId::from_raw(process_group as u32)
    };
    if (flags & SPAWN_NEW_PROCESS_GROUP != 0 || process_group != 0)
        && crate::process::set_process_group_for_spawn(transaction, requested_group).is_err()
    {
        discard_spawn(transaction);
        return Err(());
    }
    if crate::process::publish_spawn(transaction).is_err() {
        discard_spawn(transaction);
        return Err(());
    }
    Ok(())
}

fn discard_spawn(transaction: crate::process::SpawnTransaction) {
    let _ = crate::process::discard_child(transaction.parent, transaction.child);
    let _ = crate::user_runtime::discard(transaction.child);
}

fn current_fd_info(fd: u32) -> Result<(u32, bool, bool, bool), Errno> {
    let (open_file, _, nonblocking, readable, writable) = crate::process::current_fd_info(fd)
        .map_err(|error| match error {
            crate::process::Error::InvalidFd | crate::process::Error::InvalidId => Errno::Ebadf,
            _ => Errno::Einval,
        })?;
    Ok((open_file, nonblocking, readable, writable))
}

fn vfs_errno(error: crate::vfs::Error) -> Errno {
    match error {
        crate::vfs::Error::NotFound => Errno::Enoent,
        crate::vfs::Error::PermissionDenied => Errno::Eperm,
        crate::vfs::Error::ReadOnly => Errno::Erofs,
        crate::vfs::Error::NoSpace => Errno::Enospc,
        crate::vfs::Error::AlreadyExists => Errno::Eexist,
        crate::vfs::Error::NotDirectory => Errno::Enotdir,
        crate::vfs::Error::IsDirectory => Errno::Eisdir,
        crate::vfs::Error::NotEmpty => Errno::Enotempty,
        crate::vfs::Error::NameTooLong => Errno::Enametoolong,
        crate::vfs::Error::MountPointBusy | crate::vfs::Error::Busy => Errno::Eperm,
        crate::vfs::Error::InvalidHandle => Errno::Ebadf,
        crate::vfs::Error::InvalidPath => Errno::Einval,
        crate::vfs::Error::OffsetOutOfRange => Errno::Einval,
        _ => Errno::Einval,
    }
}

fn pipe_errno(error: crate::pipe::Error) -> Errno {
    match error {
        crate::pipe::Error::WouldBlock => Errno::Eagain,
        crate::pipe::Error::BrokenPipe => Errno::Epipe,
        crate::pipe::Error::InvalidHandle | crate::pipe::Error::InvalidEndpoint => Errno::Ebadf,
        crate::pipe::Error::NoSpace => Errno::Enomem,
    }
}

pub const fn is_error(value: UserWord) -> bool {
    value >= UserWord::MAX - 4095
}

pub fn contract_self_check() {
    assert_eq!(ABI_VERSION, 2);
    assert!(!is_error(EXIT_TO_KERNEL));
    assert!(!is_error(SWITCH_TO_USER));
    assert_ne!(EXIT_TO_KERNEL, SWITCH_TO_USER);
    assert_eq!(TABLE.len(), 38);
    assert!(TABLE.iter().all(|entry| entry.arguments <= MAX_ARGS as u8));
    assert_eq!(TABLE[0].number as UserWord, Number::Read as UserWord);
    assert!(TABLE.iter().all(|entry| !entry.name.is_empty()));
    assert_eq!(Number::Fcntl as UserWord, 432);
    assert_eq!(TABLE[37].number as UserWord, Number::Fcntl as UserWord);
    assert_eq!(TABLE[37].arguments, 3);
    assert!(TABLE
        .iter()
        .any(|entry| entry.restart == RestartPolicy::Restartable));
    assert_eq!(
        core::mem::size_of::<Args>(),
        MAX_ARGS * core::mem::size_of::<UserWord>()
    );
    assert_eq!(
        core::mem::align_of::<Timespec>(),
        core::mem::align_of::<u64>()
    );
    assert_eq!(core::mem::size_of::<PipeFds>(), 16);
    assert_eq!(core::mem::size_of::<WaitStatus>(), 16);
    assert_eq!(core::mem::size_of::<SpawnSpec>(), 88);
    assert_eq!(core::mem::size_of::<DelegatedSpawnSpec>(), 112);
    assert_eq!(core::mem::size_of::<Credentials>(), 32);
    assert_eq!(core::mem::size_of::<SessionSpec>(), 72);
    assert_eq!(core::mem::size_of::<Stat>(), 24);
    assert_eq!(core::mem::size_of::<TtyInfo>(), 20);
    assert_eq!(core::mem::size_of::<DirEntry>(), 56);
    let _open_flags = [
        OPEN_READ,
        OPEN_WRITE,
        OPEN_CREATE,
        OPEN_TRUNCATE,
        OPEN_APPEND,
    ];
    let _wait_kinds = [WAIT_EXITED, WAIT_SIGNALED, WAIT_STOPPED, WAIT_CONTINUED];
    assert_eq!(PIPE_NONBLOCK, 1);
    assert_eq!(SPAWN_NEW_PROCESS_GROUP | SPAWN_FOREGROUND, 3);
    assert_eq!(SPAWN_INHERIT_CREDENTIALS, 4);
    assert_eq!(WAIT_NONBLOCK, 1);
    assert_eq!(F_GETFD, 1);
    assert_eq!(F_SETFD, 2);
    assert_eq!(FD_CLOEXEC, 1);
    let _pointer: UserPointer = 0;
    let _errno_values = [
        Errno::Eperm,
        Errno::Enoent,
        Errno::Enomem,
        Errno::Enotty,
        Errno::Epipe,
        Errno::Ebadf,
        Errno::Echild,
        Errno::Eintr,
        Errno::Eagain,
        Errno::Efault,
        Errno::Einval,
        Errno::Enosys,
        Errno::Eoverflow,
    ];
    let result = dispatch(u64::MAX, Args::empty());
    assert!(is_error(result));
    assert_eq!(result, Errno::Enosys.return_value());
}

pub fn runtime_contract_self_check() {
    assert_eq!(dispatch(Number::GetPid as UserWord, Args::empty()), 1);
    assert_eq!(dispatch(Number::GetTid as UserWord, Args::empty()), 1);
    assert_eq!(dispatch(Number::Yield as UserWord, Args::empty()), 0);
    assert_eq!(dispatch(Number::Sleep as UserWord, Args::empty()), 0);
    assert_eq!(
        dispatch(Number::Wait as UserWord, Args::empty()),
        Errno::Echild.return_value()
    );
    assert_eq!(
        dispatch(Number::GetProcessGroup as UserWord, Args::empty()),
        1
    );
    assert_eq!(
        dispatch(
            Number::SetProcessGroup as UserWord,
            Args {
                values: [0, 1, 0, 0, 0, 0],
            },
        ),
        0
    );
    assert_eq!(
        dispatch(
            Number::KillProcessGroup as UserWord,
            Args {
                values: [0, 2, 0, 0, 0, 0],
            },
        ),
        0
    );
    let source_fd = crate::process::with_process_table(|table| {
        table
            .open_fd(crate::process::ProcessId::INIT, 1, true, true)
            .unwrap()
            .get()
    });
    assert_eq!(
        dispatch(
            Number::Dup2 as UserWord,
            Args {
                values: [source_fd as UserWord, 4, 0, 0, 0, 0],
            },
        ),
        4
    );
    assert_eq!(
        dispatch(
            Number::Fcntl as UserWord,
            Args {
                values: [source_fd as UserWord, F_GETFD, 0, 0, 0, 0],
            },
        ),
        0
    );
    assert_eq!(
        dispatch(
            Number::Fcntl as UserWord,
            Args {
                values: [source_fd as UserWord, F_SETFD, FD_CLOEXEC, 0, 0, 0],
            },
        ),
        0
    );
    assert_eq!(
        dispatch(
            Number::Fcntl as UserWord,
            Args {
                values: [source_fd as UserWord, F_GETFD, 0, 0, 0, 0],
            },
        ),
        FD_CLOEXEC
    );
    assert_eq!(
        dispatch(
            Number::Fcntl as UserWord,
            Args {
                values: [source_fd as UserWord, F_SETFD, FD_CLOEXEC | 2, 0, 0, 0],
            },
        ),
        Errno::Einval.return_value()
    );
    assert_eq!(
        dispatch(
            Number::Fcntl as UserWord,
            Args {
                values: [4, F_GETFD, 0, 0, 0, 0],
            },
        ),
        0
    );
    assert_eq!(crate::process::close_current(source_fd), Ok(()));
    assert_eq!(crate::process::close_current(4), Ok(()));
    assert_eq!(
        dispatch(
            Number::Close as UserWord,
            Args {
                values: [u64::MAX, 0, 0, 0, 0, 0],
            },
        ),
        Errno::Ebadf.return_value()
    );
    assert_eq!(
        dispatch(
            Number::Write as UserWord,
            Args {
                values: [1, 0, 0, 0, 0, 0],
            },
        ),
        0
    );
    assert_eq!(
        dispatch(
            Number::Fsync as UserWord,
            Args {
                values: [1, 0, 0, 0, 0, 0],
            },
        ),
        Errno::Enotsup.return_value()
    );
    assert_eq!(
        dispatch(
            Number::Read as UserWord,
            Args {
                values: [1, 0, 0, 0, 0, 0],
            },
        ),
        Errno::Ebadf.return_value()
    );
    assert_eq!(
        dispatch(
            Number::Write as UserWord,
            Args {
                values: [1, u64::MAX, 1, 0, 0, 0],
            },
        ),
        Errno::Efault.return_value()
    );
    assert_eq!(
        dispatch(
            Number::Write as UserWord,
            Args {
                values: [1, 0, (MAX_IO + 1) as u64, 0, 0, 0],
            },
        ),
        Errno::Eoverflow.return_value()
    );
}
