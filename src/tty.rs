#![allow(
    dead_code,
    reason = "Staged ABI and hardware hooks are not all enabled in the bootstrap profile"
)]

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::process::ProcessId;

const CONTROLLING_FD: u32 = 0;
const MAX_PTYS: usize = 8;
const BUFFER_CAPACITY: usize = 1024;
const READY_LINE_CAPACITY: usize = 32;
const MAX_SIGNAL_EVENTS: usize = 16;
const LOCK_SPIN_LIMIT: usize = 1_000_000;
const TTY_TAG: u32 = 1 << 28;
const TTY_MASTER_TAG: u32 = 1 << 27;
const TTY_RAW_ALLOWED: u32 = TTY_TAG | TTY_MASTER_TAG | (0xffff << 8) | 0xff;

pub const FLAG_AVAILABLE: u32 = 1 << 0;
pub const FLAG_SERIAL: u32 = 1 << 1;
pub const FLAG_PTY: u32 = 1 << 2;
pub const FLAG_MASTER: u32 = 1 << 3;

pub const TTY_CONTROL_RELEASE: u32 = 1 << 0;
pub const TTY_CONTROL_FORCE: u32 = 1 << 1;

pub const TCSANOW: u32 = 0;
pub const TCSADRAIN: u32 = 1;
pub const TCSAFLUSH: u32 = 2;

pub const VINTR: usize = 0;
pub const VQUIT: usize = 1;
pub const VERASE: usize = 2;
pub const VKILL: usize = 3;
pub const VEOF: usize = 4;
pub const VTIME: usize = 5;
pub const VMIN: usize = 6;
pub const VSTART: usize = 8;
pub const VSTOP: usize = 9;
pub const VSUSP: usize = 10;
pub const VEOL: usize = 11;
pub const NCCS: usize = 16;

pub const IFLAG_IGNBRK: u32 = 0x0001;
pub const IFLAG_BRKINT: u32 = 0x0002;
pub const IFLAG_IGNPAR: u32 = 0x0004;
pub const IFLAG_PARMRK: u32 = 0x0008;
pub const IFLAG_INPCK: u32 = 0x0010;
pub const IFLAG_ISTRIP: u32 = 0x0020;
pub const IFLAG_INLCR: u32 = 0x0040;
pub const IFLAG_IGNCR: u32 = 0x0080;
pub const IFLAG_ICRNL: u32 = 0x0100;

pub const OFLAG_OPOST: u32 = 0x0001;
pub const OFLAG_ONLCR: u32 = 0x0004;

pub const CFLAG_CSIZE: u32 = 0x0030;
pub const CFLAG_CS8: u32 = 0x0030;
pub const CFLAG_CREAD: u32 = 0x0080;
pub const CFLAG_CLOCAL: u32 = 0x0800;

pub const LFLAG_ISIG: u32 = 0x0001;
pub const LFLAG_ICANON: u32 = 0x0002;
pub const LFLAG_ECHO: u32 = 0x0008;
pub const LFLAG_ECHOE: u32 = 0x0010;
pub const LFLAG_ECHOK: u32 = 0x0020;
pub const LFLAG_ECHONL: u32 = 0x0040;
pub const LFLAG_ECHOCTL: u32 = 0x0200;
pub const LFLAG_IEXTEN: u32 = 0x8000;

const SUPPORTED_INPUT_FLAGS: u32 = IFLAG_INLCR | IFLAG_IGNCR | IFLAG_ICRNL | IFLAG_ISTRIP;
const SUPPORTED_OUTPUT_FLAGS: u32 = OFLAG_OPOST | OFLAG_ONLCR;
const SUPPORTED_CONTROL_FLAGS: u32 = CFLAG_CSIZE | CFLAG_CREAD | CFLAG_CLOCAL;
const SUPPORTED_LOCAL_FLAGS: u32 = LFLAG_ISIG
    | LFLAG_ICANON
    | LFLAG_ECHO
    | LFLAG_ECHOE
    | LFLAG_ECHOK
    | LFLAG_ECHONL
    | LFLAG_ECHOCTL
    | LFLAG_IEXTEN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NotTty,
    PermissionDenied,
    InvalidGroup,
    InvalidWindow,
    InvalidTermios,
    Unsupported,
    Busy,
    NotSessionLeader,
    WouldBlock,
    Broken,
    Unavailable,
    NoSpace,
    InvalidHandle,
    InvalidEndpoint,
    Exhausted,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Termios {
    pub input_flags: u32,
    pub output_flags: u32,
    pub control_flags: u32,
    pub local_flags: u32,
    pub control_chars: [u8; NCCS],
    pub input_speed: u32,
    pub output_speed: u32,
    pub reserved: u32,
}

impl Termios {
    pub const DEFAULT: Self = {
        let mut control_chars = [0; NCCS];
        control_chars[VINTR] = 3;
        control_chars[VQUIT] = 28;
        control_chars[VERASE] = 127;
        control_chars[VKILL] = 21;
        control_chars[VEOF] = 4;
        control_chars[VMIN] = 1;
        control_chars[VSTART] = 17;
        control_chars[VSTOP] = 19;
        control_chars[VSUSP] = 26;
        Self {
            input_flags: IFLAG_ICRNL,
            output_flags: OFLAG_OPOST | OFLAG_ONLCR,
            control_flags: CFLAG_CS8 | CFLAG_CREAD | CFLAG_CLOCAL,
            local_flags: LFLAG_ISIG
                | LFLAG_ICANON
                | LFLAG_ECHO
                | LFLAG_ECHOE
                | LFLAG_ECHOK
                | LFLAG_IEXTEN
                | LFLAG_ECHOCTL,
            control_chars,
            input_speed: 38_400,
            output_speed: 38_400,
            reserved: 0,
        }
    };

