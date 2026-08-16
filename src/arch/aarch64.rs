use core::arch::asm;

pub const NAME: &str = "aarch64";

pub mod gic;
pub mod paging;
pub mod syscall;
pub mod tables;

pub fn start_kernel() -> ! {
    unsafe { tables::norx_aarch64_kernel_start_entry() }
}

const EARLY_SERIAL_BASE: usize = 0x0900_0000;

#[repr(C, align(4096))]
struct FirmwareRuntimeContext {
    firmware_sp_el0: u64,
    firmware_vbar: u64,
    firmware_el: u8,
    firmware_ttbr0: u64,
    runtime_vbar: u64,
    runtime_sp_el0: u64,
    runtime_ttbr0: u64,
    runtime_active: bool,
}

static mut FIRMWARE_CONTEXT: FirmwareRuntimeContext = FirmwareRuntimeContext {
    firmware_sp_el0: 0,
    firmware_vbar: 0,
    firmware_el: 0,
    firmware_ttbr0: 0,
    runtime_vbar: 0,
    runtime_sp_el0: 0,
    runtime_ttbr0: 0,
    runtime_active: false,
};

pub fn init() {
    let current_el: u8;
    let firmware_sp_el0: u64;
    unsafe {
        let value: u64;
        asm!(
            "mrs {}, CurrentEL",
            out(reg) value,
            options(nomem, nostack, preserves_flags)
        );
        current_el = ((value >> 2) & 3) as u8;
        asm!(
            "mrs {}, sp_el0",
            out(reg) firmware_sp_el0,
            options(nomem, nostack, preserves_flags)
        );
        (*core::ptr::addr_of_mut!(FIRMWARE_CONTEXT)).firmware_sp_el0 = firmware_sp_el0;
        (*core::ptr::addr_of_mut!(FIRMWARE_CONTEXT)).firmware_vbar = read_vbar(current_el);
        (*core::ptr::addr_of_mut!(FIRMWARE_CONTEXT)).firmware_el = current_el;
        (*core::ptr::addr_of_mut!(FIRMWARE_CONTEXT)).firmware_ttbr0 = paging::current_ttbr0_value();
        asm!(
            "msr daifset, #0xf",
            options(nomem, nostack, preserves_flags)
        )
    };
    let initialized = crate::drivers::serial::pl011::init(EARLY_SERIAL_BASE);
    if !initialized {
        crate::drivers::serial::mark_failed(crate::drivers::serial::FailureReason::Init);
    } else if !crate::drivers::serial::pl011::tx_ready(EARLY_SERIAL_BASE) {
        crate::drivers::serial::mark_failed(crate::drivers::serial::FailureReason::TxTimeout);
    }
}

pub fn early_serial_read() -> Option<u8> {
    crate::drivers::serial::pl011::read(EARLY_SERIAL_BASE)
}

