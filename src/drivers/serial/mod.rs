use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_arch = "x86_64")]
pub mod ns16550;
#[cfg(target_arch = "aarch64")]
pub mod pl011;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FlowControl {
    None,
    RtsCts,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SerialConfig {
    pub clock_hz: u32,
    pub baud: u32,
    pub fifo: bool,
    pub flow_control: FlowControl,
}

impl SerialConfig {
    pub const fn new(clock_hz: u32, baud: u32, fifo: bool, flow_control: FlowControl) -> Self {
        Self {
            clock_hz,
            baud,
            fifo,
            flow_control,
        }
    }
}

static FAILED: AtomicBool = AtomicBool::new(false);

pub struct Serial;

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' && !write_byte(b'\r') {
                return Err(fmt::Error);
            }
            if !write_byte(byte) {
                return Err(fmt::Error);
            }
        }
        Ok(())
    }
}

pub fn write(args: fmt::Arguments) -> bool {
    Serial.write_fmt(args).is_ok()
}

pub fn write_str(s: &str) -> bool {
    Serial.write_str(s).is_ok()
}

pub fn read() -> Option<u8> {
    if FAILED.load(Ordering::Relaxed) {
        return None;
    }
    crate::arch::early_serial_read()
}

fn write_byte(byte: u8) -> bool {
    if FAILED.load(Ordering::Relaxed) {
        return false;
    }
    let ok = crate::arch::early_serial_write(byte);
    if !ok {
        FAILED.store(true, Ordering::Relaxed);
    }
    ok
}

pub(crate) fn mark_failed() {
    FAILED.store(true, Ordering::Relaxed);
}
