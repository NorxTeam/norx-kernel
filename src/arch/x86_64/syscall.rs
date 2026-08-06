use core::arch::{asm, global_asm};

const IA32_EFER: u32 = 0xc000_0080;
const IA32_STAR: u32 = 0xc000_0081;
const IA32_LSTAR: u32 = 0xc000_0082;
const IA32_FMASK: u32 = 0xc000_0084;
const EFER_SCE: u64 = 1;
const ENOSYS: u64 = 38;

#[no_mangle]
static mut KERNEL_STACK_TOP: u64 = 0;

global_asm!(
    r#"
    .global norx_x86_64_syscall_entry
norx_x86_64_syscall_entry:
    mov r12, rsp
    mov rsp, qword ptr [rip + KERNEL_STACK_TOP]
    and rsp, -16
    push r12
    push rcx
    push r11
    sub rsp, 8
    mov r9, r8
    mov r8, r10
    mov rcx, rdx
    mov rdx, rsi
    mov rsi, rdi
    mov rdi, rax
    call norx_x86_64_syscall_rust
    add rsp, 8
    pop r11
    pop rcx
    pop r12
    mov rsp, r12
    sysretq
"#
);

extern "C" {
    fn norx_x86_64_syscall_entry();
}

pub fn init() {
    unsafe {
        KERNEL_STACK_TOP = crate::arch::tables::syscall_stack_top();

        let efer = rdmsr(IA32_EFER) | EFER_SCE;
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
    0u64.wrapping_sub(ENOSYS)
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
