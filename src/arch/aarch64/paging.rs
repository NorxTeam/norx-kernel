use core::arch::asm;

const DESC_VALID: u64 = 1 << 0;
const DESC_TABLE: u64 = 1 << 1;
const ATTR_NORMAL: u64 = 3 << 2;
const AP_EL0_RW: u64 = 1 << 6;
const AP_EL0_RO: u64 = 3 << 6;
const SH_INNER: u64 = 3 << 8;
const AF: u64 = 1 << 10;
const PXN: u64 = 1 << 53;
const UXN: u64 = 1 << 54;
const ADDR_MASK: u64 = 0x0000_ffff_ffff_f000;

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

#[derive(Clone, Copy)]
pub struct Lookup {
    pub level: u8,
    pub descriptor: u64,
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

pub fn map_user_probe(code_frame: u64, stack_frame: u64) -> bool {
    if code_frame == 0 || stack_frame == 0 {
        return false;
    }
    unsafe {
        let code = map_page(
            crate::arch::user::USER_CODE_BASE,
            code_frame,
            ATTR_NORMAL | AP_EL0_RO | SH_INNER | AF | PXN | DESC_VALID | DESC_TABLE,
        );
        let stack = map_page(
            crate::arch::user::USER_STACK_TOP - 4096,
            stack_frame,
            ATTR_NORMAL | AP_EL0_RW | SH_INNER | AF | PXN | UXN | DESC_VALID | DESC_TABLE,
        );
        if code && stack {
            asm!(
                "dsb ishst; tlbi vmalle1; dsb ish; isb",
                options(nostack, preserves_flags)
            );
        }
        code && stack
    }
}

pub fn lookup(virtual_address: usize) -> Option<Lookup> {
    unsafe {
        let mut table = (status().ttbr0 & 0x0000_ffff_ffff_f000) as *const u64;
        for level in 0..4 {
            let index = (virtual_address >> (39 - level * 9)) & 0x1ff;
            let descriptor = table.add(index).read_volatile();
            if descriptor & 1 == 0 {
                return Some(Lookup { level, descriptor });
            }
            if level == 3 || descriptor & 0b10 == 0 {
                return Some(Lookup { level, descriptor });
            }
            table = (descriptor & 0x0000_ffff_ffff_f000) as *const u64;
        }
    }
    None
}

unsafe fn map_page(virtual_address: usize, frame: u64, flags: u64) -> bool {
    let root = (status().ttbr0 & ADDR_MASK) as *mut u64;
    let Some(l1) = next_table(root.add((virtual_address >> 39) & 0x1ff)) else {
        return false;
    };
    let Some(l2) = next_table(l1.add((virtual_address >> 30) & 0x1ff)) else {
        return false;
    };
    let Some(l3) = next_table(l2.add((virtual_address >> 21) & 0x1ff)) else {
        return false;
    };
    l3.add((virtual_address >> 12) & 0x1ff)
        .write_volatile((frame & ADDR_MASK) | flags);
    true
}

unsafe fn next_table(entry: *mut u64) -> Option<*mut u64> {
    let descriptor = entry.read_volatile();
    if descriptor & DESC_VALID == 0 {
        let frame = crate::memory::alloc_frame()?;
        zero_frame(frame);
        entry.write_volatile((frame & ADDR_MASK) | DESC_VALID | DESC_TABLE);
        TABLES_USED += 1;
        return Some(frame as *mut u64);
    }
    if descriptor & DESC_TABLE == 0 {
        return None;
    }
    Some((descriptor & ADDR_MASK) as *mut u64)
}

unsafe fn zero_frame(frame: u64) {
    let ptr = frame as *mut u64;
    for i in 0..512 {
        ptr.add(i).write_volatile(0);
    }
}
