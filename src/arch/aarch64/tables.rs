use core::arch::{asm, global_asm};

#[repr(C, align(16))]
struct KernelStack([u8; 512 * 1024]);

#[no_mangle]
static mut norx_aarch64_kernel_stack: KernelStack = KernelStack([0; 512 * 1024]);

global_asm!(
    r#"
    .align 11
    .global norx_aarch64_kernel_start_entry
norx_aarch64_kernel_start_entry:
    mov x17, #1
    msr spsel, x17
    adrp x17, norx_aarch64_kernel_stack
    add x17, x17, :lo12:norx_aarch64_kernel_stack
    mov x18, #8
    lsl x18, x18, #16
    add x17, x17, x18
    mov sp, x17
    b kernel_start

    .align 11
    .global norx_exception_vectors
norx_exception_vectors:
    b norx_aarch64_sync_exception
    .space 124
    b norx_aarch64_irq_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_sync_exception
    .space 124
    b norx_aarch64_irq_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_sync_exception
    .space 124
    b norx_aarch64_irq_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_irq_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124

    .global norx_aarch64_exception_entry
norx_aarch64_exception_entry:
    mov x17, #1
    msr spsel, x17
    adrp x17, norx_aarch64_kernel_stack
    add x17, x17, :lo12:norx_aarch64_kernel_stack
    mov x18, #8
    lsl x18, x18, #16
    add x17, x17, x18
    mov sp, x17
    b norx_aarch64_exception

    .global norx_aarch64_irq_entry
norx_aarch64_irq_entry:
    stp x0, x1, [sp, #-16]!
    stp x2, x3, [sp, #-16]!
    stp x4, x5, [sp, #-16]!
    stp x6, x7, [sp, #-16]!
    stp x8, x9, [sp, #-16]!
    stp x10, x11, [sp, #-16]!
    stp x12, x13, [sp, #-16]!
    stp x14, x15, [sp, #-16]!
    stp x16, x17, [sp, #-16]!
    stp x18, x19, [sp, #-16]!
    stp x20, x21, [sp, #-16]!
    stp x22, x23, [sp, #-16]!
    stp x24, x25, [sp, #-16]!
    stp x26, x27, [sp, #-16]!
    stp x28, x29, [sp, #-16]!
    stp x30, xzr, [sp, #-16]!
    mrs x16, spsr_el1
    mrs x17, elr_el1
    stp x16, x17, [sp, #-16]!
    bl norx_aarch64_irq
    ldp x16, x17, [sp], #16
    msr spsr_el1, x16
    msr elr_el1, x17
    ldp x30, xzr, [sp], #16
    ldp x28, x29, [sp], #16
    ldp x26, x27, [sp], #16
    ldp x24, x25, [sp], #16
    ldp x22, x23, [sp], #16
    ldp x20, x21, [sp], #16
    ldp x18, x19, [sp], #16
    ldp x16, x17, [sp], #16
    ldp x14, x15, [sp], #16
    ldp x12, x13, [sp], #16
    ldp x10, x11, [sp], #16
    ldp x8, x9, [sp], #16
    ldp x6, x7, [sp], #16
    ldp x4, x5, [sp], #16
    ldp x2, x3, [sp], #16
    ldp x0, x1, [sp], #16
    eret

    .global norx_aarch64_sync_exception
norx_aarch64_sync_exception:
    stp x0, x16, [sp, #-32]!
    stp x17, x18, [sp, #16]
    mrs x17, spsr_el1
    and x17, x17, #0xf
    cbnz x17, norx_aarch64_restore_kernel_frame
    adrp x18, norx_aarch64_user_depth
    add x18, x18, :lo12:norx_aarch64_user_depth
    ldr x17, [x18]
    cmp x17, #1
    b.lo norx_aarch64_user_stack_fault
    cmp x17, #4
    b.hs norx_aarch64_user_stack_fault
    mov x17, #1
    msr spsel, x17
    adrp x17, norx_aarch64_kernel_stack
    add x17, x17, :lo12:norx_aarch64_kernel_stack
    adrp x18, norx_aarch64_user_depth
    add x18, x18, :lo12:norx_aarch64_user_depth
    ldr x18, [x18]
    lsl x18, x18, #17
    mov x16, #8
    lsl x16, x16, #16
    add x17, x17, x16
    sub x17, x17, x18
    mov sp, x17
    ldp x0, x16, [sp, #-32]
    ldp x17, x18, [sp, #-16]
    b norx_aarch64_exception_frame
norx_aarch64_restore_kernel_frame:
    ldp x0, x16, [sp]
    ldp x17, x18, [sp, #16]
    add sp, sp, #32
norx_aarch64_exception_frame:
    sub sp, sp, #160
    stp x1, x2, [sp, #0]
    stp x3, x4, [sp, #16]
    stp x5, x6, [sp, #32]
    stp x7, x8, [sp, #48]
    stp x9, x10, [sp, #64]
    stp x11, x12, [sp, #80]
    stp x13, x14, [sp, #96]
    stp x29, x30, [sp, #112]
    stp x15, x16, [sp, #128]
    stp x17, x18, [sp, #144]
    mrs x9, CurrentEL
    ubfx x9, x9, #2, #2
    cmp x9, #1
    b.eq 1f
    cmp x9, #2
    b.eq 2f
    cmp x9, #3
    b.eq 3f
    b 9f
1:
    mrs x10, esr_el1
    b 4f
2:
    mrs x10, esr_el2
    b 4f
3:
    mrs x10, esr_el3
4:
    ubfx x12, x10, #26, #6
    cmp x12, #0x15
    b.eq 6f
    cmp x12, #0x20
    b.eq 55f
    cmp x12, #0x21
    b.eq 55f
    cmp x12, #0x24
    b.eq 5f
    cmp x12, #0x25
    b.eq 5f
    b 9f
5:
    cmp x9, #1
    b.eq 51f
    cmp x9, #2
    b.eq 52f
    cmp x9, #3
    b.eq 53f
    b 9f
51:
    mrs x0, elr_el1
    bl norx_aarch64_usercopy_fault
    cbz x0, 9f
    msr elr_el1, x0
    b 7f
52:
    mrs x0, elr_el2
    bl norx_aarch64_usercopy_fault
    cbz x0, 9f
    msr elr_el2, x0
    b 7f
53:
    mrs x0, elr_el3
    bl norx_aarch64_usercopy_fault
    cbz x0, 9f
    msr elr_el3, x0
    b 7f
55:
    bl norx_aarch64_user_fault
    mov x9, #1
    cmp x0, x9
    b.eq 7f
    mov x9, #-4098
    cmp x0, x9
    b.eq 10f
    mov x9, #-4097
    cmp x0, x9
    b.eq 8f
    b 9f
6:
    mov x9, x0
    mov x10, x1
    mov x11, x2
    mov x12, x3
    mov x13, x4
    mov x14, x5
    mov x0, x8
    mov x1, x9
    mov x2, x10
    mov x3, x11
    mov x4, x12
    mov x5, x13
    mov x6, x14
    bl norx_aarch64_syscall_rust
    mov x9, #-4098
    cmp x0, x9
    b.eq 10f
    mov x9, #-4097
    cmp x0, x9
    b.eq 8f
    ldp x1, x2, [sp, #0]
    ldp x3, x4, [sp, #16]
    ldp x5, x6, [sp, #32]
    ldp x7, x8, [sp, #48]
    ldp x9, x10, [sp, #64]
    ldp x11, x12, [sp, #80]
    ldp x13, x14, [sp, #96]
    ldp x29, x30, [sp, #112]
    ldp x15, x16, [sp, #128]
    ldp x17, x18, [sp, #144]
    add sp, sp, #160
    eret
10:
    mrs x9, sp_el0
    adrp x10, SWITCH_SAVED_SP
    add x10, x10, :lo12:SWITCH_SAVED_SP
    str x9, [x10]
    mrs x9, elr_el1
    adrp x10, SWITCH_SAVED_PC
    add x10, x10, :lo12:SWITCH_SAVED_PC
    str x9, [x10]
    mrs x9, spsr_el1
    adrp x10, SWITCH_SAVED_PSTATE
    add x10, x10, :lo12:SWITCH_SAVED_PSTATE
    str x9, [x10]
    adrp x10, SWITCH_SAVED_X19
    add x10, x10, :lo12:SWITCH_SAVED_X19
    str x19, [x10]
    adrp x10, SWITCH_SAVED_X20
    add x10, x10, :lo12:SWITCH_SAVED_X20
    str x20, [x10]
    adrp x10, SWITCH_SAVED_X21
    add x10, x10, :lo12:SWITCH_SAVED_X21
    str x21, [x10]
    adrp x10, SWITCH_SAVED_X22
    add x10, x10, :lo12:SWITCH_SAVED_X22
    str x22, [x10]
    adrp x10, SWITCH_SAVED_X23
    add x10, x10, :lo12:SWITCH_SAVED_X23
    str x23, [x10]
    adrp x10, SWITCH_SAVED_X24
    add x10, x10, :lo12:SWITCH_SAVED_X24
    str x24, [x10]
    adrp x10, SWITCH_SAVED_X25
    add x10, x10, :lo12:SWITCH_SAVED_X25
    str x25, [x10]
    adrp x10, SWITCH_SAVED_X26
    add x10, x10, :lo12:SWITCH_SAVED_X26
    str x26, [x10]
    adrp x10, SWITCH_SAVED_X27
    add x10, x10, :lo12:SWITCH_SAVED_X27
    str x27, [x10]
    adrp x10, SWITCH_SAVED_X28
    add x10, x10, :lo12:SWITCH_SAVED_X28
    str x28, [x10]
    adrp x10, SWITCH_SAVED_X29
    add x10, x10, :lo12:SWITCH_SAVED_X29
    str x29, [x10]
    adrp x10, SWITCH_SAVED_X30
    add x10, x10, :lo12:SWITCH_SAVED_X30
    str x30, [x10]
    mov x1, x0
    mov x0, sp
    bl norx_aarch64_switch_user_rust
    cbz x0, 8f
    mov x20, x0
    adrp x9, norx_aarch64_kernel_stack
    add x9, x9, :lo12:norx_aarch64_kernel_stack
    adrp x10, norx_aarch64_user_depth
    add x10, x10, :lo12:norx_aarch64_user_depth
    ldr x10, [x10]
    lsl x10, x10, #17
    mov x11, #8
    lsl x11, x11, #16
    add x9, x9, x11
    sub x9, x9, x10
    mov sp, x9
    ldr x0, [x20, #24]
    ldp x1, x2, [x20, #32]
    ldp x3, x4, [x20, #48]
    ldp x5, x6, [x20, #64]
    ldp x7, x8, [x20, #80]
    ldp x9, x10, [x20, #96]
    ldp x11, x12, [x20, #112]
    ldp x13, x14, [x20, #128]
    ldp x15, x16, [x20, #144]
    ldr x17, [x20, #160]
    ldr x18, [x20, #168]
    ldr x19, [x20, #176]
    ldr x21, [x20, #192]
    ldr x22, [x20, #200]
    ldr x23, [x20, #208]
    ldr x24, [x20, #216]
    ldr x25, [x20, #224]
    ldr x26, [x20, #232]
    ldr x27, [x20, #240]
    ldr x28, [x20, #248]
    ldr x29, [x20, #256]
    ldr x30, [x20, #264]
    ldr x9, [x20, #0]
    msr elr_el1, x9
    ldr x9, [x20, #16]
    msr spsr_el1, x9
    ldr x9, [x20, #8]
    msr sp_el0, x9
    ldr x9, [x20, #96]
    ldr x20, [x20, #184]
    eret
8:
    adrp x9, norx_aarch64_user_depth
    add x9, x9, :lo12:norx_aarch64_user_depth
    ldr x10, [x9]
    cbz x10, norx_aarch64_user_stack_fault
    sub x10, x10, #1
    str x10, [x9]
    lsl x12, x10, #3
    adrp x9, norx_aarch64_user_return_sp_stack
    add x9, x9, :lo12:norx_aarch64_user_return_sp_stack
    ldr x10, [x9, x12]
    adrp x9, norx_aarch64_user_return_pc_stack
    add x9, x9, :lo12:norx_aarch64_user_return_pc_stack
    ldr x11, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x19_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x19_stack
    ldr x19, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x20_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x20_stack
    ldr x20, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x21_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x21_stack
    ldr x21, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x22_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x22_stack
    ldr x22, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x23_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x23_stack
    ldr x23, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x24_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x24_stack
    ldr x24, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x25_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x25_stack
    ldr x25, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x26_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x26_stack
    ldr x26, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x27_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x27_stack
    ldr x27, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x28_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x28_stack
    ldr x28, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x29_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x29_stack
    ldr x29, [x9, x12]
    adrp x9, norx_aarch64_user_return_elr_stack
    add x9, x9, :lo12:norx_aarch64_user_return_elr_stack
    ldr x13, [x9, x12]
    adrp x9, norx_aarch64_user_return_spsr_stack
    add x9, x9, :lo12:norx_aarch64_user_return_spsr_stack
    ldr x14, [x9, x12]
    msr elr_el1, x13
    msr spsr_el1, x14
    mov x30, x11
    mov sp, x10
    ret
7:
    ldp x1, x2, [sp, #0]
    ldp x3, x4, [sp, #16]
    ldp x5, x6, [sp, #32]
    ldp x7, x8, [sp, #48]
    ldp x9, x10, [sp, #64]
    ldp x11, x12, [sp, #80]
    ldp x13, x14, [sp, #96]
    ldp x29, x30, [sp, #112]
    ldp x15, x16, [sp, #128]
    ldp x17, x18, [sp, #144]
    mov x0, #14
    add sp, sp, #160
    eret
9:
    add sp, sp, #160
    b norx_aarch64_exception_entry
"#
);

extern "C" {
    pub fn norx_aarch64_kernel_start_entry() -> !;
}

extern "C" {
    static norx_exception_vectors: u8;
}

pub fn init() -> bool {
    let current_el: u64;
    let vectors = unsafe { &norx_exception_vectors as *const u8 as u64 };
    if vectors & 0x7ff != 0 {
        crate::bootlog::fail("aarch64 exception vectors are not 2048-byte aligned");
        return false;
    }
    crate::bootlog::ok_fmt(format_args!("aarch64 exception vectors=0x{:x}", vectors));

    unsafe {
        asm!(
            "msr daifset, #0xf",
            options(nomem, nostack, preserves_flags)
        );
        asm!("mrs {}, CurrentEL", out(reg) current_el, options(nomem, nostack, preserves_flags));
        match (current_el >> 2) & 3 {
            1 => {
                asm!("msr vbar_el1, {}", in(reg) vectors, options(nomem, nostack, preserves_flags))
            }
            2 => {
                asm!("msr vbar_el2, {}", in(reg) vectors, options(nomem, nostack, preserves_flags))
            }
            3 => {
                asm!("msr vbar_el3, {}", in(reg) vectors, options(nomem, nostack, preserves_flags))
            }
            _ => {}
        }
        asm!("isb", options(nomem, nostack, preserves_flags));
    }
    true
}

pub fn register_timer_handler() -> bool {
    let info = crate::boot::info().interrupt_info;
    let timer_registration = if info.timer_irq != 0 {
        crate::irq::register_system(
            crate::drivers::framework::IrqKind::Gic,
            info.timer_irq,
            info.timer_irq,
            timer_hard,
            Some(timer_deferred),
        )
    } else {
        Err(crate::irq::IrqError::InvalidRegistration)
    };
    let registered = timer_registration.is_ok();
    crate::bootlog::ok_fmt(format_args!(
        "aarch64 GIC timer registration vector={} result={}",
        info.timer_irq, registered,
    ));
    if let Ok(id) = timer_registration {
        crate::irq::register_timer(id);
        crate::bootlog::ok_fmt(format_args!(
            "aarch64 GIC timer handler registered vector={} slot={}",
            info.timer_irq,
            id.slot()
        ));
    }
    registered
}

pub fn current_el() -> u8 {
    let value: u64;
    unsafe {
        asm!(
            "mrs {}, CurrentEL",
            out(reg) value,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((value >> 2) & 3) as u8
}

#[derive(Clone, Copy)]
pub struct ExceptionState {
    pub current_el: u8,
    pub syndrome: u64,
    pub fault_address: u64,
    pub return_address: u64,
    pub saved_program_status: u64,
}

#[derive(Clone, Copy)]
pub struct UsercopyFault {
    pub exception: ExceptionState,
    pub recovery_address: u64,
}

static mut LAST_USERCOPY_FAULT: Option<UsercopyFault> = None;

pub fn record_usercopy_fault(exception: ExceptionState, recovery_address: u64) {
    unsafe {
        LAST_USERCOPY_FAULT = Some(UsercopyFault {
            exception,
            recovery_address,
        });
    }
}

pub fn last_usercopy_fault() -> Option<UsercopyFault> {
    unsafe { LAST_USERCOPY_FAULT }
}

pub fn exception_state() -> ExceptionState {
    let current_el: u64;
    unsafe {
        asm!(
            "mrs {}, CurrentEL",
            out(reg) current_el,
            options(nomem, nostack, preserves_flags)
        );
    }
    let current_el = ((current_el >> 2) & 3) as u8;
    let (syndrome, fault_address, return_address, saved_program_status) = unsafe {
        match current_el {
            1 => {
                let syndrome: u64;
                let fault_address: u64;
                let return_address: u64;
                let saved_program_status: u64;
                asm!("mrs {}, esr_el1", out(reg) syndrome, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, far_el1", out(reg) fault_address, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, elr_el1", out(reg) return_address, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, spsr_el1", out(reg) saved_program_status, options(nomem, nostack, preserves_flags));
                (
                    syndrome,
                    fault_address,
                    return_address,
                    saved_program_status,
                )
            }
            2 => {
                let syndrome: u64;
                let fault_address: u64;
                let return_address: u64;
                let saved_program_status: u64;
                asm!("mrs {}, esr_el2", out(reg) syndrome, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, far_el2", out(reg) fault_address, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, elr_el2", out(reg) return_address, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, spsr_el2", out(reg) saved_program_status, options(nomem, nostack, preserves_flags));
                (
                    syndrome,
                    fault_address,
                    return_address,
                    saved_program_status,
                )
            }
            3 => {
                let syndrome: u64;
                let fault_address: u64;
                let return_address: u64;
                let saved_program_status: u64;
                asm!("mrs {}, esr_el3", out(reg) syndrome, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, far_el3", out(reg) fault_address, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, elr_el3", out(reg) return_address, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, spsr_el3", out(reg) saved_program_status, options(nomem, nostack, preserves_flags));
                (
                    syndrome,
                    fault_address,
                    return_address,
                    saved_program_status,
                )
            }
            _ => (0, 0, 0, 0),
        }
    };
    ExceptionState {
        current_el,
        syndrome,
        fault_address,
        return_address,
        saved_program_status,
    }
}

#[no_mangle]
extern "C" fn norx_aarch64_kernel_stack_top() -> u64 {
    core::ptr::addr_of!(norx_aarch64_kernel_stack) as u64
        + core::mem::size_of::<KernelStack>() as u64
}

fn timer_hard() -> bool {
    crate::irq::timer();
    true
}

fn timer_deferred() {
    for _ in 0..crate::irq::take_timer_ticks(32) {
        crate::sched::on_timer_tick();
    }
    if crate::irq::timer_pending() {
        crate::irq::requeue_timer();
    }
}

#[no_mangle]
extern "C" fn norx_aarch64_irq() {
    crate::arch::handle_irq();
}

#[no_mangle]
extern "C" fn norx_aarch64_exception() -> ! {
    let state = exception_state();
    let exception_class = ((state.syndrome >> 26) & 0x3f) as u8;
    let (message, code) = match exception_class {
        0x20 | 0x21 => ("aarch64 instruction abort", exception_class as u64),
        0x24 | 0x25 => ("aarch64 data abort", exception_class as u64),
        _ => ("aarch64 exception", exception_class as u64),
    };
    crate::irq::exception();
    crate::kprintln!(
        "  frame: el{} elr=0x{:016x} spsr=0x{:016x} esr=0x{:016x} far=0x{:016x}",
        state.current_el,
        state.return_address,
        state.saved_program_status,
        state.syndrome,
        state.fault_address,
    );
    crate::crash::fatal(crate::error::KernelError::arch_cpu_exception(
        message,
        code,
        "arg0 is ESR_ELx syndrome; arg1 is FAR_ELx fault address",
        state.syndrome,
        state.fault_address,
    ))
}
