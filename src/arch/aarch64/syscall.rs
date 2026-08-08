use core::arch::global_asm;

#[no_mangle]
static mut norx_aarch64_user_return_sp_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_user_return_pc_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_user_depth: u64 = 0;

#[no_mangle]
static mut norx_aarch64_user_return_elr_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_user_return_spsr_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_kernel_return_x19_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_kernel_return_x20_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_kernel_return_x21_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_kernel_return_x22_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_kernel_return_x23_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_kernel_return_x24_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_kernel_return_x25_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_kernel_return_x26_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_kernel_return_x27_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_kernel_return_x28_stack: [u64; 4] = [0; 4];

#[no_mangle]
static mut norx_aarch64_kernel_return_x29_stack: [u64; 4] = [0; 4];

global_asm!(
    r#"
    .text
    .align 2
    .global norx_aarch64_enter_user
norx_aarch64_enter_user:
    mov x10, sp
    adrp x9, norx_aarch64_user_depth
    add x9, x9, :lo12:norx_aarch64_user_depth
    ldr x11, [x9]
    cmp x11, #4
    b.hs norx_aarch64_user_stack_fault
    lsl x12, x11, #3
    adrp x13, norx_aarch64_user_return_sp_stack
    add x13, x13, :lo12:norx_aarch64_user_return_sp_stack
    str x10, [x13, x12]
    adrp x13, norx_aarch64_user_return_pc_stack
    add x13, x13, :lo12:norx_aarch64_user_return_pc_stack
    str x30, [x13, x12]
    mrs x15, elr_el1
    adrp x13, norx_aarch64_user_return_elr_stack
    add x13, x13, :lo12:norx_aarch64_user_return_elr_stack
    str x15, [x13, x12]
    mrs x15, spsr_el1
    adrp x13, norx_aarch64_user_return_spsr_stack
    add x13, x13, :lo12:norx_aarch64_user_return_spsr_stack
    str x15, [x13, x12]
    adrp x13, norx_aarch64_kernel_return_x19_stack
    add x13, x13, :lo12:norx_aarch64_kernel_return_x19_stack
    str x19, [x13, x12]
    adrp x13, norx_aarch64_kernel_return_x20_stack
    add x13, x13, :lo12:norx_aarch64_kernel_return_x20_stack
    str x20, [x13, x12]
    adrp x13, norx_aarch64_kernel_return_x21_stack
    add x13, x13, :lo12:norx_aarch64_kernel_return_x21_stack
    str x21, [x13, x12]
    adrp x13, norx_aarch64_kernel_return_x22_stack
    add x13, x13, :lo12:norx_aarch64_kernel_return_x22_stack
    str x22, [x13, x12]
    adrp x13, norx_aarch64_kernel_return_x23_stack
    add x13, x13, :lo12:norx_aarch64_kernel_return_x23_stack
    str x23, [x13, x12]
    adrp x13, norx_aarch64_kernel_return_x24_stack
    add x13, x13, :lo12:norx_aarch64_kernel_return_x24_stack
    str x24, [x13, x12]
    adrp x13, norx_aarch64_kernel_return_x25_stack
    add x13, x13, :lo12:norx_aarch64_kernel_return_x25_stack
    str x25, [x13, x12]
    adrp x13, norx_aarch64_kernel_return_x26_stack
    add x13, x13, :lo12:norx_aarch64_kernel_return_x26_stack
    str x26, [x13, x12]
    adrp x13, norx_aarch64_kernel_return_x27_stack
    add x13, x13, :lo12:norx_aarch64_kernel_return_x27_stack
    str x27, [x13, x12]
    adrp x13, norx_aarch64_kernel_return_x28_stack
    add x13, x13, :lo12:norx_aarch64_kernel_return_x28_stack
    str x28, [x13, x12]
    adrp x13, norx_aarch64_kernel_return_x29_stack
    add x13, x13, :lo12:norx_aarch64_kernel_return_x29_stack
    str x29, [x13, x12]
    add x11, x11, #1
    str x11, [x9]
    mov x14, #1
    msr spsel, x14
    lsl x12, x11, #17
    adrp x13, norx_aarch64_kernel_stack
    add x13, x13, :lo12:norx_aarch64_kernel_stack
    mov x14, #8
    lsl x14, x14, #16
    add x13, x13, x14
    sub x13, x13, x12
    mov sp, x13
    msr sp_el0, x1
    msr elr_el1, x0
    mov x9, #0x3c0
    msr spsr_el1, x9
    isb
    eret
"#
);

#[no_mangle]
extern "C" fn norx_aarch64_user_stack_fault() -> ! {
    crate::bootlog::warn("aarch64 userspace return stack bound violated");
    crate::arch::halt()
}

pub fn init() -> bool {
    crate::bootlog::ok_fmt(format_args!(
        "aarch64 user entry stub=0x{:x}",
        norx_aarch64_enter_user as *const () as usize,
    ));
    unsafe {
        norx_aarch64_user_depth = 0;
        norx_aarch64_user_return_sp_stack = [0; 4];
        norx_aarch64_user_return_pc_stack = [0; 4];
        norx_aarch64_user_return_elr_stack = [0; 4];
        norx_aarch64_user_return_spsr_stack = [0; 4];
        norx_aarch64_kernel_return_x19_stack = [0; 4];
        norx_aarch64_kernel_return_x20_stack = [0; 4];
        norx_aarch64_kernel_return_x21_stack = [0; 4];
        norx_aarch64_kernel_return_x22_stack = [0; 4];
        norx_aarch64_kernel_return_x23_stack = [0; 4];
        norx_aarch64_kernel_return_x24_stack = [0; 4];
        norx_aarch64_kernel_return_x25_stack = [0; 4];
        norx_aarch64_kernel_return_x26_stack = [0; 4];
        norx_aarch64_kernel_return_x27_stack = [0; 4];
        norx_aarch64_kernel_return_x28_stack = [0; 4];
        norx_aarch64_kernel_return_x29_stack = [0; 4];
    }
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
