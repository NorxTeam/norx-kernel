use core::arch::asm;

pub const NAME: &str = "x86_64";

pub mod apic;
pub mod paging;
pub mod syscall;
pub mod tables;
pub mod user;

pub fn init() {
    crate::drivers::serial::ns16550::init_port_io(0x3f8);
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) };
}

pub fn halt() -> ! {
    loop {
        unsafe { asm!("cli; hlt", options(nomem, nostack, preserves_flags)) };
    }
}

pub unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}

pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    value
}

pub fn ticks() -> u64 {
    let high: u32;
    let low: u32;
    unsafe {
        asm!("rdtsc", out("edx") high, out("eax") low, options(nomem, nostack, preserves_flags))
    };
    ((high as u64) << 32) | low as u64
}

pub fn timer_frequency_hz() -> Option<u64> {
    None
}

pub fn init_timer_interrupts() -> bool {
    unsafe {
        remap_pic();
        init_pit(crate::time::scheduler_hz() as u16);
        asm!("sti", options(nomem, nostack, preserves_flags));
    }
    true
}

pub fn init_interrupt_controller() {
    let status = apic::init();
    if status.present {
        crate::bootlog::ok_fmt(format_args!(
            "local apic id {} version {} base 0x{:x} x2apic={} enabled={} software={}",
            status.id,
            status.version,
            status.base,
            status.x2apic,
            status.enabled,
            status.software_enabled
        ));
    } else {
        crate::bootlog::warn("local apic unavailable; using legacy pic");
    }
}

pub fn init_syscalls() {
    syscall::init();
    let status = syscall::status();
    crate::bootlog::ok_fmt(format_args!(
        "syscall entry lstar 0x{:x} fmask 0x{:x} kstack 0x{:x}",
        status.lstar, status.fmask, status.kernel_stack_top
    ));
}

pub fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
    let flags: u64;
    unsafe {
        asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags));
        asm!("cli", options(nomem, nostack, preserves_flags));
    }
    let value = f();
    if flags & (1 << 9) != 0 {
        unsafe { asm!("sti", options(nomem, nostack, preserves_flags)) };
    }
    value
}

pub fn end_timer_interrupt() {
    unsafe { outb(0x20, 0x20) };
}

pub fn fault_address() -> usize {
    let value: usize;
    unsafe { asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

unsafe fn remap_pic() {
    let master_mask = inb(0x21);
    let slave_mask = inb(0xa1);

    outb(0x20, 0x11);
    io_wait();
    outb(0xa0, 0x11);
    io_wait();
    outb(0x21, 0x20);
    io_wait();
    outb(0xa1, 0x28);
    io_wait();
    outb(0x21, 0x04);
    io_wait();
    outb(0xa1, 0x02);
    io_wait();
    outb(0x21, 0x01);
    io_wait();
    outb(0xa1, 0x01);
    io_wait();
    outb(0x21, master_mask & !0b11);
    outb(0xa1, slave_mask);
}

unsafe fn init_pit(hz: u16) {
    let divisor = (1_193_182u32 / hz.max(1) as u32).min(u16::MAX as u32) as u16;
    outb(0x43, 0x36);
    outb(0x40, divisor as u8);
    outb(0x40, (divisor >> 8) as u8);
}

unsafe fn io_wait() {
    outb(0x80, 0);
}

pub fn map_lazy_page(virtual_address: usize) -> bool {
    paging::map_lazy_page(virtual_address)
}

pub fn supports_lazy_pages() -> bool {
    true
}

pub fn user_mode_ready() -> bool {
    tables::user_segments_ready() && user::context().ready
}

pub fn syscall_ready() -> bool {
    syscall::status().ready
}

pub fn prepare_user_mode() -> bool {
    user::prepare()
}
