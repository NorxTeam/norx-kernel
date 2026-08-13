use core::arch::asm;

const PAGE_SIZE: u64 = 4096;
const TABLE_ENTRIES: usize = 512;
const ADDRESS_MASK: u64 = 0x0000_ffff_ffff_f000;
const PHYSICAL_MASK: u64 = 0x0000_ffff_ffff_ffff;
const VALID: u64 = 1;
const TABLE_OR_PAGE: u64 = 1 << 1;
const ATTR_NORMAL: u64 = 1 << 2;
const INNER_SHAREABLE: u64 = 3 << 8;
const ACCESS_FLAG: u64 = 1 << 10;
const AP_USER_RW: u64 = 1 << 6;
const AP_USER_RO: u64 = 3 << 6;
const PXN: u64 = 1 << 53;
const UXN: u64 = 1 << 54;

static mut TABLES_USED: usize = 0;
static mut KERNEL_TTBR0: u64 = 0;

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
        asm!(
            "mrs {}, ttbr0_el1",
            out(reg) ttbr0,
            options(nomem, nostack, preserves_flags)
        );
        asm!(
            "mrs {}, ttbr1_el1",
            out(reg) ttbr1,
            options(nomem, nostack, preserves_flags)
        );
        asm!(
            "mrs {}, tcr_el1",
            out(reg) tcr,
            options(nomem, nostack, preserves_flags)
        );
        asm!(
            "mrs {}, mair_el1",
            out(reg) mair,
            options(nomem, nostack, preserves_flags)
        );
        asm!(
            "mrs {}, sctlr_el1",
            out(reg) sctlr,
            options(nomem, nostack, preserves_flags)
        );
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

pub fn current_ttbr0_value() -> u64 {
    read_ttbr0()
}

pub fn switch_ttbr0(value: u64) {
    write_ttbr0(value);
}

pub fn physical_to_virtual(address: crate::address::PhysAddr) -> Option<crate::address::VirtAddr> {
    (address.value() <= usize::MAX as u64)
        .then_some(crate::address::VirtAddr::new(address.value() as usize))
}

pub fn virtual_to_physical(address: crate::address::VirtAddr) -> Option<crate::address::PhysAddr> {
    Some(crate::address::PhysAddr::new(address.value() as u64))
}

pub fn user_space_prepare(root: crate::address::PhysAddr) -> bool {
    let current = read_ttbr0() & ADDRESS_MASK;
    let kernel = unsafe {
        if KERNEL_TTBR0 == 0 {
            current
        } else {
            KERNEL_TTBR0
        }
    };
    if kernel == 0 || root.value() == 0 || root.value() == kernel {
        return false;
    }
    if !copy_table(kernel, root.value()) {
        return false;
    }
    unsafe {
        KERNEL_TTBR0 = kernel;
        TABLES_USED = 0;
    }
    true
}

