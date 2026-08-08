use core::arch::asm;
use core::sync::atomic::{fence, Ordering};

pub const NAME: &str = "x86_64";

pub mod apic;
pub mod paging;
pub mod syscall;
pub mod tables;

const EARLY_SERIAL_PORT: u16 = 0x3f8;

static mut FIRMWARE_CR3: u64 = 0;
static mut RUNTIME_CR3: u64 = 0;

pub fn init() {
    unsafe { FIRMWARE_CR3 = paging::current_cr3_value() };
    if !crate::drivers::serial::ns16550::init_port_io(EARLY_SERIAL_PORT) {
        crate::drivers::serial::mark_failed();
    }
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) };
}

pub fn early_serial_read() -> Option<u8> {
    crate::drivers::serial::ns16550::read_port_io(EARLY_SERIAL_PORT)
}

pub fn early_serial_write(byte: u8) -> bool {
    crate::drivers::serial::ns16550::write_port_io(EARLY_SERIAL_PORT, byte)
}

pub fn halt() -> ! {
    loop {
        unsafe { asm!("cli; hlt", options(nomem, nostack, preserves_flags)) };
    }
}

pub fn prepare_firmware_runtime() -> bool {
    let firmware_cr3 = unsafe { FIRMWARE_CR3 };
    if firmware_cr3 == 0 {
        return false;
    }
    let current_cr3 = paging::current_cr3_value();
    if current_cr3 != firmware_cr3 {
        unsafe { RUNTIME_CR3 = current_cr3 };
        paging::switch_cr3(firmware_cr3);
    }
    true
}

pub fn finish_firmware_runtime() {
    let runtime_cr3 = unsafe { RUNTIME_CR3 };
    if runtime_cr3 != 0 {
        paging::switch_cr3(runtime_cr3);
        unsafe { RUNTIME_CR3 = 0 };
    }
}

pub fn port_write(port: u16, value: u8) {
    fence(Ordering::SeqCst);
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
    }
    fence(Ordering::SeqCst);
}

pub fn port_read(port: u16) -> u8 {
    let value: u8;
    fence(Ordering::SeqCst);
    unsafe {
        asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    fence(Ordering::SeqCst);
    value
}

pub fn port_write_u16(port: u16, value: u16) {
    fence(Ordering::SeqCst);
    unsafe {
        asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags));
    }
    fence(Ordering::SeqCst);
}

pub fn port_read_u16(port: u16) -> u16 {
    let value: u16;
    fence(Ordering::SeqCst);
    unsafe {
        asm!("in ax, dx", out("ax") value, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    fence(Ordering::SeqCst);
    value
}

pub fn port_write_u32(port: u16, value: u32) {
    fence(Ordering::SeqCst);
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
    }
    fence(Ordering::SeqCst);
}

pub fn port_read_u32(port: u16) -> u32 {
    let value: u32;
    fence(Ordering::SeqCst);
    unsafe {
        asm!("in eax, dx", out("eax") value, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    fence(Ordering::SeqCst);
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
    remap_pic();
    init_pit(crate::time::scheduler_hz() as u16);
    let input_ok = crate::drivers::ps2::enable_interrupts();
    let ps2 = crate::drivers::ps2::status();
    if input_ok && ps2.controller {
        crate::bootlog::info("ps/2 IRQ1/IRQ12 routing enabled");
    } else if !input_ok && ps2.controller {
        crate::bootlog::warn("ps/2 interrupt routing unavailable; input remains polled");
    } else {
        crate::bootlog::info("ps/2 IRQ routing skipped; controller unavailable");
    }
    unsafe { asm!("sti", options(nomem, nostack, preserves_flags)) };
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

pub fn init_syscalls() -> bool {
    let ready = syscall::init();
    if ready {
        crate::bootlog::ok("x86_64 syscall entry initialized");
    } else {
        crate::bootlog::fail("x86_64 syscall entry stack unavailable");
    }
    ready
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
    port_write(0x20, 0x20);
}

pub fn end_legacy_interrupt(line: u8) {
    if line >= 8 {
        port_write(0xa0, 0x20);
    }
    port_write(0x20, 0x20);
}

pub fn enable_legacy_irq(line: u8) {
    if line < 8 {
        let mask = port_read(0x21) & !(1 << line);
        port_write(0x21, mask);
        return;
    }
    let slave_line = line - 8;
    let slave_mask = port_read(0xa1) & !(1 << slave_line);
    port_write(0xa1, slave_mask);
    let master_mask = port_read(0x21) & !(1 << 2);
    port_write(0x21, master_mask);
}

pub fn fault_address() -> usize {
    let value: usize;
    unsafe { asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

fn remap_pic() {
    let master_mask = port_read(0x21);
    let slave_mask = port_read(0xa1);

    port_write(0x20, 0x11);
    io_wait();
    port_write(0xa0, 0x11);
    io_wait();
    port_write(0x21, 0x20);
    io_wait();
    port_write(0xa1, 0x28);
    io_wait();
    port_write(0x21, 0x04);
    io_wait();
    port_write(0xa1, 0x02);
    io_wait();
    port_write(0x21, 0x01);
    io_wait();
    port_write(0xa1, 0x01);
    io_wait();
    port_write(0x21, master_mask & !0b11);
    port_write(0xa1, slave_mask);
}

fn init_pit(hz: u16) {
    let divisor = (1_193_182u32 / hz.max(1) as u32).min(u16::MAX as u32) as u16;
    port_write(0x43, 0x36);
    port_write(0x40, divisor as u8);
    port_write(0x40, (divisor >> 8) as u8);
}

fn io_wait() {
    port_write(0x80, 0);
}

pub fn map_lazy_page(virtual_address: usize) -> bool {
    paging::map_lazy_page(virtual_address)
}

pub fn supports_lazy_pages() -> bool {
    true
}

pub fn physical_to_virtual(address: crate::address::PhysAddr) -> Option<crate::address::VirtAddr> {
    paging::direct_map_ptr(address.value())
        .map(|pointer| crate::address::VirtAddr::new(pointer as usize))
}

pub fn virtual_to_physical(address: crate::address::VirtAddr) -> Option<crate::address::PhysAddr> {
    paging::physical_from_direct_map(address.value()).map(crate::address::PhysAddr::new)
}

pub fn user_space_prepare(root: crate::address::PhysAddr) -> bool {
    paging::user_space_prepare(root)
}

pub fn user_space_map(
    root: crate::address::PhysAddr,
    mapping: crate::address_space::MappingInfo,
    tables: &mut [Option<crate::address::PhysAddr>],
) -> bool {
    paging::user_space_map(root, mapping, tables)
}

pub fn user_space_unmap(root: crate::address::PhysAddr, virtual_address: usize) -> bool {
    paging::user_space_unmap(root, virtual_address)
}

pub fn user_space_reset(root: crate::address::PhysAddr) {
    paging::user_space_reset(root)
}

pub fn switch_to_user(root: crate::address::PhysAddr) -> bool {
    paging::switch_to_user(root)
}

pub fn restore_kernel_address_space() {
    paging::restore_kernel()
}

pub fn write_physical(physical: crate::address::PhysAddr, offset: usize, bytes: &[u8]) -> bool {
    paging::write_physical(physical, offset, bytes)
}

pub fn zero_physical_page(physical: crate::address::PhysAddr) -> bool {
    paging::zero_physical_page(physical)
}

pub fn enter_user(registers: crate::elf::InitialRegisters) -> bool {
    syscall::enter_user(registers)
}
