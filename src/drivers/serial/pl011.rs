const DR: usize = 0x00;
const FR: usize = 0x18;
const IBRD: usize = 0x24;
const FBRD: usize = 0x28;
const LCRH: usize = 0x2c;
const CR: usize = 0x30;
const IMSC: usize = 0x38;
const ICR: usize = 0x44;

pub fn init(base: usize) {
    unsafe {
        write_reg(base, CR, 0);
        write_reg(base, ICR, 0x7ff);
        write_reg(base, IBRD, 1);
        write_reg(base, FBRD, 40);
        write_reg(base, LCRH, 0x70);
        write_reg(base, IMSC, 0);
        write_reg(base, CR, 0x301);
    }
}

pub fn write(base: usize, byte: u8) {
    unsafe {
        while read_reg(base, FR) & (1 << 5) != 0 {}
        write_reg(base, DR, byte as u32);
    }
}

pub fn read(base: usize) -> Option<u8> {
    unsafe {
        if read_reg(base, FR) & (1 << 4) != 0 {
            None
        } else {
            Some(read_reg(base, DR) as u8)
        }
    }
}

unsafe fn read_reg(base: usize, reg: usize) -> u32 {
    ((base + reg) as *const u32).read_volatile()
}

unsafe fn write_reg(base: usize, reg: usize, value: u32) {
    ((base + reg) as *mut u32).write_volatile(value);
}
