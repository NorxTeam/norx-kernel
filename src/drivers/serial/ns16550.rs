#[cfg(target_arch = "x86_64")]
const RBR_THR: usize = 0;
#[cfg(target_arch = "x86_64")]
const IER: usize = 1;
#[cfg(target_arch = "x86_64")]
const FCR: usize = 2;
#[cfg(target_arch = "x86_64")]
const LCR: usize = 3;
#[cfg(target_arch = "x86_64")]
const MCR: usize = 4;
const LSR: usize = 5;
#[cfg(target_arch = "x86_64")]
const DLL: usize = 0;
#[cfg(target_arch = "x86_64")]
const DLM: usize = 1;

#[cfg(target_arch = "x86_64")]
pub fn init_port_io(base: u16) {
    unsafe {
        crate::arch::outb(base + IER as u16, 0x00);
        crate::arch::outb(base + LCR as u16, 0x80);
        crate::arch::outb(base + DLL as u16, 0x03);
        crate::arch::outb(base + DLM as u16, 0x00);
        crate::arch::outb(base + LCR as u16, 0x03);
        crate::arch::outb(base + FCR as u16, 0xc7);
        crate::arch::outb(base + MCR as u16, 0x0b);
    }
}

#[cfg(target_arch = "x86_64")]
pub fn write_port_io(base: u16, byte: u8) {
    unsafe {
        while crate::arch::inb(base + LSR as u16) & 0x20 == 0 {}
        crate::arch::outb(base + RBR_THR as u16, byte);
    }
}

#[cfg(target_arch = "x86_64")]
pub fn read_port_io(base: u16) -> Option<u8> {
    unsafe {
        if crate::arch::inb(base + LSR as u16) & 1 == 0 {
            None
        } else {
            Some(crate::arch::inb(base + RBR_THR as u16))
        }
    }
}

#[cfg(target_arch = "riscv64")]
pub fn init_mmio(base: usize) {
    unsafe {
        write_reg(base, 1, 0x00);
        write_reg(base, 3, 0x80);
        write_reg(base, 0, 0x03);
        write_reg(base, 1, 0x00);
        write_reg(base, 3, 0x03);
        write_reg(base, 2, 0xc7);
        write_reg(base, 4, 0x0b);
    }
}

#[cfg(target_arch = "riscv64")]
pub fn write_mmio(base: usize, byte: u8) {
    unsafe {
        while read_reg(base, LSR) & 0x20 == 0 {}
        write_reg(base, 0, byte);
    }
}

#[cfg(target_arch = "riscv64")]
unsafe fn read_reg(base: usize, reg: usize) -> u8 {
    ((base + reg) as *const u8).read_volatile()
}

#[cfg(target_arch = "riscv64")]
unsafe fn write_reg(base: usize, reg: usize, value: u8) {
    ((base + reg) as *mut u8).write_volatile(value);
}
