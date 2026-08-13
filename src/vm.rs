pub const LAZY_BASE: usize = 0xffff_9000_0000_0000;
const LAZY_PAGES: usize = 16;
const PAGE_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind {
    Translation,
    Protection,
    AccessFlag,
    External,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaultInfo {
    pub address: usize,
    pub raw: u64,
    pub kind: FaultKind,
    pub write: bool,
    pub instruction: bool,
    pub user: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultResult {
    Resolved,
    UserFault(FaultReason),
    KernelFatal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultReason {
    InvalidAddress,
    Protection,
    GuardPage,
    StackOverflow,
    OutOfMemory,
}

impl FaultInfo {
    #[cfg(target_arch = "x86_64")]
    pub const fn x86_page_fault(address: usize, error_code: u64) -> Self {
        Self {
            address,
            raw: error_code,
            kind: if error_code & 1 == 0 {
                FaultKind::Translation
            } else {
                FaultKind::Protection
            },
            write: error_code & (1 << 1) != 0,
            instruction: error_code & (1 << 4) != 0,
            user: error_code & (1 << 2) != 0,
        }
    }

    #[cfg(target_arch = "aarch64")]
    pub const fn aarch64_data_abort(address: usize, syndrome: u64, from_user: bool) -> Self {
        let status = syndrome & 0x3f;
        Self {
            address,
            raw: syndrome,
            kind: match status {
                0b000100..=0b000111 => FaultKind::Translation,
                0b001000..=0b001011 => FaultKind::AccessFlag,
                0b001100..=0b001111 => FaultKind::Protection,
                0b010000 | 0b010001 | 0b010100 | 0b010101 => FaultKind::External,
                _ => FaultKind::Unknown,
            },
            write: syndrome & (1 << 6) != 0,
            instruction: false,
            user: from_user,
        }
    }
}

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

pub fn contract_self_check() -> bool {
    let kernel_missing = FaultInfo {
        address: LAZY_BASE,
        raw: 0,
        kind: FaultKind::Translation,
        write: false,
        instruction: false,
        user: false,
    };
    assert_eq!(
        classify_fault(kernel_missing, false),
        FaultResult::KernelFatal
    );
    let user_missing = FaultInfo {
        address: 0x1234_0000,
        user: true,
        ..kernel_missing
    };
    assert_eq!(
        classify_fault(user_missing, false),
        FaultResult::UserFault(FaultReason::InvalidAddress)
    );

    #[cfg(target_arch = "x86_64")]
    {
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
        let missing = FaultInfo::x86_page_fault(LAZY_BASE, 0);
        assert_eq!(classify_fault(missing, false), FaultResult::KernelFatal);
        let user_missing = FaultInfo::x86_page_fault(0x1234_0000, 4);
        assert_eq!(
            classify_fault(user_missing, false),
            FaultResult::UserFault(FaultReason::InvalidAddress)
        );

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
    {
        let syndrome = (0x24u64 << 26) | (1 << 6) | 0x05;
        let fault = FaultInfo::aarch64_data_abort(0x1234_5000, syndrome, true);
        assert_eq!(fault.kind, FaultKind::Translation);
        assert!(fault.write);
        assert!(fault.user);
        true
    }
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn handle_page_fault(fault: FaultInfo) -> bool {
    if fault.kind != FaultKind::Translation
        || fault.instruction
        || fault.user
        || !is_lazy_address(fault.address)
    {
        return false;
    }
    crate::arch::map_lazy_page(fault.address & !(PAGE_SIZE - 1))
}

pub fn classify_fault(fault: FaultInfo, resolved: bool) -> FaultResult {
    if resolved {
        FaultResult::Resolved
    } else if fault.user {
        if fault.kind == FaultKind::Translation && is_lazy_address(fault.address) {
            FaultResult::UserFault(FaultReason::OutOfMemory)
        } else if fault.kind == FaultKind::Protection {
            FaultResult::UserFault(FaultReason::Protection)
        } else {
            FaultResult::UserFault(FaultReason::InvalidAddress)
        }
    } else {
        FaultResult::KernelFatal
    }
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
fn is_lazy_address(address: usize) -> bool {
    (LAZY_BASE..LAZY_BASE + LAZY_PAGES * PAGE_SIZE).contains(&address)
}
