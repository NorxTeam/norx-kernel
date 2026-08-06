use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const IA32_EFER: u32 = 0xc000_0080;
const IA32_STAR: u32 = 0xc000_0081;
const IA32_LSTAR: u32 = 0xc000_0082;
const IA32_FMASK: u32 = 0xc000_0084;
const EFER_SCE: u64 = 1;

static READY: AtomicBool = AtomicBool::new(false);
static TRAPS: AtomicU64 = AtomicU64::new(0);
#[no_mangle]
static mut NORR_X86_64_USER_DISPATCH_TRAPS: u64 = 0;
#[no_mangle]
static mut NORR_X86_64_INT80_HITS: u64 = 0;
#[no_mangle]
static mut NORR_X86_64_INT80_FAST_HITS: u64 = 0;
#[no_mangle]
static mut NORR_X86_64_INT80_LAST_OP: u64 = 0;

#[no_mangle]
static mut NORR_X86_64_SYSCALL_STACK_TOP: u64 = 0;
#[no_mangle]
static mut NORR_X86_64_USER_PROBE_ACTIVE: u8 = 0;
#[no_mangle]
static mut NORR_X86_64_USER_PROBE_DONE: u8 = 0;
#[no_mangle]
static mut NORR_X86_64_USER_PROBE_RETURN_RSP: u64 = 0;
#[no_mangle]
static mut NORR_X86_64_USER_PROBE_RETURN_RIP: u64 = 0;
#[no_mangle]
static mut NORR_X86_64_USER_PROBE_VALUE: u64 = 0;
#[no_mangle]
static mut NORR_X86_64_USER_PROBE_FAULT_RIP: u64 = 0;

global_asm!(
    r#"
    .global norr_x86_64_syscall_entry
norr_x86_64_syscall_entry:
    mov r12, rsp
    mov rsp, qword ptr [rip + NORR_X86_64_SYSCALL_STACK_TOP]
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
    call norr_x86_64_syscall_rust
    cmp byte ptr [rip + NORR_X86_64_USER_PROBE_DONE], 0
    jne 3f
    add rsp, 8
    pop r11
    pop rcx
    pop r12
    mov rsp, r12
    sysretq
3:
    mov rsp, qword ptr [rip + NORR_X86_64_USER_PROBE_RETURN_RSP]
    mov rax, qword ptr [rip + NORR_X86_64_USER_PROBE_VALUE]
    mov r11, qword ptr [rip + NORR_X86_64_USER_PROBE_RETURN_RIP]
    mov byte ptr [rip + NORR_X86_64_USER_PROBE_DONE], 0
    mov byte ptr [rip + NORR_X86_64_USER_PROBE_ACTIVE], 0
    mov dx, 0x10
    mov ds, dx
    mov es, dx
    jmp r11

"#
);

extern "C" {
    fn norr_x86_64_syscall_entry();
}

#[derive(Clone, Copy)]
pub struct Status {
    pub ready: bool,
    pub traps: u64,
    pub int80_hits: u64,
    pub int80_fast_hits: u64,
    pub int80_last_op: u64,
    pub lstar: u64,
    pub star: u64,
    pub fmask: u64,
    pub kernel_stack_top: u64,
    pub kernel_stack_slot: u64,
    pub user_probe_active: u8,
    pub user_probe_done: u8,
    pub user_probe_fault_rip: u64,
}

pub fn init() {
    unsafe {
        NORR_X86_64_SYSCALL_STACK_TOP = crate::arch::tables::tss_status().rsp0;

        let efer = rdmsr(IA32_EFER) | EFER_SCE;
        wrmsr(IA32_EFER, efer);

        let kernel = crate::arch::tables::KERNEL_CODE_SELECTOR as u64;
        let user = (crate::arch::tables::USER_CODE_SELECTOR as u64).saturating_sub(16);
        wrmsr(IA32_STAR, (user << 48) | (kernel << 32));
        wrmsr(
            IA32_LSTAR,
            norr_x86_64_syscall_entry as *const () as usize as u64,
        );
        wrmsr(IA32_FMASK, 1 << 9);
    }
    READY.store(true, Ordering::Relaxed);
}

