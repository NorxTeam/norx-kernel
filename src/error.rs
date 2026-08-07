#[derive(Clone, Copy)]
pub enum ErrorKind {
    Panic,
    CpuException,
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    PageFault,
}

#[derive(Clone, Copy)]
pub struct KernelError {
    pub kind: ErrorKind,
    pub code: u64,
    pub message: &'static str,
    pub detail: &'static str,
    pub arg0: u64,
    pub arg1: u64,
}

impl KernelError {
    pub const fn panic() -> Self {
        Self {
            kind: ErrorKind::Panic,
            code: 0x10,
            message: "kernel panic",
            detail: "panic handler entered",
            arg0: 0,
            arg1: 0,
        }
    }

    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub const fn cpu_exception(message: &'static str, vector: u8, error_code: u64) -> Self {
        Self {
            kind: ErrorKind::CpuException,
            code: 0x2000 | vector as u64,
            message,
            detail: "processor exception",
            arg0: vector as u64,
            arg1: error_code,
        }
    }

    #[cfg_attr(target_arch = "x86_64", allow(dead_code))]
    pub const fn arch_cpu_exception(
        message: &'static str,
        code: u64,
        detail: &'static str,
        arg0: u64,
        arg1: u64,
    ) -> Self {
        Self {
            kind: ErrorKind::CpuException,
            code,
            message,
            detail,
            arg0,
            arg1,
        }
    }

    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub const fn page_fault(address: u64, error_code: u64) -> Self {
        Self {
            kind: ErrorKind::PageFault,
            code: 0x2014,
            message: "page fault",
            detail: "unhandled memory access fault",
            arg0: address,
            arg1: error_code,
        }
    }

    pub fn title(self) -> &'static str {
        match self.kind {
            ErrorKind::Panic => "KERNEL PANIC",
            ErrorKind::CpuException => "CPU EXCEPTION",
            ErrorKind::PageFault => "PAGE FAULT",
        }
    }

    pub fn kind_name(self) -> &'static str {
        match self.kind {
            ErrorKind::Panic => "panic",
            ErrorKind::CpuException => "cpu-exception",
            ErrorKind::PageFault => "page-fault",
        }
    }
}

pub fn report(error: KernelError) {
    crate::kprintln!();
    crate::bootlog::fail_fmt(format_args!(":( {}", error.title()));
    crate::bootlog::info_fmt(format_args!("kind: {}", error.kind_name()));
    crate::bootlog::info_fmt(format_args!("code: 0x{:016x}", error.code));
    crate::bootlog::info_fmt(format_args!("message: {}", error.message));
    crate::bootlog::info_fmt(format_args!("detail: {}", error.detail));
    crate::bootlog::info_fmt(format_args!("arch: {}", crate::arch::NAME));
    crate::bootlog::info_fmt(format_args!("ticks: {}", crate::time::ticks()));
    crate::bootlog::info_fmt(format_args!("arg0: 0x{:016x}", error.arg0));
    crate::bootlog::info_fmt(format_args!("arg1: 0x{:016x}", error.arg1));
    if matches!(error.kind, ErrorKind::PageFault) {
        report_page_fault_code(error.arg0, error.arg1);
    }
    crate::bootlog::fail("SYSTEM HALTED");
}

fn report_page_fault_code(address: u64, code: u64) {
    crate::bootlog::info_fmt(format_args!("address: 0x{:016x}", address));
    crate::bootlog::info_fmt(format_args!(
        "fault: present={} write={} user={} reserved={} instr={} pk={} shadow_stack={} sgx={}",
        yes(code & 1),
        yes(code & 2),
        yes(code & 4),
        yes(code & 8),
        yes(code & 16),
        yes(code & 32),
        yes(code & 64),
        yes(code & 32768),
    ));
}

fn yes(value: u64) -> &'static str {
    if value == 0 {
        "no"
    } else {
        "yes"
    }
}
