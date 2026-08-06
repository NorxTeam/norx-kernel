use core::{
    arch::asm,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

const PROBE_CODE: &[u8] = &[
    0xbf, 0x2a, 0x00, 0x00, 0x00, // mov edi, 42
    0xb8, 0x01, 0x00, 0x00, 0x00, // mov eax, Exit
    0xcd, 0x80, // int 0x80 exits through the universal syscall dispatcher
    0x0f, 0x0b, // ud2 if Exit returns to user
];
pub const ARGV_BASE: usize = crate::arch::paging::USER_STACK_TOP - 256;
const ARGV_MAX: usize = 4;
const ARGV_TABLE_BYTES: usize = 128;
const ARG_BYTES: usize = 32;

static READY: AtomicBool = AtomicBool::new(false);
static CODE_FRAME: AtomicU64 = AtomicU64::new(0);
static STACK_FRAME: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub struct Context {
    pub ready: bool,
    pub rip: usize,
    pub rsp: usize,
    pub code_frame: u64,
    pub stack_frame: u64,
    pub cs: u16,
    pub ss: u16,
    pub code_prefix: [u8; 16],
}

pub fn prepare() -> bool {
    if READY.load(Ordering::Relaxed) {
        return true;
    }
    let Some(code) = crate::arch::paging::map_user_page(crate::arch::paging::USER_CODE_BASE, false)
    else {
        return false;
    };
    let stack_base = crate::arch::paging::USER_STACK_TOP - 4096;
    let Some(stack) = crate::arch::paging::map_user_page(stack_base, true) else {
        return false;
    };
    let Some(code_ptr) = crate::arch::paging::direct_map_ptr(code) else {
        return false;
    };
    if !mark_transition_pages() {
        return false;
    }
    unsafe {
        for (i, byte) in PROBE_CODE.iter().enumerate() {
            code_ptr.add(i).write_volatile(*byte);
        }
    }

    CODE_FRAME.store(code, Ordering::Relaxed);
    STACK_FRAME.store(stack, Ordering::Relaxed);
    READY.store(true, Ordering::Relaxed);
    true
}

pub fn context() -> Context {
    let code_frame = CODE_FRAME.load(Ordering::Relaxed);
    Context {
        ready: READY.load(Ordering::Relaxed),
        rip: crate::arch::paging::USER_CODE_BASE,
        rsp: crate::arch::paging::USER_STACK_TOP - 16,
        code_frame,
        stack_frame: STACK_FRAME.load(Ordering::Relaxed),
        cs: crate::arch::tables::USER_CODE_SELECTOR,
        ss: crate::arch::tables::USER_DATA_SELECTOR,
        code_prefix: code_prefix(code_frame),
    }
}

#[allow(dead_code)]
pub fn probe() -> Option<u64> {
    if !prepare() || !crate::arch::syscall::begin_user_probe() {
        return None;
    }
    let ctx = context();
    Some(crate::arch::syscall::enter_user_probe(
        ctx.rip as u64,
        ctx.rsp as u64,
    ))
}

pub fn load_code(bytes: &[u8]) -> bool {
    if bytes.len() > 4096 || !prepare() {
        return false;
    }
    let frame = CODE_FRAME.load(Ordering::Relaxed);
    let Some(ptr) = crate::arch::paging::direct_map_ptr(frame) else {
        return false;
    };
    unsafe {
        for i in 0..4096 {
            ptr.add(i).write_volatile(0);
        }
        for (i, byte) in bytes.iter().enumerate() {
            ptr.add(i).write_volatile(*byte);
        }
    }
    true
}

pub fn read_bytes(virtual_address: u64, len: usize, mut f: impl FnMut(u8)) -> bool {
    if len > 256 {
        return false;
    }
    let Some((frame, offset)) = frame_offset(virtual_address, len) else {
        return false;
    };
    let Some(ptr) = crate::arch::paging::direct_map_ptr(frame) else {
        return false;
    };
    unsafe {
        for i in 0..len {
            f(ptr.add(offset + i).read_volatile());
        }
    }
    true
}

pub fn load_argv(args: &[&str]) -> bool {
    if !prepare() {
        return false;
    }
    let frame = STACK_FRAME.load(Ordering::Relaxed);
    let Some(ptr) = crate::arch::paging::direct_map_ptr(frame) else {
        return false;
    };
    let offset = ARGV_BASE - (crate::arch::paging::USER_STACK_TOP - 4096);
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
    let code_base = crate::arch::paging::USER_CODE_BASE as u64;
    let stack_base = (crate::arch::paging::USER_STACK_TOP - 4096) as u64;
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

fn mark_transition_pages() -> bool {
    let pages = crate::arch::tables::transition_pages();
    crate::arch::paging::mark_user_accessible(pages.gdt)
        && crate::arch::paging::mark_user_accessible(pages.idt)
        && crate::arch::paging::mark_user_accessible(pages.idt + 4095)
        && crate::arch::paging::mark_user_accessible(pages.tss)
        && crate::arch::paging::mark_user_accessible(pages.int80)
        // TODO(paging): GRUB's initial page tables keep the live kernel stack in a
        // supervisor large page; replace this with a Norx-owned CR3 before real userspace.
        && crate::arch::paging::mark_user_accessible(current_stack())
        // TODO(paging): inherited firmware tables also fault on a low transition read.
        && crate::arch::paging::mark_user_accessible(0x1000)
}

fn code_prefix(frame: u64) -> [u8; 16] {
    let mut out = [0; 16];
    if let Some(ptr) = crate::arch::paging::direct_map_ptr(frame) {
        unsafe {
            for (i, byte) in out.iter_mut().enumerate() {
                *byte = ptr.add(i).read_volatile();
            }
        }
    }
    out
}

fn current_stack() -> usize {
    let rsp: usize;
    unsafe { asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack, preserves_flags)) };
    rsp
}