pub fn status() -> Status {
    Status {
        ready: READY.load(Ordering::Relaxed),
        traps: TRAPS.load(Ordering::Relaxed)
            + unsafe { core::ptr::addr_of!(NORR_X86_64_USER_DISPATCH_TRAPS).read_volatile() },
        int80_hits: unsafe { core::ptr::addr_of!(NORR_X86_64_INT80_HITS).read_volatile() },
        int80_fast_hits: unsafe {
            core::ptr::addr_of!(NORR_X86_64_INT80_FAST_HITS).read_volatile()
        },
        int80_last_op: unsafe { core::ptr::addr_of!(NORR_X86_64_INT80_LAST_OP).read_volatile() },
        lstar: unsafe { rdmsr(IA32_LSTAR) },
        star: unsafe { rdmsr(IA32_STAR) },
        fmask: unsafe { rdmsr(IA32_FMASK) },
        kernel_stack_top: unsafe { NORR_X86_64_SYSCALL_STACK_TOP },
        kernel_stack_slot: (&raw const NORR_X86_64_SYSCALL_STACK_TOP) as u64,
        user_probe_active: unsafe { NORR_X86_64_USER_PROBE_ACTIVE },
        user_probe_done: unsafe { NORR_X86_64_USER_PROBE_DONE },
        user_probe_fault_rip: unsafe {
            core::ptr::addr_of!(NORR_X86_64_USER_PROBE_FAULT_RIP).read_volatile()
        },
    }
}

pub fn smoke() -> crate::abi::syscall::Return {
    handle(crate::abi::syscall::UniversalOp::Clock as u64, [0; 6])
}

pub fn user_probe_active() -> bool {
    unsafe { NORR_X86_64_USER_PROBE_ACTIVE != 0 }
}

#[allow(dead_code)]
pub fn begin_user_probe() -> bool {
    if !READY.load(Ordering::Relaxed) {
        return false;
    }
    unsafe {
        NORR_X86_64_USER_PROBE_VALUE = 0;
        NORR_X86_64_USER_PROBE_FAULT_RIP = 0;
        NORR_X86_64_USER_PROBE_DONE = 0;
        NORR_X86_64_USER_PROBE_ACTIVE = 1;
    }
    true
}

#[allow(dead_code)]
pub fn enter_user_probe(rip: u64, rsp: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "lea rax, [rip + 2f]",
            "mov qword ptr [rip + NORR_X86_64_USER_PROBE_RETURN_RIP], rax",
            "mov qword ptr [rip + NORR_X86_64_USER_PROBE_RETURN_RSP], rsp",
            "mov ax, 0x1b",
            "mov ds, ax",
            "mov es, ax",
            "mov rax, 0x1b",
            "push rax",
            "push r13",
            "mov rax, 0x2",
            "push rax",
            "mov rax, 0x23",
            "push rax",
            "push r12",
            "iretq",
            "2:",
            in("r12") rip,
            in("r13") rsp,
            lateout("rax") ret,
            lateout("rcx") _,
            lateout("rdx") _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("r11") _,
        );
    }
    ret
}

fn handle(op: u64, args: [u64; 6]) -> crate::abi::syscall::Return {
    TRAPS.fetch_add(1, Ordering::Relaxed);
    let op = match op {
        0 => crate::abi::syscall::UniversalOp::Activate,
        1 => crate::abi::syscall::UniversalOp::Exit,
        2 => crate::abi::syscall::UniversalOp::Read,
        3 => crate::abi::syscall::UniversalOp::Write,
        4 => crate::abi::syscall::UniversalOp::Open,
        5 => crate::abi::syscall::UniversalOp::Close,
        6 => crate::abi::syscall::UniversalOp::Map,
        7 => crate::abi::syscall::UniversalOp::Unmap,
        8 => crate::abi::syscall::UniversalOp::Clock,
        9 => crate::abi::syscall::UniversalOp::Spawn,
        _ => {
            return crate::abi::syscall::Return {
                value: 0,
                error: 38,
            }
        }
    };
    let ret = crate::abi::syscall::dispatch(crate::abi::syscall::UniversalCall { op, args });
    if matches!(op, crate::abi::syscall::UniversalOp::Exit) && ret.error == 0 {
        unsafe {
            if NORR_X86_64_USER_PROBE_ACTIVE != 0 {
                NORR_X86_64_USER_PROBE_VALUE = ret.value;
                NORR_X86_64_USER_PROBE_DONE = 1;
            }
        }
    }
    ret
}

#[no_mangle]
extern "sysv64" fn norr_x86_64_syscall_rust(
    op: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
) -> u64 {
    let ret = handle(op, [a0, a1, a2, a3, a4, a5]);
    if ret.error == 0 {
        ret.value
    } else {
        (!0u64).saturating_sub(ret.error).saturating_add(1)
    }
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
