use core::arch::asm;

static mut TABLES_USED: usize = 0;

#[derive(Clone, Copy)]
pub struct Status {
    pub ttbr0: u64,
    pub ttbr1: u64,
    pub tcr: u64,
    pub mair: u64,
    pub sctlr: u64,
    pub tables_used: usize,
}

pub fn status() -> Status {
    let ttbr0: u64;
    let ttbr1: u64;
    let tcr: u64;
    let mair: u64;
    let sctlr: u64;
    unsafe {
        asm!("mrs {}, ttbr0_el1", out(reg) ttbr0, options(nomem, nostack, preserves_flags));
        asm!("mrs {}, ttbr1_el1", out(reg) ttbr1, options(nomem, nostack, preserves_flags));
        asm!("mrs {}, tcr_el1", out(reg) tcr, options(nomem, nostack, preserves_flags));
        asm!("mrs {}, mair_el1", out(reg) mair, options(nomem, nostack, preserves_flags));
        asm!("mrs {}, sctlr_el1", out(reg) sctlr, options(nomem, nostack, preserves_flags));
    }
    Status {
        ttbr0,
        ttbr1,
        tcr,
        mair,
        sctlr,
        tables_used: unsafe { TABLES_USED },
    }
}
