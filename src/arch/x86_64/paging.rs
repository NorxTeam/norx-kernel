use core::arch::asm;

const LAZY_PAGES: usize = 16;
const TABLE_PAGES: usize = 32;
const HUGE_PAGE_BYTES: usize = 2 * 1024 * 1024;
const DIRECT_MAP_LIMIT: usize = 1usize << 47;
pub const DIRECT_MAP_BASE: usize = 0xffff_8000_0000_0000;

const PTE_PRESENT: u64 = 1 << 0;
const PTE_WRITABLE: u64 = 1 << 1;
const PTE_USER: u64 = 1 << 2;
const PTE_HUGE: u64 = 1 << 7;
const PTE_NX: u64 = 1 << 63;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

#[repr(align(4096))]
#[allow(dead_code)]
#[derive(Clone, Copy)]
struct Page([u64; 512]);

impl Page {
    const fn data() -> Self {
        Self([0xcccc_cccc_cccc_cccc; 512])
    }
}

#[derive(Clone, Copy)]
pub struct Stats {
    pub table_pages_used: usize,
    pub table_pages_total: usize,
    pub lazy_pages: usize,
    pub direct_map_base: usize,
    pub direct_map_bytes: usize,
    pub direct_map_ready: bool,
    pub norx_cr3_ready: bool,
    pub norx_cr3: u64,
}

#[link_section = ".data"]
static mut TABLE_POOL: [Page; TABLE_PAGES] = [Page::data(); TABLE_PAGES];
static mut TABLE_POOL_NEXT: usize = 0;
static mut DIRECT_MAP_READY: bool = false;
static mut DIRECT_MAP_BYTES: usize = 0;
static mut NORX_CR3_READY: bool = false;
static mut NORX_CR3: u64 = 0;

pub fn init_direct_map() -> bool {
    unsafe {
        if DIRECT_MAP_READY {
            return true;
        }
        let firmware_limit = current_cr3().saturating_add(4096);
        let Some(limit) = usize::try_from(crate::memory::physical_limit().max(firmware_limit)).ok()
        else {
            return false;
        };
        let Some(bytes) = limit
            .checked_add(HUGE_PAGE_BYTES - 1)
            .map(|value| value & !(HUGE_PAGE_BYTES - 1))
        else {
            return false;
        };
        if bytes == 0 || bytes > DIRECT_MAP_LIMIT {
            return false;
        }
        let cr0 = disable_write_protect();
        let mut mapped = true;
        let mut physical = 0usize;
        while physical < bytes {
            if !map_huge_to(DIRECT_MAP_BASE + physical, physical as u64) {
                mapped = false;
                break;
            }
            physical += HUGE_PAGE_BYTES;
            if physical.is_multiple_of(HUGE_PAGE_BYTES * 2) {
                crate::bootlog::pulse();
            }
        }
        restore_cr0(cr0);
        if mapped {
            DIRECT_MAP_BYTES = bytes;
        }
        DIRECT_MAP_READY = mapped;
        mapped
    }
}

pub fn map_lazy_page(virtual_address: usize) -> bool {
    if !virtual_address.is_multiple_of(4096)
        || !(crate::vm::LAZY_BASE..crate::vm::LAZY_BASE + LAZY_PAGES * 4096)
            .contains(&virtual_address)
    {
        return false;
    }
    let status = stats();
    if !status.direct_map_ready || !status.norx_cr3_ready {
        return false;
    }
    crate::arch::without_interrupts(|| unsafe { map_page(virtual_address) })
}

pub fn direct_map_ptr(physical: u64) -> Option<*mut u8> {
    unsafe {
        if !DIRECT_MAP_READY || physical as usize >= DIRECT_MAP_BYTES {
            return None;
        }
    }
    Some((DIRECT_MAP_BASE + physical as usize) as *mut u8)
}

pub fn physical_from_direct_map(virtual_address: usize) -> Option<u64> {
    unsafe {
        if !DIRECT_MAP_READY
            || virtual_address < DIRECT_MAP_BASE
            || virtual_address - DIRECT_MAP_BASE >= DIRECT_MAP_BYTES
        {
            return None;
        }
    }
    Some((virtual_address - DIRECT_MAP_BASE) as u64)
}

pub fn stats() -> Stats {
    unsafe {
        Stats {
            table_pages_used: TABLE_POOL_NEXT,
            table_pages_total: TABLE_PAGES,
            lazy_pages: LAZY_PAGES,
            direct_map_base: DIRECT_MAP_BASE,
            direct_map_bytes: DIRECT_MAP_BYTES,
            direct_map_ready: DIRECT_MAP_READY,
            norx_cr3_ready: NORX_CR3_READY,
            norx_cr3: NORX_CR3,
        }
    }
}

