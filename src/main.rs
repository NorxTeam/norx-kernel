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
    bootlog::start(0, "accepting GRUB hand-off");
    bootlog::ok_fmt(format_args!(
        "GRUB hand-off accepted arch={} memory={} modules={}",
        boot.architecture.name(),
        boot.memory_len,
        boot.modules_len
    ));
    if !boot.cmdline().is_empty() {
        bootlog::start(1, "reading kernel command line");
        bootlog::info(boot.cmdline());
    }
    if let Some(raw) = boot.framebuffer {
        bootlog::start(2, "initializing framebuffer");
        let fb = framebuffer::init(raw);
        bootlog::ok_fmt(format_args!(
            "framebuffer {}x{} pitch {}",
            fb.width(),
            fb.height(),
            fb.pitch()
        ));
    }
    bootlog::start(3, "initializing kernel clock");
    time::init();
    bootlog::ok("kernel clock initialized");
    bootlog::start(0, "probing built-in drivers");
    drivers::init();
    bootlog::ok("driver framework initialized");
    bootlog::start(1, "checking virtual filesystem");
    vfs::init();
    bootlog::ok("vfs initialized");
    bootlog::start(2, "initializing serial-debugger");
    bootlog::ok("serial-debugger input ready");

    if boot.framebuffer.is_none() {
        bootlog::start(3, "checking framebuffer");
        bootlog::warn("framebuffer unavailable; serial remains active");
    }

    bootlog::start(2, "checking physical memory map");
    let summary = memory::init(boot);
    bootlog::ok_fmt(format_args!("memory map {} regions", summary.descriptors));
    bootlog::start(3, "checking physical allocator");
    bootlog::ok_fmt(format_args!(
        "physical allocator {} KiB usable",
        summary.usable_pages * 4
    ));
    if summary.skipped_ranges != 0 {
        bootlog::start(0, "checking memory range coverage");
        bootlog::warn_fmt(format_args!(
            "physical allocator skipped {} memory ranges",
            summary.skipped_ranges
        ));
    }
    bootlog::start(1, "allocating first free frame");
    if let Some(frame) = memory::alloc_frame() {
        bootlog::ok_fmt(format_args!("first free frame 0x{:x}", frame));
    } else {
        bootlog::fail("physical allocator has no free frames");
    }

    bootlog::start(3, "checking architecture tables");
    arch::tables::init();
    bootlog::ok("architecture tables initialized");
    bootlog::start(0, "checking interrupt controller");
    arch::init_interrupt_controller();
    bootlog::start(1, "initializing syscall entry");
    arch::init_syscalls();
    paging::init();
    vm::init();
    bootlog::start(1, "checking scheduler");
    sched::self_check();
    bootlog::ok("scheduler self-check passed");
    bootlog::start(2, "initializing scheduler runtime");
    sched::init_runtime();
    bootlog::ok("scheduler runtime initialized");
    bootlog::start(2, "checking timer source");
    let timer_ready = timer::init();
    if timer_ready {
        bootlog::ok("hardware scheduler timer initialized");
    } else {
        bootlog::warn("hardware scheduler timer unavailable; using polling");
    }
    bootlog::start(3, "selecting timer source");
    if timer_ready {
        bootlog::info("timer source irq");
    } else {
        bootlog::info("timer source polling");
    }
    bootlog::start(0, "reporting architecture");
    bootlog::ok_fmt(format_args!("architecture {}", arch::NAME));
    bootlog::start(1, "reading timer ticks");
    bootlog::ok_fmt(format_args!("timer ticks {}", time::ticks()));
    bootlog::start(2, "checking future OS hand-off");
    bootlog::warn("future OS bootloader hand-off deferred");
    bootlog::start(3, "finalizing kernel initialization");
    bootlog::ok("kernel initialization complete");
    serial_debugger::run()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("panic: {}", info);
    crash::fatal(error::KernelError::panic())
}
