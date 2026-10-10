use core::fmt::{self, Write};
use core::sync::atomic::{AtomicU8, Ordering};

#[cfg(target_arch = "x86_64")]
pub mod ns16550;
#[cfg(target_arch = "aarch64")]
pub mod pl011;

pub(crate) const EARLY_TX_POLL_LIMIT: usize = 4096;

const _: () = {
    assert!(EARLY_TX_POLL_LIMIT > 0);
    assert!(EARLY_TX_POLL_LIMIT <= 4096);
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureReason {
    Init,
    TxTimeout,
}

impl FailureReason {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::TxTimeout => "tx-timeout",
        }
    }

    const fn code(self) -> u8 {
        match self {
            Self::Init => 1,
            Self::TxTimeout => 2,
        }
    }
}

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

const STATE_READY: u8 = 0;

static STATE: AtomicU8 = AtomicU8::new(STATE_READY);

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

pub fn write_bytes(bytes: &[u8]) -> bool {
    bytes.iter().copied().all(|byte| {
        if byte == b'\n' && !write_byte(b'\r') {
            return false;
        }
        write_byte(byte)
    })
}

pub fn read() -> Option<u8> {
    if STATE.load(Ordering::Acquire) != STATE_READY {
        return None;
    }
    crate::arch::early_serial_read()
}

pub fn available() -> bool {
    STATE.load(Ordering::Acquire) == STATE_READY
}

pub fn failure_reason() -> Option<FailureReason> {
    failure_reason_code(STATE.load(Ordering::Acquire))
}

pub const fn backend_name() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "ns16550"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "pl011"
    }
}

fn write_byte(byte: u8) -> bool {
    if STATE.load(Ordering::Acquire) != STATE_READY {
        return false;
    }
    let ok = crate::arch::early_serial_write(byte);
    if !ok {
        mark_failed(FailureReason::TxTimeout);
    }
    ok
}

pub(crate) fn mark_failed(reason: FailureReason) {
    mark_failure(&STATE, reason);
}

fn mark_failure(state: &AtomicU8, reason: FailureReason) {
    let _ = state.compare_exchange(
        STATE_READY,
        reason.code(),
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

fn failure_reason_code(code: u8) -> Option<FailureReason> {
    match code {
        1 => Some(FailureReason::Init),
        2 => Some(FailureReason::TxTimeout),
        _ => None,
    }
}

pub fn contract_self_check() {
    let state = AtomicU8::new(STATE_READY);
    mark_failure(&state, FailureReason::Init);
    mark_failure(&state, FailureReason::TxTimeout);
    assert_eq!(state.load(Ordering::Acquire), FailureReason::Init.code());
    assert!(failure_reason_code(STATE_READY).is_none());
    assert!(failure_reason_code(FailureReason::Init.code()) == Some(FailureReason::Init));
    assert!(failure_reason_code(FailureReason::TxTimeout.code()) == Some(FailureReason::TxTimeout));
    assert_eq!(FailureReason::Init.name(), "init");
    assert_eq!(FailureReason::TxTimeout.name(), "tx-timeout");
}
