use core::arch::{asm, global_asm};

#[repr(C, align(16))]
struct KernelStack([u8; 512 * 1024]);

#[no_mangle]
static mut norx_aarch64_kernel_stack: KernelStack = KernelStack([0; 512 * 1024]);

#[no_mangle]
static mut norx_aarch64_vector_saved_x16: u64 = 0;

global_asm!(
    r#"
    .align 11
    .global norx_exception_vectors
norx_exception_vectors:
    b norx_aarch64_sync_exception
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_sync_exception
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_sync_exception
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124
    b norx_aarch64_exception_entry
    .space 124

    .global norx_aarch64_exception_entry
norx_aarch64_exception_entry:
    mov x17, #1
    msr spsel, x17
    adrp x17, norx_aarch64_kernel_stack
    add x17, x17, :lo12:norx_aarch64_kernel_stack
    mov x18, #8
    lsl x18, x18, #16
    add x17, x17, x18
    mov sp, x17
    b norx_aarch64_exception

    .global norx_aarch64_sync_exception
norx_aarch64_sync_exception:
    mrs x17, spsr_el1
    and x17, x17, #0xf
    cbnz x17, 1f
    adrp x18, norx_aarch64_user_depth
    add x18, x18, :lo12:norx_aarch64_user_depth
    ldr x17, [x18]
    cmp x17, #1
    b.lo norx_aarch64_user_stack_fault
    cmp x17, #4
    b.hs norx_aarch64_user_stack_fault
    adrp x18, norx_aarch64_vector_saved_x16
    add x18, x18, :lo12:norx_aarch64_vector_saved_x16
    str x16, [x18]
    mov x17, #1
    msr spsel, x17
    adrp x17, norx_aarch64_kernel_stack
    add x17, x17, :lo12:norx_aarch64_kernel_stack
    adrp x18, norx_aarch64_user_depth
    add x18, x18, :lo12:norx_aarch64_user_depth
    ldr x18, [x18]
    lsl x18, x18, #17
    mov x16, #8
    lsl x16, x16, #16
    add x17, x17, x16
    sub x17, x17, x18
    mov sp, x17
    adrp x17, norx_aarch64_vector_saved_x16
    add x17, x17, :lo12:norx_aarch64_vector_saved_x16
    ldr x16, [x17]
1:
    sub sp, sp, #160
    stp x1, x2, [sp, #0]
    stp x3, x4, [sp, #16]
    stp x5, x6, [sp, #32]
    stp x7, x8, [sp, #48]
    stp x9, x10, [sp, #64]
    stp x11, x12, [sp, #80]
    stp x13, x14, [sp, #96]
    stp x29, x30, [sp, #112]
    stp x15, x16, [sp, #128]
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
    b.eq 6f
    cmp x12, #0x24
    b.eq 5f
    cmp x12, #0x25
    b.eq 5f
    b 9f
5:
    cmp x9, #1
    b.eq 51f
    cmp x9, #2
    b.eq 52f
    cmp x9, #3
    b.eq 53f
    b 9f
51:
    mrs x0, elr_el1
    bl norx_aarch64_usercopy_fault
    cbz x0, 9f
    msr elr_el1, x0
    b 7f
52:
    mrs x0, elr_el2
    bl norx_aarch64_usercopy_fault
    cbz x0, 9f
    msr elr_el2, x0
    b 7f
53:
    mrs x0, elr_el3
    bl norx_aarch64_usercopy_fault
    cbz x0, 9f
    msr elr_el3, x0
    b 7f
6:
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
    mov x9, #-2
    cmp x0, x9
    b.eq 8f
    ldp x1, x2, [sp, #0]
    ldp x3, x4, [sp, #16]
    ldp x5, x6, [sp, #32]
    ldp x7, x8, [sp, #48]
    ldp x9, x10, [sp, #64]
    ldp x11, x12, [sp, #80]
    ldp x13, x14, [sp, #96]
    ldp x29, x30, [sp, #112]
    ldp x15, x16, [sp, #128]
    add sp, sp, #160
    eret
8:
    adrp x9, norx_aarch64_user_depth
    add x9, x9, :lo12:norx_aarch64_user_depth
    ldr x10, [x9]
    cbz x10, norx_aarch64_user_stack_fault
    sub x10, x10, #1
    str x10, [x9]
    lsl x12, x10, #3
    adrp x9, norx_aarch64_user_return_sp_stack
    add x9, x9, :lo12:norx_aarch64_user_return_sp_stack
    ldr x10, [x9, x12]
    adrp x9, norx_aarch64_user_return_pc_stack
    add x9, x9, :lo12:norx_aarch64_user_return_pc_stack
    ldr x11, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x19_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x19_stack
    ldr x19, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x20_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x20_stack
    ldr x20, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x21_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x21_stack
    ldr x21, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x22_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x22_stack
    ldr x22, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x23_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x23_stack
    ldr x23, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x24_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x24_stack
    ldr x24, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x25_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x25_stack
    ldr x25, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x26_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x26_stack
    ldr x26, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x27_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x27_stack
    ldr x27, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x28_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x28_stack
    ldr x28, [x9, x12]
    adrp x9, norx_aarch64_kernel_return_x29_stack
    add x9, x9, :lo12:norx_aarch64_kernel_return_x29_stack
    ldr x29, [x9, x12]
    adrp x9, norx_aarch64_user_return_elr_stack
    add x9, x9, :lo12:norx_aarch64_user_return_elr_stack
    ldr x13, [x9, x12]
    adrp x9, norx_aarch64_user_return_spsr_stack
    add x9, x9, :lo12:norx_aarch64_user_return_spsr_stack
    ldr x14, [x9, x12]
    msr elr_el1, x13
    msr spsr_el1, x14
    mov x30, x11
    mov sp, x10
    ret
7:
    ldp x1, x2, [sp, #0]
    ldp x3, x4, [sp, #16]
    ldp x5, x6, [sp, #32]
    ldp x7, x8, [sp, #48]
    ldp x9, x10, [sp, #64]
    ldp x11, x12, [sp, #80]
    ldp x13, x14, [sp, #96]
    ldp x29, x30, [sp, #112]
    ldp x15, x16, [sp, #128]
    mov x0, #14
    add sp, sp, #160
    eret
9:
    add sp, sp, #160
    b norx_aarch64_exception_entry
"#
);

extern "C" {
    static norx_exception_vectors: u8;
}

pub fn init() -> bool {
    let current_el: u64;
    let vectors = unsafe { &norx_exception_vectors as *const u8 as u64 };
    crate::bootlog::ok_fmt(format_args!("aarch64 exception vectors=0x{:x}", vectors));

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
    true
}

pub fn current_el() -> u8 {
    let value: u64;
    unsafe {
        asm!(
            "mrs {}, CurrentEL",
            out(reg) value,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((value >> 2) & 3) as u8
}

#[no_mangle]
extern "C" fn norx_aarch64_kernel_stack_top() -> u64 {
    core::ptr::addr_of!(norx_aarch64_kernel_stack) as u64
        + core::mem::size_of::<KernelStack>() as u64
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
