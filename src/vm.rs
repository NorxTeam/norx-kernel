pub const LAZY_BASE: usize = 0xffff_9000_0000_0000;
const LAZY_PAGES: usize = 16;
const PAGE_SIZE: usize = 4096;

pub fn init() {
    crate::bootlog::start(2, "checking lazy virtual memory");
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

#[cfg(target_arch = "x86_64")]
pub fn contract_self_check() -> bool {
    if !crate::arch::supports_lazy_pages() {
        return false;
    }
    let paging = crate::paging::stats();
    if !paging.direct_map_ready || !paging.norx_cr3_ready {
        return false;
    }

    assert!(is_lazy_address(LAZY_BASE));
    assert!(is_lazy_address(LAZY_BASE + (LAZY_PAGES - 1) * PAGE_SIZE));
    assert!(!is_lazy_address(LAZY_BASE - PAGE_SIZE));
    assert!(!is_lazy_address(LAZY_BASE + LAZY_PAGES * PAGE_SIZE));
    assert!(!handle_page_fault(LAZY_BASE, 1));

    let address = LAZY_BASE as *mut u64;
    let marker = 0x4e4f_5258_4c41_5a59;
    unsafe {
        assert_eq!(core::ptr::read_volatile(address), 0);
        core::ptr::write_volatile(address, marker);
        assert_eq!(core::ptr::read_volatile(address), marker);
    }
    let pte = crate::arch::paging::pte_flags(LAZY_BASE).expect("lazy page PTE");
    assert_ne!(pte & 1, 0);
    assert_ne!(pte & (1 << 1), 0);
    assert_eq!(pte & (1 << 2), 0);
    assert_ne!(pte & (1 << 63), 0);
    let frame = crate::arch::paging::pte_frame(LAZY_BASE).expect("lazy page frame");
    let alias = crate::arch::paging::direct_map_ptr(frame).expect("lazy frame direct map");
    unsafe { assert_eq!(core::ptr::read_volatile(alias.cast::<u64>()), marker) };
    true
}

#[cfg(target_arch = "aarch64")]
pub fn contract_self_check() -> bool {
    false
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn handle_page_fault(address: usize, code: u64) -> bool {
    const UNSUPPORTED: u64 = (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 6) | (1 << 15);
    if code & (1 | UNSUPPORTED) != 0 || !is_lazy_address(address) {
        return false;
    }
    crate::arch::map_lazy_page(address & !(PAGE_SIZE - 1))
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn is_lazy_address(address: usize) -> bool {
    (LAZY_BASE..LAZY_BASE + LAZY_PAGES * PAGE_SIZE).contains(&address)
}
