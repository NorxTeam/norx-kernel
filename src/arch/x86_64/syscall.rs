#![allow(
    dead_code,
    reason = "Staged architecture and fault hooks are not all enabled in bootstrap"
)]

use core::{
    arch::{asm, global_asm},
    cell::UnsafeCell,
};

const IA32_EFER: u32 = 0xc000_0080;
const IA32_STAR: u32 = 0xc000_0081;
const IA32_LSTAR: u32 = 0xc000_0082;
const IA32_FMASK: u32 = 0xc000_0084;
const IA32_GS_BASE: u32 = 0xc000_0101;
const EFER_SCE: u64 = 1;
pub use crate::process::UserContext;

pub const MAX_CPUS: usize = 64;
const RETURN_STACK_DEPTH: usize = 4;

#[repr(C)]
#[derive(Clone, Copy)]
struct CpuState {
    syscall_stack_top: u64,
    user_return_rsp_stack: [u64; RETURN_STACK_DEPTH],
    user_return_depth: u64,
    kernel_return_r12_stack: [u64; RETURN_STACK_DEPTH],
    kernel_return_r13_stack: [u64; RETURN_STACK_DEPTH],
    kernel_return_r14_stack: [u64; RETURN_STACK_DEPTH],
    kernel_return_r15_stack: [u64; RETURN_STACK_DEPTH],
    kernel_return_rbx_stack: [u64; RETURN_STACK_DEPTH],
    kernel_return_rbp_stack: [u64; RETURN_STACK_DEPTH],
    syscall_op: u64,
    syscall_arg2: u64,
    syscall_arg3: u64,
    syscall_user_rsp: u64,
    switch_from: u64,
    switch_to: u64,
    page_fault_target_context: u64,
}

impl CpuState {
    const EMPTY: Self = Self {
        syscall_stack_top: 0,
        user_return_rsp_stack: [0; RETURN_STACK_DEPTH],
        user_return_depth: 0,
        kernel_return_r12_stack: [0; RETURN_STACK_DEPTH],
        kernel_return_r13_stack: [0; RETURN_STACK_DEPTH],
        kernel_return_r14_stack: [0; RETURN_STACK_DEPTH],
        kernel_return_r15_stack: [0; RETURN_STACK_DEPTH],
        kernel_return_rbx_stack: [0; RETURN_STACK_DEPTH],
        kernel_return_rbp_stack: [0; RETURN_STACK_DEPTH],
        syscall_op: 0,
        syscall_arg2: 0,
        syscall_arg3: 0,
        syscall_user_rsp: 0,
        switch_from: 0,
        switch_to: 0,
        page_fault_target_context: 0,
    };
}

// These offsets are part of the private assembly/Rust contract above.
pub const STATE_SYSCALL_STACK_TOP: usize = core::mem::offset_of!(CpuState, syscall_stack_top);
pub const STATE_USER_RETURN_RSP_STACK: usize =
    core::mem::offset_of!(CpuState, user_return_rsp_stack);
pub const STATE_USER_RETURN_DEPTH: usize = core::mem::offset_of!(CpuState, user_return_depth);
pub const STATE_KERNEL_RETURN_R12_STACK: usize =
    core::mem::offset_of!(CpuState, kernel_return_r12_stack);
pub const STATE_KERNEL_RETURN_R13_STACK: usize =
    core::mem::offset_of!(CpuState, kernel_return_r13_stack);
pub const STATE_KERNEL_RETURN_R14_STACK: usize =
    core::mem::offset_of!(CpuState, kernel_return_r14_stack);
pub const STATE_KERNEL_RETURN_R15_STACK: usize =
    core::mem::offset_of!(CpuState, kernel_return_r15_stack);
pub const STATE_KERNEL_RETURN_RBX_STACK: usize =
    core::mem::offset_of!(CpuState, kernel_return_rbx_stack);
pub const STATE_KERNEL_RETURN_RBP_STACK: usize =
    core::mem::offset_of!(CpuState, kernel_return_rbp_stack);
pub const STATE_SYSCALL_OP: usize = core::mem::offset_of!(CpuState, syscall_op);
pub const STATE_SYSCALL_ARG2: usize = core::mem::offset_of!(CpuState, syscall_arg2);
pub const STATE_SYSCALL_ARG3: usize = core::mem::offset_of!(CpuState, syscall_arg3);
pub const STATE_SYSCALL_USER_RSP: usize = core::mem::offset_of!(CpuState, syscall_user_rsp);
pub const STATE_SWITCH_FROM: usize = core::mem::offset_of!(CpuState, switch_from);
pub const STATE_SWITCH_TO: usize = core::mem::offset_of!(CpuState, switch_to);
pub const STATE_PAGE_FAULT_TARGET_CONTEXT: usize =
    core::mem::offset_of!(CpuState, page_fault_target_context);

