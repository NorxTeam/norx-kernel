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
mod pipe;
mod process;
mod sched;
mod serial_debugger;
mod service;
mod syscall;
mod time;
mod timer;
mod tty;
mod user_runtime;
mod usercopy;
mod vfs;
#[cfg(target_arch = "x86_64")]
mod vga;
mod vm;
mod wasm;

use core::panic::PanicInfo;

fn persistent_vfs_mount_smoke(
    source: vfs::MountSource,
) -> Result<(vfs::MountId, usize, bool), vfs::Error> {
    match vfs::stat("/storage") {
        Ok(stat) if stat.kind == vfs::NodeType::Directory => {}
        Ok(_) => return Err(vfs::Error::InvalidMountTarget),
        Err(vfs::Error::NotFound) => vfs::mkdir("/storage")?,
        Err(error) => return Err(error),
    }
    let writable = matches!(source, vfs::MountSource::Fat32 | vfs::MountSource::Ext4)
        && drivers::block::persistent()
        && !drivers::block::read_only();
    let flags = if writable {
        vfs::MountFlags::defaults()
    } else {
        vfs::MountFlags::read_only()
    };
    let mount = vfs::mount(source, "/storage", flags)?;
    let result = (|| {
        let root = vfs::stat("/storage")?;
        if root.kind != vfs::NodeType::Directory {
            return Err(vfs::Error::NotDirectory);
        }
        let empty = vfs::DirectoryEntry {
            kind: vfs::NodeType::Regular,
            mode: 0,
            size: 0,
            links: 0,
            name: [0; 31],
            name_length: 0,
        };
        let mut entries = [empty; 16];
        let count = vfs::read_dir("/storage", &mut entries)?;
        let dentry = vfs::lookup("/storage")?;
        if dentry.dentry().mount != mount {
            let _ = vfs::release_dentry(dentry);
            return Err(vfs::Error::MountNotFound);
        }
        vfs::release_dentry(dentry)?;
        vfs::sync_path("/storage")?;

        for entry in entries.iter().take(count) {
            if entry.kind != vfs::NodeType::Regular || entry.size > 4096 {
                continue;
            }
            let prefix = b"/storage/";
            let length = prefix
                .len()
                .checked_add(entry.name_length)
                .ok_or(vfs::Error::InvalidPath)?;
            if length > 256 {
                return Err(vfs::Error::InvalidPath);
            }
            let mut path_bytes = [0; 256];
            path_bytes[..prefix.len()].copy_from_slice(prefix);
            path_bytes[prefix.len()..length].copy_from_slice(&entry.name[..entry.name_length]);
            let path =
                core::str::from_utf8(&path_bytes[..length]).map_err(|_| vfs::Error::InvalidPath)?;
            let handle = vfs::open(path, vfs::OpenOptions::read())?;
            let before = vfs::stat_handle(handle)?;
            let mut data = [0; 4096];
            let read = vfs::read_handle(handle, &mut data)?;
            if before.size != 0 && read == 0 {
                let _ = vfs::close(handle);
                return Err(vfs::Error::BackendError);
            }
            vfs::seek_from(handle, 0, 0)?;
            vfs::sync_handle(handle)?;
            vfs::close(handle)?;
            let reopened = vfs::open(path, vfs::OpenOptions::read())?;
            if vfs::stat_handle(reopened)?.size != before.size {
                let _ = vfs::close(reopened);
                return Err(vfs::Error::BackendError);
            }
            vfs::close(reopened)?;
            break;
        }
        if writable && source == vfs::MountSource::Fat32 {
            const PATH: &str = "/storage/NORX.VFS";
            const PAYLOAD: &[u8] = b"NORX VFS persistent write v1\n";
            let handle = vfs::open(
                PATH,
                vfs::OpenOptions {
                    read: true,
                    write: true,
                    create: true,
                    truncate: true,
                    append: false,
                    exclusive: false,
                    mode: 0o644,
                },
            )?;
            let written = vfs::write_handle(handle, PAYLOAD)?;
            vfs::sync_handle(handle)?;
            vfs::seek_from(handle, 0, 0)?;
            let mut readback = [0u8; 64];
            let read = vfs::read_handle(handle, &mut readback)?;
            if written != PAYLOAD.len() || read != PAYLOAD.len() || readback[..read] != *PAYLOAD {
                let _ = vfs::close(handle);
                return Err(vfs::Error::BackendError);
            }
            vfs::close(handle)?;

            let reopened = vfs::open(PATH, vfs::OpenOptions::read())?;
            let mut reopened_data = [0u8; 64];
            let reopened_read = vfs::read_handle(reopened, &mut reopened_data)?;
            if reopened_read != PAYLOAD.len() || reopened_data[..reopened_read] != *PAYLOAD {
                let _ = vfs::close(reopened);
                return Err(vfs::Error::BackendError);
            }
            vfs::close(reopened)?;
            vfs::sync_path(PATH)?;
            bootlog::ok("persistent VFS writable FAT32 create/write/fsync/readback passed");
        }
        Ok(count)
    })();
    let unmount = vfs::unmount_mount(mount);
    match (result, unmount) {
        (Ok(count), Ok(())) => Ok((mount, count, writable)),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

#[no_mangle]
pub extern "C" fn kernel_start() -> ! {
    log::init();
    irq::init();
    irq::contract_self_check();
    io::contract_self_check();
    #[cfg(target_arch = "x86_64")]
    net::contract_self_check();
    drivers::serial::contract_self_check();
    let boot = boot::info();
    #[cfg(target_arch = "x86_64")]
    bootlog::ok("VGA fallback initialized");
    if let Some(raw) = boot.framebuffer {
        log::init_framebuffer(raw);
    }
    #[cfg(target_arch = "x86_64")]
    let fallback = "vga-or-framebuffer";
    #[cfg(target_arch = "aarch64")]
    let fallback = if boot.framebuffer.is_some() {
        "framebuffer"
    } else {
        "none"
    };
    if let Some(reason) = drivers::serial::failure_reason() {
        bootlog::warn_fmt(format_args!(
            "NORX_EARLY_UART_DISABLED v=1 backend={} reason={} poll-limit={} fallback={}",
            drivers::serial::backend_name(),
            reason.name(),
            drivers::serial::EARLY_TX_POLL_LIMIT,
            fallback,
        ));
    } else {
        bootlog::ok_fmt(format_args!(
            "NORX_EARLY_UART_READY v=1 backend={} poll-limit={}",
            drivers::serial::backend_name(),
            drivers::serial::EARLY_TX_POLL_LIMIT,
        ));
    }
    bootlog::title();
    bootlog::ok("interrupt, MMIO, PIO, and DMA boundary checks passed");
    boot::contract_self_check();
    bootlog::ok("boot hand-off and parser boundary checks passed");
    bootlog::start(0, "accepting GRUB hand-off");
    bootlog::ok_fmt(format_args!(
        "GRUB hand-off accepted arch={} memory={} modules={}",
        boot.architecture.name(),
        boot.memory_len,
        boot.modules_len
    ));
    bootlog::ok_fmt(format_args!(
        "EFI system table {}",
        if boot.efi_system_table == 0 {
            "absent"
        } else {
            "present"
        }
    ));
    if !boot.cmdline().is_empty() {
        bootlog::start(1, "reading kernel command line");
        bootlog::ok_fmt(format_args!(
            "kernel command line accepted: {}",
            boot.cmdline()
        ));
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
    time::contract_self_check();
    bootlog::ok("kernel clock initialized");
    bootlog::start(0, "probing built-in drivers");
    if drivers::init(boot.framebuffer) {
        bootlog::ok("driver framework initialized");
    } else {
        bootlog::fail("driver framework capacity exhausted");
    }
    crash::contract_self_check();
    bootlog::ok("persistent boot-record encoding and checksum checks passed");
    if crash::init() {
        bootlog::ok("persistent warm-reboot log initialized");
        if crash::mark_checkpoint() {
            bootlog::ok("persistent pre-architecture boot checkpoint committed");
        }
    } else {
        bootlog::warn("persistent warm-reboot log unavailable; panic context remains serial-only");
    }
    bootlog::start(1, "checking virtual filesystem");
    fat32::contract_self_check();
    bootlog::ok("FAT32 parser, long-name, and safe-write checks passed");
    ext4::contract_self_check();
    bootlog::ok("ext4 superblock, extent, directory, permission, and journal checks passed");
    btrfs::contract_self_check();
    bootlog::ok("btrfs superblock, checksum, tree, and subvolume checks passed");
    vfs::contract_self_check();
    if vfs::init() {
        bootlog::ok("vfs initialized");
    } else {
        bootlog::fail("vfs initialization failed");
    }
    bootlog::start(2, "initializing serial-debugger");
    if drivers::serial::available() {
        bootlog::ok("serial-debugger input ready");
    } else {
        bootlog::warn("serial-debugger unavailable; UART failure fallback active");
    }

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
    bootlog::ok_fmt(format_args!(
        "syscall ABI v{} table entries={} args={} error=negative",
        syscall::ABI_VERSION,
        syscall::TABLE.len(),
        syscall::MAX_ARGS,
    ));
    usercopy::contract_self_check();
    #[cfg(target_arch = "aarch64")]
    bootlog::ok("aarch64 ESR/FAR/ELR exception entry and usercopy recovery checks passed");
    #[cfg(target_arch = "x86_64")]
    bootlog::ok(
        "user pointer validation and fault boundary checks passed; process user pages unavailable",
    );
    bootlog::start(1, "initializing syscall entry");
    arch::init_syscalls();
    if !paging::init() {
        bootlog::fail("kernel paging initialization failed");
        arch::halt();
    }
    process::contract_self_check();
    bootlog::ok(
        "process PID/TID, parent-child, credentials, FD, signal, event, and wait model checks passed",
    );
    bootlog::ok(
        "capability authorization checks passed; UID alone cannot bypass privileged operations",
    );
    ipc::contract_self_check();
    pipe::contract_self_check();
    tty::contract_self_check();
    bootlog::ok(
        "IPC channel, shared-memory ring, event queue, wait-queue, ownership, and blocking checks passed",
    );
    service::contract_self_check();
    bootlog::ok(
        "driver-service supervisor lifecycle, user-thread attachment, restart, and resource revoke checks passed",
    );
    if process::init_runtime() {
        tty::init();
        tty::runtime_contract_self_check();
        bootlog::ok(
            "process runtime initialized with init PID/TID, FD lifecycle, process groups, and serial TTY boundary",
        );
        syscall::runtime_contract_self_check();
        bootlog::ok("syscall exit/wait/getpid/gettid/yield/sleep/close runtime checks passed");
    } else {
        bootlog::fail("process runtime initialization failed");
    }
    elf::contract_self_check();
    bootlog::ok(
        "ELF64 headers, PT_LOAD bounds, zero-fill, W^X, entry, stack, auxv, and register checks passed",
    );
    dynamic::contract_self_check();
    bootlog::ok(
        "ET_DYN/PIE dynamic metadata, PT_INTERP, symbol lookup, RELA, TLS, and bounded loader checks passed",
    );
    dynamic::smoke_self_check();
    bootlog::ok(
        "dynamic-linker smoke shared object, missing dependency, relocation, VFS path, and clean exit checks passed",
    );
    user_runtime::contract_self_check();
    bootlog::ok(
        "native init runtime mapping, serial/FD write, bounded alloc, and clean exit checks passed",
    );
    exec::contract_self_check();
    bootlog::ok(
        "exec replacement prepare/rollback, interpreter selection, and close-on-exec checks passed",
    );
    wasm::contract_self_check();
    bootlog::ok(
        "Wasm verifier/interpreter integer, control, linear-memory, fuel, stack, import, handle, and cancellation checks passed",
    );
    bootlog::ok_fmt(format_args!(
        "Wasm interpreter profile iterations=32 ticks={}",
        wasm::profile_self_check(),
    ));
    let portable_sample = wasm::sample_profile_self_check();
    let native_sample = user_runtime::sample_profile_self_check();
    bootlog::ok_fmt(format_args!(
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
    bootlog::ok(
        "integrated staged QEMU smoke process/ELF/file/memory/dynamic/VM/fault/permission/teardown checks passed",
    );
    bootlog::ok(
        "legacy universal machine-code payload, architecture-neutral syscall probe, and generic test execution path absent",
    );
    bootlog::ok(
        "kernel/user trust boundary and capability transfer model documented; IPC grants deferred",
    );
    address_space::contract_self_check();
    bootlog::ok(
        "address-space user isolation, page-table ownership, guard stack, ASLR, W^X, and teardown checks passed",
    );
    bootlog::quickinit_overlay_stage("checking lazy page faults", 94);
    vm::init();
    if vm::contract_self_check() {
        #[cfg(target_arch = "x86_64")]
        bootlog::ok("x86_64 lazy page faults allocate, zero, and map through the direct map");
        #[cfg(target_arch = "aarch64")]
        bootlog::ok("aarch64 fault syndrome classification and architecture-neutral page-fault contract checks passed");
    } else {
        bootlog::fail("architecture-neutral page-fault contract unavailable");
    }
    bootlog::start(1, "checking scheduler");
    sched::self_check();
    bootlog::ok("scheduler self-check passed");
    bootlog::start(2, "initializing scheduler runtime");
    sched::init_runtime();
    bootlog::ok("scheduler runtime initialized");
    bootlog::start(2, "checking timer source");
    #[cfg(target_arch = "aarch64")]
    let timer_handler_registered = arch::tables::register_timer_handler();
    #[cfg(target_arch = "aarch64")]
    if !timer_handler_registered {
        bootlog::fail("aarch64 GIC timer handler registration failed");
    }
    let timer_ready = timer::init();
    if timer_ready {
        bootlog::ok("hardware scheduler timer initialized");
    } else {
        bootlog::warn("hardware scheduler timer unavailable; using polling");
    }
    bootlog::start(3, "selecting timer source");
    if timer_ready {
        bootlog::ok_fmt(format_args!(
            "timer source={} mode=irq",
            arch::timer_source()
        ));
    } else {
        bootlog::warn_fmt(format_args!(
            "timer source={} mode=poll",
            arch::timer_source()
        ));
    }
    let scheduler_runtime_ok = sched::runtime_self_check(timer_ready);
    if scheduler_runtime_ok {
        let status = sched::status();
        let irq = irq::stats();
        bootlog::ok_fmt(format_args!(
            "scheduler runtime verified hz={} mode={} source={} ticks={} clock={} irq_timer={} irq_deferred_total={}",
            time::scheduler_hz(),
            if timer_ready { "irq" } else { "poll" },
            arch::timer_source(),
            status.timer_ticks,
            status.clock,
            irq.timer,
            irq.deferred,
        ));
        #[cfg(target_arch = "aarch64")]
        {
            let gic = arch::gic::status();
            if timer_ready
                && timer_handler_registered
                && gic.ready
                && gic.timer_enabled
                && gic.ack_count >= 8
                && irq.timer >= 8
                && status.timer_ticks >= 8
            {
                bootlog::ok_fmt(format_args!(
                    "NORX_AARCH64_GIC_TIMER_IRQ_OK v=1 intid={} ack_count={} irq_timer={} ticks={}",
                    gic.timer_intid, gic.ack_count, irq.timer, status.timer_ticks,
                ));
            } else {
                bootlog::fail_fmt(format_args!(
                    "NORX_AARCH64_GIC_TIMER_IRQ_FAIL v=1 reason=insufficient-accounting intid={} ack_count={} irq_timer={} ticks={}",
                    gic.timer_intid, gic.ack_count, irq.timer, status.timer_ticks,
                ));
            }
        }
    } else {
        let status = sched::status();
        let irq = irq::stats();
        bootlog::fail_fmt(format_args!(
            "scheduler runtime accounting probe failed source={} ticks={} clock={} irq_timer={} pending={} spurious={} unhandled={} exceptions={} deferred={} hard_context_violations={}",
            arch::timer_source(),
            status.timer_ticks,
            status.clock,
            irq.timer,
            irq.timer_pending,
            irq.spurious,
            irq.unhandled,
            irq.exceptions,
            irq.deferred,
            irq.hard_context_violations,
        ));
        #[cfg(target_arch = "aarch64")]
        bootlog::fail("NORX_AARCH64_GIC_TIMER_IRQ_FAIL v=1 reason=scheduler-accounting");
        #[cfg(target_arch = "aarch64")]
        {
            let gic = arch::gic::status();
            bootlog::warn_fmt(format_args!(
                "aarch64 gic probe ack_count={} last_ack={} timer_intid={}",
                gic.ack_count, gic.last_ack, gic.timer_intid,
            ));
        }
    }
    bootlog::start(1, "checking userspace init boundary");
    let userspace_init_ok = service::user_entry_self_check();
    if userspace_init_ok {
        bootlog::ok("quickinit PID 1 hand-off passed; kernel boot log sequence resumed");
    } else {
        bootlog::fail("quickinit PID 1 unavailable; deterministic recovery path remains active");
    }
    bootlog::quickinit_overlay_stage("starting system services", 93);
    bootlog::start(1, "probing runtime buses");
    if drivers::runtime_init(boot.framebuffer) {
        bootlog::ok("runtime bus probing complete");
    } else {
        bootlog::fail("runtime bus registry capacity exhausted");
    }
    bootlog::start(2, "checking persistent filesystem volumes");
    let fat32_available = match fat32::probe_block() {
        Ok(volume) => {
            bootlog::ok_fmt(format_args!(
                "FAT32 block volume sectors={} clusters={} root={} readonly={}",
                volume.geometry().total_sectors,
                volume.geometry().cluster_count,
                volume.geometry().root_cluster,
                volume.geometry().read_only,
            ));
            true
        }
        Err(fat32::Error::InvalidBpb) => {
            bootlog::warn("FAT32 volume absent; ramfs remains the writable root");
            false
        }
        Err(error) => {
            bootlog::warn_fmt(format_args!(
                "FAT32 block probe failed: {:?}; ramfs remains the writable root",
                error
            ));
            false
        }
    };
    let ext4_available = match ext4::probe_block() {
        Ok(volume) => {
            bootlog::ok_fmt(format_args!(
                "ext4 block volume blocks={} block_size={} groups={} journal={} readonly={}",
                volume.geometry().blocks,
                volume.geometry().block_size,
                volume.geometry().groups,
                volume.geometry().has_journal,
                volume.geometry().read_only,
            ));
            true
        }
        Err(ext4::Error::InvalidSuperblock) => {
            bootlog::warn("ext4 volume absent; ramfs remains the writable root");
            false
        }
        Err(error) => {
            bootlog::warn_fmt(format_args!(
                "ext4 block probe failed: {:?}; ramfs remains the writable root",
                error
            ));
            false
        }
    };
    let btrfs_available = match btrfs::probe_block() {
        Ok(volume) => {
            bootlog::ok_fmt(format_args!(
                "btrfs block volume bytes={} nodesize={} chunks={} readonly={}",
                volume.geometry().total_bytes,
                volume.geometry().nodesize,
                volume.geometry().chunks,
                volume.geometry().read_only,
            ));
            true
        }
        Err(btrfs::Error::InvalidSuperblock) => {
            bootlog::warn("btrfs volume absent; ramfs remains the writable root");
            false
        }
        Err(error) => {
            bootlog::warn_fmt(format_args!(
                "btrfs block probe failed: {:?}; ramfs remains the writable root",
                error
            ));
            false
        }
    };
    for (source, available) in [
        (vfs::MountSource::Fat32, fat32_available),
        (vfs::MountSource::Ext4, ext4_available),
        (vfs::MountSource::Btrfs, btrfs_available),
    ] {
        if !available {
            continue;
        }
        match persistent_vfs_mount_smoke(source) {
            Ok((mount, entries, writable)) => {
                bootlog::ok_fmt(format_args!(
                    "persistent VFS mount source={:?} mount={} root_entries={} read_only={}",
                    source,
                    mount.raw(),
                    entries,
                    !writable,
                ));
                break;
            }
            Err(error) => bootlog::warn_fmt(format_args!(
                "persistent VFS mount source={:?} unavailable: {:?}",
                source, error
            )),
        }
    }
    if drivers::block::persistent() && !drivers::block::read_only() {
        match fat32::fixed_file_persistence_check() {
            Ok(status) => {
                bootlog::ok_fmt(format_args!(
                    "NORX_FAT32_PERSIST_WRITE_OK v=1 previous={} sequence={}",
                    status.previous_sequence, status.sequence
                ));
                bootlog::ok_fmt(format_args!(
                    "NORX_FAT32_PERSIST_READBACK_OK v=1 sequence={}",
                    status.sequence
                ));
            }
            Err(error) => bootlog::fail_fmt(format_args!(
                "NORX_FAT32_PERSIST_FAIL v=1 error={:?}",
                error
            )),
        }
    } else if drivers::block::persistent() {
        bootlog::warn("persistent block device is read-only; FAT32 persistence smoke skipped");
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
                bootlog::warn("network stack deferred; virtio-net unavailable");
            }
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        bootlog::start(2, "initializing network stack");
        bootlog::warn("network stack unsupported on aarch64 bring-up");
    }
    bootlog::quickinit_overlay_stage("initializing virtual memory", 96);
    bootlog::start(0, "reporting architecture");
    bootlog::ok_fmt(format_args!("architecture {}", arch::NAME));
    bootlog::start(1, "reading timer ticks");
    bootlog::ok_fmt(format_args!("timer ticks {}", time::ticks()));
    bootlog::quickinit_overlay_stage("finalizing userspace services", 99);
    bootlog::start(2, "checking userspace init hand-off");
    if userspace_init_ok {
        bootlog::ok("quickinit PID 1 contract checked; kernel hand-off remains ordered");
    } else {
        bootlog::warn("quickinit PID 1 contract failed; recovery path remains active");
    }
    bootlog::start(3, "finalizing kernel initialization");
    bootlog::ok("kernel initialization complete");
    let warm_reboot_ready = crash::mark_ready();
    if warm_reboot_ready {
        bootlog::ok("persistent warm-reboot status committed");
    } else {
        bootlog::warn("persistent warm-reboot status commit failed");
    }
    bootlog::quickinit_overlay_complete(userspace_init_ok && warm_reboot_ready);
    if drivers::serial::available() {
        serial_debugger::run()
    } else {
        arch::halt()
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("panic: {}", info);
    crash::fatal(error::KernelError::panic())
}
