use core::arch::global_asm;

#[no_mangle]
static mut norx_aarch64_user_return_sp: u64 = 0;

#[no_mangle]
static mut norx_aarch64_user_return_pc: u64 = 0;

global_asm!(
    r#"
    .text
    .align 2
    .global norx_aarch64_enter_user
norx_aarch64_enter_user:
    mov x10, sp
    mov x9, #1
    msr spsel, x9
    mov sp, x10
    adrp x9, norx_aarch64_user_return_sp
    add x9, x9, :lo12:norx_aarch64_user_return_sp
    str x10, [x9]
    adrp x9, norx_aarch64_user_return_pc
    add x9, x9, :lo12:norx_aarch64_user_return_pc
    str x30, [x9]
    mov x19, x0
    mov x20, x1
    bl norx_aarch64_kernel_stack_top
    mov sp, x0
    msr sp_el0, x20
    msr elr_el1, x19
    mov x9, #0x3c0
    msr spsr_el1, x9
    isb
    eret
"#
);

pub fn init() -> bool {
    crate::bootlog::info_fmt(format_args!(
        "aarch64 user entry stub=0x{:x}",
        norx_aarch64_enter_user as *const () as usize,
    ));
    true
}

extern "C" {
    fn norx_aarch64_enter_user(instruction_pointer: u64, stack_pointer: u64);
}

pub fn enter_user(registers: crate::elf::InitialRegisters) -> bool {
    unsafe {
        norx_aarch64_enter_user(
            registers.instruction_pointer as u64,
            registers.stack_pointer as u64,
        );
    }
    true
}

#[no_mangle]
extern "C" fn norx_aarch64_usercopy_fault(rip: u64) -> u64 {
    if crate::usercopy::handles_fault(rip) {
        crate::usercopy::recovery_address()
    } else {
        0
    }
}

#[no_mangle]
extern "C" fn norx_aarch64_syscall_rust(
    _op: u64,
    _a0: u64,
    _a1: u64,
    _a2: u64,
    _a3: u64,
    _a4: u64,
    _a5: u64,
) -> u64 {
    crate::syscall::dispatch(
        _op,
        crate::syscall::Args {
            values: [_a0, _a1, _a2, _a3, _a4, _a5],
        },
    )
}
