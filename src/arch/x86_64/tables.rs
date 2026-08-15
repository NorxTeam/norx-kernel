use core::{
    arch::{asm, global_asm},
    ptr,
};

global_asm!(
    r#"
    .section .text,"ax"
    .global norx_page_fault_entry
norx_page_fault_entry:
    push rax
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push rbx
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    mov r15, qword ptr [rsp + 120]
    mov rbx, rsp
    and rsp, -16
    cld
    mov rdi, qword ptr [rbx + 128]
    mov rsi, r15
    call norx_page_fault_dispatch
    mov r15, rax
    mov rax, r15
    test rax, rax
    jz 1f
    cmp rax, -4099
    je 2f
    cmp rax, -4100
    je 3f
    mov qword ptr [rbx + 128], rax
    mov rsp, rbx
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbx
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rax
    add rsp, 8
    iretq
2:
    mov rdx, qword ptr [rip + PAGE_FAULT_TARGET_CONTEXT]
    test rdx, rdx
    jz 1f
    mov rsp, rbx
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
3:
    mov rax, qword ptr [rip + USER_RETURN_DEPTH]
    test rax, rax
    jz 1f
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
1:
    mov rsp, rbx
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbx
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rax
    ud2
"#
);

extern "C" {
    fn norx_page_fault_entry();
}

#[repr(C, packed)]
struct Pointer {
    limit: u16,
    base: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    options: u16,
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            options: 0,
            offset_mid: 0,
            offset_high: 0,
            zero: 0,
        }
    }

    fn set(&mut self, handler: extern "x86-interrupt" fn(InterruptStackFrame)) {
        self.set_addr(handler as usize as u64);
    }

    fn set_err(&mut self, handler: extern "x86-interrupt" fn(InterruptStackFrame, u64)) {
        self.set_addr(handler as usize as u64);
    }

    fn set_addr(&mut self, addr: u64) {
        self.offset_low = addr as u16;
        self.selector = KERNEL_CODE_SELECTOR;
        self.options = 0x8e00;
        self.offset_mid = (addr >> 16) as u16;
        self.offset_high = (addr >> 32) as u32;
    }
}

#[repr(C)]
pub struct InterruptStackFrame {
    instruction_pointer: u64,
    code_segment: u64,
    cpu_flags: u64,
    stack_pointer: u64,
    stack_segment: u64,
}

pub const KERNEL_CODE_SELECTOR: u16 = 0x08;
pub const KERNEL_DATA_SELECTOR: u16 = 0x10;
pub const USER_CODE_SELECTOR: u16 = 0x20 | 3;
pub const TSS_SELECTOR: u16 = 0x28;

const TSS_GDT_INDEX: usize = 5;
pub const KERNEL_STACK_SIZE: usize = 512 * 1024;
pub const SYSCALL_STACK_SIZE: usize = 512 * 1024;

#[repr(C, align(16))]
struct KernelStack([u8; KERNEL_STACK_SIZE]);

#[repr(C, packed)]
struct TaskStateSegment {
    reserved0: u32,
    rsp: [u64; 3],
    reserved1: u64,
    ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    iomap_base: u16,
}

impl TaskStateSegment {
    const fn empty() -> Self {
        Self {
            reserved0: 0,
            rsp: [0; 3],
            reserved1: 0,
            ist: [0; 7],
            reserved2: 0,
            reserved3: 0,
            iomap_base: 0,
        }
    }
}

static mut GDT: [u64; 7] = [
    0,
    0x00af9a000000ffff,
    0x00af92000000ffff,
    0x00aff2000000ffff,
    0x00affa000000ffff,
    0,
    0,
];

static mut KERNEL_STACK: KernelStack = KernelStack([0; KERNEL_STACK_SIZE]);
static mut SYSCALL_STACK: KernelStack = KernelStack([0; SYSCALL_STACK_SIZE]);
static mut TSS: TaskStateSegment = TaskStateSegment::empty();
static mut TSS_READY: bool = false;
static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];

pub fn init() -> bool {
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) };
    load_gdt();
    load_idt();
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) };
    let timer_registration = crate::irq::register_system(
        crate::drivers::framework::IrqKind::Legacy,
        0,
        32,
        timer_hard,
        Some(timer_deferred),
    );
    if let Ok(id) = timer_registration {
        crate::irq::register_timer(id);
    }
    let timer = timer_registration.is_ok();
    timer && crate::drivers::ps2::register_irqs()
}