pub fn user_space_map(
    root: crate::address::PhysAddr,
    mapping: crate::address_space::MappingInfo,
    tables: &mut [Option<crate::address::PhysAddr>],
) -> bool {
    if !mapping.flags.user
        || !mapping.virtual_address.is_multiple_of(PAGE_SIZE as usize)
        || mapping.virtual_address >= crate::address_space::USER_LIMIT
        || !mapping.physical_frame.is_aligned(PAGE_SIZE as usize)
    {
        return false;
    }

    let indices = [
        (mapping.virtual_address >> 39) & 0x1ff,
        (mapping.virtual_address >> 30) & 0x1ff,
        (mapping.virtual_address >> 21) & 0x1ff,
        (mapping.virtual_address >> 12) & 0x1ff,
    ];
    let mut table = root.value();
    for index in indices[..3].iter().copied() {
        let Some(table_ptr) = table_ptr(table) else {
            return false;
        };
        let descriptor = unsafe { table_ptr.add(index).read_volatile() };
        if descriptor & VALID != 0 {
            if descriptor & TABLE_OR_PAGE != TABLE_OR_PAGE {
                return false;
            }
            let next = descriptor & ADDRESS_MASK;
            if !owned_table(tables, next) {
                return false;
            }
            table = next;
        } else {
            let Some(next) = allocate_table(tables) else {
                return false;
            };
            unsafe {
                table_ptr
                    .add(index)
                    .write_volatile(next | VALID | TABLE_OR_PAGE)
            };
            table = next;
        }
    }

    let Some(leaf_table_ptr) = table_ptr(table) else {
        return false;
    };
    let slot = unsafe { leaf_table_ptr.add(indices[3]) };
    if unsafe { slot.read_volatile() } & VALID != 0 {
        return false;
    }
    let mut descriptor = mapping.physical_frame.value() & ADDRESS_MASK;
    descriptor |= VALID | TABLE_OR_PAGE | ATTR_NORMAL | INNER_SHAREABLE | ACCESS_FLAG | PXN;
    descriptor |= if mapping.flags.writable {
        AP_USER_RW
    } else {
        AP_USER_RO
    };
    if !mapping.flags.executable {
        descriptor |= UXN;
    }
    unsafe { slot.write_volatile(descriptor) };
    flush_tlb();
    true
}

pub fn user_space_unmap(
    root: crate::address::PhysAddr,
    virtual_address: usize,
    owned_tables: &mut [Option<crate::address::PhysAddr>],
) -> bool {
    if !virtual_address.is_multiple_of(PAGE_SIZE as usize) {
        return false;
    }
    let indices = [
        (virtual_address >> 39) & 0x1ff,
        (virtual_address >> 30) & 0x1ff,
        (virtual_address >> 21) & 0x1ff,
        (virtual_address >> 12) & 0x1ff,
    ];
    let mut table = root.value();
    let mut child_tables = [0u64; 3];
    let mut parent_entries = [core::ptr::null_mut(); 3];
    for (depth, index) in indices[..3].iter().copied().enumerate() {
        let Some(table_ptr) = table_ptr(table) else {
            return false;
        };
        parent_entries[depth] = unsafe { table_ptr.add(index) };
        let descriptor = unsafe { table_ptr.add(index).read_volatile() };
        if descriptor & (VALID | TABLE_OR_PAGE) != (VALID | TABLE_OR_PAGE) {
            return false;
        }
        table = descriptor & ADDRESS_MASK;
        if !owned_table(owned_tables, table) {
            return false;
        }
        child_tables[depth] = table;
    }
    let Some(leaf_table_ptr) = table_ptr(table) else {
        return false;
    };
    let slot = unsafe { leaf_table_ptr.add(indices[3]) };
    if unsafe { slot.read_volatile() } & VALID == 0 {
        return false;
    }
    unsafe { slot.write_volatile(0) };
    flush_tlb();
    for depth in (0..3).rev() {
        let Some(child_table_ptr) = table_ptr(child_tables[depth]) else {
            break;
        };
        let empty = unsafe {
            (0..TABLE_ENTRIES).all(|index| child_table_ptr.add(index).read_volatile() & VALID == 0)
        };
        if !empty {
            break;
        }
        let entry = parent_entries[depth];
        let Some(slot_index) = owned_tables.iter().position(|slot| {
            slot.as_ref()
                .is_some_and(|frame| frame.value() == child_tables[depth])
        }) else {
            return false;
        };
        let frame = owned_tables[slot_index].expect("owned page table");
        if !crate::memory::free_frame(frame.value()) {
            return false;
        }
        unsafe { entry.write_volatile(0) };
        owned_tables[slot_index] = None;
    }
    true
}

pub fn user_space_reset(root: crate::address::PhysAddr) {
    if (read_ttbr0() & ADDRESS_MASK) == root.value() {
        let kernel = unsafe { KERNEL_TTBR0 };
        if kernel != 0 {
            write_ttbr0(kernel);
        }
    }
    unsafe { TABLES_USED = 0 };
}