    pub fn validate(self) -> Result<(), Error> {
        if self.input_flags & !SUPPORTED_INPUT_FLAGS != 0
            || self.output_flags & !SUPPORTED_OUTPUT_FLAGS != 0
            || self.control_flags & !SUPPORTED_CONTROL_FLAGS != 0
            || self.control_flags & CFLAG_CSIZE != CFLAG_CS8
            || self.local_flags & !SUPPORTED_LOCAL_FLAGS != 0
            || self.input_speed != 38_400
            || self.output_speed != 38_400
            || self.reserved != 0
            || self.control_chars[VTIME] != 0
        {
            return Err(Error::InvalidTermios);
        }
        Ok(())
    }

    fn canonical(self) -> bool {
        self.local_flags & LFLAG_ICANON != 0
    }

    fn signals(self) -> bool {
        self.local_flags & LFLAG_ISIG != 0
    }

    fn echo(self) -> bool {
        self.local_flags & LFLAG_ECHO != 0
    }
}

impl Default for Termios {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowSize {
    pub rows: u16,
    pub columns: u16,
    pub xpixel: u16,
    pub ypixel: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PtyFds {
    pub master: u64,
    pub slave: u64,
}

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
struct ByteRing {
    bytes: [u8; BUFFER_CAPACITY],
    head: u16,
    tail: u16,
    len: u16,
}

impl ByteRing {
    const EMPTY: Self = Self {
        bytes: [0; BUFFER_CAPACITY],
        head: 0,
        tail: 0,
        len: 0,
    };

    fn len(self) -> usize {
        self.len as usize
    }

    fn is_empty(self) -> bool {
        self.len == 0
    }

    fn push(&mut self, byte: u8) -> bool {
        if self.len as usize == BUFFER_CAPACITY {
            return false;
        }
        self.bytes[self.head as usize] = byte;
        self.head = (self.head as usize + 1) as u16;
        if self.head as usize == BUFFER_CAPACITY {
            self.head = 0;
        }
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let byte = self.bytes[self.tail as usize];
        self.tail = (self.tail as usize + 1) as u16;
        if self.tail as usize == BUFFER_CAPACITY {
            self.tail = 0;
        }
        self.len -= 1;
        Some(byte)
    }

    fn pop_into(&mut self, output: &mut [u8]) -> usize {
        let mut count = 0;
        while count < output.len() {
            let Some(byte) = self.pop() else {
                break;
            };
            output[count] = byte;
            count += 1;
        }
        count
    }

    fn remove_last(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        self.head = if self.head == 0 {
            (BUFFER_CAPACITY - 1) as u16
        } else {
            self.head - 1
        };
        self.len -= 1;
        Some(self.bytes[self.head as usize])
    }

    fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.len = 0;
    }
}

#[derive(Clone, Copy)]
struct ReadyLines {
    lengths: [u16; READY_LINE_CAPACITY],
    head: u8,
    tail: u8,
    len: u8,
}

impl ReadyLines {
    const EMPTY: Self = Self {
        lengths: [0; READY_LINE_CAPACITY],
        head: 0,
        tail: 0,
        len: 0,
    };

    fn push(&mut self, length: u16) -> bool {
        if self.len as usize == READY_LINE_CAPACITY {
            return false;
        }
        self.lengths[self.head as usize] = length;
        self.head = (self.head as usize + 1) as u8;
        if self.head as usize == READY_LINE_CAPACITY {
            self.head = 0;
        }
        self.len += 1;
        true
    }

    fn front(self) -> Option<u16> {
        (self.len != 0).then_some(self.lengths[self.tail as usize])
    }

    fn consume(&mut self, count: u16) {
        let Some(length) = self.front() else {
            return;
        };
        if count < length {
            self.lengths[self.tail as usize] = length - count;
            return;
        }
        self.tail = (self.tail as usize + 1) as u8;
        if self.tail as usize == READY_LINE_CAPACITY {
            self.tail = 0;
        }
        self.len -= 1;
    }

    fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.len = 0;
    }
}

#[derive(Clone, Copy)]
struct Terminal {
    available: bool,
    serial: bool,
    session: Option<ProcessId>,
    controlling_process: Option<ProcessId>,
    foreground_group: Option<ProcessId>,
    window: WindowSize,
    termios: Termios,
    input: ByteRing,
    output: ByteRing,
    ready_lines: ReadyLines,
    current_line_len: u16,
    master_refs: u16,
    slave_refs: u16,
}

impl Terminal {
    const fn serial() -> Self {
        Self {
            available: true,
            serial: true,
            session: Some(ProcessId::INIT),
            controlling_process: Some(ProcessId::INIT),
            foreground_group: Some(ProcessId::INIT),
            window: WindowSize {
                rows: 25,
                columns: 80,
                xpixel: 0,
                ypixel: 0,
            },
            termios: Termios::DEFAULT,
            input: ByteRing::EMPTY,
            output: ByteRing::EMPTY,
            ready_lines: ReadyLines::EMPTY,
            current_line_len: 0,
            master_refs: 0,
            slave_refs: 0,
        }
    }

