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
mod input;
mod irq;
mod log;
mod memory;
mod paging;
mod sched;
mod shell;
mod time;
mod timer;
mod vfs;
mod vm;

use core::panic::PanicInfo;

pub fn kernel_start() -> ! {
    log::init();
    let boot = boot::info();
    bootlog::ok_fmt(format_args!(
        "GRUB hand-off accepted arch={} memory={} modules={}",
        boot.architecture.name(),
        boot.memory_len,
        boot.modules_len
    ));
    if !boot.cmdline().is_empty() {
        bootlog::info(boot.cmdline());
    }
    time::init();
    bootlog::ok("kernel clock initialized");
    bootlog::spin(0, "probing built-in drivers");
    drivers::init();
    bootlog::ok("driver framework initialized");
    vfs::init();
    bootlog::ok("vfs initialized");
    bootlog::spin(1, "probing input devices");
    input::init();
    bootlog::ok("input subsystem initialized");
    bootlog::warn("usb hid input deferred; using early console input");

    if let Some(raw) = boot.framebuffer {
        crash::init(raw);
        let fb = framebuffer::init(raw);
        log::init_framebuffer(raw);
        bootlog::ok_fmt(format_args!(
            "framebuffer {}x{} pitch {}",
            fb.width(),
            fb.height(),
            fb.pitch()
        ));
    } else {
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
    shell::run()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("panic: {}", info);
    crash::fatal(error::KernelError::panic())
}