pub fn current_cr3_value() -> u64 {
    unsafe { current_cr3() }
}

pub fn switch_cr3(value: u64) {
    unsafe { asm!("mov cr3, {}", in(reg) value, options(nostack, preserves_flags)) };
}

pub fn user_space_prepare(root: crate::address::PhysAddr) -> bool {
    unsafe {
        let Some(destination) = direct_map_ptr(root.value()).map(|ptr| ptr.cast::<u64>()) else {
            return false;
        };
        let Some(source) = direct_map_ptr(NORX_CR3).map(|ptr| ptr.cast::<u64>()) else {
            return false;
        };
        for index in 0..512 {
            destination
                .add(index)
                .write_volatile(source.add(index).read_volatile());
        }
    }
    true
}

pub fn user_space_map(
    root: crate::address::PhysAddr,
    mapping: crate::address_space::MappingInfo,
    tables: &mut [Option<crate::address::PhysAddr>],
) -> bool {
    if !mapping.flags.is_valid()
        || mapping.virtual_address < crate::address_space::PAGE_SIZE
        || mapping.virtual_address >= crate::address_space::USER_LIMIT
        || mapping
            .virtual_address
            .checked_add(crate::address_space::PAGE_SIZE)
            .is_none_or(|end| end > crate::address_space::USER_LIMIT)
        || !mapping.virtual_address.is_multiple_of(4096)
        || !mapping.physical_frame.value().is_multiple_of(4096)
    {
        return false;
    }
    unsafe {
        let Some(mut table) = direct_map_ptr(root.value()).map(|ptr| ptr.cast::<u64>()) else {
            return false;
        };
        let indices = [
            (mapping.virtual_address >> 39) & 0x1ff,
            (mapping.virtual_address >> 30) & 0x1ff,
            (mapping.virtual_address >> 21) & 0x1ff,
            (mapping.virtual_address >> 12) & 0x1ff,
        ];
        for index in indices[..3].iter().copied() {
            let entry = table.add(index);
            let value = entry.read_volatile();
            if value & PTE_PRESENT == 0 {
                let Some(frame) = allocate_user_table(tables) else {
                    return false;
                };
                entry.write_volatile(frame.value() | PTE_PRESENT | PTE_WRITABLE | PTE_USER);
                table = match direct_map_ptr(frame.value()).map(|ptr| ptr.cast::<u64>()) {
                    Some(table) => table,
                    None => return false,
                };
            } else {
                if value & PTE_USER == 0 || value & (1 << 7) != 0 {
                    return false;
                }
                table = match direct_map_ptr((value & ADDRESS_MASK) as usize as u64)
                    .map(|ptr| ptr.cast::<u64>())
                {
                    Some(table) => table,
                    None => return false,
                };
            }
        }
        let leaf = table.add(indices[3]);
        if leaf.read_volatile() & PTE_PRESENT != 0 {
            return false;
        }
        let mut flags = PTE_PRESENT | PTE_USER;
        if mapping.flags.writable {
            flags |= PTE_WRITABLE;
        }
        if !mapping.flags.executable {
            flags |= PTE_NX;
        }
        leaf.write_volatile(mapping.physical_frame.value() | flags);
        asm!("invlpg [{}]", in(reg) mapping.virtual_address, options(nostack, preserves_flags));
    }
    true
}

pub fn user_space_unmap(
    root: crate::address::PhysAddr,
    virtual_address: usize,
    owned_tables: &mut [Option<crate::address::PhysAddr>],
) -> bool {
    unsafe {
        let Some(mut table) = direct_map_ptr(root.value()).map(|ptr| ptr.cast::<u64>()) else {
            return false;
        };
        let indices = [
            (virtual_address >> 39) & 0x1ff,
            (virtual_address >> 30) & 0x1ff,
            (virtual_address >> 21) & 0x1ff,
            (virtual_address >> 12) & 0x1ff,
        ];
        let mut child_tables = [core::ptr::null_mut(); 3];
        let mut parent_entries = [core::ptr::null_mut(); 3];
        for (depth, index) in indices[..3].iter().copied().enumerate() {
            parent_entries[depth] = table.add(index);
            let value = table.add(index).read_volatile();
            if value & PTE_PRESENT == 0 || value & (1 << 7) != 0 {
                return false;
            }
            table = match direct_map_ptr((value & ADDRESS_MASK) as usize as u64)
                .map(|ptr| ptr.cast::<u64>())
            {
                Some(table) => table,
                None => return false,
            };
            child_tables[depth] = table;
        }
        let leaf = table.add(indices[3]);
        if leaf.read_volatile() & PTE_PRESENT == 0 {
            return false;
        }
        leaf.write_volatile(0);
        asm!("invlpg [{}]", in(reg) virtual_address, options(nostack, preserves_flags));
        for depth in (0..3).rev() {
            let child = child_tables[depth];
            let parent_entry = parent_entries[depth];
            let empty = (0..512).all(|index| child.add(index).read_volatile() & PTE_PRESENT == 0);
            if !empty {
                break;
            }
            let child_frame = parent_entry.read_volatile() & ADDRESS_MASK;
            let Some(slot_index) = owned_tables.iter().position(|slot| {
                slot.as_ref()
                    .is_some_and(|frame| frame.value() == child_frame)
            }) else {
                return false;
            };
            let frame = owned_tables[slot_index].expect("owned page table");
            if !crate::memory::free_frame(frame.value()) {
                return false;
            }
            parent_entry.write_volatile(0);
            owned_tables[slot_index] = None;
        }
    }
    true
}

