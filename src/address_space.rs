#![allow(
    dead_code,
    reason = "Staged ABI and hardware hooks are not all enabled in the bootstrap profile"
)]

use crate::address::PhysAddr;
use crate::process::ProcessId;

pub const PAGE_SIZE: usize = 4096;

#[cfg(target_arch = "x86_64")]
pub const USER_LIMIT: usize = 0x0000_8000_0000_0000;
#[cfg(target_arch = "aarch64")]
pub const USER_LIMIT: usize = 0x0000_1000_0000_0000;

const MAX_MAPPINGS: usize = 128;
const MAX_TABLE_FRAMES: usize = 192;
const INITIAL_STACK_PAGES: usize = 2;
const MAX_STACK_PAGES: usize = 8;
const SIGNAL_TRAMPOLINE_WORDS: usize = 2;
const DEFAULT_PAGE_QUOTA: usize = 16 * 1024;
const MAX_SHARED_FRAMES: usize = MAX_MAPPINGS * 4;

#[cfg(target_arch = "x86_64")]
const SIGNAL_TRAMPOLINE: [u8; SIGNAL_TRAMPOLINE_WORDS * 4] = [0xb8, 15, 0, 0, 0, 0x0f, 0x05, 0xcc];
#[cfg(target_arch = "aarch64")]
const SIGNAL_TRAMPOLINE: [u8; SIGNAL_TRAMPOLINE_WORDS * 4] =
    [0xe8, 0x01, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageFlags {
    pub user: bool,
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
}

impl PageFlags {
    pub const USER_READ: Self = Self {
        user: true,
        readable: true,
        writable: false,
        executable: false,
    };
    pub const USER_RW: Self = Self {
        user: true,
        readable: true,
        writable: true,
        executable: false,
    };
    pub const USER_RX: Self = Self {
        user: true,
        readable: true,
        writable: false,
        executable: true,
    };

    pub(crate) fn validate(self) -> Result<(), Error> {
        if !self.user {
            return Err(Error::KernelMapping);
        }
        if self.writable && self.executable {
            return Err(Error::WriteExecute);
        }
        if !self.readable {
            return Err(Error::NoAccess);
        }
        Ok(())
    }

    pub(crate) const fn is_valid(self) -> bool {
        self.user && self.readable && !(self.writable && self.executable)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingKind {
    Anonymous,
    Stack,
    Heap,
    SignalTrampoline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingInfo {
    pub virtual_address: usize,
    pub physical_frame: PhysAddr,
    pub present: bool,
    pub flags: PageFlags,
    pub kind: MappingKind,
    pub cow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mapping {
    virtual_address: usize,
    physical_frame: Option<PhysAddr>,
    flags: PageFlags,
    kind: MappingKind,
    shared: bool,
    cow: bool,
}

#[derive(Clone, Copy)]
struct SharedFrame {
    frame: PhysAddr,
    references: u16,
}

static SHARED_FRAMES: crate::sync::SmpLock<[Option<SharedFrame>; MAX_SHARED_FRAMES]> =
    crate::sync::SmpLock::new([None; MAX_SHARED_FRAMES]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NoFrames,
    NoSpace,
    MappingCapacity,
    InvalidAddress,
    Unaligned,
    Overlap,
    GuardPage,
    KernelMapping,
    WriteExecute,
    NoAccess,
    InvalidState,
    FrameReturn,
    HardwareUnavailable,
    HardwareMapping,
    QuotaExceeded,
    CopyOnWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AslrHook {
    seed: u64,
}

impl AslrHook {
    const WINDOW: usize = 256 * 1024 * 1024;

    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    pub fn choose_base(self, hint: usize, span: usize, alignment: usize) -> Option<usize> {
        if span == 0 || !alignment.is_power_of_two() {
            return None;
        }
        let span = align_up(span, PAGE_SIZE)?;
        let alignment_mask = alignment - 1;
        let slots = (Self::WINDOW / alignment).max(1);
        let jitter = self.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let offset = ((jitter >> 12) as usize % slots).checked_mul(alignment)?;
        let base = hint.checked_add(offset)? & !alignment_mask;
        let end = base.checked_add(span)?;
        if base < PAGE_SIZE || end > USER_LIMIT {
            return None;
        }
        Some(base)
    }

    pub fn choose_down(
        self,
        top: usize,
        floor: usize,
        span: usize,
        alignment: usize,
    ) -> Option<usize> {
        if span == 0 || !alignment.is_power_of_two() || floor > top {
            return None;
        }
        let span = align_up(span, PAGE_SIZE)?;
        let top = top & !(alignment - 1);
        let floor = align_up(floor, alignment)?;
        let available = top.checked_sub(floor)?;
        if span > available {
            return None;
        }
        let slots = (available - span) / alignment + 1;
        let jitter = self.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let offset = (jitter >> 12) as usize % slots;
        top.checked_sub(span)?
            .checked_sub(offset.checked_mul(alignment)?)
    }
}

#[derive(Debug)]
pub struct AddressSpace {
    owner: Option<ProcessId>,
    root_frame: Option<PhysAddr>,
    mappings: [Option<Mapping>; MAX_MAPPINGS],
    stack_top: usize,
    stack_guard: Option<usize>,
    stack_pages: usize,
    aslr: AslrHook,
    active: bool,
    table_frames: [Option<PhysAddr>; MAX_TABLE_FRAMES],
    page_quota: usize,
    reserved_pages: usize,
}

impl AddressSpace {
    pub fn new(owner: ProcessId, aslr: AslrHook) -> Result<Self, Error> {
        Self::new_with_quota(owner, aslr, DEFAULT_PAGE_QUOTA)
    }

    pub fn new_with_quota(
        owner: ProcessId,
        aslr: AslrHook,
        page_quota: usize,
    ) -> Result<Self, Error> {
        if page_quota == 0 {
            return Err(Error::QuotaExceeded);
        }
        let root_frame = crate::memory::alloc_frame()
            .map(PhysAddr::new)
            .ok_or(Error::NoFrames)?;
        if !crate::arch::zero_physical_page(root_frame) {
            let _ = crate::memory::free_frame(root_frame.value());
            return Err(Error::HardwareUnavailable);
        }
        Ok(Self {
            owner: Some(owner),
            root_frame: Some(root_frame),
            mappings: [None; MAX_MAPPINGS],
            stack_top: aslr
                .choose_base(
                    USER_LIMIT.saturating_sub(AslrHook::WINDOW),
                    PAGE_SIZE,
                    PAGE_SIZE,
                )
                .unwrap_or(USER_LIMIT - PAGE_SIZE),
            stack_guard: None,
            stack_pages: 0,
            aslr,
            active: false,
            table_frames: [None; MAX_TABLE_FRAMES],
            page_quota,
            reserved_pages: 0,
        })
    }

    pub fn owner(&self) -> Option<ProcessId> {
        self.owner
    }

    pub fn root_frame(&self) -> Option<PhysAddr> {
        self.root_frame
    }

    pub fn aslr_base(&self, hint: usize, span: usize, alignment: usize) -> Option<usize> {
        self.aslr.choose_base(hint, span, alignment)
    }

    pub fn stack_top(&self) -> usize {
        self.stack_top
    }

    pub fn page_quota(&self) -> usize {
        self.page_quota
    }

    pub fn mapped_pages(&self) -> usize {
        self.reserved_pages
    }

    pub fn set_page_quota(&mut self, page_quota: usize) -> Result<(), Error> {
        if page_quota == 0 || page_quota < self.reserved_pages {
            return Err(Error::QuotaExceeded);
        }
        self.page_quota = page_quota;
        Ok(())
    }

    pub fn mmap_top(&self) -> Option<usize> {
        self.stack_top
            .checked_sub((MAX_STACK_PAGES + 1) * PAGE_SIZE)
    }

    pub fn heap_base(&self, hint: usize) -> Option<usize> {
        let base = self.aslr.choose_base(hint, PAGE_SIZE, PAGE_SIZE)?;
        (base < self.heap_limit()).then_some(base)
    }

    pub fn signal_trampoline(&self) -> Option<usize> {
        self.stack_top
            .checked_sub((MAX_STACK_PAGES + 2) * PAGE_SIZE)
    }

    pub(crate) fn heap_limit(&self) -> usize {
        self.stack_guard.unwrap_or(self.stack_top)
    }

    pub fn map_anonymous(
        &mut self,
        virtual_address: usize,
        flags: PageFlags,
    ) -> Result<MappingInfo, Error> {
        self.map_page(virtual_address, flags, MappingKind::Anonymous, true)
    }

    pub fn map_anonymous_lazy(
        &mut self,
        virtual_address: usize,
        flags: PageFlags,
    ) -> Result<MappingInfo, Error> {
        self.map_page(virtual_address, flags, MappingKind::Anonymous, false)
    }

    pub fn map_range(
        &mut self,
        hint: usize,
        length: usize,
        flags: PageFlags,
        fixed: bool,
        kind: MappingKind,
    ) -> Result<usize, Error> {
        self.map_range_with_mode(hint, length, flags, fixed, kind, true)
    }

    pub fn map_range_lazy(
        &mut self,
        hint: usize,
        length: usize,
        flags: PageFlags,
        fixed: bool,
        kind: MappingKind,
    ) -> Result<usize, Error> {
        self.map_range_with_mode(hint, length, flags, fixed, kind, false)
    }

    fn map_range_with_mode(
        &mut self,
        hint: usize,
        length: usize,
        flags: PageFlags,
        fixed: bool,
        kind: MappingKind,
        present: bool,
    ) -> Result<usize, Error> {
        let length = align_up(length, PAGE_SIZE).ok_or(Error::InvalidAddress)?;
        if length == 0 || length / PAGE_SIZE > MAX_MAPPINGS {
            return Err(Error::MappingCapacity);
        }
        let base = if fixed {
            if hint == 0 || !hint.is_multiple_of(PAGE_SIZE) {
                return Err(Error::Unaligned);
            }
            hint
        } else {
            self.find_free_range(hint, length).ok_or(Error::NoSpace)?
        };
        let end = base.checked_add(length).ok_or(Error::InvalidAddress)?;
        if base < PAGE_SIZE || end > self.heap_limit() || !self.range_is_free(base, end) {
            return Err(Error::InvalidAddress);
        }
        let mut mapped = 0;
        while mapped < length {
            if let Err(error) = self.map_page(base + mapped, flags, kind, present) {
                while mapped != 0 {
                    mapped -= PAGE_SIZE;
                    let _ = self.remove_mapping(base + mapped);
                }
                return Err(error);
            }
            mapped += PAGE_SIZE;
        }
        Ok(base)
    }

    pub fn map_stack(&mut self) -> Result<(), Error> {
        if self.stack_pages != 0 || self.root_frame.is_none() {
            return Err(Error::InvalidState);
        }
        let first = self
            .stack_top
            .checked_sub(INITIAL_STACK_PAGES * PAGE_SIZE)
            .ok_or(Error::InvalidAddress)?;
        for index in 0..INITIAL_STACK_PAGES {
            let address = first + index * PAGE_SIZE;
            if let Err(error) = self.map_page(address, PageFlags::USER_RW, MappingKind::Stack, true)
            {
                for rollback in 0..index {
                    let _ = self.remove_mapping(first + rollback * PAGE_SIZE);
                }
                return Err(error);
            }
        }
        self.stack_pages = INITIAL_STACK_PAGES;
        self.stack_guard = first.checked_sub(PAGE_SIZE);
        let trampoline = self.signal_trampoline().ok_or(Error::InvalidAddress)?;
        if let Err(error) = self.map_page(
            trampoline,
            PageFlags::USER_RX,
            MappingKind::SignalTrampoline,
            true,
        ) {
            for index in 0..INITIAL_STACK_PAGES {
                let _ = self.remove_mapping(first + index * PAGE_SIZE);
            }
            self.stack_pages = 0;
            self.stack_guard = None;
            return Err(error);
        }
        if let Err(error) = self.load(trampoline, &SIGNAL_TRAMPOLINE) {
            let _ = self.remove_mapping(trampoline);
            for index in 0..INITIAL_STACK_PAGES {
                let _ = self.remove_mapping(first + index * PAGE_SIZE);
            }
            self.stack_pages = 0;
            self.stack_guard = None;
            return Err(error);
        }
        Ok(())
    }

    pub fn stack_guard(&self) -> Option<usize> {
        self.stack_guard
    }

    pub fn grow_stack(&mut self, fault_address: usize) -> Result<bool, Error> {
        let Some(guard) = self.stack_guard else {
            return Ok(false);
        };
        if fault_address & !(PAGE_SIZE - 1) != guard {
            return Ok(false);
        }
        if self.stack_pages == MAX_STACK_PAGES {
            return Ok(false);
        }
        self.map_page(guard, PageFlags::USER_RW, MappingKind::Stack, true)?;
        self.stack_pages += 1;
        self.stack_guard = guard.checked_sub(PAGE_SIZE);
        Ok(true)
    }

    pub fn handle_fault(&mut self, fault: crate::vm::FaultInfo) -> crate::vm::FaultResult {
        if !fault.user {
            return crate::vm::FaultResult::KernelFatal;
        }
        let Some(page) = fault.page_address() else {
            return crate::vm::FaultResult::UserFault(crate::vm::FaultReason::InvalidAddress);
        };
        if self.root_frame.is_none() || self.owner.is_none() {
            return crate::vm::FaultResult::UserFault(crate::vm::FaultReason::InvalidAddress);
        }
        if !fault.is_translation() {
            return match fault.kind {
                crate::vm::FaultKind::Protection | crate::vm::FaultKind::AccessFlag => {
                    crate::vm::FaultResult::UserFault(crate::vm::FaultReason::Protection)
                }
                crate::vm::FaultKind::External | crate::vm::FaultKind::Unknown => {
                    crate::vm::FaultResult::UserFault(crate::vm::FaultReason::InvalidAddress)
                }
                crate::vm::FaultKind::Translation => unreachable!(),
            };
        }
        if self.stack_guard == Some(page) {
            if !fault.write || fault.instruction {
                return crate::vm::FaultResult::UserFault(crate::vm::FaultReason::Protection);
            }
            return match self.grow_stack(fault.address) {
                Ok(true) => crate::vm::FaultResult::Resolved,
                Ok(false) => {
                    crate::vm::FaultResult::UserFault(crate::vm::FaultReason::StackOverflow)
                }
                Err(Error::NoFrames | Error::MappingCapacity | Error::QuotaExceeded) => {
                    crate::vm::FaultResult::UserFault(crate::vm::FaultReason::OutOfMemory)
                }
                Err(Error::GuardPage | Error::InvalidAddress) => {
                    crate::vm::FaultResult::UserFault(crate::vm::FaultReason::GuardPage)
                }
                Err(_) => crate::vm::FaultResult::KernelFatal,
            };
        }
        if let Some(mapping) = self.mapping(page) {
            if (fault.write && !mapping.flags.writable)
                || (fault.instruction && !mapping.flags.executable)
                || (!fault.write && !fault.instruction && !mapping.flags.readable)
            {
                return crate::vm::FaultResult::UserFault(crate::vm::FaultReason::Protection);
            }
            if !mapping.present {
                return match self.materialize(page) {
                    Ok(()) => crate::vm::FaultResult::Resolved,
                    Err(Error::NoFrames) => {
                        crate::vm::FaultResult::UserFault(crate::vm::FaultReason::OutOfMemory)
                    }
                    Err(_) => crate::vm::FaultResult::KernelFatal,
                };
            }
        }
        crate::vm::FaultResult::UserFault(crate::vm::FaultReason::InvalidAddress)
    }

    pub fn mapping(&self, virtual_address: usize) -> Option<MappingInfo> {
        self.mappings
            .iter()
            .flatten()
            .find(|mapping| mapping.virtual_address == virtual_address)
            .map(|mapping| MappingInfo {
                virtual_address: mapping.virtual_address,
                physical_frame: mapping.physical_frame.unwrap_or(PhysAddr::new(0)),
                present: mapping.physical_frame.is_some(),
                cow: mapping.cow,
                flags: mapping.flags,
                kind: mapping.kind,
            })
    }

    pub fn unmap_page(&mut self, virtual_address: usize) -> Result<(), Error> {
        let mapping = self.mapping(virtual_address).ok_or(Error::InvalidAddress)?;
        if matches!(
            mapping.kind,
            MappingKind::Stack | MappingKind::SignalTrampoline
        ) {
            return Err(Error::InvalidState);
        }
        self.remove_mapping(virtual_address)
    }

    pub fn unmap_range(&mut self, address: usize, length: usize) -> Result<(), Error> {
        let length = align_up(length, PAGE_SIZE).ok_or(Error::InvalidAddress)?;
        if length == 0 || !address.is_multiple_of(PAGE_SIZE) {
            return Err(Error::Unaligned);
        }
        let end = address.checked_add(length).ok_or(Error::InvalidAddress)?;
        if end > USER_LIMIT {
            return Err(Error::InvalidAddress);
        }
        let mut page = address;
        while page < end {
            let mapping = self.mapping(page).ok_or(Error::InvalidAddress)?;
            if matches!(
                mapping.kind,
                MappingKind::Stack | MappingKind::SignalTrampoline
            ) {
                return Err(Error::InvalidState);
            }
            page += PAGE_SIZE;
        }
        page = address;
        while page < end {
            self.remove_mapping(page)?;
            page += PAGE_SIZE;
        }
        Ok(())
    }

    pub fn protect_range(
        &mut self,
        address: usize,
        length: usize,
        flags: PageFlags,
    ) -> Result<(), Error> {
        flags.validate()?;
        let length = align_up(length, PAGE_SIZE).ok_or(Error::InvalidAddress)?;
        if length == 0 || !address.is_multiple_of(PAGE_SIZE) {
            return Err(Error::Unaligned);
        }
        let end = address.checked_add(length).ok_or(Error::InvalidAddress)?;
        if end > USER_LIMIT {
            return Err(Error::InvalidAddress);
        }
        let mut page = address;
        while page < end {
            let mapping = self.mapping(page).ok_or(Error::InvalidAddress)?;
            if mapping.kind == MappingKind::SignalTrampoline {
                return Err(Error::InvalidState);
            }
            page += PAGE_SIZE;
        }
        page = address;
        while page < end {
            let slot = self
                .mappings
                .iter()
                .position(|entry| entry.is_some_and(|entry| entry.virtual_address == page))
                .ok_or(Error::InvalidAddress)?;
            self.mappings[slot]
                .as_mut()
                .ok_or(Error::InvalidAddress)?
                .flags = flags;
            if self.active
                && self.mappings[slot].is_some_and(|entry| entry.physical_frame.is_some())
            {
                let root = self.root_frame.ok_or(Error::InvalidState)?;
                if !crate::arch::user_space_protect(root, page, flags) {
                    return Err(Error::HardwareMapping);
                }
            }
            page += PAGE_SIZE;
        }
        Ok(())
    }

    pub fn contains_executable(&self, address: usize) -> bool {
        let page = address & !(PAGE_SIZE - 1);
        self.mapping(page)
            .is_some_and(|mapping| mapping.flags.executable)
    }

    fn find_free_range(&self, hint: usize, length: usize) -> Option<usize> {
        let mut base = if hint == 0 {
            #[cfg(target_arch = "x86_64")]
            {
                0x0000_1000_0000_0000
            }
            #[cfg(target_arch = "aarch64")]
            {
                0x0000_0020_0000_0000
            }
        } else {
            hint & !(PAGE_SIZE - 1)
        };
        base = self.aslr.choose_base(base, length, PAGE_SIZE)?;
        let end = base.checked_add(length)?;
        if end <= self.heap_limit() && self.range_is_free(base, end) {
            Some(base)
        } else {
            let mut candidate = PAGE_SIZE;
            while candidate.checked_add(length)? <= self.heap_limit() {
                let end = candidate + length;
                if self.range_is_free(candidate, end) {
                    return Some(candidate);
                }
                candidate = candidate.checked_add(PAGE_SIZE)?;
            }
            None
        }
    }

    fn range_is_free(&self, start: usize, end: usize) -> bool {
        self.mappings
            .iter()
            .flatten()
            .all(|mapping| mapping.virtual_address < start || mapping.virtual_address >= end)
    }

    pub fn destroy(&mut self) -> Result<(), Error> {
        let root = self.root_frame.ok_or(Error::InvalidState)?;
        if self.active {
            let current = crate::arch::user_space_is_current(root);
            crate::arch::user_space_reset(root);
            if current {
                crate::arch::restore_kernel_address_space();
            }
            self.active = false;
        }
        if self.root_frame.is_none() || self.active {
            return Err(Error::InvalidState);
        }
        let mut error = None;
        for mapping in self.mappings.iter_mut().flatten() {
            if mapping
                .physical_frame
                .is_some_and(|frame| !crate::memory::free_frame(frame.value()))
                && error.is_none()
            {
                error = Some(Error::FrameReturn);
            }
        }
        self.mappings = [None; MAX_MAPPINGS];
        self.reserved_pages = 0;
        if !self.release_table_frames() && error.is_none() {
            error = Some(Error::FrameReturn);
        }
        let root = self.root_frame.take().ok_or(Error::InvalidState)?;
        if !crate::memory::free_frame(root.value()) && error.is_none() {
            error = Some(Error::FrameReturn);
        }
        self.owner = None;
        self.stack_guard = None;
        self.stack_pages = 0;
        error.map_or(Ok(()), Err)
    }

    pub fn is_destroyed(&self) -> bool {
        self.root_frame.is_none()
    }

    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        self.active
    }

    #[allow(dead_code)]
    pub fn activate(&mut self) -> Result<(), Error> {
        let root = self.root_frame.ok_or(Error::InvalidState)?;
        if self.active {
            return Err(Error::InvalidState);
        }
        if !crate::arch::user_space_prepare(root) {
            return Err(Error::HardwareUnavailable);
        }
        for mapping in self
            .mappings
            .iter()
            .flatten()
            .copied()
            .filter(|mapping| mapping.physical_frame.is_some())
        {
            if !crate::arch::user_space_map(
                root,
                MappingInfo {
                    virtual_address: mapping.virtual_address,
                    physical_frame: mapping.physical_frame.unwrap_or(PhysAddr::new(0)),
                    present: mapping.physical_frame.is_some(),
                    cow: mapping.cow,
                    flags: mapping.flags,
                    kind: mapping.kind,
                },
                &mut self.table_frames,
            ) {
                crate::arch::user_space_reset(root);
                let _ = self.release_table_frames();
                return Err(Error::HardwareMapping);
            }
        }
        self.active = true;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn load(&self, virtual_address: usize, bytes: &[u8]) -> Result<(), Error> {
        if self.root_frame.is_none() || self.active {
            return Err(Error::InvalidState);
        }
        let mut cursor = virtual_address;
        let mut offset = 0;
        while offset < bytes.len() {
            let page = cursor & !(PAGE_SIZE - 1);
            let mapping = self.mapping(page).ok_or(Error::InvalidAddress)?;
            if !mapping.present {
                return Err(Error::InvalidState);
            }
            let page_offset = cursor - page;
            let count = (PAGE_SIZE - page_offset).min(bytes.len() - offset);
            if !crate::arch::write_physical(
                mapping.physical_frame,
                page_offset,
                &bytes[offset..offset + count],
            ) {
                return Err(Error::HardwareUnavailable);
            }
            cursor = cursor.checked_add(count).ok_or(Error::InvalidAddress)?;
            offset += count;
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn load_words(&self, virtual_address: usize, words: &[u64]) -> Result<(), Error> {
        for (index, word) in words.iter().copied().enumerate() {
            let address = virtual_address
                .checked_add(
                    index
                        .checked_mul(core::mem::size_of::<u64>())
                        .ok_or(Error::InvalidAddress)?,
                )
                .ok_or(Error::InvalidAddress)?;
            self.load(address, &word.to_le_bytes())?;
        }
        Ok(())
    }

    fn map_page(
        &mut self,
        virtual_address: usize,
        flags: PageFlags,
        kind: MappingKind,
        present: bool,
    ) -> Result<MappingInfo, Error> {
        if self.root_frame.is_none() {
            return Err(Error::InvalidState);
        }
        flags.validate()?;
        validate_page(virtual_address)?;
        if self.stack_guard == Some(virtual_address) && kind != MappingKind::Stack {
            return Err(Error::GuardPage);
        }
        if self
            .mappings
            .iter()
            .flatten()
            .any(|mapping| mapping.virtual_address == virtual_address)
        {
            return Err(Error::Overlap);
        }
        let slot = self
            .mappings
            .iter()
            .position(Option::is_none)
            .ok_or(Error::MappingCapacity)?;
        if self.reserved_pages >= self.page_quota {
            return Err(Error::QuotaExceeded);
        }
        let physical_frame = if present {
            Some(
                crate::memory::alloc_frame()
                    .map(PhysAddr::new)
                    .ok_or(Error::NoFrames)?,
            )
        } else {
            None
        };
        if physical_frame.is_some_and(|frame| !crate::arch::zero_physical_page(frame)) {
            let _ = physical_frame.is_none_or(|frame| crate::memory::free_frame(frame.value()));
            return Err(Error::HardwareUnavailable);
        }
        let mapping = Mapping {
            virtual_address,
            physical_frame,
            flags,
            kind,
            shared: false,
            cow: false,
        };
        self.mappings[slot] = Some(mapping);
        if self.active && present {
            let root = self.root_frame.ok_or(Error::InvalidState)?;
            if !crate::arch::user_space_map(
                root,
                MappingInfo {
                    virtual_address: mapping.virtual_address,
                    physical_frame: mapping.physical_frame.unwrap_or(PhysAddr::new(0)),
                    present: mapping.physical_frame.is_some(),
                    cow: mapping.cow,
                    flags: mapping.flags,
                    kind: mapping.kind,
                },
                &mut self.table_frames,
            ) {
                self.mappings[slot] = None;
                let _ = physical_frame.is_none_or(|frame| crate::memory::free_frame(frame.value()));
                return Err(Error::HardwareMapping);
            }
        }
        self.reserved_pages += 1;
        Ok(MappingInfo {
            virtual_address,
            physical_frame: physical_frame.unwrap_or(PhysAddr::new(0)),
            present,
            cow: false,
            flags,
            kind,
        })
    }

    fn materialize(&mut self, address: usize) -> Result<(), Error> {
        let slot = self
            .mappings
            .iter()
            .position(|entry| entry.is_some_and(|mapping| mapping.virtual_address == address))
            .ok_or(Error::InvalidAddress)?;
        let mapping = self.mappings[slot].ok_or(Error::InvalidAddress)?;
        if mapping.physical_frame.is_some() {
            return Ok(());
        }
        let frame = crate::memory::alloc_frame()
            .map(PhysAddr::new)
            .ok_or(Error::NoFrames)?;
        if !crate::arch::zero_physical_page(frame) {
            let _ = crate::memory::free_frame(frame.value());
            return Err(Error::HardwareUnavailable);
        }
        if self.active
            && !crate::arch::user_space_map(
                self.root_frame.ok_or(Error::InvalidState)?,
                MappingInfo {
                    virtual_address: address,
                    physical_frame: frame,
                    present: true,
                    flags: mapping.flags,
                    kind: mapping.kind,
                    cow: false,
                },
                &mut self.table_frames,
            )
        {
            let _ = crate::memory::free_frame(frame.value());
            return Err(Error::HardwareMapping);
        }
        self.mappings[slot]
            .as_mut()
            .ok_or(Error::InvalidState)?
            .physical_frame = Some(frame);
        Ok(())
    }

    fn remove_mapping(&mut self, virtual_address: usize) -> Result<(), Error> {
        let slot = self
            .mappings
            .iter()
            .position(|mapping| {
                mapping.is_some_and(|mapping| mapping.virtual_address == virtual_address)
            })
            .ok_or(Error::InvalidAddress)?;
        if self.active
            && self.mappings[slot].is_some_and(|mapping| mapping.physical_frame.is_some())
        {
            let root = self.root_frame.ok_or(Error::InvalidState)?;
            if !crate::arch::user_space_unmap(root, virtual_address, &mut self.table_frames) {
                return Err(Error::HardwareMapping);
            }
        }
        let mapping = self.mappings[slot].take().ok_or(Error::InvalidAddress)?;
        if mapping
            .physical_frame
            .is_some_and(|frame| !crate::memory::free_frame(frame.value()))
        {
            self.mappings[slot] = Some(mapping);
            return Err(Error::FrameReturn);
        }
        self.reserved_pages -= 1;
        Ok(())
    }

    fn release_table_frames(&mut self) -> bool {
        let mut released = true;
        for frame in &mut self.table_frames {
            if let Some(frame) = frame.take() {
                if !crate::memory::free_frame(frame.value()) {
                    released = false;
                }
            }
        }
        released
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        if self.root_frame.is_some() {
            let _ = self.destroy();
        }
    }
}

fn validate_page(virtual_address: usize) -> Result<(), Error> {
    if !virtual_address.is_multiple_of(PAGE_SIZE) {
        return Err(Error::Unaligned);
    }
    let end = virtual_address
        .checked_add(PAGE_SIZE)
        .ok_or(Error::InvalidAddress)?;
    if virtual_address < PAGE_SIZE || end > USER_LIMIT {
        return Err(Error::InvalidAddress);
    }
    Ok(())
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}

pub fn contract_self_check() {
    let owner = ProcessId::INIT;
    let mut space = AddressSpace::new(owner, AslrHook::new(7)).unwrap();
    assert_eq!(space.owner(), Some(owner));
    assert!(space.root_frame().is_some());
    #[cfg(target_arch = "x86_64")]
    let base_hint = 0x4000_0000_0000;
    #[cfg(target_arch = "aarch64")]
    let base_hint = 0x0800_0000_0000;
    let base = space.aslr_base(base_hint, PAGE_SIZE * 2, PAGE_SIZE);
    assert!(base.is_some());
    let base = base.unwrap();
    assert_eq!(
        space.map_anonymous(base, PageFlags::USER_RX).unwrap().kind,
        MappingKind::Anonymous
    );
    assert_eq!(
        space.map_anonymous(base, PageFlags::USER_RW),
        Err(Error::Overlap)
    );
    assert_ne!(
        space.aslr_base(base_hint, PAGE_SIZE, PAGE_SIZE),
        AslrHook::new(8).choose_base(base_hint, PAGE_SIZE, PAGE_SIZE)
    );
    assert_eq!(
        space.map_anonymous(
            base + PAGE_SIZE * 4,
            PageFlags {
                user: true,
                readable: false,
                writable: false,
                executable: true,
            },
        ),
        Err(Error::NoAccess)
    );
    assert_eq!(
        space.map_anonymous(
            base + PAGE_SIZE * 2,
            PageFlags {
                user: true,
                readable: true,
                writable: true,
                executable: true
            }
        ),
        Err(Error::WriteExecute)
    );
    let load_page = base + PAGE_SIZE * 3;
    space.map_anonymous(load_page, PageFlags::USER_RX).unwrap();
    space.activate().unwrap();
    assert_eq!(space.load(load_page, &[0xaa]), Err(Error::InvalidState));
    space.destroy().unwrap();
    let mut space = AddressSpace::new(owner, AslrHook::new(7)).unwrap();
    let base = space
        .aslr_base(base_hint, PAGE_SIZE * 2, PAGE_SIZE)
        .unwrap();
    space.map_anonymous(base, PageFlags::USER_RX).unwrap();
    space.map_stack().unwrap();
    let guard = space.stack_guard().unwrap();
    assert_eq!(
        space.handle_fault(crate::vm::FaultInfo {
            address: guard,
            raw: 0,
            kind: crate::vm::FaultKind::Translation,
            write: true,
            instruction: false,
            user: true,
        }),
        crate::vm::FaultResult::Resolved
    );
    assert_eq!(space.mapping(guard).unwrap().kind, MappingKind::Stack);
    assert_eq!(space.unmap_page(guard), Err(Error::InvalidState));
    assert_eq!(
        space.map_anonymous(guard - PAGE_SIZE, PageFlags::USER_READ),
        Err(Error::GuardPage)
    );
    assert_eq!(
        space
            .map_anonymous(base + PAGE_SIZE * 3, PageFlags::USER_READ)
            .unwrap()
            .kind,
        MappingKind::Anonymous
    );
    assert_eq!(
        space.map_anonymous(0, PageFlags::USER_READ),
        Err(Error::InvalidAddress)
    );
    assert_eq!(
        space.map_anonymous(base + 1, PageFlags::USER_READ),
        Err(Error::Unaligned)
    );
    space.activate().unwrap();
    assert!(space.is_active());
    space.unmap_page(base).unwrap();
    let zero_page = space
        .mapping(base + PAGE_SIZE * 3)
        .expect("anonymous page mapping");
    let zero_bytes = [0xff; 16];
    assert!(crate::arch::write_physical(
        zero_page.physical_frame,
        0,
        &zero_bytes
    ));
    assert!(crate::arch::zero_physical_page(zero_page.physical_frame));
    let mut read_back = [0xff; 16];
    assert!(crate::arch::read_physical(
        zero_page.physical_frame,
        0,
        &mut read_back
    ));
    assert_eq!(read_back, [0; 16]);
    space.unmap_page(base + PAGE_SIZE * 3).unwrap();
    assert!(space.mapping(base + PAGE_SIZE * 3).is_none());
    space.destroy().unwrap();
    assert!(space.is_destroyed());
    assert_eq!(space.owner(), None);
}