    const fn pty() -> Self {
        Self {
            available: true,
            serial: false,
            session: None,
            controlling_process: None,
            foreground_group: None,
            window: WindowSize {
                rows: 25,
                columns: 80,
                xpixel: 0,
                ypixel: 0,
            },
            termios: Termios::DEFAULT,
            input: ByteRing::EMPTY,
            output: ByteRing::EMPTY,
            ready_lines: ReadyLines::EMPTY,
            current_line_len: 0,
            master_refs: 1,
            slave_refs: 1,
        }
    }

    const fn empty() -> Self {
        Self {
            available: false,
            serial: false,
            session: None,
            controlling_process: None,
            foreground_group: None,
            window: WindowSize {
                rows: 0,
                columns: 0,
                xpixel: 0,
                ypixel: 0,
            },
            termios: Termios::DEFAULT,
            input: ByteRing::EMPTY,
            output: ByteRing::EMPTY,
            ready_lines: ReadyLines::EMPTY,
            current_line_len: 0,
            master_refs: 0,
            slave_refs: 0,
        }
    }

    fn clear_buffers(&mut self) {
        self.input.clear();
        self.output.clear();
        self.ready_lines.clear();
        self.current_line_len = 0;
    }

    fn push_echo(&mut self, byte: u8) {
        let _ = self.output.push(byte);
    }

    fn push_echo_control(&mut self, byte: u8) {
        if self.termios.local_flags & LFLAG_ECHOCTL != 0 && byte < 0x20 {
            self.push_echo(b'^');
            self.push_echo(byte.wrapping_add(b'@'));
        } else {
            self.push_echo(byte);
        }
    }

    fn erase(&mut self) -> bool {
        if self.current_line_len == 0 {
            return true;
        }
        if self.input.remove_last().is_none() {
            return false;
        }
        self.current_line_len -= 1;
        if self.termios.local_flags & LFLAG_ECHOE != 0 {
            self.push_echo(8);
            self.push_echo(b' ');
            self.push_echo(8);
        }
        true
    }

    fn kill_line(&mut self) -> bool {
        while self.current_line_len != 0 {
            if self.input.remove_last().is_none() {
                return false;
            }
            self.current_line_len -= 1;
        }
        if self.termios.local_flags & LFLAG_ECHOK != 0 {
            self.push_echo(b'\n');
        }
        true
    }

    fn feed_input(&mut self, mut byte: u8) -> FeedResult {
        if self.termios.input_flags & IFLAG_IGNCR != 0 && byte == b'\r' {
            return FeedResult::Accepted(None);
        }
        if self.termios.input_flags & IFLAG_ICRNL != 0 && byte == b'\r' {
            byte = b'\n';
        } else if self.termios.input_flags & IFLAG_INLCR != 0 && byte == b'\n' {
            byte = b'\r';
        }
        if self.termios.input_flags & IFLAG_ISTRIP != 0 {
            byte &= 0x7f;
        }
        if self.termios.signals() {
            let signal = if byte == self.termios.control_chars[VINTR] {
                Some(2)
            } else if byte == self.termios.control_chars[VQUIT] {
                Some(3)
            } else if byte == self.termios.control_chars[VSUSP] {
                Some(20)
            } else {
                None
            };
            if signal.is_some() {
                self.clear_buffers();
                if self.termios.echo() {
                    self.push_echo_control(byte);
                    self.push_echo(b'\n');
                }
                return FeedResult::Accepted(signal);
            }
        }
        if self.termios.canonical() {
            if byte == self.termios.control_chars[VERASE] {
                let _ = self.erase();
                return FeedResult::Accepted(None);
            }
            if byte == self.termios.control_chars[VKILL] {
                let _ = self.kill_line();
                return FeedResult::Accepted(None);
            }
            if byte == self.termios.control_chars[VEOF] {
                if !self.ready_lines.push(self.current_line_len) {
                    return FeedResult::Full;
                }
                self.current_line_len = 0;
                return FeedResult::Accepted(None);
            }
            if !self.input.push(byte) {
                return FeedResult::Full;
            }
            self.current_line_len = self.current_line_len.saturating_add(1);
            if self.termios.echo()
                && (byte != b'\n' || self.termios.local_flags & LFLAG_ECHONL != 0)
            {
                self.push_echo_control(byte);
            }
            if byte == b'\n' {
                if !self.ready_lines.push(self.current_line_len) {
                    let _ = self.input.remove_last();
                    self.current_line_len -= 1;
                    return FeedResult::Full;
                }
                self.current_line_len = 0;
            }
            FeedResult::Accepted(None)
        } else {
            if !self.input.push(byte) {
                return FeedResult::Full;
            }
            if self.termios.echo() {
                self.push_echo_control(byte);
            }
            FeedResult::Accepted(None)
        }
    }

