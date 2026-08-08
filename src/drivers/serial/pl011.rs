use super::{FlowControl, SerialConfig};

const DR: usize = 0x00;
const FR: usize = 0x18;
const IBRD: usize = 0x24;
const FBRD: usize = 0x28;
const LCRH: usize = 0x2c;
const CR: usize = 0x30;
const IMSC: usize = 0x38;
const ICR: usize = 0x44;
const TX_POLL_LIMIT: usize = 1_000_000;
const UART_CLOCK_HZ: u32 = 24_000_000;
const DEFAULT_BAUD: u32 = 115_200;

const DEFAULT_CONFIG: SerialConfig =
    SerialConfig::new(UART_CLOCK_HZ, DEFAULT_BAUD, true, FlowControl::None);

fn region(base: usize) -> Option<crate::io::MmioRegion> {
    unsafe { crate::io::MmioRegion::new(base, 0x100) }
}

pub fn init(base: usize) -> bool {
    init_with_config(base, DEFAULT_CONFIG)
}

pub fn init_with_config(base: usize, config: SerialConfig) -> bool {
    let Some(mmio) = region(base) else {
        return false;
    };
    let Some((ibrd, fbrd)) = divisor(config) else {
        return false;
    };
    let (lcrh, cr) = control_values(config);
    mmio.write_u32_le(CR, 0)
        && mmio.write_u32_le(ICR, 0x7ff)
        && mmio.write_u32_le(IBRD, ibrd)
        && mmio.write_u32_le(FBRD, fbrd)
        && mmio.write_u32_le(LCRH, lcrh)
        && mmio.write_u32_le(IMSC, 0)
        && mmio.write_u32_le(CR, cr)
}

pub fn write(base: usize, byte: u8) -> bool {
    let Some(mmio) = region(base) else {
        return false;
    };
    for _ in 0..TX_POLL_LIMIT {
        if mmio.read_u32_le(FR).unwrap_or(1 << 5) & (1 << 5) == 0 {
            return mmio.write_u32_le(DR, byte as u32);
        }
    }
    false
}

pub fn read(base: usize) -> Option<u8> {
    let mmio = region(base)?;
    if mmio.read_u32_le(FR)? & (1 << 4) != 0 {
        None
    } else {
        mmio.read_u32_le(DR).map(|value| value as u8)
    }
}

fn control_values(config: SerialConfig) -> (u32, u32) {
    let lcrh = if config.fifo { 0x70 } else { 0x60 };
    let mut cr = 0x301;
    if config.flow_control == FlowControl::RtsCts {
        cr |= (1 << 14) | (1 << 15);
    }
    (lcrh, cr)
}

fn divisor(config: SerialConfig) -> Option<(u32, u32)> {
    if config.clock_hz == 0 || config.baud == 0 {
        return None;
    }
    let scaled = config
        .clock_hz
        .checked_mul(4)?
        .checked_add(config.baud / 2)?
        / config.baud;
    let integer = scaled / 64;
    if integer == 0 || integer > u16::MAX as u32 {
        return None;
    }
    Some((integer, scaled % 64))
}

pub fn contract_self_check() {
    assert_eq!(divisor(DEFAULT_CONFIG), Some((13, 1)));
    assert!(divisor(SerialConfig::new(0, DEFAULT_BAUD, true, FlowControl::None)).is_none());
    assert_eq!(control_values(DEFAULT_CONFIG), (0x70, 0x301));
    assert_eq!(
        control_values(SerialConfig::new(
            UART_CLOCK_HZ,
            DEFAULT_BAUD,
            false,
            FlowControl::RtsCts,
        )),
        (0x60, 0xc301)
    );
}
