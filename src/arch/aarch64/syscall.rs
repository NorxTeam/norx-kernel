use core::{
    arch::{asm, global_asm},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

global_asm!(
    r#"
    .global boa_aarch64_enter_user_probe
boa_aarch64_enter_user_probe:
    adrp x9, BOA_AARCH64_USER_PROBE_RETURN_PC
    add x9, x9, :lo12:BOA_AARCH64_USER_PROBE_RETURN_PC
    str x30, [x9]
    adrp x9, BOA_AARCH64_USER_PROBE_RETURN_SP
    add x9, x9, :lo12:BOA_AARCH64_USER_PROBE_RETURN_SP
    mov x10, sp
    str x10, [x9]
    msr sp_el0, x1
    msr elr_el1, x0
    msr spsr_el1, xzr
    isb
    eret
"#
);

extern "C" {
    fn boa_aarch64_enter_user_probe(rip: u64, rsp: u64) -> u64;
}

static DISPATCHER_READY: AtomicBool = AtomicBool::new(false);
static SVC_READY: AtomicBool = AtomicBool::new(false);
static TRAPS: AtomicU64 = AtomicU64::new(0);
#[no_mangle]
static BOA_AARCH64_LAST_OP: AtomicU64 = AtomicU64::new(0);
#[no_mangle]
static mut BOA_AARCH64_USER_PROBE_ACTIVE: u64 = 0;
#[no_mangle]
static mut BOA_AARCH64_USER_PROBE_DONE: u64 = 0;
#[no_mangle]
static mut BOA_AARCH64_USER_PROBE_RETURN_SP: u64 = 0;
#[no_mangle]
static mut BOA_AARCH64_USER_PROBE_RETURN_PC: u64 = 0;
#[no_mangle]
static mut BOA_AARCH64_USER_PROBE_VALUE: u64 = 0;

#[derive(Clone, Copy)]
pub struct Status {
    pub dispatcher_ready: bool,
    pub svc_ready: bool,
    pub traps: u64,
    pub last_op: u64,
    pub user_probe_active: u64,
    pub user_probe_done: u64,
}

pub fn init() {
    DISPATCHER_READY.store(true, Ordering::Relaxed);
    SVC_READY.store(true, Ordering::Relaxed);
}

pub fn status() -> Status {
    Status {
        dispatcher_ready: DISPATCHER_READY.load(Ordering::Relaxed),
        svc_ready: SVC_READY.load(Ordering::Relaxed),
        traps: TRAPS.load(Ordering::Relaxed),
        last_op: BOA_AARCH64_LAST_OP.load(Ordering::Relaxed),
        user_probe_active: unsafe {
            core::ptr::addr_of!(BOA_AARCH64_USER_PROBE_ACTIVE).read_volatile()
        },
        user_probe_done: unsafe {
            core::ptr::addr_of!(BOA_AARCH64_USER_PROBE_DONE).read_volatile()
        },
    }
}

pub fn smoke() -> crate::abi::syscall::Return {
    if !SVC_READY.load(Ordering::Relaxed) {
        return crate::abi::syscall::Return {
            value: 0,
            error: 38,
        };
    }
    let value: u64;
    unsafe {
        asm!(
            "svc #0",
            in("x8") crate::abi::syscall::UniversalOp::Clock as u64,
            lateout("x0") value,
            lateout("x1") _,
            lateout("x2") _,
            lateout("x3") _,
            lateout("x4") _,
            lateout("x5") _,
            lateout("x6") _,
            lateout("x7") _,
            options(nostack)
        );
    }
    crate::abi::syscall::Return { value, error: 0 }
}

fn handle(op: u64, args: [u64; 6]) -> crate::abi::syscall::Return {
    TRAPS.fetch_add(1, Ordering::Relaxed);
    BOA_AARCH64_LAST_OP.store(op, Ordering::Relaxed);
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
            if BOA_AARCH64_USER_PROBE_ACTIVE != 0 {
                BOA_AARCH64_USER_PROBE_VALUE = ret.value;
                BOA_AARCH64_USER_PROBE_DONE = 1;
            }
        }
    }
    ret
}

pub fn begin_user_probe() -> bool {
    if !SVC_READY.load(Ordering::Relaxed) {
        return false;
    }
    unsafe {
        BOA_AARCH64_USER_PROBE_VALUE = 0;
        BOA_AARCH64_USER_PROBE_DONE = 0;
        BOA_AARCH64_USER_PROBE_ACTIVE = 1;
    }
    true
}

pub fn enter_user_probe(rip: u64, rsp: u64) -> u64 {
    unsafe { boa_aarch64_enter_user_probe(rip, rsp) }
}

#[no_mangle]
extern "C" fn boa_aarch64_syscall_rust(
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
