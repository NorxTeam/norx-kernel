use core::{
    arch::{asm, global_asm},
    ptr,
};

global_asm!(
    r#"
    .global boa_x86_64_user_probe_finish
boa_x86_64_user_probe_finish:
    mov qword ptr [rip + BOA_X86_64_USER_PROBE_VALUE], 42
    mov rsp, qword ptr [rip + BOA_X86_64_USER_PROBE_RETURN_RSP]
    mov rax, qword ptr [rip + BOA_X86_64_USER_PROBE_VALUE]
    mov r11, qword ptr [rip + BOA_X86_64_USER_PROBE_RETURN_RIP]
    mov byte ptr [rip + BOA_X86_64_USER_PROBE_DONE], 0
    mov byte ptr [rip + BOA_X86_64_USER_PROBE_ACTIVE], 0
    mov dx, 0x10
    mov ds, dx
    mov es, dx
    jmp r11

    .global boa_x86_64_user_probe_trap
boa_x86_64_user_probe_trap:
    mov qword ptr [rip + BOA_X86_64_USER_PROBE_VALUE], rdi
    mov rsp, qword ptr [rip + BOA_X86_64_USER_PROBE_RETURN_RSP]
    mov rax, qword ptr [rip + BOA_X86_64_USER_PROBE_VALUE]
    mov r11, qword ptr [rip + BOA_X86_64_USER_PROBE_RETURN_RIP]
    mov byte ptr [rip + BOA_X86_64_USER_PROBE_DONE], 0
    mov byte ptr [rip + BOA_X86_64_USER_PROBE_ACTIVE], 0
    mov dx, 0x10
    mov ds, dx
    mov es, dx
    jmp r11

    .global boa_x86_64_int80_entry
boa_x86_64_int80_entry:
    inc qword ptr [rip + BOA_X86_64_INT80_HITS]
    mov qword ptr [rip + BOA_X86_64_INT80_LAST_OP], rax
    cmp byte ptr [rip + BOA_X86_64_USER_PROBE_ACTIVE], 0
    je .Lboa_int80_full
    cmp rax, 1
    jne .Lboa_int80_full
    inc qword ptr [rip + BOA_X86_64_INT80_FAST_HITS]
    mov rdi, 1
    mov rsi, 42
    xor rdx, rdx
    xor rcx, rcx
    xor r8, r8
    xor r9, r9
    call boa_x86_64_syscall_rust
    mov rsp, qword ptr [rip + BOA_X86_64_USER_PROBE_RETURN_RSP]
    mov r11, qword ptr [rip + BOA_X86_64_USER_PROBE_RETURN_RIP]
    mov byte ptr [rip + BOA_X86_64_USER_PROBE_ACTIVE], 0
    mov byte ptr [rip + BOA_X86_64_USER_PROBE_DONE], 0
    mov dx, 0x10
    mov ds, dx
    mov es, dx
    jmp r11
.Lboa_int80_full:
    push rdi
    push rsi
    push rdx
    push r10
    push r8
    push r9
    push rcx
    push r11
    sub rsp, 8
    mov r9, r8
    mov r8, r10
    mov rcx, rdx
    mov rdx, rsi
    mov rsi, rdi
    mov rdi, rax
    call boa_x86_64_syscall_rust
    cmp byte ptr [rip + BOA_X86_64_USER_PROBE_DONE], 0
    jne .Lboa_int80_done
    add rsp, 8
    pop r11
    pop rcx
    pop r9
    pop r8
    pop r10
    pop rdx
    pop rsi
    pop rdi
    iretq
.Lboa_int80_done:
    mov rsp, qword ptr [rip + BOA_X86_64_USER_PROBE_RETURN_RSP]
    mov rax, qword ptr [rip + BOA_X86_64_USER_PROBE_VALUE]
    mov r11, qword ptr [rip + BOA_X86_64_USER_PROBE_RETURN_RIP]
    mov byte ptr [rip + BOA_X86_64_USER_PROBE_DONE], 0
    mov byte ptr [rip + BOA_X86_64_USER_PROBE_ACTIVE], 0
    mov dx, 0x10
    mov ds, dx
    mov es, dx
    jmp r11
"#
);

