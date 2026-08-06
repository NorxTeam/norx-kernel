#![allow(dead_code)]

use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

pub const SCLA_NUMBER: u64 = 0x5c1a;
pub const SCLA_MAGIC: u64 = 0x424f415f53434c41;

#[repr(u64)]
#[derive(Clone, Copy)]
pub enum UniversalOp {
    Activate = 0,
    Exit = 1,
    Read = 2,
    Write = 3,
    Open = 4,
    Close = 5,
    Map = 6,
    Unmap = 7,
    Clock = 8,
    Spawn = 9,
}

#[derive(Clone, Copy)]
pub struct UniversalCall {
    pub op: UniversalOp,
    pub args: [u64; 6],
}

#[derive(Clone, Copy)]
pub struct Return {
    pub value: u64,
    pub error: u64,
}

#[derive(Clone, Copy)]
pub enum IoTarget {
    Kernel = 0,
    User = 1,
    Serial = 2,
}

impl IoTarget {
    pub const fn name(self) -> &'static str {
        match self {
            IoTarget::Kernel => "kernel",
            IoTarget::User => "user",
            IoTarget::Serial => "serial",
        }
    }
}

#[derive(Clone, Copy)]
pub struct ProcessIo {
    pub stdin: IoTarget,
    pub stdout: IoTarget,
    pub stderr: IoTarget,
}

impl ProcessIo {
    pub const fn kernel() -> Self {
        Self {
            stdin: IoTarget::Kernel,
            stdout: IoTarget::Kernel,
            stderr: IoTarget::Kernel,
        }
    }

    pub const fn session(target: IoTarget) -> Self {
        Self {
            stdin: target,
            stdout: target,
            stderr: target,
        }
    }
}

static STDIN: AtomicU8 = AtomicU8::new(IoTarget::Kernel as u8);
static STDOUT: AtomicU8 = AtomicU8::new(IoTarget::Kernel as u8);
static STDERR: AtomicU8 = AtomicU8::new(IoTarget::Kernel as u8);
const IO_RING: usize = 256;

static USER_STDOUT: Ring = Ring::new();
static USER_STDERR: Ring = Ring::new();
static SERIAL_STDOUT: Ring = Ring::new();
static SERIAL_STDERR: Ring = Ring::new();

pub struct IoGuard(ProcessIo);

pub fn enter_io(io: ProcessIo) -> IoGuard {
    IoGuard(swap_process_io(io))
}

impl Drop for IoGuard {
    fn drop(&mut self) {
        swap_process_io(self.0);
    }
}

fn swap_process_io(io: ProcessIo) -> ProcessIo {
    ProcessIo {
        stdin: swap_target(&STDIN, io.stdin),
        stdout: swap_target(&STDOUT, io.stdout),
        stderr: swap_target(&STDERR, io.stderr),
    }
}

fn swap_target(slot: &AtomicU8, target: IoTarget) -> IoTarget {
    decode_target(slot.swap(target as u8, Ordering::Relaxed))
}

pub fn dispatch(call: UniversalCall) -> Return {
    dispatch_with(crate::capability::KERNEL, call)
}

pub fn dispatch_with(caps: crate::capability::Set, call: UniversalCall) -> Return {
    match call.op {
        UniversalOp::Activate => Return {
            value: SCLA_MAGIC,
            error: 0,
        },
        UniversalOp::Exit => Return {
            value: call.args[0],
            error: 0,
        },
        UniversalOp::Clock => {
            if caps.contains(crate::capability::Set::CLOCK) {
                Return {
                    value: crate::time::ticks(),
                    error: 0,
                }
            } else {
                Return { value: 0, error: 1 }
            }
        }
        UniversalOp::Read => match read_input_byte() {
            Some(byte) => Return {
                value: byte as u64,
                error: 0,
            },
            None => Return {
                value: 0,
                error: 11,
            },
        },
        UniversalOp::Write => {
            if !caps.contains(crate::capability::Set::LOG_WRITE) {
                return Return { value: 0, error: 1 };
            }
            let len = call.args[1] as usize;
            if len == 0 {
                let byte = call.args[0] as u8;
                if writable_byte(byte) {
                    write_output_byte(byte, call.args[2]);
                    return Return { value: 1, error: 0 };
                }
                return Return {
                    value: 0,
                    error: 22,
                };
            }
            let mut written = 0;
            let mut ok = true;
            if !crate::arch::user::read_bytes(call.args[0], len, |byte| {
                if writable_byte(byte) {
                    write_output_byte(byte, call.args[2]);
                    written += 1;
                } else {
                    ok = false;
                }
            }) {
                return Return {
                    value: 0,
                    error: 14,
                };
            }
            if ok {
                Return {
                    value: written,
                    error: 0,
                }
            } else {
                Return {
                    value: written,
                    error: 22,
                }
            }
        }
        UniversalOp::Spawn => Return {
            value: 0,
            error: 38,
        },
        _ => Return {
            value: 0,
            error: 38,
        },
    }
}

fn decode_target(value: u8) -> IoTarget {
    match value {
        1 => IoTarget::User,
        2 => IoTarget::Serial,
        _ => IoTarget::Kernel,
    }
}

fn write_target(fd: u64) -> IoTarget {
    if fd == 2 {
        decode_target(STDERR.load(Ordering::Relaxed))
    } else {
        decode_target(STDOUT.load(Ordering::Relaxed))
    }
}

