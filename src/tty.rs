use crate::process::ProcessId;

const CONTROLLING_FD: u32 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NotTty,
    PermissionDenied,
    InvalidGroup,
    InvalidWindow,
}

pub const FLAG_AVAILABLE: u32 = 1 << 0;
pub const FLAG_SERIAL: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub flags: u32,
    pub controlling_process: u32,
    pub foreground_group: u32,
    pub columns: u16,
    pub rows: u16,
    pub reserved: u32,
}

#[derive(Clone, Copy)]
struct Tty {
    controlling_process: ProcessId,
    foreground_group: ProcessId,
    available: bool,
    columns: u16,
    rows: u16,
}

impl Tty {
    const INITIAL: Self = Self {
        controlling_process: ProcessId::INIT,
        foreground_group: ProcessId::INIT,
        available: true,
        columns: 80,
        rows: 25,
    };
}

static mut TTY: Tty = Tty::INITIAL;

pub fn init() {
    crate::arch::without_interrupts(|| unsafe {
        TTY = Tty::INITIAL;
        TTY.available = crate::drivers::serial::available();
    });
}

pub fn get_info(fd: u32) -> Result<Info, Error> {
    with_tty(|tty| {
        if fd != CONTROLLING_FD {
            return Err(Error::NotTty);
        }
        let mut flags = FLAG_SERIAL;
        if tty.available {
            flags |= FLAG_AVAILABLE;
        }
        Ok(Info {
            flags,
            controlling_process: tty.controlling_process.get(),
            foreground_group: tty.foreground_group.get(),
            columns: tty.columns,
            rows: tty.rows,
            reserved: 0,
        })
    })
}

pub fn set_window(fd: u32, columns: u16, rows: u16) -> Result<(), Error> {
    let process = crate::process::current_process_id().ok_or(Error::PermissionDenied)?;
    if columns == 0 || rows == 0 || columns > 512 || rows > 256 {
        return Err(Error::InvalidWindow);
    }
    with_tty(|tty| {
        require_owner(tty, process, fd)?;
        tty.columns = columns;
        tty.rows = rows;
        Ok(())
    })
}

pub fn get_foreground(fd: u32) -> Result<u32, Error> {
    let process = crate::process::current_process_id().ok_or(Error::PermissionDenied)?;
    with_tty(|tty| {
        require_owner(tty, process, fd)?;
        Ok(tty.foreground_group.get())
    })
}

pub fn set_foreground(fd: u32, group: ProcessId) -> Result<(), Error> {
    let process = crate::process::current_process_id().ok_or(Error::PermissionDenied)?;
    let group = if group.get() == 0 {
        crate::process::current_process_group_id().ok_or(Error::InvalidGroup)?
    } else {
        group
    };
    if !crate::process::with_process_table(|table| table.process_group_has_live_member(group)) {
        return Err(Error::InvalidGroup);
    }
    with_tty(|tty| {
        require_owner(tty, process, fd)?;
        tty.foreground_group = group;
        Ok(())
    })
}

fn require_owner(tty: &Tty, process: ProcessId, fd: u32) -> Result<(), Error> {
    if fd != CONTROLLING_FD {
        return Err(Error::NotTty);
    }
    if process != tty.controlling_process {
        return Err(Error::PermissionDenied);
    }
    Ok(())
}

fn with_tty<R>(f: impl FnOnce(&mut Tty) -> Result<R, Error>) -> Result<R, Error> {
    crate::arch::without_interrupts(|| unsafe {
        let tty = &mut *core::ptr::addr_of_mut!(TTY);
        f(tty)
    })
}

pub fn contract_self_check() {
    let mut tty = Tty::INITIAL;
    assert_eq!(tty.controlling_process, ProcessId::INIT);
    assert_eq!(tty.foreground_group, ProcessId::INIT);
    assert!(tty.available);
    assert_eq!((tty.columns, tty.rows), (80, 25));
    tty.foreground_group = ProcessId::from_raw(2);
    assert_eq!(tty.foreground_group.get(), 2);
}

pub fn runtime_contract_self_check() {
    let info = get_info(CONTROLLING_FD).unwrap();
    assert_eq!(info.controlling_process, ProcessId::INIT.get());
    assert_eq!(info.foreground_group, ProcessId::INIT.get());
    assert_eq!((info.columns, info.rows), (80, 25));
    assert_eq!(get_foreground(CONTROLLING_FD), Ok(ProcessId::INIT.get()));
    assert_eq!(get_foreground(1), Err(Error::NotTty));
    assert_eq!(set_foreground(CONTROLLING_FD, ProcessId::INIT), Ok(()));
    assert_eq!(
        set_foreground(CONTROLLING_FD, ProcessId::from_raw(99)),
        Err(Error::InvalidGroup)
    );
    assert_eq!(set_window(CONTROLLING_FD, 100, 40), Ok(()));
    let info = get_info(CONTROLLING_FD).unwrap();
    assert_eq!((info.columns, info.rows), (100, 40));
    assert_eq!(set_window(CONTROLLING_FD, 0, 40), Err(Error::InvalidWindow));
}
