use super::{FlowControl, SerialConfig, EARLY_TX_POLL_LIMIT};

const RBR_THR: usize = 0;
const IER: usize = 1;
const FCR: usize = 2;
const LCR: usize = 3;
const MCR: usize = 4;
const LSR: usize = 5;
const SCRATCH: usize = 7;
const DLL: usize = 0;
const DLM: usize = 1;
const UART_CLOCK_HZ: u32 = 1_843_200;
const DEFAULT_BAUD: u32 = 115_200;

const DEFAULT_CONFIG: SerialConfig =
    SerialConfig::new(UART_CLOCK_HZ, DEFAULT_BAUD, true, FlowControl::None);

pub struct Port {
    io: crate::io::PioRegion,
    config: SerialConfig,
}

impl Port {
    pub fn new(base: u16, config: SerialConfig) -> Option<Self> {
        Some(Self {
            io: crate::io::PioRegion::new(base, 8)?,
            config,
        })
    }

    pub fn init(&self) -> bool {
        let Some(divisor) = divisor(self.config) else {
            return false;
        };
        let (mcr, fcr) = control_values(self.config);
        self.io.write_u8(IER, 0x00)
            && self.io.write_u8(LCR, 0x80)
            && self.io.write_u8(DLL, divisor as u8)
            && self.io.write_u8(DLM, (divisor >> 8) as u8)
            && self.io.write_u8(LCR, 0x03)
            && self.io.write_u8(FCR, fcr)
            && self.io.write_u8(MCR, mcr)
            && probe_scratch(&self.io)
    }

    pub fn write(&self, byte: u8) -> bool {
        for _ in 0..EARLY_TX_POLL_LIMIT {
            if self.tx_ready() {
                return self.io.write_u8(RBR_THR, byte);
            }
        }
        false
    }

    pub fn tx_ready(&self) -> bool {
        self.io.read_u8(LSR).unwrap_or(0) & 0x20 != 0
    }

    pub fn read(&self) -> Option<u8> {
        if self.io.read_u8(LSR)? & 1 == 0 {
            None
        } else {
            self.io.read_u8(RBR_THR)
        }
    }
}

fn probe_scratch(io: &crate::io::PioRegion) -> bool {
    const PROBE: u8 = 0xa5;
    io.write_u8(SCRATCH, PROBE) && io.read_u8(SCRATCH) == Some(PROBE)
}

fn control_values(config: SerialConfig) -> (u8, u8) {
    let mcr = match config.flow_control {
        FlowControl::None => 0x0b,
        FlowControl::RtsCts => 0x2b,
    };
    let fcr = if config.fifo { 0xc7 } else { 0 };
    (mcr, fcr)
}

fn divisor(config: SerialConfig) -> Option<u16> {
    if config.clock_hz == 0 || config.baud == 0 {
        return None;
    }
    let denominator = config.baud.checked_mul(16)?;
    let divisor = config.clock_hz.checked_add(denominator / 2)? / denominator;
    if divisor == 0 || divisor > u16::MAX as u32 {
        None
    } else {
        Some(divisor as u16)
    }
}

pub fn init_port_io(base: u16) -> bool {
    Port::new(base, DEFAULT_CONFIG)
        .map(|port| port.init())
        .unwrap_or(false)
}

pub fn write_port_io(base: u16, byte: u8) -> bool {
    Port::new(base, DEFAULT_CONFIG)
        .map(|port| port.write(byte))
        .unwrap_or(false)
}

pub fn tx_ready_port_io(base: u16) -> bool {
    Port::new(base, DEFAULT_CONFIG)
        .map(|port| port.tx_ready())
        .unwrap_or(false)
}

pub fn read_port_io(base: u16) -> Option<u8> {
    Port::new(base, DEFAULT_CONFIG)?.read()
}

pub fn contract_self_check() {
    assert_eq!(divisor(DEFAULT_CONFIG), Some(1));
    assert!(divisor(SerialConfig::new(0, DEFAULT_BAUD, true, FlowControl::None)).is_none());
    assert_eq!(control_values(DEFAULT_CONFIG), (0x0b, 0xc7));
    assert_eq!(
        control_values(SerialConfig::new(
            UART_CLOCK_HZ,
            DEFAULT_BAUD,
            false,
            FlowControl::RtsCts,
        )),
        (0x2b, 0)
    );
}
