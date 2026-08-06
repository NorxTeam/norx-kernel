use core::{arch::asm, ptr};

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
const KERNEL_STACK_SIZE: usize = 16 * 1024;

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
static mut TSS: TaskStateSegment = TaskStateSegment::empty();
static mut TSS_READY: bool = false;
static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];

pub fn init() {
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) };
    load_gdt();
    load_idt();
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) };
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

pub fn syscall_stack_top() -> u64 {
    unsafe { ptr::addr_of!(TSS.rsp).cast::<u64>().read_unaligned() }
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

fn kernel_stack_top() -> u64 {
    ((&raw const KERNEL_STACK) as u64) + KERNEL_STACK_SIZE as u64
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
        (*idt.add(14)).set_err(page_fault);
        (*idt.add(32)).set(timer_interrupt);
        (*idt.add(33)).set(keyboard_interrupt);
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
    crate::irq::timer();
    crate::sched::on_timer_tick();
    super::end_timer_interrupt();
}

extern "x86-interrupt" fn keyboard_interrupt(_stack: InterruptStackFrame) {
    crate::irq::keyboard();
    crate::drivers::keyboard::handle_interrupt();
    super::end_timer_interrupt();
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

extern "x86-interrupt" fn page_fault(stack: InterruptStackFrame, code: u64) {
    let address = super::fault_address();
    if crate::vm::handle_page_fault(address, code) {
        return;
    }
    crate::irq::exception();
    crate::kprintln!(
        "  frame: rip=0x{:016x} rsp=0x{:016x} cs=0x{:016x} ss=0x{:016x}",
        stack.instruction_pointer,
        stack.stack_pointer,
        stack.code_segment,
        stack.stack_segment
    );
    if let Some(flags) = super::paging::pte_flags(address) {
        crate::kprintln!("  pte: 0x{:016x}", flags);
    } else {
        crate::kprintln!("  pte: not-present");
    }
    crate::crash::fatal(crate::error::KernelError::page_fault(address as u64, code));
}
