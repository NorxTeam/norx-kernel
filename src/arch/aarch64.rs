use core::arch::asm;

pub const NAME: &str = "aarch64";

pub mod paging;
pub mod syscall;
pub mod tables;

pub fn init() {
    unsafe {
        asm!(
            "msr daifset, #0xf",
            options(nomem, nostack, preserves_flags)
        )
    };
    crate::drivers::serial::pl011::init(0x0900_0000);
}

pub fn halt() -> ! {
    loop {
        unsafe {
            asm!(
                "msr daifset, #0xf; wfi",
                options(nomem, nostack, preserves_flags)
            )
        };
    }
}

pub fn ticks() -> u64 {
    let value: u64;
    unsafe { asm!("mrs {}, cntvct_el0", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

pub fn timer_frequency_hz() -> Option<u64> {
    let value: u64;
    unsafe { asm!("mrs {}, cntfrq_el0", out(reg) value, options(nomem, nostack, preserves_flags)) };
    Some(value)
}

pub fn init_timer_interrupts() -> bool {
    false
}

pub fn init_interrupt_controller() {
    crate::bootlog::warn("gic interrupt controller init deferred");
}

pub fn init_syscalls() {
    syscall::init();
    crate::bootlog::ok("aarch64 SVC syscall entry initialized");
}

pub fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
    let daif: u64;
    unsafe {
        asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack, preserves_flags));
        asm!(
            "msr daifset, #0xf",
            options(nomem, nostack, preserves_flags)
        );
    }
    let value = f();
    unsafe {
        asm!("msr daif, {}", in(reg) daif, options(nomem, nostack, preserves_flags));
    }
    value
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn map_lazy_page(_virtual_address: usize) -> bool {
    false
}

pub fn supports_lazy_pages() -> bool {
    false
}
