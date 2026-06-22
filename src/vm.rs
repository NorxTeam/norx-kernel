pub const LAZY_BASE: usize = 0x4000_0000_0000;
const LAZY_PAGES: usize = 16;
const PAGE_SIZE: usize = 4096;

#[derive(Clone, Copy)]
pub struct Stats {
    pub lazy_base: usize,
    pub lazy_pages: usize,
    pub lazy_supported: bool,
}

pub fn init() {
    if crate::arch::supports_lazy_pages() {
        crate::bootlog::ok_fmt(format_args!(
            "lazy vm range 0x{:x}..0x{:x}",
            LAZY_BASE,
            LAZY_BASE + LAZY_PAGES * PAGE_SIZE
        ));
    } else {
        crate::bootlog::warn("lazy vm fault mapper unavailable on this arch");
    }
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn handle_page_fault(address: usize, code: u64) -> bool {
    if code & 1 != 0 || !is_lazy_address(address) {
        return false;
    }
    crate::arch::map_lazy_page(address & !(PAGE_SIZE - 1))
}

pub fn lazy_probe() -> Option<u8> {
    if !crate::arch::supports_lazy_pages() {
        return None;
    }
    let ptr = LAZY_BASE as *mut u8;
    unsafe {
        ptr.write_volatile(0x5a);
        Some(ptr.read_volatile())
    }
}

pub fn stats() -> Stats {
    Stats {
        lazy_base: LAZY_BASE,
        lazy_pages: LAZY_PAGES,
        lazy_supported: crate::arch::supports_lazy_pages(),
    }
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn is_lazy_address(address: usize) -> bool {
    (LAZY_BASE..LAZY_BASE + LAZY_PAGES * PAGE_SIZE).contains(&address)
}