#[repr(C, align(16))]
struct KernelStack([u8; crate::arch::tables::SYSCALL_STACK_SIZE]);

struct PerCpuStorage {
    states: [UnsafeCell<CpuState>; MAX_CPUS],
    syscall_stacks: [UnsafeCell<KernelStack>; MAX_CPUS],
}

impl PerCpuStorage {
    const fn new() -> Self {
        Self {
            states: [const { UnsafeCell::new(CpuState::EMPTY) }; MAX_CPUS],
            syscall_stacks: [const {
                UnsafeCell::new(KernelStack([0; crate::arch::tables::SYSCALL_STACK_SIZE]))
            }; MAX_CPUS],
        }
    }
}

unsafe impl Sync for PerCpuStorage {}

static PER_CPU: PerCpuStorage = PerCpuStorage::new();

fn cpu_index() -> Option<usize> {
    let cpu = crate::arch::cpu_id();
    (cpu < MAX_CPUS).then_some(cpu)
}

fn state_ptr() -> Option<*mut CpuState> {
    let index = cpu_index()?;
    Some(PER_CPU.states[index].get())
}

fn syscall_stack_top(index: usize) -> u64 {
    (PER_CPU.syscall_stacks[index].get() as usize)
        .saturating_add(crate::arch::tables::SYSCALL_STACK_SIZE) as u64
}

global_asm!(
    r#"
    .section .text,"ax"
    .global norx_x86_64_syscall_entry
norx_x86_64_syscall_entry:
    mov qword ptr gs:[{syscall_op}], rax
    mov qword ptr gs:[{syscall_arg2}], rdx
    mov qword ptr gs:[{syscall_arg3}], r10
    mov qword ptr gs:[{syscall_user_rsp}], rsp
    mov r10, rsp
    mov rax, qword ptr gs:[{user_return_depth}]
    test rax, rax
    jz norx_x86_64_user_return_underflow
    cmp rax, 4
    jae norx_x86_64_kernel_stack_overflow
    dec rax
    shl rax, 17
    mov rdx, qword ptr gs:[{syscall_stack_top}]
    sub rdx, rax
    mov rsp, rdx
    and rsp, -16
    push r9
    push r8
    push qword ptr gs:[{syscall_arg2}]
    push rsi
    push rdi
    push r15
    push r14
    push r13
    push r12
    push rbp
    push rbx
    push rcx
    push r11
    sub rsp, 8
    mov qword ptr [rsp], r9
    mov r9, r8
    mov r8, qword ptr gs:[{syscall_arg3}]
    mov rcx, qword ptr gs:[{syscall_arg2}]
    mov rdx, rsi
    mov rsi, rdi
    mov rdi, qword ptr gs:[{syscall_op}]
    call norx_x86_64_syscall_rust
    cmp rax, -4098
    je norx_x86_64_user_switch
    cmp rax, -4097
    je norx_x86_64_user_exit
    add rsp, 8
    pop r11
    pop rcx
    pop rbx
    pop rbp
    pop r12
    pop r13
    pop r14
    pop r15
    pop rdi
    pop rsi
    pop rdx
    pop r8
    pop r9
    mov rsp, qword ptr gs:[{syscall_user_rsp}]
    mov r10, qword ptr gs:[{syscall_arg3}]
    sysretq

norx_x86_64_user_switch:
    mov rdi, rsp
    mov rsi, rax
    call norx_x86_64_switch_user_rust
    test rax, rax
    jz norx_x86_64_user_exit
    mov rdx, rax
    push 0x1b
    push qword ptr [rdx + 8]
    push qword ptr [rdx + 16]
    push 0x23
    push qword ptr [rdx + 0]
    mov rax, qword ptr [rdx + 24]
    mov rbx, qword ptr [rdx + 32]
    mov rbp, qword ptr [rdx + 40]
    mov r12, qword ptr [rdx + 48]
    mov r13, qword ptr [rdx + 56]
    mov r14, qword ptr [rdx + 64]
    mov r15, qword ptr [rdx + 72]
    mov rdi, qword ptr [rdx + 80]
    mov rsi, qword ptr [rdx + 88]
    mov r8, qword ptr [rdx + 104]
    mov r9, qword ptr [rdx + 112]
    mov r10, qword ptr [rdx + 120]
    mov rdx, qword ptr [rdx + 96]
    iretq

norx_x86_64_user_exit:
    mov rax, qword ptr gs:[{user_return_depth}]
    test rax, rax
    jz norx_x86_64_user_return_underflow
    dec rax
    mov qword ptr gs:[{user_return_depth}], rax
    mov rbx, qword ptr gs:[{kernel_return_rbx_stack} + rax*8]
    mov rbp, qword ptr gs:[{kernel_return_rbp_stack} + rax*8]
    mov r12, qword ptr gs:[{kernel_return_r12_stack} + rax*8]
    mov r13, qword ptr gs:[{kernel_return_r13_stack} + rax*8]
    mov r14, qword ptr gs:[{kernel_return_r14_stack} + rax*8]
    mov r15, qword ptr gs:[{kernel_return_r15_stack} + rax*8]
    mov rsp, qword ptr gs:[{user_return_rsp_stack} + rax*8]
    sti
    ret

norx_x86_64_user_return_underflow:
    cli
    hlt

norx_x86_64_kernel_stack_overflow:
    cli
    hlt

    .global norx_x86_64_enter_user
norx_x86_64_enter_user:
    mov r8, rdx
    mov rax, qword ptr gs:[{user_return_depth}]
    cmp rax, 4
    jae norx_x86_64_user_return_overflow
    mov qword ptr gs:[{kernel_return_rbx_stack} + rax*8], rbx
    mov qword ptr gs:[{kernel_return_rbp_stack} + rax*8], rbp
    mov qword ptr gs:[{kernel_return_r12_stack} + rax*8], r12
    mov qword ptr gs:[{kernel_return_r13_stack} + rax*8], r13
    mov qword ptr gs:[{kernel_return_r14_stack} + rax*8], r14
    mov qword ptr gs:[{kernel_return_r15_stack} + rax*8], r15
    mov qword ptr gs:[{user_return_rsp_stack} + rax*8], rsp
    inc rax
    mov qword ptr gs:[{user_return_depth}], rax
    push 0x1b
    push rsi
    push r8
    push 0x23
    push rdi
    iretq

norx_x86_64_user_return_overflow:
    cli
    hlt
"#,
    syscall_op = const STATE_SYSCALL_OP,
    syscall_arg2 = const STATE_SYSCALL_ARG2,
    syscall_arg3 = const STATE_SYSCALL_ARG3,
    syscall_user_rsp = const STATE_SYSCALL_USER_RSP,
    syscall_stack_top = const STATE_SYSCALL_STACK_TOP,
    user_return_depth = const STATE_USER_RETURN_DEPTH,
    user_return_rsp_stack = const STATE_USER_RETURN_RSP_STACK,
    kernel_return_r12_stack = const STATE_KERNEL_RETURN_R12_STACK,
    kernel_return_r13_stack = const STATE_KERNEL_RETURN_R13_STACK,
    kernel_return_r14_stack = const STATE_KERNEL_RETURN_R14_STACK,
    kernel_return_r15_stack = const STATE_KERNEL_RETURN_R15_STACK,
    kernel_return_rbx_stack = const STATE_KERNEL_RETURN_RBX_STACK,
    kernel_return_rbp_stack = const STATE_KERNEL_RETURN_RBP_STACK,
);

