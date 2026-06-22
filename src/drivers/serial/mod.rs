use core::fmt::{self, Write};

#[cfg(any(target_arch = "riscv64", target_arch = "x86_64"))]
pub mod ns16550;
#[cfg(target_arch = "aarch64")]
pub mod pl011;

pub struct Serial;

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                write_byte(b'\r');
            }
            write_byte(byte);
        }
        Ok(())
    }
}

pub fn write(args: fmt::Arguments) {
    let _ = Serial.write_fmt(args);
}

pub fn write_str(s: &str) {
    let _ = Serial.write_str(s);
}

pub fn read() -> Option<u8> {
    #[cfg(target_arch = "x86_64")]
    {
        ns16550::read_port_io(0x3f8)
    }
    #[cfg(target_arch = "aarch64")]
    {
        pl011::read(0x0900_0000)
    }
}

fn write_byte(byte: u8) {
    #[cfg(target_arch = "x86_64")]
    ns16550::write_port_io(0x3f8, byte);
    #[cfg(target_arch = "riscv64")]
    ns16550::write_mmio(0x1000_0000, byte);
    #[cfg(target_arch = "aarch64")]
    pl011::write(0x0900_0000, byte);
}
