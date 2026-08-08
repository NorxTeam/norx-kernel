use core::arch::{asm, global_asm};

const IA32_EFER: u32 = 0xc000_0080;
const IA32_STAR: u32 = 0xc000_0081;
const IA32_LSTAR: u32 = 0xc000_0082;
const IA32_FMASK: u32 = 0xc000_0084;
const EFER_SCE: u64 = 1;

#[no_mangle]
static mut KERNEL_STACK_TOP: u64 = 0;

#[no_mangle]
static mut USER_RETURN_RSP_STACK: [u64; 4] = [0; 4];

#[no_mangle]
static mut USER_RETURN_DEPTH: u64 = 0;

#[no_mangle]
static mut KERNEL_RETURN_R12_STACK: [u64; 4] = [0; 4];

#[no_mangle]
static mut KERNEL_RETURN_R13_STACK: [u64; 4] = [0; 4];

#[no_mangle]
static mut KERNEL_RETURN_R14_STACK: [u64; 4] = [0; 4];

#[no_mangle]
static mut KERNEL_RETURN_R15_STACK: [u64; 4] = [0; 4];

#[no_mangle]
static mut KERNEL_RETURN_RBX_STACK: [u64; 4] = [0; 4];

#[no_mangle]
static mut KERNEL_RETURN_RBP_STACK: [u64; 4] = [0; 4];

#[no_mangle]
static mut SYSCALL_OP: u64 = 0;

#[no_mangle]
static mut SYSCALL_ARG2: u64 = 0;

#[no_mangle]
static mut SYSCALL_ARG3: u64 = 0;

#[no_mangle]
static mut SYSCALL_USER_RSP: u64 = 0;

global_asm!(
    r#"
    .section .text,"ax"
    .global norx_x86_64_syscall_entry
norx_x86_64_syscall_entry:
    mov qword ptr [rip + SYSCALL_OP], rax
    mov qword ptr [rip + SYSCALL_ARG2], rdx
    mov qword ptr [rip + SYSCALL_ARG3], r10
    mov qword ptr [rip + SYSCALL_USER_RSP], rsp
    mov r10, rsp
    mov rax, qword ptr [rip + USER_RETURN_DEPTH]
    test rax, rax
    jz norx_x86_64_user_return_underflow
    cmp rax, 4
    jae norx_x86_64_kernel_stack_overflow
    dec rax
    shl rax, 17
    mov rdx, qword ptr [rip + KERNEL_STACK_TOP]
    sub rdx, rax
    mov rsp, rdx
    and rsp, -16
    push r9
    push r8
    push qword ptr [rip + SYSCALL_ARG2]
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
    mov r8, qword ptr [rip + SYSCALL_ARG3]
    mov rcx, qword ptr [rip + SYSCALL_ARG2]
    mov rdx, rsi
    mov rsi, rdi
    mov rdi, qword ptr [rip + SYSCALL_OP]
    call norx_x86_64_syscall_rust
    cmp rax, -2
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
    mov rsp, qword ptr [rip + SYSCALL_USER_RSP]
    mov r10, qword ptr [rip + SYSCALL_ARG3]
    sysretq

norx_x86_64_user_exit:
    mov rax, qword ptr [rip + USER_RETURN_DEPTH]
    test rax, rax
    jz norx_x86_64_user_return_underflow
    dec rax
    mov qword ptr [rip + USER_RETURN_DEPTH], rax
    lea rdx, [rip + KERNEL_RETURN_RBX_STACK]
    mov rbx, qword ptr [rdx + rax*8]
    lea rdx, [rip + KERNEL_RETURN_RBP_STACK]
    mov rbp, qword ptr [rdx + rax*8]
    lea rdx, [rip + KERNEL_RETURN_R12_STACK]
    mov r12, qword ptr [rdx + rax*8]
    lea rdx, [rip + KERNEL_RETURN_R13_STACK]
    mov r13, qword ptr [rdx + rax*8]
    lea rdx, [rip + KERNEL_RETURN_R14_STACK]
    mov r14, qword ptr [rdx + rax*8]
    lea rdx, [rip + KERNEL_RETURN_R15_STACK]
    mov r15, qword ptr [rdx + rax*8]
    lea rdx, [rip + USER_RETURN_RSP_STACK]
    mov rsp, qword ptr [rdx + rax*8]
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
    mov rax, qword ptr [rip + USER_RETURN_DEPTH]
    cmp rax, 4
    jae norx_x86_64_user_return_overflow
    lea rdx, [rip + KERNEL_RETURN_RBX_STACK]
    mov qword ptr [rdx + rax*8], rbx
    lea rdx, [rip + KERNEL_RETURN_RBP_STACK]
    mov qword ptr [rdx + rax*8], rbp
    lea rdx, [rip + KERNEL_RETURN_R12_STACK]
    mov qword ptr [rdx + rax*8], r12
    lea rdx, [rip + KERNEL_RETURN_R13_STACK]
    mov qword ptr [rdx + rax*8], r13
    lea rdx, [rip + KERNEL_RETURN_R14_STACK]
    mov qword ptr [rdx + rax*8], r14
    lea rdx, [rip + KERNEL_RETURN_R15_STACK]
    mov qword ptr [rdx + rax*8], r15
    lea rdx, [rip + USER_RETURN_RSP_STACK]
    mov qword ptr [rdx + rax*8], rsp
    inc rax
    mov qword ptr [rip + USER_RETURN_DEPTH], rax
    push 0x1b
    push rsi
    push r8
    push 0x23
    push rdi
    iretq

norx_x86_64_user_return_overflow:
    cli
    hlt
"#
);

extern "C" {
    fn norx_x86_64_syscall_entry();
}

extern "sysv64" {
    fn norx_x86_64_enter_user(rip: usize, rsp: usize, flags: u64);
}

pub fn init() -> bool {
    unsafe {
        KERNEL_STACK_TOP = crate::arch::tables::syscall_stack_top();
        USER_RETURN_DEPTH = 0;
        if KERNEL_STACK_TOP == 0 {
            return false;
        }

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