fn read_target() -> IoTarget {
    decode_target(STDIN.load(Ordering::Relaxed))
}

fn write_output_byte(byte: u8, fd: u64) {
    let buf = [byte];
    let text = core::str::from_utf8(&buf).unwrap_or("");
    let target = write_target(fd);
    record_output(target, fd, byte);
    match target {
        IoTarget::Kernel => crate::kprint!("{}", byte as char),
        IoTarget::User => crate::log::screen_write_str(text),
        IoTarget::Serial => crate::drivers::serial::write_str(text),
    }
}

fn read_input_byte() -> Option<u8> {
    match read_target() {
        IoTarget::Serial => crate::drivers::serial::read(),
        IoTarget::User => key_byte(crate::input::poll_user()?),
        IoTarget::Kernel => None,
    }
}

fn key_byte(key: crate::input::Key) -> Option<u8> {
    match key {
        crate::input::Key::Char(byte) => Some(byte),
        crate::input::Key::Enter => Some(b'\n'),
        crate::input::Key::Backspace => Some(8),
        _ => None,
    }
}

fn writable_byte(byte: u8) -> bool {
    byte.is_ascii_graphic() || byte == b' ' || byte == b'\n'
}

pub struct IoStatus {
    pub user_stdout: usize,
    pub user_stderr: usize,
    pub serial_stdout: usize,
    pub serial_stderr: usize,
}

pub fn io_status() -> IoStatus {
    IoStatus {
        user_stdout: USER_STDOUT.written(),
        user_stderr: USER_STDERR.written(),
        serial_stdout: SERIAL_STDOUT.written(),
        serial_stderr: SERIAL_STDERR.written(),
    }
}

pub fn io_tail(target: IoTarget, fd: u64, out: &mut [u8]) -> usize {
    let Some(ring) = output_ring(target, fd) else {
        return 0;
    };
    ring.tail(out)
}

pub fn io_read(target: IoTarget, fd: u64, cursor: &mut usize, out: &mut [u8]) -> usize {
    let Some(ring) = output_ring(target, fd) else {
        return 0;
    };
    ring.read_from(cursor, out)
}

fn record_output(target: IoTarget, fd: u64, byte: u8) {
    match (target, fd) {
        (IoTarget::User, 2) => USER_STDERR.push(byte),
        (IoTarget::User, _) => USER_STDOUT.push(byte),
        (IoTarget::Serial, 2) => SERIAL_STDERR.push(byte),
        (IoTarget::Serial, _) => SERIAL_STDOUT.push(byte),
        (IoTarget::Kernel, _) => {}
    }
}

fn output_ring(target: IoTarget, fd: u64) -> Option<&'static Ring> {
    match (target, fd) {
        (IoTarget::User, 2) => Some(&USER_STDERR),
        (IoTarget::User, _) => Some(&USER_STDOUT),
        (IoTarget::Serial, 2) => Some(&SERIAL_STDERR),
        (IoTarget::Serial, _) => Some(&SERIAL_STDOUT),
        (IoTarget::Kernel, _) => None,
    }
}

struct Ring {
    bytes: [AtomicU8; IO_RING],
    head: AtomicUsize,
    written: AtomicUsize,
}

impl Ring {
    const fn new() -> Self {
        Self {
            bytes: [const { AtomicU8::new(0) }; IO_RING],
            head: AtomicUsize::new(0),
            written: AtomicUsize::new(0),
        }
    }

    fn push(&self, byte: u8) {
        let head = self.head.fetch_add(1, Ordering::Relaxed) % IO_RING;
        self.bytes[head].store(byte, Ordering::Relaxed);
        self.written.fetch_add(1, Ordering::Relaxed);
    }

    fn written(&self) -> usize {
        self.written.load(Ordering::Relaxed)
    }

    fn tail(&self, out: &mut [u8]) -> usize {
        let len = self.written().min(IO_RING).min(out.len());
        let head = self.head.load(Ordering::Relaxed);
        let start = head.saturating_sub(len);
        for (i, slot) in out.iter_mut().take(len).enumerate() {
            *slot = self.bytes[(start + i) % IO_RING].load(Ordering::Relaxed);
        }
        len
    }

    fn read_from(&self, cursor: &mut usize, out: &mut [u8]) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let start = (*cursor).max(head.saturating_sub(IO_RING));
        let len = head.saturating_sub(start).min(out.len());
        for (i, slot) in out.iter_mut().take(len).enumerate() {
            *slot = self.bytes[(start + i) % IO_RING].load(Ordering::Relaxed);
        }
        *cursor = start + len;
        len
    }
}

pub fn native_registers() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64: nr/op=rax args=rdi,rsi,rdx,r10,r8,r9 ret=rax"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64: nr/op=x8 args=x0..x5 ret=x0"
    }
}

pub fn activation_registers() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "SCLA trap: syscall nr=0x5c1a magic=rdi"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "SCLA trap: svc #0 nr=x8=0x5c1a magic=x0"
    }
}

pub fn universal_registers() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "Norx universal: op=rax args=rdi,rsi,rdx,r10,r8,r9"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "Norx universal: op=x8 args=x0..x5"
    }
}

pub fn is_scla(number: u64, magic: u64) -> bool {
    number == SCLA_NUMBER && magic == SCLA_MAGIC
}