fn load_gdt() {
    init_tss();
    let ptr = Pointer {
        limit: (core::mem::size_of::<[u64; 7]>() - 1) as u16,
        base: (&raw const GDT) as u64,
    };

    unsafe {
        asm!("lgdt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
        asm!(
            "push 0x08",
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            out("rax") _,
            options(preserves_flags)
        );
        asm!(
            "mov ds, ax",
            "mov es, ax",
            "mov ss, ax",
            in("ax") KERNEL_DATA_SELECTOR,
            options(nostack, preserves_flags)
        );
        asm!("ltr ax", in("ax") TSS_SELECTOR, options(nostack, preserves_flags));
    }
}

pub fn kernel_stack_top() -> u64 {
    ((&raw const KERNEL_STACK) as u64) + KERNEL_STACK_SIZE as u64
}

pub fn syscall_stack_top() -> u64 {
    ((&raw const SYSCALL_STACK) as u64) + SYSCALL_STACK_SIZE as u64
}

fn init_tss() {
    unsafe {
        let stack_top = kernel_stack_top();
        let tss = ptr::addr_of_mut!(TSS);
        ptr::addr_of_mut!((*tss).rsp)
            .cast::<u64>()
            .write_unaligned(stack_top);
        ptr::addr_of_mut!((*tss).iomap_base)
            .write_unaligned(core::mem::size_of::<TaskStateSegment>() as u16);

        let base = tss as u64;
        let limit = (core::mem::size_of::<TaskStateSegment>() - 1) as u64;
        let low = (limit & 0xffff)
            | ((base & 0xffff) << 16)
            | (((base >> 16) & 0xff) << 32)
            | (0x89u64 << 40)
            | (((limit >> 16) & 0x0f) << 48)
            | (((base >> 24) & 0xff) << 56);
        let high = base >> 32;

        let gdt = (&raw mut GDT).cast::<u64>();
        *gdt.add(TSS_GDT_INDEX) = low;
        *gdt.add(TSS_GDT_INDEX + 1) = high;
        TSS_READY = true;
    }
}

fn load_idt() {
    unsafe {
        let idt = (&raw mut IDT).cast::<IdtEntry>();
        for i in 0..256 {
            (*idt.add(i)).set(spurious);
        }
        for i in 0..32 {
            (*idt.add(i)).set(exception);
        }
        (*idt.add(0)).set(divide_error);
        (*idt.add(3)).set(breakpoint);
        (*idt.add(6)).set(invalid_opcode);
        (*idt.add(8)).set_err(double_fault);
        (*idt.add(13)).set_err(general_protection);
        (*idt.add(14)).set_addr(norx_page_fault_entry as *const () as u64);
        (*idt.add(32)).set(timer_interrupt);
        (*idt.add(33)).set(keyboard_interrupt);
        (*idt.add(34)).set(legacy_irq_2_interrupt);
        (*idt.add(35)).set(legacy_irq_3_interrupt);
        (*idt.add(36)).set(legacy_irq_4_interrupt);
        (*idt.add(37)).set(legacy_irq_5_interrupt);
        (*idt.add(38)).set(legacy_irq_6_interrupt);
        (*idt.add(39)).set(legacy_irq_7_interrupt);
        (*idt.add(40)).set(legacy_irq_8_interrupt);
        (*idt.add(41)).set(legacy_irq_9_interrupt);
        (*idt.add(42)).set(legacy_irq_10_interrupt);
        (*idt.add(43)).set(legacy_irq_11_interrupt);
        (*idt.add(44)).set(mouse_interrupt);
        (*idt.add(45)).set(legacy_irq_13_interrupt);
        (*idt.add(46)).set(legacy_irq_14_interrupt);
        (*idt.add(47)).set(legacy_irq_15_interrupt);
        let ptr = Pointer {
            limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: (&raw const IDT) as u64,
        };
        asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
    }
}

extern "x86-interrupt" fn exception(_stack: InterruptStackFrame) {
    crate::irq::exception();
    crate::crash::fatal(crate::error::KernelError::cpu_exception(
        "unhandled exception",
        0xff,
        0,
    ));
}

extern "x86-interrupt" fn spurious(_stack: InterruptStackFrame) {
    crate::irq::spurious();
}

extern "x86-interrupt" fn timer_interrupt(_stack: InterruptStackFrame) {
    crate::irq::dispatch(32);
    super::end_timer_interrupt();
}

extern "x86-interrupt" fn keyboard_interrupt(_stack: InterruptStackFrame) {
    crate::irq::dispatch(33);
    super::end_legacy_interrupt(1);
}

extern "x86-interrupt" fn mouse_interrupt(_stack: InterruptStackFrame) {
    crate::irq::dispatch(44);
    super::end_legacy_interrupt(12);
}

macro_rules! legacy_irq_interrupt {
    ($name:ident, $vector:literal, $line:literal) => {
        extern "x86-interrupt" fn $name(_stack: InterruptStackFrame) {
            crate::irq::dispatch($vector);
            super::end_legacy_interrupt($line);
        }
    };
}

legacy_irq_interrupt!(legacy_irq_2_interrupt, 34, 2);
legacy_irq_interrupt!(legacy_irq_3_interrupt, 35, 3);
legacy_irq_interrupt!(legacy_irq_4_interrupt, 36, 4);
legacy_irq_interrupt!(legacy_irq_5_interrupt, 37, 5);
legacy_irq_interrupt!(legacy_irq_6_interrupt, 38, 6);
legacy_irq_interrupt!(legacy_irq_7_interrupt, 39, 7);
legacy_irq_interrupt!(legacy_irq_8_interrupt, 40, 8);
legacy_irq_interrupt!(legacy_irq_9_interrupt, 41, 9);
legacy_irq_interrupt!(legacy_irq_10_interrupt, 42, 10);
legacy_irq_interrupt!(legacy_irq_11_interrupt, 43, 11);
legacy_irq_interrupt!(legacy_irq_13_interrupt, 45, 13);
legacy_irq_interrupt!(legacy_irq_14_interrupt, 46, 14);
legacy_irq_interrupt!(legacy_irq_15_interrupt, 47, 15);

fn timer_hard() -> bool {
    crate::irq::timer();
    true
}

fn timer_deferred() {
    for _ in 0..crate::irq::take_timer_ticks(32) {
        crate::sched::on_timer_tick();
    }
    if crate::irq::timer_pending() {
        crate::irq::requeue_timer();
    }
}

extern "x86-interrupt" fn divide_error(_stack: InterruptStackFrame) {
    crate::irq::exception();
    crate::crash::fatal(crate::error::KernelError::cpu_exception(
        "divide error",
        0,
        0,
    ));
}

extern "x86-interrupt" fn breakpoint(_stack: InterruptStackFrame) {
    crate::irq::exception();
    crate::crash::fatal(crate::error::KernelError::cpu_exception("breakpoint", 3, 0));
}

extern "x86-interrupt" fn invalid_opcode(stack: InterruptStackFrame) {
    crate::irq::exception();
    crate::kprintln!(
        "  frame: rip=0x{:016x} rsp=0x{:016x} cs=0x{:016x} ss=0x{:016x}",
        stack.instruction_pointer,
        stack.stack_pointer,
        stack.code_segment,
        stack.stack_segment
    );
    let (depth, stack_top, saved_rsp, saved_return) =
        crate::arch::x86_64::syscall::return_debug_state();
    crate::kprintln!(
        "  x86 user-return: depth={} stack_top=0x{:016x} saved_rsp=0x{:016x} saved_return=0x{:016x}",
        depth,
        stack_top,
        saved_rsp,
        saved_return
    );
    crate::crash::fatal(crate::error::KernelError::cpu_exception(
        "invalid opcode",
        6,
        0,
    ));
}

extern "x86-interrupt" fn double_fault(_stack: InterruptStackFrame, code: u64) {
    crate::irq::exception();
    crate::crash::fatal(crate::error::KernelError::cpu_exception(
        "double fault",
        8,
        code,
    ));
}

extern "x86-interrupt" fn general_protection(_stack: InterruptStackFrame, code: u64) {
    crate::irq::exception();
    crate::crash::fatal(crate::error::KernelError::cpu_exception(
        "general protection fault",
        13,
        code,
    ));
}

#[no_mangle]
extern "C" fn norx_page_fault_dispatch(rip: u64, code: u64) -> u64 {
    let address = super::fault_address();
    if crate::usercopy::handles_fault(rip) {
        return crate::usercopy::recovery_address();
    }
    let fault = crate::vm::FaultInfo::x86_page_fault(address, code);
    if crate::vm::handle_page_fault(fault) {
        return rip;
    }
    if fault.user {
        match crate::vm::handle_user_fault(fault) {
            crate::vm::FaultResult::Resolved => return rip,
            crate::vm::FaultResult::UserFault(_) => {
                let result = crate::arch::syscall::terminate_user_fault();
                if result != 0 {
                    return result;
                }
            }
            crate::vm::FaultResult::KernelFatal => {}
        }
    }
    crate::irq::exception();
    crate::kprintln!("  frame: rip=0x{:016x}", rip);
    if let Some(flags) = super::paging::pte_flags(address) {
        crate::kprintln!("  pte: 0x{:016x}", flags);
    } else {
        crate::kprintln!("  pte: not-present");
    }
    crate::crash::fatal(crate::error::KernelError::page_fault(address as u64, code));
}
