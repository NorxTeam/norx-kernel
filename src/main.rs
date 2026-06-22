#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod abi;
mod arch;
mod bootlog;
mod capability;
mod crash;
mod drivers;
mod error;
mod font;
mod framebuffer;
mod heap;
mod input;
mod irq;
mod log;
mod memory;
mod paging;
mod process;
mod sched;
mod shell;
mod time;
mod timer;
mod uefi;
mod vfs;
mod vm;

use core::panic::PanicInfo;

#[no_mangle]
pub extern "efiapi" fn efi_main(
    image: uefi::Handle,
    system_table: *mut uefi::SystemTable,
) -> uefi::Status {
    arch::init();
    log::init();
    bootlog::ok("arch serial initialized");
    time::init(system_table);
    if let Some(time) = time::boot_time() {
        bootlog::ok_fmt(format_args!(
            "uefi time {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            time.year, time.month, time.day, time.hour, time.minute, time.second
        ));
    } else {
        bootlog::fail("uefi time unavailable");
    }
    bootlog::spin(0, "probing built-in drivers");
    drivers::init();
    bootlog::ok("driver framework initialized");
    vfs::init();
    bootlog::ok("vfs initialized");
    bootlog::spin(1, "probing input devices");
    input::init(system_table);
    bootlog::ok("input subsystem initialized");
    bootlog::warn("usb hid input deferred; using early console input");

    match unsafe { uefi::gop_framebuffer(system_table) } {
        Some(raw) => {
            crash::init(raw);
            let fb = framebuffer::init(raw);
            log::init_framebuffer(raw);
            bootlog::ok_fmt(format_args!(
                "framebuffer {}x{} pitch {}",
                fb.width(),
                fb.height(),
                fb.pitch()
            ));
        }
        None => {
            bootlog::fail("framebuffer unavailable");
            error::report(error::KernelError::uefi("framebuffer unavailable"));
        }
    }

    match memory::exit_boot_services(image, system_table) {
        Some(summary) => {
            bootlog::ok_fmt(format_args!(
                "memory map {} entries desc {} v{}",
                summary.descriptors, summary.descriptor_size, summary.descriptor_version
            ));
            bootlog::ok_fmt(format_args!(
                "physical allocator {} KiB usable",
                summary.usable_pages * 4
            ));
            if let Some(frame) = memory::alloc_frame() {
                bootlog::ok_fmt(format_args!("first free frame 0x{:x}", frame));
            } else {
                bootlog::fail("physical allocator has no free frames");
            }
            match heap::init() {
                Some(heap) => {
                    if heap::alloc_bytes(64, 8).is_none() {
                        bootlog::fail("bootstrap heap self-check failed");
                    }
                    bootlog::ok_fmt(format_args!("bootstrap heap {} KiB", heap.bytes / 1024))
                }
                None => bootlog::fail("bootstrap heap init failed"),
            }
        }
        None => {
            bootlog::fail("ExitBootServices failed");
            crash::fatal(error::KernelError::uefi("ExitBootServices failed"));
        }
    }

    arch::tables::init();
    bootlog::ok("architecture tables initialized");
    arch::init_interrupt_controller();
    arch::init_syscalls();
    paging::init();
    if arch::prepare_user_mode() {
        bootlog::ok("user launch context prepared");
    } else {
        #[cfg(target_arch = "aarch64")]
        {
            let ctx = arch::user::context();
            if ctx.payload_ready {
                bootlog::ok_fmt(format_args!(
                    "user payload prepared code=0x{:x} stack=0x{:x} mapped={}",
                    ctx.code_frame, ctx.stack_frame, ctx.mapped
                ));
            }
        }
        bootlog::warn("user launch context unavailable");
    }
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
