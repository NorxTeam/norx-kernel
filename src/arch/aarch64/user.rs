use core::{
    arch::asm,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

pub const USER_CODE_BASE: usize = 0x0000_0800_0000_0000;
pub const USER_STACK_TOP: usize = 0x0000_0800_0010_0000;
pub const ARGV_BASE: usize = USER_STACK_TOP - 256;
const ARGV_MAX: usize = 4;
const ARGV_TABLE_BYTES: usize = 128;
const ARG_BYTES: usize = 32;

const PROBE_CODE: &[u8] = &[
    0x40, 0x05, 0x80, 0xd2, // mov x0, #42
    0x28, 0x00, 0x80, 0xd2, // mov x8, #Exit
    0x01, 0x00, 0x00, 0xd4, // svc #0
    0x00, 0x00, 0x20, 0xd4, // brk #0 if Exit returns
];

static PLANNED: AtomicBool = AtomicBool::new(false);
static PAYLOAD_READY: AtomicBool = AtomicBool::new(false);
static MAPPED: AtomicBool = AtomicBool::new(false);
static PROBED: AtomicBool = AtomicBool::new(false);
static CODE_FRAME: AtomicU64 = AtomicU64::new(0);
static STACK_FRAME: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub struct Context {
    pub planned: bool,
    pub ready: bool,
    pub payload_ready: bool,
    pub mapped: bool,
    pub el: u8,
    pub rip: usize,
    pub rsp: usize,
    pub spsr: u64,
    pub code_frame: u64,
    pub stack_frame: u64,
    pub code_prefix: [u8; 16],
}

pub fn prepare() -> bool {
    PLANNED.store(true, Ordering::Relaxed);
    if PAYLOAD_READY.load(Ordering::Relaxed) {
        return false;
    }
    let Some(code) = crate::memory::alloc_frame() else {
        return false;
    };
    let Some(stack) = crate::memory::alloc_frame() else {
        return false;
    };
    write_payload(code);
    zero_frame(stack);
    CODE_FRAME.store(code, Ordering::Relaxed);
    STACK_FRAME.store(stack, Ordering::Relaxed);
    PAYLOAD_READY.store(true, Ordering::Relaxed);
    MAPPED.store(
        crate::arch::paging::map_user_probe(code, stack),
        Ordering::Relaxed,
    );
    false
}

pub fn context() -> Context {
    let code_frame = CODE_FRAME.load(Ordering::Relaxed);
    Context {
        planned: PLANNED.load(Ordering::Relaxed),
        ready: PROBED.load(Ordering::Relaxed),
        payload_ready: PAYLOAD_READY.load(Ordering::Relaxed),
        mapped: MAPPED.load(Ordering::Relaxed),
        el: current_el(),
        rip: USER_CODE_BASE,
        rsp: USER_STACK_TOP - 16,
        spsr: 0,
        code_frame,
        stack_frame: STACK_FRAME.load(Ordering::Relaxed),
        code_prefix: code_prefix(code_frame),
    }
}

pub fn probe() -> Option<u64> {
    prepare();
    if !MAPPED.load(Ordering::Relaxed) || !crate::arch::syscall::begin_user_probe() {
        return None;
    }
    let ctx = context();
    let value = crate::arch::syscall::enter_user_probe(ctx.rip as u64, ctx.rsp as u64);
    if value == 42 {
        PROBED.store(true, Ordering::Relaxed);
    }
    Some(value)
}

pub fn load_code(bytes: &[u8]) -> bool {
    prepare();
    if bytes.len() > 4096 || !MAPPED.load(Ordering::Relaxed) {
        return false;
    }
    let frame = CODE_FRAME.load(Ordering::Relaxed);
    if frame == 0 {
        return false;
    }
    write_code(frame, bytes);
    true
}

pub fn read_bytes(virtual_address: u64, len: usize, mut f: impl FnMut(u8)) -> bool {
    if len > 256 {
        return false;
    }
    let Some((frame, offset)) = frame_offset(virtual_address, len) else {
        return false;
    };
    if frame == 0 {
        return false;
    }
    let ptr = frame as *const u8;
    unsafe {
        for i in 0..len {
            f(ptr.add(offset + i).read_volatile());
        }
    }
    true
}

pub fn load_argv(args: &[&str]) -> bool {
    prepare();
    if !MAPPED.load(Ordering::Relaxed) {
        return false;
    }
    let frame = STACK_FRAME.load(Ordering::Relaxed);
    if frame == 0 {
        return false;
    }
    let ptr = frame as *mut u8;
    let offset = ARGV_BASE - (USER_STACK_TOP - 4096);
    let argc = args.len().min(ARGV_MAX);
    unsafe {
        for i in 0..256 {
            ptr.add(offset + i).write_volatile(0);
        }
        ptr.add(offset).write_volatile(argc as u8);
        for (i, arg) in args.iter().take(argc).enumerate() {
            let bytes = arg.as_bytes();
            let len = bytes.len().min(ARG_BYTES - 1);
            let string_offset = offset + ARGV_TABLE_BYTES + i * ARG_BYTES;
            let string_va = (ARGV_BASE + ARGV_TABLE_BYTES + i * ARG_BYTES) as u64;
            write_u64(ptr, offset + 8 + i * 16, string_va);
            write_u64(ptr, offset + 16 + i * 16, len as u64);
            for (j, byte) in bytes.iter().take(len).enumerate() {
                ptr.add(string_offset + j).write_volatile(*byte);
            }
        }
    }
    true
}

unsafe fn write_u64(ptr: *mut u8, offset: usize, value: u64) {
    for (i, byte) in value.to_le_bytes().iter().enumerate() {
        ptr.add(offset + i).write_volatile(*byte);
    }
}

fn frame_offset(virtual_address: u64, len: usize) -> Option<(u64, usize)> {
    let code_base = USER_CODE_BASE as u64;
    let stack_base = (USER_STACK_TOP - 4096) as u64;
    for (base, frame) in [
        (code_base, CODE_FRAME.load(Ordering::Relaxed)),
        (stack_base, STACK_FRAME.load(Ordering::Relaxed)),
    ] {
        let offset = virtual_address.saturating_sub(base) as usize;
        if virtual_address >= base && offset.saturating_add(len) <= 4096 {
            return Some((frame, offset));
        }
    }
    None
}

fn current_el() -> u8 {
    let value: u64;
    unsafe { asm!("mrs {}, CurrentEL", out(reg) value, options(nomem, nostack, preserves_flags)) };
    ((value >> 2) & 3) as u8
}

fn write_payload(frame: u64) {
    // TODO(paging): the GRUB/ARM64 initial map keeps allocated RAM identity-mapped;
    // replace this with the real aarch64 page-table writer when paging lands.
    write_code(frame, PROBE_CODE);
}

fn write_code(frame: u64, bytes: &[u8]) {
    let ptr = frame as *mut u8;
    unsafe {
        for i in 0..4096 {
            ptr.add(i).write_volatile(0);
        }
        for (i, byte) in bytes.iter().enumerate() {
            ptr.add(i).write_volatile(*byte);
        }
        asm!("dc cvau, {}", in(reg) frame, options(nostack, preserves_flags));
        asm!("dsb ish", options(nostack, preserves_flags));
        asm!("ic ivau, {}", in(reg) USER_CODE_BASE, options(nostack, preserves_flags));
        asm!("dsb ish; isb", options(nostack, preserves_flags));
    }
}

fn zero_frame(frame: u64) {
    let ptr = frame as *mut u8;
    unsafe {
        for i in 0..4096 {
            ptr.add(i).write_volatile(0);
        }
    }
}

fn code_prefix(frame: u64) -> [u8; 16] {
    let mut out = [0; 16];
    if frame == 0 {
        return out;
    }
    let ptr = frame as *const u8;
    unsafe {
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = ptr.add(i).read_volatile();
        }
    }
    out
}
