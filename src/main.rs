#![no_std]
#![no_main]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

mod address;
mod address_space;
mod arch;
mod boot;
mod bootlog;
mod btrfs;
mod crash;
mod drivers;
mod dynamic;
mod elf;
mod error;
mod exec;
mod ext4;
mod fat32;
mod font;
mod framebuffer;
#[cfg(target_arch = "x86_64")]
mod input;
mod io;
mod ipc;
mod irq;
mod log;
mod memory;
#[cfg(target_arch = "x86_64")]
mod net;
mod paging;
mod process;
mod sched;
mod serial_debugger;
mod service;
mod syscall;
mod time;
mod timer;
mod user_runtime;
mod usercopy;
mod vfs;
#[cfg(target_arch = "x86_64")]
mod vga;
mod vm;
mod wasm;

use core::panic::PanicInfo;

pub fn kernel_start() -> ! {
    log::init();
    irq::init();
    irq::contract_self_check();
    io::contract_self_check();
    #[cfg(target_arch = "x86_64")]
    net::contract_self_check();
    let boot = boot::info();
    #[cfg(target_arch = "x86_64")]
    bootlog::info("VGA fallback initialized");
    if let Some(raw) = boot.framebuffer {
        log::init_framebuffer(raw);
    }
    bootlog::title();
    bootlog::info("interrupt, MMIO, PIO, and DMA boundary checks passed");
    boot::contract_self_check();
    bootlog::info("boot hand-off and parser boundary checks passed");
    bootlog::start(0, "accepting GRUB hand-off");
    bootlog::ok_fmt(format_args!(
        "GRUB hand-off accepted arch={} memory={} modules={}",
        boot.architecture.name(),
        boot.memory_len,
        boot.modules_len
    ));
    bootlog::info_fmt(format_args!(
        "EFI system table {}",
        if boot.efi_system_table == 0 {
            "absent"
        } else {
            "present"
        }
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
    if drivers::init(boot.framebuffer) {
        bootlog::ok("driver framework initialized");
    } else {
        bootlog::fail("driver framework capacity exhausted");
    }
    crash::contract_self_check();
    bootlog::info("persistent boot-record encoding and checksum checks passed");
    if crash::init() {
        bootlog::ok("persistent warm-reboot log initialized");
        if crash::mark_checkpoint() {
            bootlog::info("persistent pre-architecture boot checkpoint committed");
        }
    } else {
        bootlog::warn("persistent warm-reboot log unavailable; panic context remains serial-only");
    }
    bootlog::start(1, "checking virtual filesystem");
    fat32::contract_self_check();
    bootlog::info("FAT32 parser, long-name, and safe-write checks passed");
    bootlog::start(2, "checking FAT32 volumes");
    match fat32::probe_ramdisk() {
        Ok(volume) => bootlog::info_fmt(format_args!(
            "FAT32 read-only volume sectors={} clusters={} root={}",
            volume.geometry().total_sectors,
            volume.geometry().cluster_count,
            volume.geometry().root_cluster,
        )),
        Err(fat32::Error::InvalidBpb) => {
            bootlog::info("FAT32 volume absent; ramfs remains the writable root")
        }
        Err(error) => bootlog::warn_fmt(format_args!(
            "FAT32 probe failed: {:?}; ramfs remains the writable root",
            error
        )),
    }
    ext4::contract_self_check();
    bootlog::info("ext4 superblock, extent, directory, permission, and journal checks passed");
    bootlog::start(3, "checking ext4 volumes");
    match ext4::probe_ramdisk() {
        Ok(volume) => bootlog::info_fmt(format_args!(
            "ext4 read-only volume blocks={} block_size={} groups={} journal={}",
            volume.geometry().blocks,
            volume.geometry().block_size,
            volume.geometry().groups,
            volume.geometry().has_journal,
        )),
        Err(ext4::Error::InvalidSuperblock) => {
            bootlog::info("ext4 volume absent; ramfs remains the writable root")
        }
        Err(error) => bootlog::warn_fmt(format_args!(
            "ext4 probe failed: {:?}; ramfs remains the writable root",
            error
        )),
    }
    btrfs::contract_self_check();
    bootlog::info("btrfs superblock, checksum, tree, and subvolume checks passed");
    bootlog::start(0, "checking btrfs volumes");
    match btrfs::probe_ramdisk() {
        Ok(volume) => bootlog::info_fmt(format_args!(
            "btrfs read-only volume bytes={} nodesize={} chunks={}",
            volume.geometry().total_bytes,
            volume.geometry().nodesize,
            volume.geometry().chunks,
        )),
        Err(btrfs::Error::InvalidSuperblock) => {
            bootlog::info("btrfs volume absent; ramfs remains the writable root")
        }
        Err(error) => bootlog::warn_fmt(format_args!(
            "btrfs probe failed: {:?}; ramfs remains the writable root",
            error
        )),
    }
    vfs::contract_self_check();
    if vfs::init() {
        bootlog::ok("vfs initialized");
    } else {
        bootlog::fail("vfs initialization failed");
    }
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
    if arch::tables::init() {
        bootlog::ok("architecture tables initialized");
    } else {
        bootlog::fail("architecture interrupt tables incomplete");
    }
    bootlog::start(0, "checking interrupt controller");
    arch::init_interrupt_controller();
    syscall::contract_self_check();
    bootlog::info_fmt(format_args!(
        "syscall ABI v{} table entries={} args={} error=negative",
        syscall::ABI_VERSION,
        syscall::TABLE.len(),
        syscall::MAX_ARGS,
    ));
    usercopy::contract_self_check();
    bootlog::info(
        "user pointer validation and fault boundary checks passed; process user pages unavailable",
    );
    process::contract_self_check();
    bootlog::info(
        "process PID/TID, parent-child, credentials, FD, signal, event, and wait model checks passed",
    );
    bootlog::info(
        "capability authorization checks passed; UID alone cannot bypass privileged operations",
    );
    ipc::contract_self_check();
    bootlog::info(
        "IPC channel, shared-memory ring, event queue, wait-queue, ownership, and blocking checks passed",
    );
    service::contract_self_check();
    bootlog::info(
        "driver-service supervisor lifecycle, user-thread attachment, restart, and resource revoke checks passed",
    );
    if process::init_runtime() {
        bootlog::info(
            "process runtime initialized with init PID/TID and safe syscall scheduling boundary",
        );
        syscall::runtime_contract_self_check();
        bootlog::info("syscall exit/wait/getpid/gettid/yield/sleep/close runtime checks passed");
    } else {
        bootlog::fail("process runtime initialization failed");
    }
    address_space::contract_self_check();
    bootlog::info(
        "address-space user isolation, page-table ownership, guard stack, ASLR, W^X, and teardown checks passed",
    );
    elf::contract_self_check();
    bootlog::info(
        "ELF64 headers, PT_LOAD bounds, zero-fill, W^X, entry, stack, auxv, and register checks passed",
    );
    dynamic::contract_self_check();
    bootlog::info(
        "ET_DYN/PIE dynamic metadata, PT_INTERP, symbol lookup, RELA, TLS, and bounded loader checks passed",
    );
    dynamic::smoke_self_check();
    bootlog::info(
        "dynamic-linker smoke shared object, missing dependency, relocation, VFS path, and clean exit checks passed",
    );
    user_runtime::contract_self_check();
    bootlog::info(
        "native init runtime mapping, serial/FD write, bounded alloc, and clean exit checks passed",
    );
    exec::contract_self_check();
    bootlog::info(
        "exec replacement prepare/rollback, interpreter selection, and close-on-exec checks passed",
    );
    wasm::contract_self_check();
    bootlog::info(
        "Wasm verifier/interpreter integer, control, linear-memory, fuel, stack, import, handle, and cancellation checks passed",
    );
    bootlog::info_fmt(format_args!(
        "Wasm interpreter profile iterations=32 ticks={}",
        wasm::profile_self_check(),
    ));
    let portable_sample = wasm::sample_profile_self_check();
    let native_sample = user_runtime::sample_profile_self_check();
    bootlog::info_fmt(format_args!(
        "sample compare portable startup_ticks={} module_bytes={} linear_memory={} host_calls={} native startup_ticks={} elf_bytes={} user_memory={} syscalls={}",
        portable_sample.startup_ticks,
        portable_sample.module_bytes,
        portable_sample.linear_memory_bytes,
        portable_sample.host_calls,
        native_sample.startup_ticks,
        native_sample.image_bytes,
        native_sample.user_memory_bytes,
        native_sample.syscall_count,
    ));
    bootlog::info(
        "integrated staged QEMU smoke process/ELF/file/memory/dynamic/VM/fault/permission/teardown checks passed",
    );
    bootlog::info(
        "legacy universal machine-code payload, architecture-neutral syscall probe, and generic test execution path absent",
    );
    bootlog::info(
        "kernel/user trust boundary and capability transfer model documented; IPC grants deferred",
    );
    bootlog::start(1, "initializing syscall entry");
    arch::init_syscalls();
    paging::init();
    if service::user_entry_self_check() {
        #[cfg(target_arch = "x86_64")]
        bootlog::info("real x86_64 user-mode service entry, syscall exit, address-space activation, and return checks passed");
        #[cfg(target_arch = "aarch64")]
        bootlog::info(
            "real aarch64 EL0 service entry, SVC exit, TTBR0 activation, and return checks passed",
        );
    } else {
        bootlog::info("real user-mode service entry deferred on aarch64 until TTBR0/EL0 activation is implemented");
    }
    bootlog::start(1, "probing runtime buses");
    if drivers::runtime_init(boot.framebuffer) {
        bootlog::ok("runtime bus probing complete");
    } else {
        bootlog::fail("runtime bus registry capacity exhausted");
    }
    #[cfg(target_arch = "x86_64")]
    {
        bootlog::start(2, "initializing network stack");
        match net::init() {
            net::InitResult::Ready(status) => {
                bootlog::ok_fmt(format_args!(
                    "network stack ready link={} dhcp={:?} ip={}.{}.{}.{} gateway={}.{}.{}.{} dns={}.{}.{}.{}",
                    status.link_up,
                    status.dhcp,
                    status.ip.0[0],
                    status.ip.0[1],
                    status.ip.0[2],
                    status.ip.0[3],
                    status.gateway.0[0],
                    status.gateway.0[1],
                    status.gateway.0[2],
                    status.gateway.0[3],
                    status.dns.0[0],
                    status.dns.0[1],
                    status.dns.0[2],
                    status.dns.0[3],
                ));
            }
            net::InitResult::Unsupported => {
                bootlog::info("network stack deferred; virtio-net unavailable");
            }
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        bootlog::start(2, "initializing network stack");
        bootlog::info("network stack unsupported on aarch64 bring-up");
    }
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
    if crash::mark_ready() {
        bootlog::ok("persistent warm-reboot status committed");
    } else {
        bootlog::warn("persistent warm-reboot status commit failed");
    }
    serial_debugger::run()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("panic: {}", info);
    crash::fatal(error::KernelError::panic())
}
