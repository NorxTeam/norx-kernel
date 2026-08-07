#[derive(Clone, Copy)]
pub struct Stats {
    pub direct_map_base: usize,
    pub direct_map_bytes: usize,
    pub direct_map_ready: bool,
    pub norx_cr3_ready: bool,
    pub norx_cr3: u64,
    pub lazy_pages: usize,
    pub table_pages_used: usize,
    pub table_pages_total: usize,
}

pub fn init() {
    #[cfg(target_arch = "x86_64")]
    crate::bootlog::start(0, "building direct map");
    #[cfg(target_arch = "aarch64")]
    crate::bootlog::start(0, "checking direct map support");
    #[cfg(target_arch = "x86_64")]
    let _ = crate::arch::paging::init_direct_map();

    let stats_after_map = stats();
    if stats_after_map.direct_map_ready {
        crate::bootlog::ok_fmt(format_args!(
            "direct map ready base 0x{:x}",
            stats_after_map.direct_map_base
        ));
    } else {
        crate::bootlog::warn_fmt(format_args!(
            "direct map planned base 0x{:x}",
            stats_after_map.direct_map_base
        ));
    }

    #[cfg(target_arch = "x86_64")]
    crate::bootlog::start(1, "installing kernel page tables");
    #[cfg(target_arch = "x86_64")]
    let _ = crate::arch::paging::init_norx_cr3();

    let stats = stats();
    if stats.norx_cr3_ready {
        crate::bootlog::ok_fmt(format_args!("norx cr3 ready 0x{:x}", stats.norx_cr3));
    }
}

pub fn stats() -> Stats {
    arch_stats()
}

#[cfg(target_arch = "x86_64")]
fn arch_stats() -> Stats {
    let stats = crate::arch::paging::stats();
    Stats {
        direct_map_base: stats.direct_map_base,
        direct_map_bytes: stats.direct_map_bytes,
        direct_map_ready: stats.direct_map_ready,
        norx_cr3_ready: stats.norx_cr3_ready,
        norx_cr3: stats.norx_cr3,
        lazy_pages: stats.lazy_pages,
        table_pages_used: stats.table_pages_used,
        table_pages_total: stats.table_pages_total,
    }
}

#[cfg(target_arch = "aarch64")]
fn arch_stats() -> Stats {
    Stats {
        direct_map_base: 0xffff_0000_0000_0000,
        direct_map_bytes: 0,
        direct_map_ready: false,
        norx_cr3_ready: false,
        norx_cr3: 0,
        lazy_pages: 0,
        table_pages_used: 0,
        table_pages_total: 0,
    }
}