extern "C" {
    fn boa_x86_64_int80_entry();
    fn boa_x86_64_user_probe_finish() -> !;
    static mut BOA_X86_64_USER_PROBE_FAULT_RIP: u64;
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

    fn set_user_addr(&mut self, addr: u64) {
        self.set_addr(addr);
        self.options = 0xee00;
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
pub const USER_DATA_SELECTOR: u16 = 0x18 | 3;
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

#[derive(Clone, Copy)]
pub struct TssStatus {
    pub ready: bool,
    pub selector: u16,
    pub rsp0: u64,
    pub stack_top: u64,
    pub stack_size: usize,
}

#[derive(Clone, Copy)]
pub struct TransitionPages {
    pub gdt: usize,
    pub idt: usize,
    pub tss: usize,
    pub int80: usize,
}

#[derive(Clone, Copy)]
pub struct CpuTables {
    pub gdtr_base: u64,
    pub idtr_base: u64,
    pub tr: u16,
}

#[derive(Clone, Copy)]
pub struct GateStatus {
    pub offset: u64,
    pub selector: u16,
    pub options: u16,
}

#[derive(Clone, Copy)]
pub struct TssDescriptorStatus {
    pub base: u64,
    pub limit: u32,
    pub access: u8,
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

pub fn user_segments_ready() -> bool {
    USER_CODE_SELECTOR == 0x23 && USER_DATA_SELECTOR == 0x1b && tss_status().ready
}

pub fn tss_status() -> TssStatus {
    unsafe {
        let tss = ptr::addr_of!(TSS);
        let rsp0 = ptr::addr_of!((*tss).rsp).cast::<u64>().read_unaligned();
        let stack_top = kernel_stack_top();
        TssStatus {
            ready: TSS_READY,
            selector: TSS_SELECTOR,
            rsp0,
            stack_top,
            stack_size: KERNEL_STACK_SIZE,
        }
    }
}

pub fn transition_pages() -> TransitionPages {
    TransitionPages {
        gdt: (&raw const GDT) as usize,
        idt: (&raw const IDT) as usize,
        tss: (&raw const TSS) as usize,
        int80: boa_x86_64_int80_entry as *const () as usize,
    }
}

pub fn cpu_tables() -> CpuTables {
    unsafe {
        let mut gdtr = Pointer { limit: 0, base: 0 };
        let mut idtr = Pointer { limit: 0, base: 0 };
        let tr: u16;
        asm!("sgdt [{}]", in(reg) &mut gdtr, options(nostack, preserves_flags));
        asm!("sidt [{}]", in(reg) &mut idtr, options(nostack, preserves_flags));
        asm!("str ax", out("ax") tr, options(nomem, nostack, preserves_flags));
        CpuTables {
            gdtr_base: ptr::addr_of!(gdtr.base).read_unaligned(),
            idtr_base: ptr::addr_of!(idtr.base).read_unaligned(),
            tr,
        }
    }
}

pub fn int80_gate() -> GateStatus {
    unsafe {
        let entry = (&raw const IDT).cast::<IdtEntry>().add(0x80);
        let offset_low = ptr::addr_of!((*entry).offset_low).read_unaligned() as u64;
        let offset_mid = ptr::addr_of!((*entry).offset_mid).read_unaligned() as u64;
        let offset_high = ptr::addr_of!((*entry).offset_high).read_unaligned() as u64;
        GateStatus {
            offset: offset_low | (offset_mid << 16) | (offset_high << 32),
            selector: ptr::addr_of!((*entry).selector).read_unaligned(),
            options: ptr::addr_of!((*entry).options).read_unaligned(),
        }
    }
}

pub fn tss_descriptor() -> TssDescriptorStatus {
    unsafe {
        let gdt = (&raw const GDT).cast::<u64>();
        let low = gdt.add(TSS_GDT_INDEX).read_volatile();
        let high = gdt.add(TSS_GDT_INDEX + 1).read_volatile();
        let limit = ((low & 0xffff) | (((low >> 48) & 0x0f) << 16)) as u32;
        let base = ((low >> 16) & 0xffff)
            | (((low >> 32) & 0xff) << 16)
            | (((low >> 56) & 0xff) << 24)
            | ((high & 0xffff_ffff) << 32);
        TssDescriptorStatus {
            base,
            limit,
            access: ((low >> 40) & 0xff) as u8,
        }
    }
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
        (*idt.add(0x80)).set_user_addr(boa_x86_64_int80_entry as *const () as usize as u64);
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
    if stack.code_segment == USER_CODE_SELECTOR as u64 && crate::arch::syscall::user_probe_active()
    {
        unsafe {
            BOA_X86_64_USER_PROBE_FAULT_RIP = stack.instruction_pointer;
            boa_x86_64_user_probe_finish()
        };
    }
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
