#[derive(Clone, Copy)]
pub struct Stats {
    pub direct_map_base: usize,
    pub direct_map_bytes: usize,
    pub direct_map_ready: bool,
    pub boa_cr3_ready: bool,
    pub boa_cr3: u64,
    pub lazy_pages: usize,
    pub table_pages_used: usize,
    pub table_pages_total: usize,
    pub user_code_base: usize,
    pub user_stack_top: usize,
}

pub fn init() {
    #[cfg(target_arch = "x86_64")]
    let _ = crate::arch::paging::init_direct_map();
    #[cfg(target_arch = "x86_64")]
    let _ = crate::arch::paging::init_boa_cr3();

    let stats = stats();
    if stats.direct_map_ready {
        crate::bootlog::ok_fmt(format_args!(
            "direct map ready base 0x{:x}",
            stats.direct_map_base
        ));
    } else {
        crate::bootlog::warn_fmt(format_args!(
            "direct map planned base 0x{:x}",
            stats.direct_map_base
        ));
    }
    if stats.boa_cr3_ready {
        crate::bootlog::ok_fmt(format_args!("boa cr3 ready 0x{:x}", stats.boa_cr3));
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
        boa_cr3_ready: stats.boa_cr3_ready,
        boa_cr3: stats.boa_cr3,
        lazy_pages: stats.lazy_pages,
        table_pages_used: stats.table_pages_used,
        table_pages_total: stats.table_pages_total,
        user_code_base: stats.user_code_base,
        user_stack_top: stats.user_stack_top,
    }
}

#[cfg(target_arch = "aarch64")]
fn arch_stats() -> Stats {
    Stats {
        direct_map_base: 0xffff_0000_0000_0000,
        direct_map_bytes: 0,
        direct_map_ready: false,
        boa_cr3_ready: false,
        boa_cr3: 0,
        lazy_pages: 0,
        table_pages_used: 0,
        table_pages_total: 0,
        user_code_base: crate::arch::user::USER_CODE_BASE,
        user_stack_top: crate::arch::user::USER_STACK_TOP,
    }
}