pub fn user_space_reset(root: crate::address::PhysAddr) {
    unsafe {
        if let Some(table) = direct_map_ptr(root.value()).map(|ptr| ptr.cast::<u64>()) {
            for index in 0..512 {
                table.add(index).write_volatile(0);
            }
        }
    }
}

pub fn switch_to_user(root: crate::address::PhysAddr) -> bool {
    if !stats().norx_cr3_ready || direct_map_ptr(root.value()).is_none() {
        return false;
    }
    unsafe { asm!("mov cr3, {}", in(reg) root.value(), options(nostack, preserves_flags)) };
    true
}

pub fn restore_kernel() {
    unsafe {
        if NORX_CR3 != 0 {
            asm!("mov cr3, {}", in(reg) NORX_CR3, options(nostack, preserves_flags));
        }
    }
}

pub fn write_physical(physical: crate::address::PhysAddr, offset: usize, bytes: &[u8]) -> bool {
    if offset >= 4096 || bytes.len() > 4096 - offset {
        return false;
    }
    let Some(pointer) = direct_map_ptr(physical.value().saturating_add(offset as u64)) else {
        return false;
    };
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len()) };
    true
}

pub fn read_physical(physical: crate::address::PhysAddr, offset: usize, bytes: &mut [u8]) -> bool {
    if offset >= 4096 || bytes.len() > 4096 - offset {
        return false;
    }
    let Some(pointer) = direct_map_ptr(physical.value().saturating_add(offset as u64)) else {
        return false;
    };
    unsafe { core::ptr::copy_nonoverlapping(pointer, bytes.as_mut_ptr(), bytes.len()) };
    true
}

pub fn zero_physical_page(physical: crate::address::PhysAddr) -> bool {
    let Some(pointer) = direct_map_ptr(physical.value()) else {
        return false;
    };
    unsafe { core::ptr::write_bytes(pointer, 0, 4096) };
    true
}

unsafe fn allocate_user_table(
    tables: &mut [Option<crate::address::PhysAddr>],
) -> Option<crate::address::PhysAddr> {
    let frame = crate::memory::alloc_frame().map(crate::address::PhysAddr::new)?;
    if !zero_physical_page(frame) {
        let _ = crate::memory::free_frame(frame.value());
        return None;
    }
    let slot = tables.iter().position(Option::is_none)?;
    tables[slot] = Some(frame);
    Some(frame)
}

pub fn init_norx_cr3() -> bool {
    unsafe {
        if NORX_CR3_READY {
            return true;
        }

        let old_cr3 = current_cr3();
        let Some(new_p4) = crate::memory::alloc_frame() else {
            crate::bootlog::fail("cannot allocate new root table frame");
            return false;
        };
        let Some(old) = direct_map_ptr(old_cr3 & ADDRESS_MASK).map(|ptr| ptr.cast::<u64>()) else {
            crate::bootlog::fail_fmt(format_args!(
                "firmware CR3 outside direct map cr3=0x{:x} bytes=0x{:x}",
                old_cr3, DIRECT_MAP_BYTES
            ));
            let _ = crate::memory::free_frame(new_p4);
            return false;
        };
        let Some(new) = direct_map_ptr(new_p4).map(|ptr| ptr.cast::<u64>()) else {
            crate::bootlog::fail_fmt(format_args!(
                "new CR3 outside direct map frame=0x{:x} bytes=0x{:x}",
                new_p4, DIRECT_MAP_BYTES
            ));
            let _ = crate::memory::free_frame(new_p4);
            return false;
        };
        core::ptr::write_bytes(new, 0, 512);
        for i in 0..512 {
            new.add(i).write_volatile(old.add(i).read_volatile());
        }
        NORX_CR3 = new_p4;
        asm!("mov cr3, {}", in(reg) new_p4, options(nostack, preserves_flags));
        NORX_CR3_READY = true;
        true
    }
}

