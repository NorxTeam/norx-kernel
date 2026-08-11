use core::arch::global_asm;

const MAX_USER_CONTEXTS: usize = 64;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UserContext {
    registers: [u64; 35],
}

impl UserContext {
    const EMPTY: Self = Self { registers: [0; 35] };
}

static mut USER_CONTEXTS: [UserContext; MAX_USER_CONTEXTS] =
    [UserContext::EMPTY; MAX_USER_CONTEXTS];
static mut CONTEXT_VALID: [bool; MAX_USER_CONTEXTS] = [false; MAX_USER_CONTEXTS];
static mut SWITCH_FROM: u64 = 0;
static mut SWITCH_TO: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_SP: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_PC: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_PSTATE: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_X19: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_X20: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_X21: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_X22: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_X23: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_X24: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_X25: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_X26: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_X27: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_X28: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_X29: u64 = 0;
#[no_mangle]
static mut SWITCH_SAVED_X30: u64 = 0;

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
        CONTEXT_VALID = [false; MAX_USER_CONTEXTS];
    }
    true
}

pub fn install_user_context(thread: u32, registers: crate::elf::InitialRegisters) -> bool {
    let Some(index) = context_index(thread) else {
        return false;
    };
    unsafe {
        let mut context = UserContext::EMPTY;
        context.registers[0] = registers.instruction_pointer as u64;
        context.registers[1] = registers.stack_pointer as u64;
        context.registers[2] = 0x3c0;
        context.registers[3] = registers.arg0;
        context.registers[4] = registers.arg1;
        context.registers[5] = registers.arg2;
        USER_CONTEXTS[index] = context;
        CONTEXT_VALID[index] = true;
    }
    true
}

pub fn request_user_switch(from: u32, to: u32) {
    unsafe {
        SWITCH_FROM = from as u64;
        SWITCH_TO = to as u64;
    }
}

pub fn has_user_context(thread: u32) -> bool {
    context_index(thread).is_some_and(|index| unsafe { CONTEXT_VALID[index] })
}

fn context_index(thread: u32) -> Option<usize> {
    let slot = (thread & 0xffff) as usize;
    (slot != 0 && slot <= MAX_USER_CONTEXTS).then_some(slot - 1)
}

fn context_pointer(thread: u32) -> *const UserContext {
    let Some(index) = context_index(thread) else {
        return core::ptr::null();
    };
    unsafe { core::ptr::addr_of!(USER_CONTEXTS[index]) }
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

#[no_mangle]
extern "C" fn norx_aarch64_switch_user_rust(
    frame: *const u64,
    _switch_marker: u64,
) -> *const UserContext {
    let (from, to) = unsafe { (SWITCH_FROM as u32, SWITCH_TO as u32) };
    let Some(index) = context_index(from) else {
        return core::ptr::null();
    };
    if frame.is_null() {
        return core::ptr::null();
    }
    unsafe {
        let frame = core::slice::from_raw_parts(frame, 20);
        let mut context = UserContext::EMPTY;
        context.registers[0] = SWITCH_SAVED_PC;
        context.registers[1] = SWITCH_SAVED_SP;
        context.registers[2] = SWITCH_SAVED_PSTATE;
        context.registers[3] = 0;
        context.registers[4..18].copy_from_slice(&frame[..14]);
        context.registers[18] = frame[16];
        context.registers[19] = frame[17];
        context.registers[20] = frame[18];
        context.registers[21] = frame[19];
        context.registers[22] = SWITCH_SAVED_X19;
        context.registers[23] = SWITCH_SAVED_X20;
        context.registers[24] = SWITCH_SAVED_X21;
        context.registers[25] = SWITCH_SAVED_X22;
        context.registers[26] = SWITCH_SAVED_X23;
        context.registers[27] = SWITCH_SAVED_X24;
        context.registers[28] = SWITCH_SAVED_X25;
        context.registers[29] = SWITCH_SAVED_X26;
        context.registers[30] = SWITCH_SAVED_X27;
        context.registers[31] = SWITCH_SAVED_X28;
        context.registers[32] = SWITCH_SAVED_X29;
        context.registers[33] = SWITCH_SAVED_X30;
        USER_CONTEXTS[index] = context;
        CONTEXT_VALID[index] = true;
    }
    let root = match crate::process::commit_user_switch(from, to) {
        Ok(root) => root,
        Err(_) => return core::ptr::null(),
    };
    if !crate::arch::switch_to_user(root) {
        return core::ptr::null();
    }
    let target = context_pointer(to);
    target
}