    fn read_input(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        if self.termios.canonical() {
            let Some(line_length) = self.ready_lines.front() else {
                return Err(Error::WouldBlock);
            };
            if line_length == 0 {
                self.ready_lines.consume(0);
                return Ok(0);
            }
            let limit = output.len().min(line_length as usize);
            let count = self.input.pop_into(&mut output[..limit]);
            self.ready_lines.consume(count as u16);
            return Ok(count);
        }
        let minimum = self.termios.control_chars[VMIN] as usize;
        if self.input.is_empty() || (minimum != 0 && self.input.len() < minimum) {
            return Err(Error::WouldBlock);
        }
        Ok(self.input.pop_into(output))
    }

    fn write_output(&mut self, input: &[u8]) -> usize {
        let mut count = 0;
        while count < input.len() {
            let byte = input[count];
            if self.termios.output_flags & (OFLAG_OPOST | OFLAG_ONLCR)
                == (OFLAG_OPOST | OFLAG_ONLCR)
                && byte == b'\n'
            {
                if self.output.len() + 2 > BUFFER_CAPACITY {
                    break;
                }
                let _ = self.output.push(b'\r');
                let _ = self.output.push(b'\n');
            } else if !self.output.push(byte) {
                break;
            }
            count += 1;
        }
        count
    }
}

enum FeedResult {
    Accepted(Option<u8>),
    Full,
}

#[derive(Clone, Copy)]
struct PtySlot {
    used: bool,
    generation: u16,
    terminal: Terminal,
}

impl PtySlot {
    const EMPTY: Self = Self {
        used: false,
        generation: 0,
        terminal: Terminal::empty(),
    };
}

struct TtyTable {
    serial: Terminal,
    ptys: [PtySlot; MAX_PTYS],
}

impl TtyTable {
    const fn new() -> Self {
        Self {
            serial: Terminal::serial(),
            ptys: [PtySlot::EMPTY; MAX_PTYS],
        }
    }
}

struct TtyLock {
    owner: AtomicUsize,
    depth: AtomicUsize,
}

impl TtyLock {
    const fn new() -> Self {
        Self {
            owner: AtomicUsize::new(0),
            depth: AtomicUsize::new(0),
        }
    }

    fn lock(&self) -> TtyLockGuard<'_> {
        let token = crate::arch::cpu_id().saturating_add(1);
        if self.owner.load(Ordering::Acquire) == token {
            self.depth.fetch_add(1, Ordering::Relaxed);
            return TtyLockGuard { lock: self, token };
        }
        let mut spins = 0;
        loop {
            if self
                .owner
                .compare_exchange(0, token, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                self.depth.store(1, Ordering::Relaxed);
                return TtyLockGuard { lock: self, token };
            }
            if spins == LOCK_SPIN_LIMIT {
                panic!("tty lock contention");
            }
            spins += 1;
            core::hint::spin_loop();
        }
    }
}

struct TtyLockGuard<'a> {
    lock: &'a TtyLock,
    token: usize,
}

impl Drop for TtyLockGuard<'_> {
    fn drop(&mut self) {
        debug_assert_eq!(self.lock.owner.load(Ordering::Relaxed), self.token);
        if self.lock.depth.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.lock.owner.store(0, Ordering::Release);
        }
    }
}

struct TtyCell {
    lock: TtyLock,
    table: UnsafeCell<TtyTable>,
}

impl TtyCell {
    const fn new() -> Self {
        Self {
            lock: TtyLock::new(),
            table: UnsafeCell::new(TtyTable::new()),
        }
    }
}

unsafe impl Sync for TtyCell {}

static TTY: TtyCell = TtyCell::new();

#[derive(Clone, Copy)]
enum Endpoint {
    Serial,
    Pty {
        index: usize,
        generation: u16,
        master: bool,
    },
}

#[derive(Clone, Copy)]
struct SignalEvent {
    signal: u8,
    group: Option<ProcessId>,
}

fn with_table<R>(f: impl FnOnce(&mut TtyTable) -> R) -> R {
    crate::arch::without_interrupts(|| {
        let _guard = TTY.lock.lock();
        unsafe { f(&mut *TTY.table.get()) }
    })
}

pub fn init() {
    with_table(|table| {
        table.serial = Terminal::serial();
        table.serial.available = crate::drivers::serial::available();
        table.ptys = [PtySlot::EMPTY; MAX_PTYS];
    });
}

pub fn is_tty_raw(raw: u32) -> bool {
    raw <= 2 || raw & TTY_TAG != 0
}

pub fn duplicate_raw(raw: u32) -> Result<(), Error> {
    let endpoint = decode_raw(raw)?;
    with_table(|table| match endpoint {
        Endpoint::Serial => Ok(()),
        Endpoint::Pty {
            index,
            generation,
            master,
        } => {
            let terminal = valid_pty(table, index, generation)?;
            let refs = if master {
                &mut terminal.master_refs
            } else {
                &mut terminal.slave_refs
            };
            *refs = refs.checked_add(1).ok_or(Error::Exhausted)?;
            Ok(())
        }
    })
}

pub fn close_raw(raw: u32) -> Result<(), Error> {
    let endpoint = decode_raw(raw)?;
    with_table(|table| match endpoint {
        Endpoint::Serial => Ok(()),
        Endpoint::Pty {
            index,
            generation,
            master,
        } => {
            let terminal = valid_pty(table, index, generation)?;
            let refs = if master {
                &mut terminal.master_refs
            } else {
                &mut terminal.slave_refs
            };
            if *refs == 0 {
                return Err(Error::InvalidHandle);
            }
            *refs -= 1;
            if terminal.master_refs == 0 && terminal.slave_refs == 0 {
                table.ptys[index].used = false;
                table.ptys[index].terminal = Terminal::empty();
            }
            Ok(())
        }
    })
}