pub fn pte_flags(virtual_address: usize) -> Option<u64> {
    unsafe {
        let mut cr3: u64;
        asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
        let p4 = direct_map_ptr(cr3 & ADDRESS_MASK)?.cast::<u64>() as *const u64;

        let p4_i = (virtual_address >> 39) & 0x1ff;
        let p3_i = (virtual_address >> 30) & 0x1ff;
        let p2_i = (virtual_address >> 21) & 0x1ff;
        let p1_i = (virtual_address >> 12) & 0x1ff;

        let e4 = *p4.add(p4_i);
        if e4 & PTE_PRESENT == 0 {
            return None;
        }
        let p3 = direct_map_ptr(e4 & ADDRESS_MASK)?.cast::<u64>() as *const u64;
        let e3 = p3.add(p3_i).read_volatile();
        if e3 & PTE_PRESENT == 0 || e3 & (1 << 7) != 0 {
            return Some(e3);
        }
        let p2 = direct_map_ptr(e3 & ADDRESS_MASK)?.cast::<u64>() as *const u64;
        let e2 = p2.add(p2_i).read_volatile();
        if e2 & PTE_PRESENT == 0 || e2 & (1 << 7) != 0 {
            return Some(e2);
        }
        let p1 = direct_map_ptr(e2 & ADDRESS_MASK)?.cast::<u64>() as *const u64;
        let e1 = p1.add(p1_i).read_volatile();
        if e1 & PTE_PRESENT == 0 {
            None
        } else {
            Some(e1)
        }
    }
}

pub fn pte_frame(virtual_address: usize) -> Option<u64> {
    let entry = pte_flags(virtual_address)?;
    (entry & PTE_PRESENT != 0 && entry & PTE_HUGE == 0).then_some(entry & ADDRESS_MASK)
}

unsafe fn map_page(virtual_address: usize) -> bool {
    let cr0 = disable_write_protect();
    let mapped = map_page_inner(virtual_address);
    restore_cr0(cr0);
    mapped
}

unsafe fn map_page_inner(virtual_address: usize) -> bool {
    let lazy_index = (virtual_address - crate::vm::LAZY_BASE) / 4096;
    let Some(frame) = lazy_frame(lazy_index) else {
        return false;
    };

    if map_to(virtual_address, frame) {
        true
    } else {
        let _ = crate::memory::free_frame(frame);
        false
    }
}

unsafe fn map_to(virtual_address: usize, frame: u64) -> bool {
    map_to_flags(virtual_address, frame, PTE_PRESENT | PTE_WRITABLE | PTE_NX)
}

unsafe fn map_huge_to(virtual_address: usize, frame: u64) -> bool {
    let p4_i = (virtual_address >> 39) & 0x1ff;
    let p3_i = (virtual_address >> 30) & 0x1ff;
    let p2_i = (virtual_address >> 21) & 0x1ff;

    let cr3 = current_cr3();
    let p4 = (cr3 & 0x000f_ffff_ffff_f000) as *mut u64;
    let Some(p3) = next_table(p4.add(p4_i)) else {
        return false;
    };
    let Some(p2) = next_table(p3.add(p3_i)) else {
        return false;
    };
    let entry = p2.add(p2_i);
    if *entry & PTE_PRESENT != 0 {
        return false;
    }
    *entry = frame | PTE_PRESENT | PTE_WRITABLE | PTE_HUGE;
    true
}

unsafe fn map_to_flags(virtual_address: usize, frame: u64, flags: u64) -> bool {
    let p4_i = (virtual_address >> 39) & 0x1ff;
    let p3_i = (virtual_address >> 30) & 0x1ff;
    let p2_i = (virtual_address >> 21) & 0x1ff;
    let p1_i = (virtual_address >> 12) & 0x1ff;

    let cr3 = current_cr3();
    let Some(p4) = direct_map_ptr(cr3 & ADDRESS_MASK).map(|ptr| ptr.cast::<u64>()) else {
        return false;
    };

    let Some(p3) = next_runtime_table(p4.add(p4_i)) else {
        return false;
    };
    let Some(p2) = next_runtime_table(p3.add(p3_i)) else {
        return false;
    };
    let Some(p1) = next_runtime_table(p2.add(p2_i)) else {
        return false;
    };

    let entry = p1.add(p1_i);
    if *entry & PTE_PRESENT != 0 {
        return false;
    }
    *entry = frame | flags;
    asm!("invlpg [{}]", in(reg) virtual_address, options(nostack, preserves_flags));
    true
}