pub fn switch_to_user(root: crate::address::PhysAddr) -> bool {
    if root.value() == 0 || unsafe { KERNEL_TTBR0 } == 0 {
        return false;
    }
    write_ttbr0(root.value());
    true
}

pub fn restore_kernel_address_space() {
    let kernel = unsafe { KERNEL_TTBR0 };
    if kernel != 0 && (read_ttbr0() & ADDRESS_MASK) != kernel {
        write_ttbr0(kernel);
    }
}

pub fn write_physical(physical: crate::address::PhysAddr, offset: usize, bytes: &[u8]) -> bool {
    if offset > PAGE_SIZE as usize || bytes.len() > PAGE_SIZE as usize - offset {
        return false;
    }
    let Some(ptr) = physical_ptr(physical.value().saturating_add(offset as u64)) else {
        return false;
    };
    unsafe {
        for (index, byte) in bytes.iter().copied().enumerate() {
            ptr.add(index).write_volatile(byte);
        }
    }
    true
}

pub fn read_physical(physical: crate::address::PhysAddr, offset: usize, bytes: &mut [u8]) -> bool {
    if offset > PAGE_SIZE as usize || bytes.len() > PAGE_SIZE as usize - offset {
        return false;
    }
    let Some(ptr) = physical_ptr(physical.value().saturating_add(offset as u64)) else {
        return false;
    };
    unsafe {
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = ptr.add(index).read_volatile();
        }
    }
    true
}

pub fn zero_physical_page(physical: crate::address::PhysAddr) -> bool {
    let Some(ptr) = physical_ptr(physical.value()) else {
        return false;
    };
    unsafe { core::ptr::write_bytes(ptr, 0, PAGE_SIZE as usize) };
    true
}

fn read_ttbr0() -> u64 {
    let value: u64;
    unsafe {
        asm!(
            "mrs {}, ttbr0_el1",
            out(reg) value,
            options(nomem, nostack, preserves_flags)
        );
    }
    value
}

fn write_ttbr0(value: u64) {
    unsafe {
        asm!(
            "dsb ishst",
            "msr ttbr0_el1, {}",
            "dsb ish",
            "isb",
            in(reg) value,
            options(nostack, preserves_flags)
        );
    }
    flush_tlb();
}

fn flush_tlb() {
    unsafe {
        asm!(
            "tlbi vmalle1",
            "dsb ish",
            "isb",
            options(nomem, nostack, preserves_flags)
        );
    }
}

fn copy_table(source: u64, destination: u64) -> bool {
    let (Some(source), Some(destination)) = (table_ptr(source), table_ptr(destination)) else {
        return false;
    };
    unsafe {
        for index in 0..TABLE_ENTRIES {
            destination
                .add(index)
                .write_volatile(source.add(index).read_volatile());
        }
    }
    true
}

fn allocate_table(tables: &mut [Option<crate::address::PhysAddr>]) -> Option<u64> {
    let slot = tables.iter_mut().find(|entry| entry.is_none())?;
    let frame = crate::memory::alloc_frame()?;
    if !zero_physical_page(crate::address::PhysAddr::new(frame)) {
        let _ = crate::memory::free_frame(frame);
        return None;
    }
    *slot = Some(crate::address::PhysAddr::new(frame));
    unsafe { TABLES_USED = TABLES_USED.saturating_add(1) };
    Some(frame)
}

fn owned_table(tables: &[Option<crate::address::PhysAddr>], frame: u64) -> bool {
    tables.iter().flatten().any(|entry| entry.value() == frame)
}

fn table_ptr(frame: u64) -> Option<*mut u64> {
    physical_ptr(frame).map(|ptr| ptr.cast::<u64>())
}

fn physical_ptr(address: u64) -> Option<*mut u8> {
    (address & !PHYSICAL_MASK == 0).then_some(address as *mut u8)
}