pub fn create_pty() -> Result<(u32, u32), Error> {
    with_table(|table| {
        let index = table
            .ptys
            .iter()
            .position(|slot| !slot.used)
            .ok_or(Error::Exhausted)?;
        let generation = table.ptys[index]
            .generation
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        table.ptys[index] = PtySlot {
            used: true,
            generation,
            terminal: Terminal::pty(),
        };
        Ok((
            encode(index, generation, true),
            encode(index, generation, false),
        ))
    })
}

pub fn get_info(fd: u32) -> Result<Info, Error> {
    let endpoint = endpoint_for_fd(fd)?;
    with_table(|table| {
        let terminal = terminal(table, endpoint)?;
        let mut flags = if terminal.available {
            FLAG_AVAILABLE
        } else {
            0
        };
        match endpoint {
            Endpoint::Serial => flags |= FLAG_SERIAL,
            Endpoint::Pty { master, .. } => {
                flags |= FLAG_PTY;
                if master {
                    flags |= FLAG_MASTER;
                }
            }
        }
        Ok(Info {
            flags,
            controlling_process: terminal.controlling_process.map_or(0, ProcessId::get),
            foreground_group: terminal.foreground_group.map_or(0, ProcessId::get),
            columns: terminal.window.columns,
            rows: terminal.window.rows,
            reserved: 0,
        })
    })
}

pub fn get_window(fd: u32) -> Result<WindowSize, Error> {
    let endpoint = endpoint_for_fd(fd)?;
    with_table(|table| Ok(terminal(table, endpoint)?.window))
}

pub fn set_window(fd: u32, columns: u16, rows: u16) -> Result<(), Error> {
    if columns == 0 || rows == 0 || columns > 512 || rows > 256 {
        return Err(Error::InvalidWindow);
    }
    let endpoint = endpoint_for_fd(fd)?;
    let caller = crate::process::current_process_id().ok_or(Error::PermissionDenied)?;
    let session = crate::process::current_session_id().ok_or(Error::PermissionDenied)?;
    with_table(|table| {
        let terminal = terminal(table, endpoint)?;
        if !window_change_allowed(terminal, endpoint, caller, session) {
            return Err(Error::PermissionDenied);
        }
        terminal.window.columns = columns;
        terminal.window.rows = rows;
        Ok(())
    })
}

pub fn get_foreground(fd: u32) -> Result<u32, Error> {
    let endpoint = endpoint_for_fd(fd)?;
    let session = crate::process::current_session_id().ok_or(Error::PermissionDenied)?;
    with_table(|table| {
        let terminal = terminal(table, endpoint)?;
        if !session_member(terminal, session)
            || matches!(endpoint, Endpoint::Pty { master: true, .. })
        {
            return Err(Error::PermissionDenied);
        }
        terminal
            .foreground_group
            .ok_or(Error::InvalidGroup)
            .map(ProcessId::get)
    })
}

pub fn set_foreground(fd: u32, group: ProcessId) -> Result<(), Error> {
    let endpoint = endpoint_for_fd(fd)?;
    if matches!(endpoint, Endpoint::Pty { master: true, .. }) {
        return Err(Error::PermissionDenied);
    }
    let session = crate::process::current_session_id().ok_or(Error::PermissionDenied)?;
    let group = if group.get() == 0 {
        crate::process::current_process_group_id().ok_or(Error::InvalidGroup)?
    } else {
        group
    };
    if !crate::process::process_group_in_session(group, session) {
        return Err(Error::InvalidGroup);
    }
    with_table(|table| {
        let terminal = terminal(table, endpoint)?;
        if !session_member(terminal, session) || terminal.foreground_group.is_none() {
            return Err(Error::PermissionDenied);
        }
        terminal.foreground_group = Some(group);
        Ok(())
    })
}

pub fn get_termios(fd: u32) -> Result<Termios, Error> {
    let endpoint = endpoint_for_fd(fd)?;
    with_table(|table| Ok(terminal(table, endpoint)?.termios))
}

pub fn set_termios(fd: u32, action: u32, termios: Termios) -> Result<(), Error> {
    termios.validate()?;
    if action != TCSANOW && action != TCSADRAIN && action != TCSAFLUSH {
        return Err(Error::Unsupported);
    }
    let endpoint = endpoint_for_fd(fd)?;
    let caller = crate::process::current_process_id().ok_or(Error::PermissionDenied)?;
    let session = crate::process::current_session_id().ok_or(Error::PermissionDenied)?;
    with_table(|table| {
        let terminal = terminal(table, endpoint)?;
        if matches!(endpoint, Endpoint::Pty { master: false, .. })
            && !session_member(terminal, session)
        {
            return Err(Error::PermissionDenied);
        }
        if matches!(endpoint, Endpoint::Serial)
            && terminal.controlling_process != Some(caller)
            && !session_member(terminal, session)
        {
            return Err(Error::PermissionDenied);
        }
        if action == TCSADRAIN && !terminal.output.is_empty() {
            return Err(Error::WouldBlock);
        }
        if action == TCSAFLUSH {
            terminal.clear_buffers();
        }
        terminal.termios = termios;
        Ok(())
    })
}

