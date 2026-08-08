use core::arch::asm;

const LAZY_PAGES: usize = 16;
const TABLE_PAGES: usize = 32;
const DIRECT_MAP_BYTES: usize = 16 * 1024 * 1024;
pub const DIRECT_MAP_BASE: usize = 0xffff_8000_0000_0000;

const PTE_PRESENT: u64 = 1 << 0;
const PTE_WRITABLE: u64 = 1 << 1;
const PTE_USER: u64 = 1 << 2;
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
static mut NORX_CR3_READY: bool = false;
static mut NORX_CR3: u64 = 0;

pub fn init_direct_map() -> bool {
    unsafe {
        if DIRECT_MAP_READY {
            return true;
        }
        let cr0 = disable_write_protect();
        let mut mapped = true;
        let mut physical = 0usize;
        while physical < DIRECT_MAP_BYTES {
            if !map_to(DIRECT_MAP_BASE + physical, physical as u64) {
                mapped = false;
                break;
            }
            physical += 4096;
            if physical.is_multiple_of(4096 * 256) {
                crate::bootlog::pulse();
            }
        }
        restore_cr0(cr0);
        DIRECT_MAP_READY = mapped;
        mapped
    }
}

pub fn map_lazy_page(virtual_address: usize) -> bool {
    unsafe { map_page(virtual_address) }
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
    if !mapping.virtual_address.is_multiple_of(4096)
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

pub fn user_space_unmap(root: crate::address::PhysAddr, virtual_address: usize) -> bool {
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
        for index in indices[..3].iter().copied() {
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
        }
        let leaf = table.add(indices[3]);
        if leaf.read_volatile() & PTE_PRESENT == 0 {
            return false;
        }
        leaf.write_volatile(0);
        asm!("invlpg [{}]", in(reg) virtual_address, options(nostack, preserves_flags));
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
        if TABLE_POOL_NEXT == TABLE_PAGES {
            return false;
        }
        let new_p4_address = (&raw mut TABLE_POOL[TABLE_POOL_NEXT]) as usize;
        let cr0 = disable_write_protect();
        if !make_mapping_writable(new_p4_address) {
            restore_cr0(cr0);
            return false;
        }
        let Some(new_p4) = table_frame() else {
            restore_cr0(cr0);
            return false;
        };
        let old = (old_cr3 & 0x000f_ffff_ffff_f000) as *const u64;
        let new = new_p4 as *mut u64;
        for i in 0..512 {
            new.add(i).write_volatile(old.add(i).read_volatile());
        }
        restore_cr0(cr0);
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
        let p4 = (cr3 & 0x000f_ffff_ffff_f000) as *const u64;

        let p4_i = (virtual_address >> 39) & 0x1ff;
        let p3_i = (virtual_address >> 30) & 0x1ff;
        let p2_i = (virtual_address >> 21) & 0x1ff;
        let p1_i = (virtual_address >> 12) & 0x1ff;

        let e4 = *p4.add(p4_i);
        if e4 & PTE_PRESENT == 0 {
            return None;
        }
        let p3 = (e4 & 0x000f_ffff_ffff_f000) as *const u64;
        let e3 = *p3.add(p3_i);
        if e3 & PTE_PRESENT == 0 || e3 & (1 << 7) != 0 {
            return Some(e3);
        }
        let p2 = (e3 & 0x000f_ffff_ffff_f000) as *const u64;
        let e2 = *p2.add(p2_i);
        if e2 & PTE_PRESENT == 0 || e2 & (1 << 7) != 0 {
            return Some(e2);
        }
        let p1 = (e2 & 0x000f_ffff_ffff_f000) as *const u64;
        let e1 = *p1.add(p1_i);
        if e1 & PTE_PRESENT == 0 {
            None
        } else {
            Some(e1)
        }
    }
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

    map_to(virtual_address, frame)
}

unsafe fn map_to(virtual_address: usize, frame: u64) -> bool {
    map_to_flags(virtual_address, frame, PTE_PRESENT | PTE_WRITABLE)
}

unsafe fn map_to_flags(virtual_address: usize, frame: u64, flags: u64) -> bool {
    let p4_i = (virtual_address >> 39) & 0x1ff;
    let p3_i = (virtual_address >> 30) & 0x1ff;
    let p2_i = (virtual_address >> 21) & 0x1ff;
    let p1_i = (virtual_address >> 12) & 0x1ff;

    let cr3 = current_cr3();
    let p4 = (cr3 & 0x000f_ffff_ffff_f000) as *mut u64;

    let Some(p3) = next_table(p4.add(p4_i)) else {
        return false;
    };
    let Some(p2) = next_table(p3.add(p3_i)) else {
        return false;
    };
    let Some(p1) = next_table(p2.add(p2_i)) else {
        return false;
    };

    *p1.add(p1_i) = frame | flags;
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
    zero_physical_frame(frame)?;
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