unsafe fn current_cr3() -> u64 {
    let cr3: u64;
    asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
    cr3
}

unsafe fn disable_write_protect() -> u64 {
    let cr0: u64;
    asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack, preserves_flags));
    asm!(
        "mov cr0, {}",
        in(reg) cr0 & !(1 << 16),
        options(nomem, nostack, preserves_flags)
    );
    cr0
}

unsafe fn restore_cr0(cr0: u64) {
    asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack, preserves_flags));
}

unsafe fn next_table(entry: *mut u64) -> Option<*mut u64> {
    if *entry & 1 == 0 {
        let frame = table_frame()?;
        make_mapping_writable(frame as usize);
        zero_page(frame);
        *entry = frame | PTE_PRESENT | PTE_WRITABLE;
    }
    Some((*entry & 0x000f_ffff_ffff_f000) as *mut u64)
}

unsafe fn next_runtime_table(entry: *mut u64) -> Option<*mut u64> {
    let value = entry.read_volatile();
    if value & PTE_HUGE != 0 {
        return None;
    }
    if value & PTE_PRESENT == 0 {
        let frame = crate::memory::alloc_frame()?;
        let Some(table) = direct_map_ptr(frame).map(|ptr| ptr.cast::<u64>()) else {
            let _ = crate::memory::free_frame(frame);
            return None;
        };
        core::ptr::write_bytes(table, 0, 512);
        entry.write_volatile(frame | PTE_PRESENT | PTE_WRITABLE);
        return Some(table);
    }
    direct_map_ptr(value & ADDRESS_MASK).map(|ptr| ptr.cast::<u64>())
}

unsafe fn table_frame() -> Option<u64> {
    if TABLE_POOL_NEXT == TABLE_PAGES {
        return None;
    }
    let frame = (&raw mut TABLE_POOL[TABLE_POOL_NEXT]) as u64;
    TABLE_POOL_NEXT += 1;
    Some(frame)
}

unsafe fn lazy_frame(index: usize) -> Option<u64> {
    if index >= LAZY_PAGES {
        return None;
    }
    let frame = crate::memory::alloc_frame()?;
    if zero_physical_frame(frame).is_none() {
        let _ = crate::memory::free_frame(frame);
        return None;
    }
    Some(frame)
}

unsafe fn zero_page(virtual_address: u64) {
    let ptr = virtual_address as *mut u64;
    for i in 0..512 {
        ptr.add(i).write_volatile(0);
    }
}

unsafe fn zero_physical_frame(frame: u64) -> Option<()> {
    let ptr = direct_map_ptr(frame)?.cast::<u64>();
    for i in 0..512 {
        ptr.add(i).write_volatile(0);
    }
    Some(())
}

unsafe fn make_mapping_writable(address: usize) -> bool {
    let mut cr3: u64;
    asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
    let p4 = (cr3 & 0x000f_ffff_ffff_f000) as *mut u64;

    let p4_i = (address >> 39) & 0x1ff;
    let p3_i = (address >> 30) & 0x1ff;
    let p2_i = (address >> 21) & 0x1ff;
    let p1_i = (address >> 12) & 0x1ff;

    let e4 = p4.add(p4_i);
    if *e4 & 1 == 0 {
        return false;
    }
    *e4 |= 0b10;

    let p3 = (*e4 & 0x000f_ffff_ffff_f000) as *mut u64;
    let e3 = p3.add(p3_i);
    if *e3 & 1 == 0 {
        return false;
    }
    *e3 |= 0b10;
    if *e3 & (1 << 7) != 0 {
        asm!("invlpg [{}]", in(reg) address, options(nostack, preserves_flags));
        return true;
    }

    let p2 = (*e3 & 0x000f_ffff_ffff_f000) as *mut u64;
    let e2 = p2.add(p2_i);
    if *e2 & 1 == 0 {
        return false;
    }
    *e2 |= 0b10;
    if *e2 & (1 << 7) != 0 {
        asm!("invlpg [{}]", in(reg) address, options(nostack, preserves_flags));
        return true;
    }

    let p1 = (*e2 & 0x000f_ffff_ffff_f000) as *mut u64;
    let e1 = p1.add(p1_i);
    if *e1 & 1 == 0 {
        return false;
    }
    *e1 |= 0b10;
    asm!("invlpg [{}]", in(reg) address, options(nostack, preserves_flags));
    true
}