pub fn control(fd: u32, flags: u32) -> Result<(), Error> {
    if flags & !(TTY_CONTROL_RELEASE | TTY_CONTROL_FORCE) != 0 {
        return Err(Error::Unsupported);
    }
    let endpoint = endpoint_for_fd(fd)?;
    if matches!(endpoint, Endpoint::Pty { master: true, .. }) {
        return Err(Error::NotTty);
    }
    let caller = crate::process::current_process_id().ok_or(Error::PermissionDenied)?;
    let session = crate::process::current_session_id().ok_or(Error::PermissionDenied)?;
    let group = crate::process::current_process_group_id().ok_or(Error::InvalidGroup)?;
    if flags & TTY_CONTROL_RELEASE != 0 {
        return with_table(|table| {
            let terminal = terminal(table, endpoint)?;
            if terminal.session != Some(session) || terminal.controlling_process != Some(caller) {
                return Err(Error::PermissionDenied);
            }
            detach(terminal);
            Ok(())
        });
    }
    if caller != session {
        return Err(Error::NotSessionLeader);
    }
    if !crate::process::process_group_in_session(group, session) {
        return Err(Error::InvalidGroup);
    }
    let credentials = crate::process::current_credentials().map_err(|_| Error::PermissionDenied)?;
    with_table(|table| {
        let terminal = terminal(table, endpoint)?;
        if let Some(existing) = terminal.session {
            if existing == session {
                return Ok(());
            }
            if flags & TTY_CONTROL_FORCE == 0
                || !credentials.has_capability(crate::process::Capability::SessionAdmin as u8)
            {
                return Err(Error::Busy);
            }
            detach(terminal);
        }
        terminal.session = Some(session);
        terminal.controlling_process = Some(caller);
        terminal.foreground_group = Some(group);
        Ok(())
    })
}

pub fn release_for_process(process: ProcessId) {
    with_table(|table| {
        if table.serial.controlling_process == Some(process) {
            detach(&mut table.serial);
        }
        for slot in &mut table.ptys {
            if slot.used && slot.terminal.controlling_process == Some(process) {
                detach(&mut slot.terminal);
            }
        }
    });
}

pub fn read_raw(raw: u32, output: &mut [u8]) -> Result<usize, Error> {
    if output.is_empty() {
        return Ok(0);
    }
    let endpoint = decode_raw(raw)?;
    if matches!(endpoint, Endpoint::Serial) {
        return read_serial(output);
    }
    with_table(|table| {
        let Endpoint::Pty {
            index,
            generation,
            master,
        } = endpoint
        else {
            return Err(Error::InvalidEndpoint);
        };
        let terminal = valid_pty(table, index, generation)?;
        let peer_closed = if master {
            terminal.slave_refs == 0
        } else {
            terminal.master_refs == 0
        };
        if master {
            if terminal.output.is_empty() && peer_closed {
                Ok(0)
            } else {
                read_output(&mut terminal.output, output)
            }
        } else {
            match terminal.read_input(output) {
                Err(Error::WouldBlock) if peer_closed => Err(Error::Broken),
                result => result,
            }
        }
    })
}

pub fn write_raw(raw: u32, input: &[u8]) -> Result<usize, Error> {
    if input.is_empty() {
        return Ok(0);
    }
    let endpoint = decode_raw(raw)?;
    if matches!(endpoint, Endpoint::Serial) {
        return write_serial(input);
    }
    let (count, events) = with_table(|table| {
        let Endpoint::Pty {
            index,
            generation,
            master,
        } = endpoint
        else {
            return (0, [None; MAX_SIGNAL_EVENTS]);
        };
        let terminal = match valid_pty(table, index, generation) {
            Ok(terminal) => terminal,
            Err(_) => return (0, [None; MAX_SIGNAL_EVENTS]),
        };
        if if master {
            terminal.slave_refs == 0
        } else {
            terminal.master_refs == 0
        } {
            return (0, [None; MAX_SIGNAL_EVENTS]);
        }
        let mut count = 0;
        let mut events = [None; MAX_SIGNAL_EVENTS];
        let mut event_count = 0;
        while count < input.len() {
            let result = if master {
                terminal.feed_input(input[count])
            } else if terminal.write_output(&input[count..count + 1]) == 0 {
                FeedResult::Full
            } else {
                FeedResult::Accepted(None)
            };
            match result {
                FeedResult::Full => break,
                FeedResult::Accepted(signal) => {
                    if let Some(signal) = signal {
                        if event_count < MAX_SIGNAL_EVENTS {
                            events[event_count] = Some(SignalEvent {
                                signal,
                                group: terminal.foreground_group,
                            });
                            event_count += 1;
                        }
                    }
                    count += 1;
                }
            }
        }
        (count, events)
    });
    deliver_signals(&events);
    if count == 0 {
        Err(Error::NoSpace)
    } else {
        Ok(count)
    }
}

fn read_output(output: &mut ByteRing, buffer: &mut [u8]) -> Result<usize, Error> {
    if output.is_empty() {
        return Err(Error::WouldBlock);
    }
    Ok(output.pop_into(buffer))
}