extern "C" {
    fn norx_x86_64_syscall_entry();
}

extern "sysv64" {
    fn norx_x86_64_enter_user(rip: usize, rsp: usize, flags: u64);
}

pub fn init() -> bool {
    let Some(index) = cpu_index() else {
        return false;
    };
    unsafe {
        let kernel_stack_top = crate::arch::tables::kernel_stack_top();
        let syscall_stack_top = syscall_stack_top(index);
        let kernel_stack_base =
            kernel_stack_top.saturating_sub(crate::arch::tables::KERNEL_STACK_SIZE as u64);
        let syscall_stack_base =
            syscall_stack_top.saturating_sub(crate::arch::tables::SYSCALL_STACK_SIZE as u64);
        if kernel_stack_base < syscall_stack_top && syscall_stack_base < kernel_stack_top {
            return false;
        }
        if kernel_stack_top == 0 || syscall_stack_top == 0 {
            return false;
        }

        let state = &mut *PER_CPU.states[index].get();
        *state = CpuState {
            syscall_stack_top,
            ..CpuState::EMPTY
        };
        wrmsr(IA32_GS_BASE, state as *mut CpuState as u64);

        let efer = rdmsr(IA32_EFER) | EFER_SCE | (1 << 11);
        wrmsr(IA32_EFER, efer);

        let kernel = crate::arch::tables::KERNEL_CODE_SELECTOR as u64;
        let user = (crate::arch::tables::USER_CODE_SELECTOR as u64).saturating_sub(16);
        wrmsr(IA32_STAR, (user << 48) | (kernel << 32));
        wrmsr(
            IA32_LSTAR,
            norx_x86_64_syscall_entry as *const () as usize as u64,
        );
        wrmsr(IA32_FMASK, 1 << 9);
    }
    true
}

