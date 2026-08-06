use core::arch::{asm, global_asm};

global_asm!(
    r#"
    .align 11
    .global norx_exception_vectors
norx_exception_vectors:
    b norx_aarch64_sync_exception
    .space 124
    b norx_aarch64_exception
    .space 124
    b norx_aarch64_exception
    .space 124
    b norx_aarch64_exception
    .space 124
    b norx_aarch64_sync_exception
    .space 124
    b norx_aarch64_exception
    .space 124
    b norx_aarch64_exception
    .space 124
    b norx_aarch64_exception
    .space 124
    b norx_aarch64_sync_exception
    .space 124
    b norx_aarch64_exception
    .space 124
    b norx_aarch64_exception
    .space 124
    b norx_aarch64_exception
    .space 124
    b norx_aarch64_exception
    .space 124
    b norx_aarch64_exception
    .space 124
    b norx_aarch64_exception
    .space 124
    b norx_aarch64_exception
    .space 124

    .global norx_aarch64_sync_exception
norx_aarch64_sync_exception:
    sub sp, sp, #160
    stp x1, x2, [sp, #0]
    stp x3, x4, [sp, #16]
    stp x5, x6, [sp, #32]
    stp x7, x8, [sp, #48]
    stp x9, x10, [sp, #64]
    stp x11, x12, [sp, #80]
    stp x13, x14, [sp, #96]
    stp x29, x30, [sp, #112]
    mrs x9, CurrentEL
    ubfx x9, x9, #2, #2
    cmp x9, #1
    b.eq 1f
    cmp x9, #2
    b.eq 2f
    cmp x9, #3
    b.eq 3f
    b 9f
1:
    mrs x10, esr_el1
    b 4f
2:
    mrs x10, esr_el2
    b 4f
3:
    mrs x10, esr_el3
4:
    ubfx x12, x10, #26, #6
    cmp x12, #0x15
    b.ne 9f
    mov x9, x0
    mov x10, x1
    mov x11, x2
    mov x12, x3
    mov x13, x4
    mov x14, x5
    mov x0, x8
    mov x1, x9
    mov x2, x10
    mov x3, x11
    mov x4, x12
    mov x5, x13
    mov x6, x14
    bl norx_aarch64_syscall_rust
    ldp x1, x2, [sp, #0]
    ldp x3, x4, [sp, #16]
    ldp x5, x6, [sp, #32]
    ldp x7, x8, [sp, #48]
    ldp x9, x10, [sp, #64]
    ldp x11, x12, [sp, #80]
    ldp x13, x14, [sp, #96]
    ldp x29, x30, [sp, #112]
    add sp, sp, #160
    eret
9:
    add sp, sp, #160
    b norx_aarch64_exception
"#
);

extern "C" {
    static norx_exception_vectors: u8;
}

pub fn init() {
    let current_el: u64;
    let vectors = unsafe { &norx_exception_vectors as *const u8 as u64 };

    unsafe {
        asm!(
            "msr daifset, #0xf",
            options(nomem, nostack, preserves_flags)
        );
        asm!("mrs {}, CurrentEL", out(reg) current_el, options(nomem, nostack, preserves_flags));
        match (current_el >> 2) & 3 {
            1 => {
                asm!("msr vbar_el1, {}", in(reg) vectors, options(nomem, nostack, preserves_flags))
            }
            2 => {
                asm!("msr vbar_el2, {}", in(reg) vectors, options(nomem, nostack, preserves_flags))
            }
            3 => {
                asm!("msr vbar_el3, {}", in(reg) vectors, options(nomem, nostack, preserves_flags))
            }
            _ => {}
        }
        asm!("isb", options(nomem, nostack, preserves_flags));
    }
}

#[no_mangle]
extern "C" fn norx_aarch64_exception() -> ! {
    let current_el: u64;
    let esr: u64;
    let far: u64;
    let elr: u64;
    let spsr: u64;

    unsafe {
        asm!("mrs {}, CurrentEL", out(reg) current_el, options(nomem, nostack, preserves_flags));
        match (current_el >> 2) & 3 {
            1 => {
                asm!("mrs {}, esr_el1", out(reg) esr, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, far_el1", out(reg) far, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, elr_el1", out(reg) elr, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, spsr_el1", out(reg) spsr, options(nomem, nostack, preserves_flags));
            }
            2 => {
                asm!("mrs {}, esr_el2", out(reg) esr, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, far_el2", out(reg) far, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, elr_el2", out(reg) elr, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, spsr_el2", out(reg) spsr, options(nomem, nostack, preserves_flags));
            }
            3 => {
                asm!("mrs {}, esr_el3", out(reg) esr, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, far_el3", out(reg) far, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, elr_el3", out(reg) elr, options(nomem, nostack, preserves_flags));
                asm!("mrs {}, spsr_el3", out(reg) spsr, options(nomem, nostack, preserves_flags));
            }
            _ => {
                esr = 0;
                far = 0;
                elr = 0;
                spsr = 0;
            }
        }
    }

    crate::irq::exception();
    crate::kprintln!("  frame: elr=0x{:016x} spsr=0x{:016x}", elr, spsr);
    crate::crash::fatal(crate::error::KernelError::arch_cpu_exception(
        "aarch64 exception",
        0x20ff,
        "arg0 contains current_el in high nibble and esr in low bits; arg1 is far",
        (((current_el >> 2) & 3) << 60) | esr,
        far,
    ))
}