fn read_serial(output: &mut [u8]) -> Result<usize, Error> {
    if !crate::drivers::serial::available() {
        return Err(Error::Unavailable);
    }
    let mut events = [None; MAX_SIGNAL_EVENTS];
    let mut event_count = 0;
    loop {
        let Some(byte) = crate::drivers::serial::read() else {
            break;
        };
        let accepted = with_table(|table| match table.serial.feed_input(byte) {
            FeedResult::Full => false,
            FeedResult::Accepted(signal) => {
                if let Some(signal) = signal {
                    if event_count < MAX_SIGNAL_EVENTS {
                        events[event_count] = Some(SignalEvent {
                            signal,
                            group: table.serial.foreground_group,
                        });
                        event_count += 1;
                    }
                }
                true
            }
        });
        if !accepted || event_count == MAX_SIGNAL_EVENTS {
            break;
        }
    }
    let mut echo = [0; BUFFER_CAPACITY];
    let echo_count = with_table(|table| table.serial.output.pop_into(&mut echo));
    if echo_count != 0 {
        crate::log::write_bytes(&echo[..echo_count]);
    }
    deliver_signals(&events);
    with_table(|table| table.serial.read_input(output))
}

fn write_serial(input: &[u8]) -> Result<usize, Error> {
    if !crate::drivers::serial::available() {
        return Err(Error::Unavailable);
    }
    let count = with_table(|table| table.serial.write_output(input));
    let mut output = [0; BUFFER_CAPACITY];
    let output_count = with_table(|table| table.serial.output.pop_into(&mut output));
    if output_count != 0 {
        crate::log::write_bytes(&output[..output_count]);
    }
    if count == 0 {
        Err(Error::NoSpace)
    } else {
        Ok(count)
    }
}

fn deliver_signals(events: &[Option<SignalEvent>; MAX_SIGNAL_EVENTS]) {
    for event in events.iter().flatten() {
        if let Some(group) = event.group {
            let _ = crate::process::raise_signal_to_group(group, event.signal);
        }
    }
}

fn endpoint_for_fd(fd: u32) -> Result<Endpoint, Error> {
    let (raw, _, _, _, _) = crate::process::current_fd_info(fd).map_err(|_| Error::NotTty)?;
    decode_raw(raw)
}

fn decode_raw(raw: u32) -> Result<Endpoint, Error> {
    match raw {
        0..=2 => Ok(Endpoint::Serial),
        value if value & TTY_TAG != 0 => {
            if value & !TTY_RAW_ALLOWED != 0 {
                return Err(Error::InvalidHandle);
            }
            let index = (value & 0xff) as usize;
            let generation = (value >> 8) as u16;
            if generation == 0 || index >= MAX_PTYS {
                return Err(Error::InvalidHandle);
            }
            Ok(Endpoint::Pty {
                index,
                generation,
                master: value & TTY_MASTER_TAG != 0,
            })
        }
        _ => Err(Error::NotTty),
    }
}

fn encode(index: usize, generation: u16, master: bool) -> u32 {
    TTY_TAG | if master { TTY_MASTER_TAG } else { 0 } | ((generation as u32) << 8) | index as u32
}

fn valid_pty(table: &mut TtyTable, index: usize, generation: u16) -> Result<&mut Terminal, Error> {
    let slot = table.ptys.get_mut(index).ok_or(Error::InvalidHandle)?;
    if !slot.used || slot.generation != generation {
        return Err(Error::InvalidHandle);
    }
    Ok(&mut slot.terminal)
}

fn terminal(table: &mut TtyTable, endpoint: Endpoint) -> Result<&mut Terminal, Error> {
    match endpoint {
        Endpoint::Serial => Ok(&mut table.serial),
        Endpoint::Pty {
            index, generation, ..
        } => valid_pty(table, index, generation),
    }
}

fn session_member(terminal: &Terminal, session: ProcessId) -> bool {
    terminal.session == Some(session)
}

fn window_change_allowed(
    terminal: &Terminal,
    endpoint: Endpoint,
    caller: ProcessId,
    session: ProcessId,
) -> bool {
    if matches!(endpoint, Endpoint::Pty { master: true, .. }) {
        return true;
    }
    terminal.controlling_process == Some(caller) || session_member(terminal, session)
}

fn detach(terminal: &mut Terminal) {
    terminal.session = None;
    terminal.controlling_process = None;
    terminal.foreground_group = None;
    terminal.clear_buffers();
}

pub fn contract_self_check() {
    assert_eq!(core::mem::size_of::<Termios>(), 44);
    assert_eq!(core::mem::offset_of!(Termios, control_chars), 16);
    assert_eq!(core::mem::offset_of!(Termios, input_speed), 32);
    assert_eq!(core::mem::offset_of!(Termios, reserved), 40);
    assert_eq!(core::mem::size_of::<WindowSize>(), 8);
    assert_eq!(core::mem::size_of::<PtyFds>(), 16);
    assert_eq!(core::mem::size_of::<Info>(), 20);
    assert!(Termios::DEFAULT.validate().is_ok());
    let mut terminal = Terminal::pty();
    assert_eq!(terminal.window.columns, 80);
    assert_eq!(terminal.window.rows, 25);
    assert!(matches!(
        terminal.feed_input(b'a'),
        FeedResult::Accepted(None)
    ));
    assert!(matches!(
        terminal.feed_input(b'\n'),
        FeedResult::Accepted(None)
    ));
    let mut output = [0; 8];
    assert_eq!(terminal.read_input(&mut output), Ok(2));
    assert_eq!(&output[..2], b"a\n");
    let invalid = Termios {
        input_flags: u32::MAX,
        ..Termios::DEFAULT
    };
    assert_eq!(invalid.validate(), Err(Error::InvalidTermios));
    assert!(decode_raw(encode(0, 1, true)).is_ok());
    assert!(matches!(
        decode_raw(encode(0, 1, true) | (1 << 24)),
        Err(Error::InvalidHandle)
    ));
}