pub fn install_user_context(thread: u32, registers: crate::elf::InitialRegisters) -> bool {
    let mut context = UserContext::EMPTY;
    context.registers[0] = registers.instruction_pointer as u64;
    context.registers[1] = registers.stack_pointer as u64;
    context.registers[2] = registers.flags;
    crate::process::set_user_context(thread, context).is_ok()
}

pub fn request_user_switch(from: u32, to: u32) {
    if let Some(state) = state_ptr() {
        unsafe {
            (*state).switch_from = from as u64;
            (*state).switch_to = to as u64;
        }
    }
}

pub fn terminate_user_fault() -> u64 {
    let Some((_, from)) = crate::process::current_ids() else {
        return 0;
    };
    let Ok(switch) = crate::process::exit_current(128 + 11) else {
        return 0;
    };
    let Some(switch) = switch else {
        return crate::syscall::PAGE_FAULT_EXIT;
    };
    let Ok(root) = crate::process::commit_user_switch(from, switch.to.get()) else {
        return 0;
    };
    if !crate::arch::switch_to_user(root) {
        return 0;
    }
    if let Some(state) = state_ptr() {
        unsafe {
            (*state).page_fault_target_context = context_pointer(switch.to.get()) as u64;
        }
    }
    crate::syscall::PAGE_FAULT_SWITCH
}

pub fn has_user_context(thread: u32) -> bool {
    crate::process::has_user_context(thread)
}

pub fn revoke_user_context(thread: u32) {
    crate::process::revoke_user_context(thread);
}

fn context_pointer(thread: u32) -> *const UserContext {
    crate::process::user_context_pointer(thread).unwrap_or(core::ptr::null())
}

pub fn enter_user(registers: crate::elf::InitialRegisters) -> bool {
    unsafe {
        norx_x86_64_enter_user(
            registers.instruction_pointer,
            registers.stack_pointer,
            registers.flags,
        );
    }
    true
}

pub fn return_debug_state() -> (u64, u64, u64, u64) {
    unsafe {
        let Some(state) = state_ptr() else {
            return (0, 0, 0, 0);
        };
        let depth = (*state).user_return_depth;
        let saved_rsp = (*state).user_return_rsp_stack[0];
        let saved_return = if saved_rsp == 0 {
            0
        } else {
            (saved_rsp as *const u64).read_volatile()
        };
        (depth, (*state).syscall_stack_top, saved_rsp, saved_return)
    }
}

#[no_mangle]
extern "sysv64" fn norx_x86_64_syscall_rust(
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
extern "sysv64" fn norx_x86_64_switch_user_rust(
    frame: *const u64,
    _switch_marker: u64,
) -> *const UserContext {
    let Some(state) = state_ptr() else {
        return core::ptr::null();
    };
    let (from, to, user_rsp, arg3) = unsafe {
        (
            (*state).switch_from as u32,
            (*state).switch_to as u32,
            (*state).syscall_user_rsp,
            (*state).syscall_arg3,
        )
    };
    if frame.is_null() {
        return core::ptr::null();
    }
    let frame = unsafe { core::slice::from_raw_parts(frame, 14) };
    let context = UserContext {
        registers: [
            frame[2], user_rsp, frame[1], 0, frame[3], frame[4], frame[5], frame[6], frame[7],
            frame[8], frame[9], frame[10], frame[11], frame[12], frame[13], arg3, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
    };
    if crate::process::save_user_context(from, context).is_err() {
        return core::ptr::null();
    }
    let root = match crate::process::commit_user_switch(from, to) {
        Ok(root) => root,
        Err(_) => return core::ptr::null(),
    };
    if !crate::arch::switch_to_user(root) {
        return core::ptr::null();
    }
    context_pointer(to)
}

unsafe fn rdmsr(msr: u32) -> u64 {
    let high: u32;
    let low: u32;
    asm!(
        "rdmsr",
        in("ecx") msr,
        out("edx") high,
        out("eax") low,
        options(nomem, nostack, preserves_flags)
    );
    ((high as u64) << 32) | low as u64
}

unsafe fn wrmsr(msr: u32, value: u64) {
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("edx") (value >> 32) as u32,
        in("eax") value as u32,
        options(nomem, nostack, preserves_flags)
    );
}

pub fn context_contract_self_check() {
    // Real generation/reap checks live in process::contract_self_check.
    assert!(!has_user_context(0));
    assert!(!install_user_context(
        0,
        crate::elf::InitialRegisters {
            instruction_pointer: 1,
            stack_pointer: 2,
            flags: 0,
            arg0: 0,
            arg1: 0,
            arg2: 0,
        }
    ));
}