pub fn early_serial_write(byte: u8) -> bool {
    crate::drivers::serial::pl011::write(EARLY_SERIAL_BASE, byte)
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

pub fn prepare_firmware_runtime() -> bool {
    let boot_context = crate::boot::efi_runtime_context();
    let (firmware_el, firmware_vbar, firmware_sp_el0, firmware_ttbr0) = unsafe {
        let context = &*core::ptr::addr_of!(FIRMWARE_CONTEXT);
        (
            if boot_context.1 != 0 {
                boot_context.0
            } else {
                context.firmware_el
            },
            if boot_context.1 != 0 {
                boot_context.1
            } else {
                context.firmware_vbar
            },
            if boot_context.1 != 0 {
                boot_context.2
            } else {
                context.firmware_sp_el0
            },
            context.firmware_ttbr0,
        )
    };
    if firmware_el == 0 || firmware_vbar == 0 {
        crate::bootlog::warn_fmt(format_args!(
            "EFI runtime context unavailable el={} vbar=0x{:x} sp_el0=0x{:x}",
            firmware_el, firmware_vbar, firmware_sp_el0,
        ));
        return false;
    }
    let current_vbar = read_vbar(firmware_el);
    let current_sp_el0 = read_sp_el0();
    let current_ttbr0 = paging::current_ttbr0_value();
    unsafe {
        let context = &mut *core::ptr::addr_of_mut!(FIRMWARE_CONTEXT);
        context.runtime_vbar = current_vbar;
        context.runtime_sp_el0 = current_sp_el0;
        context.runtime_ttbr0 = current_ttbr0;
        context.runtime_active = true;
    }
    if firmware_ttbr0 != 0 && current_ttbr0 != 0 && current_ttbr0 != firmware_ttbr0 {
        paging::switch_ttbr0(firmware_ttbr0);
    }
    write_vbar(firmware_el, firmware_vbar);
    write_sp_el0(firmware_sp_el0);
    true
}

pub fn finish_firmware_runtime() {
    let (active, el, vbar, sp_el0) = unsafe {
        let context = &*core::ptr::addr_of!(FIRMWARE_CONTEXT);
        (
            context.runtime_active,
            context.firmware_el,
            context.runtime_vbar,
            context.runtime_sp_el0,
        )
    };
    let runtime_ttbr0 = unsafe { (*core::ptr::addr_of!(FIRMWARE_CONTEXT)).runtime_ttbr0 };
    if !active {
        return;
    }
    if runtime_ttbr0 != 0 {
        paging::switch_ttbr0(runtime_ttbr0);
    }
    write_vbar(el, vbar);
    write_sp_el0(sp_el0);
    unsafe {
        let context = &mut *core::ptr::addr_of_mut!(FIRMWARE_CONTEXT);
        context.runtime_ttbr0 = 0;
        context.runtime_active = false;
    }
}

pub fn firmware_runtime_context() -> (u8, u64, u64) {
    let boot_context = crate::boot::efi_runtime_context();
    if boot_context.1 != 0 {
        return boot_context;
    }
    unsafe {
        let context = &*core::ptr::addr_of!(FIRMWARE_CONTEXT);
        (
            context.firmware_el,
            context.firmware_vbar,
            context.firmware_sp_el0,
        )
    }
}

fn read_vbar(current_el: u8) -> u64 {
    let value: u64;
    unsafe {
        match current_el {
            1 => asm!("mrs {}, vbar_el1", out(reg) value, options(nomem, nostack, preserves_flags)),
            2 => asm!("mrs {}, vbar_el2", out(reg) value, options(nomem, nostack, preserves_flags)),
            3 => asm!("mrs {}, vbar_el3", out(reg) value, options(nomem, nostack, preserves_flags)),
            _ => return 0,
        }
    }
    value
}

fn write_vbar(current_el: u8, value: u64) {
    unsafe {
        match current_el {
            1 => asm!("msr vbar_el1, {}", in(reg) value, options(nomem, nostack, preserves_flags)),
            2 => asm!("msr vbar_el2, {}", in(reg) value, options(nomem, nostack, preserves_flags)),
            3 => asm!("msr vbar_el3, {}", in(reg) value, options(nomem, nostack, preserves_flags)),
            _ => return,
        }
        asm!("isb", options(nomem, nostack, preserves_flags));
    }
}

fn read_sp_el0() -> u64 {
    let value: u64;
    unsafe {
        asm!(
            "mrs {}, sp_el0",
            out(reg) value,
            options(nomem, nostack, preserves_flags)
        );
    }
    value
}

fn write_sp_el0(value: u64) {
    unsafe {
        asm!(
            "msr sp_el0, {}",
            in(reg) value,
            options(nomem, nostack, preserves_flags)
        );
        asm!("isb", options(nomem, nostack, preserves_flags));
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

pub fn timer_source() -> &'static str {
    if gic::timer_enabled() {
        "gic"
    } else {
        "poll"
    }
}

pub fn cpu_id() -> usize {
    let mpidr: u64;
    unsafe {
        asm!(
            "mrs {}, MPIDR_EL1",
            out(reg) mpidr,
            options(nomem, nostack, preserves_flags)
        );
    }
    (mpidr & 0x00ff_ffff) as usize
}

pub fn init_timer_interrupts() -> bool {
    if !gic::status().ready || timer_frequency_hz().is_none() || !gic::enable_timer() {
        return false;
    }
    let frequency = timer_frequency_hz().unwrap_or(0);
    let interval = (frequency / crate::time::scheduler_hz()).max(1);
    unsafe {
        asm!(
            "msr cntv_ctl_el0, {disabled}",
            "msr cntv_tval_el0, {interval}",
            "msr cntv_ctl_el0, {enabled}",
            "isb",
            disabled = in(reg) 0u64,
            interval = in(reg) interval,
            enabled = in(reg) 1u64,
            options(nomem, nostack, preserves_flags)
        );
        asm!(
            "msr daifclr, #2",
            "isb",
            options(nomem, nostack, preserves_flags)
        );
    }
    crate::bootlog::ok_fmt(format_args!(
        "aarch64 generic timer IRQ enabled source=gic intid={} frequency_hz={} interval={}",
        gic::status().timer_intid,
        frequency,
        interval,
    ));
    true
}

pub fn rearm_timer() {
    let frequency = timer_frequency_hz().unwrap_or(0);
    let interval = (frequency / crate::time::scheduler_hz()).max(1);
    unsafe {
        asm!(
            "msr cntv_tval_el0, {}",
            in(reg) interval,
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub fn init_interrupt_controller() {
    gic::contract_self_check();
    if let Some(status) = gic::init(crate::boot::info().interrupt_info) {
        crate::bootlog::ok_fmt(format_args!(
            "aarch64 gic{} discovered distributor=0x{:x} cpu=0x{:x} redistributor=0x{:x} timer_intid={} flags=0x{:x}",
            status.version,
            status.distributor,
            status.cpu_interface,
            status.redistributor,
            status.timer_intid,
            crate::boot::info().interrupt_info.timer_flags,
        ));
    } else {
        crate::bootlog::warn(
            "aarch64 GIC discovery/configuration unavailable; using counter polling",
        );
    }
}

pub fn handle_irq() {
    if let Some(intid) = gic::acknowledge() {
        let _ = crate::irq::dispatch(intid);
        if intid == gic::status().timer_intid {
            rearm_timer();
        }
        gic::end_of_interrupt(intid);
    } else {
        crate::irq::spurious();
    }
}

pub fn init_syscalls() -> bool {
    crate::bootlog::ok_fmt(format_args!(
        "aarch64 current exception level EL{}",
        tables::current_el()
    ));
    let ready = syscall::init();
    if ready {
        crate::bootlog::ok("aarch64 SVC syscall entry initialized");
    } else {
        crate::bootlog::fail("aarch64 SVC syscall entry unavailable");
    }
    ready
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

pub fn physical_to_virtual(address: crate::address::PhysAddr) -> Option<crate::address::VirtAddr> {
    paging::physical_to_virtual(address)
}

pub fn virtual_to_physical(address: crate::address::VirtAddr) -> Option<crate::address::PhysAddr> {
    paging::virtual_to_physical(address)
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

pub fn user_space_unmap(
    root: crate::address::PhysAddr,
    virtual_address: usize,
    tables: &mut [Option<crate::address::PhysAddr>],
) -> bool {
    paging::user_space_unmap(root, virtual_address, tables)
}

pub fn user_space_reset(root: crate::address::PhysAddr) {
    paging::user_space_reset(root);
}

pub fn switch_to_user(root: crate::address::PhysAddr) -> bool {
    paging::switch_to_user(root)
}

pub fn user_space_is_current(root: crate::address::PhysAddr) -> bool {
    paging::user_space_is_current(root)
}

pub fn restore_kernel_address_space() {
    paging::restore_kernel_address_space();
}

pub fn write_physical(physical: crate::address::PhysAddr, offset: usize, bytes: &[u8]) -> bool {
    paging::write_physical(physical, offset, bytes)
}

pub fn read_physical(physical: crate::address::PhysAddr, offset: usize, bytes: &mut [u8]) -> bool {
    paging::read_physical(physical, offset, bytes)
}

pub fn zero_physical_page(physical: crate::address::PhysAddr) -> bool {
    paging::zero_physical_page(physical)
}

pub fn enter_user(registers: crate::elf::InitialRegisters) -> bool {
    syscall::enter_user(registers)
}

pub fn install_user_context(thread: u32, registers: crate::elf::InitialRegisters) -> bool {
    syscall::install_user_context(thread, registers)
}

pub fn request_user_switch(from: u32, to: u32) {
    syscall::request_user_switch(from, to)
}

pub fn has_user_context(thread: u32) -> bool {
    syscall::has_user_context(thread)
}
