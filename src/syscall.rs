pub const ABI_VERSION: u16 = 1;
pub const MAX_ARGS: usize = 6;

pub type UserWord = u64;
pub type UserPointer = u64;
pub const EXIT_TO_KERNEL: UserWord = UserWord::MAX - 1;

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

pub const TABLE: [Metadata; 9] = [
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
    Ebadf = 9,
    Echild = 10,
    Eintr = 4,
    Eagain = 11,
    Efault = 14,
    Einval = 22,
    Enosys = 38,
    Eoverflow = 75,
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

pub fn dispatch(number: UserWord, args: Args) -> UserWord {
    match number {
        value if value == Number::GetPid as UserWord => crate::process::current_ids()
            .map(|(process, _)| process as UserWord)
            .unwrap_or_else(|| Errno::Enosys.return_value()),
        value if value == Number::GetTid as UserWord => crate::process::current_ids()
            .map(|(_, thread)| thread as UserWord)
            .unwrap_or_else(|| Errno::Enosys.return_value()),
        value if value == Number::Exit as UserWord => {
            match crate::process::exit_current(args.values[0] as i32) {
                Ok(()) => {
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
        value if value == Number::Close as UserWord => {
            match crate::process::close_current(args.values[0] as u32) {
                Ok(()) => 0,
                Err(_) => Errno::Ebadf.return_value(),
            }
        }
        value if value == Number::Yield as UserWord => match crate::process::yield_current() {
            Ok(()) => 0,
            Err(_) => Errno::Einval.return_value(),
        },
        value if value == Number::Sleep as UserWord => match crate::process::sleep_current() {
            Ok(()) => 0,
            Err(_) => Errno::Einval.return_value(),
        },
        _ => Errno::Enosys.return_value(),
    }
}

pub const fn is_error(value: UserWord) -> bool {
    value >= UserWord::MAX - 4095
}

pub fn contract_self_check() {
    assert_eq!(ABI_VERSION, 1);
    assert_eq!(TABLE.len(), 9);
    assert!(TABLE.iter().all(|entry| entry.arguments <= MAX_ARGS as u8));
    assert_eq!(TABLE[0].number as UserWord, Number::Read as UserWord);
    assert!(TABLE.iter().all(|entry| !entry.name.is_empty()));
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
    let _pointer: UserPointer = 0;
    let _errno_values = [
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
        dispatch(Number::Close as UserWord, Args::empty()),
        Errno::Ebadf.return_value()
    );
}
