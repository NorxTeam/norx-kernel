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

    fn validate(self) -> Result<(), Error> {
        if !self.user {
            return Err(Error::KernelMapping);
        }
        if self.writable && self.executable {
            return Err(Error::WriteExecute);
        }
        if !self.readable && !self.writable && !self.executable {
            return Err(Error::NoAccess);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingKind {
    Anonymous,
    Stack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingInfo {
    pub virtual_address: usize,
    pub physical_frame: PhysAddr,
    pub flags: PageFlags,
    pub kind: MappingKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mapping {
    virtual_address: usize,
    physical_frame: PhysAddr,
    flags: PageFlags,
    kind: MappingKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NoFrames,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AslrHook {
    seed: u64,
}

impl AslrHook {
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    pub fn choose_base(self, hint: usize, span: usize, alignment: usize) -> Option<usize> {
        if span == 0 || !alignment.is_power_of_two() {
            return None;
        }
        let span = align_up(span, PAGE_SIZE)?;
        let alignment_mask = alignment - 1;
        let jitter = self.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) as usize;
        let base = hint.checked_add(jitter & alignment_mask)? & !alignment_mask;
        let end = base.checked_add(span)?;
        if base < PAGE_SIZE || end > USER_LIMIT {
            return None;
        }
        Some(base)
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
}

impl AddressSpace {
    pub fn new(owner: ProcessId, aslr: AslrHook) -> Result<Self, Error> {
        let root_frame = crate::memory::alloc_frame()
            .map(PhysAddr::new)
            .ok_or(Error::NoFrames)?;
        Ok(Self {
            owner: Some(owner),
            root_frame: Some(root_frame),
            mappings: [None; MAX_MAPPINGS],
            stack_top: USER_LIMIT - PAGE_SIZE,
            stack_guard: None,
            stack_pages: 0,
            aslr,
            active: false,
            table_frames: [None; MAX_TABLE_FRAMES],
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

    pub fn map_anonymous(
        &mut self,
        virtual_address: usize,
        flags: PageFlags,
    ) -> Result<MappingInfo, Error> {
        self.map_page(virtual_address, flags, MappingKind::Anonymous)
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
            if let Err(error) = self.map_page(address, PageFlags::USER_RW, MappingKind::Stack) {
                for rollback in 0..index {
                    let _ = self.remove_mapping(first + rollback * PAGE_SIZE);
                }
                return Err(error);
            }
        }
        self.stack_pages = INITIAL_STACK_PAGES;
        self.stack_guard = first.checked_sub(PAGE_SIZE);
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
        self.map_page(guard, PageFlags::USER_RW, MappingKind::Stack)?;
        self.stack_pages += 1;
        self.stack_guard = guard.checked_sub(PAGE_SIZE);
        Ok(true)
    }

    pub fn handle_fault(&mut self, fault: crate::vm::FaultInfo) -> crate::vm::FaultResult {
        if !fault.user {
            return crate::vm::FaultResult::KernelFatal;
        }
        if fault.kind != crate::vm::FaultKind::Translation || fault.instruction {
            return crate::vm::FaultResult::UserFault(crate::vm::FaultReason::Protection);
        }
        let page = fault.address & !(PAGE_SIZE - 1);
        if self.stack_guard == Some(page) {
            if !fault.write {
                return crate::vm::FaultResult::UserFault(crate::vm::FaultReason::Protection);
            }
            return match self.grow_stack(fault.address) {
                Ok(true) => crate::vm::FaultResult::Resolved,
                Ok(false) => {
                    crate::vm::FaultResult::UserFault(crate::vm::FaultReason::StackOverflow)
                }
                Err(Error::NoFrames) => {
                    crate::vm::FaultResult::UserFault(crate::vm::FaultReason::OutOfMemory)
                }
                Err(_) => crate::vm::FaultResult::UserFault(crate::vm::FaultReason::GuardPage),
            };
        }
        if let Some(mapping) = self.mapping(page) {
            if (fault.write && !mapping.flags.writable) || (!fault.write && !mapping.flags.readable)
            {
                return crate::vm::FaultResult::UserFault(crate::vm::FaultReason::Protection);
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
                physical_frame: mapping.physical_frame,
                flags: mapping.flags,
                kind: mapping.kind,
            })
    }

    pub fn unmap_page(&mut self, virtual_address: usize) -> Result<(), Error> {
        let mapping = self.mapping(virtual_address).ok_or(Error::InvalidAddress)?;
        if mapping.kind == MappingKind::Stack {
            return Err(Error::InvalidState);
        }
        self.remove_mapping(virtual_address)
    }

    pub fn destroy(&mut self) -> Result<(), Error> {
        let root = self.root_frame.ok_or(Error::InvalidState)?;
        if self.active {
            crate::arch::user_space_reset(root);
            crate::arch::restore_kernel_address_space();
            self.active = false;
        }
        if self.root_frame.is_none() {
            return Err(Error::InvalidState);
        }
        let mut error = None;
        for mapping in self.mappings.iter_mut().flatten() {
            if !crate::memory::free_frame(mapping.physical_frame.value()) && error.is_none() {
                error = Some(Error::FrameReturn);
            }
        }
        self.mappings = [None; MAX_MAPPINGS];
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
        for mapping in self.mappings.iter().flatten().copied() {
            if !crate::arch::user_space_map(
                root,
                MappingInfo {
                    virtual_address: mapping.virtual_address,
                    physical_frame: mapping.physical_frame,
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
        if self.root_frame.is_none() {
            return Err(Error::InvalidState);
        }
        let mut cursor = virtual_address;
        let mut offset = 0;
        while offset < bytes.len() {
            let page = cursor & !(PAGE_SIZE - 1);
            let mapping = self.mapping(page).ok_or(Error::InvalidAddress)?;
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
        let physical_frame = crate::memory::alloc_frame()
            .map(PhysAddr::new)
            .ok_or(Error::NoFrames)?;
        if !crate::arch::zero_physical_page(physical_frame) {
            let _ = crate::memory::free_frame(physical_frame.value());
            return Err(Error::HardwareUnavailable);
        }
        let mapping = Mapping {
            virtual_address,
            physical_frame,
            flags,
            kind,
        };
        self.mappings[slot] = Some(mapping);
        if self.active {
            let root = self.root_frame.ok_or(Error::InvalidState)?;
            if !crate::arch::user_space_map(
                root,
                MappingInfo {
                    virtual_address: mapping.virtual_address,
                    physical_frame: mapping.physical_frame,
                    flags: mapping.flags,
                    kind: mapping.kind,
                },
                &mut self.table_frames,
            ) {
                self.mappings[slot] = None;
                let _ = crate::memory::free_frame(physical_frame.value());
                return Err(Error::HardwareMapping);
            }
        }
        Ok(MappingInfo {
            virtual_address,
            physical_frame,
            flags,
            kind,
        })
    }

    fn remove_mapping(&mut self, virtual_address: usize) -> Result<(), Error> {
        let slot = self
            .mappings
            .iter()
            .position(|mapping| {
                mapping.is_some_and(|mapping| mapping.virtual_address == virtual_address)
            })
            .ok_or(Error::InvalidAddress)?;
        if self.active {
            let root = self.root_frame.ok_or(Error::InvalidState)?;
            if !crate::arch::user_space_unmap(root, virtual_address, &mut self.table_frames) {
                return Err(Error::HardwareMapping);
            }
        }
        let mapping = self.mappings[slot].take().ok_or(Error::InvalidAddress)?;
        if !crate::memory::free_frame(mapping.physical_frame.value()) {
            self.mappings[slot] = Some(mapping);
            return Err(Error::FrameReturn);
        }
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