pub fn runtime_contract_self_check() {
    let info = get_info(CONTROLLING_FD).unwrap();
    assert_eq!(info.controlling_process, ProcessId::INIT.get());
    assert_eq!(info.foreground_group, ProcessId::INIT.get());
    assert_eq!((info.columns, info.rows), (80, 25));
    assert_eq!(get_foreground(CONTROLLING_FD), Ok(ProcessId::INIT.get()));
    assert_eq!(get_foreground(31), Err(Error::NotTty));
    assert_eq!(set_foreground(CONTROLLING_FD, ProcessId::INIT), Ok(()));
    assert_eq!(
        set_foreground(CONTROLLING_FD, ProcessId::from_raw(99)),
        Err(Error::InvalidGroup)
    );
    assert_eq!(set_window(CONTROLLING_FD, 100, 40), Ok(()));
    assert_eq!(
        get_window(CONTROLLING_FD).unwrap(),
        WindowSize {
            columns: 100,
            rows: 40,
            xpixel: 0,
            ypixel: 0,
        }
    );
    assert_eq!(set_window(CONTROLLING_FD, 0, 40), Err(Error::InvalidWindow));
    let termios = get_termios(CONTROLLING_FD).unwrap();
    assert_eq!(set_termios(CONTROLLING_FD, TCSANOW, termios), Ok(()));

    let (master_raw, slave_raw) = create_pty().unwrap();
    assert_eq!(read_raw(master_raw, &mut [0; 1]), Err(Error::WouldBlock));
    let master_fd = crate::process::open_current_fd(master_raw, true, true, true).unwrap();
    let slave_fd = crate::process::open_current_fd(slave_raw, true, true, true).unwrap();
    assert_eq!(control(slave_fd, TTY_CONTROL_FORCE), Ok(()));
    assert_eq!(get_foreground(slave_fd), Ok(ProcessId::INIT.get()));
    assert_eq!(set_window(master_fd, 120, 50), Ok(()));
    let mut raw_termios = get_termios(slave_fd).unwrap();
    raw_termios.local_flags &= !LFLAG_ICANON;
    raw_termios.local_flags &= !LFLAG_ECHO;
    assert_eq!(set_termios(slave_fd, TCSANOW, raw_termios), Ok(()));
    assert_eq!(write_raw(master_raw, b"pty"), Ok(3));
    let mut input = [0; 8];
    assert_eq!(read_raw(slave_raw, &mut input), Ok(3));
    assert_eq!(&input[..3], b"pty");
    assert_eq!(control(slave_fd, TTY_CONTROL_RELEASE), Ok(()));
    assert_eq!(crate::process::close_current(master_fd), Ok(()));
    assert_eq!(crate::process::close_current(slave_fd), Ok(()));
    assert_eq!(read_raw(master_raw, &mut [0; 1]), Err(Error::InvalidHandle));
    assert_eq!(close_raw(slave_raw), Ok(()));
    assert_eq!(close_raw(master_raw), Ok(()));
    assert_eq!(control(CONTROLLING_FD, TTY_CONTROL_FORCE), Ok(()));
}

#[cfg(test)]
mod tests {
    use super::{Error, Terminal, Termios, TCSADRAIN, TCSAFLUSH, TCSANOW};

    #[test]
    fn canonical_line_discipline_handles_eof_and_editing() {
        let mut terminal = Terminal::pty();
        assert!(matches!(
            terminal.feed_input(b'a'),
            super::FeedResult::Accepted(None)
        ));
        assert!(matches!(
            terminal.feed_input(127),
            super::FeedResult::Accepted(None)
        ));
        assert!(matches!(
            terminal.feed_input(b'b'),
            super::FeedResult::Accepted(None)
        ));
        assert!(matches!(
            terminal.feed_input(4),
            super::FeedResult::Accepted(None)
        ));
        let mut output = [0; 4];
        assert_eq!(terminal.read_input(&mut output), Ok(1));
        assert_eq!(output[0], b'b');
        assert_eq!(terminal.read_input(&mut output), Err(Error::WouldBlock));
    }

    #[test]
    fn termios_action_and_reserved_bits_are_bounded() {
        assert_eq!(Termios::DEFAULT.validate(), Ok(()));
        assert_eq!(Terminal::pty().termios.validate(), Ok(()));
        assert_ne!(TCSANOW, TCSADRAIN);
        assert_ne!(TCSADRAIN, TCSAFLUSH);
        assert_eq!(
            Termios {
                reserved: 1,
                ..Termios::DEFAULT
            }
            .validate(),
            Err(Error::InvalidTermios)
        );
    }
}
