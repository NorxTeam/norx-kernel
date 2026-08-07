#![no_std]
#![no_main]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

mod arch;
mod boot;
mod bootlog;
mod crash;
mod drivers;
mod error;
mod font;
mod framebuffer;
mod irq;
mod log;
mod memory;
mod paging;
mod sched;
mod serial_debugger;
mod time;
mod timer;
mod vfs;
#[cfg(target_arch = "x86_64")]
mod vga;
mod vm;

use core::panic::PanicInfo;

pub fn kernel_start() -> ! {
    log::init();
    let boot = boot::info();
    #[cfg(target_arch = "x86_64")]
    bootlog::info("VGA fallback initialized");
    if let Some(raw) = boot.framebuffer {
        log::init_framebuffer(raw);
    }
    bootlog::title();
    bootlog::ok_fmt(format_args!(
        "GRUB hand-off accepted arch={} memory={} modules={}",
        boot.architecture.name(),
        boot.memory_len,
        boot.modules_len
    ));
    if !boot.cmdline().is_empty() {
        bootlog::info(boot.cmdline());
    }
    if let Some(raw) = boot.framebuffer {
        let fb = framebuffer::init(raw);
        bootlog::ok_fmt(format_args!(
            "framebuffer {}x{} pitch {}",
            fb.width(),
            fb.height(),
            fb.pitch()
        ));
    }
    time::init();
    bootlog::ok("kernel clock initialized");
    bootlog::spin(0, "probing built-in drivers");
    drivers::init();
    bootlog::ok("driver framework initialized");
    vfs::init();
    bootlog::ok("vfs initialized");
    bootlog::ok("serial-debugger input ready");

    if boot.framebuffer.is_none() {
        bootlog::warn("framebuffer unavailable; serial remains active");
    }

    let summary = memory::init(boot);
    bootlog::ok_fmt(format_args!("memory map {} regions", summary.descriptors));
    bootlog::ok_fmt(format_args!(
        "physical allocator {} KiB usable",
        summary.usable_pages * 4
    ));
    if summary.skipped_ranges != 0 {
        bootlog::warn_fmt(format_args!(
            "physical allocator skipped {} memory ranges",
            summary.skipped_ranges
        ));
    }
    if let Some(frame) = memory::alloc_frame() {
        bootlog::ok_fmt(format_args!("first free frame 0x{:x}", frame));
    } else {
        bootlog::fail("physical allocator has no free frames");
    }

    arch::tables::init();
    bootlog::ok("architecture tables initialized");
    arch::init_interrupt_controller();
    arch::init_syscalls();
    paging::init();
    vm::init();
    sched::self_check();
    sched::init_runtime();
    bootlog::ok("scheduler initialized");
    if timer::init() {
        bootlog::ok("hardware scheduler timer initialized");
        bootlog::info("timer source irq");
    } else {
        bootlog::warn("hardware scheduler timer unavailable; using polling");
        bootlog::info("timer source polling");
    }
    bootlog::ok_fmt(format_args!("architecture {}", arch::NAME));
    bootlog::ok_fmt(format_args!("timer ticks {}", time::ticks()));
    bootlog::ok("kernel alive");
    bootlog::warn("future OS bootloader hand-off deferred");
    serial_debugger::run()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("panic: {}", info);
    crash::fatal(error::KernelError::panic())
}
