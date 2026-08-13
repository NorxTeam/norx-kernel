use crate::address_space::{AddressSpace, AslrHook, PageFlags, PAGE_SIZE};
use crate::elf::{InitialRegisters, LoadPlan};
use crate::process::ProcessId;
use core::cell::UnsafeCell;

const MAX_WRITE: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleMetrics {
    pub startup_ticks: u64,
    pub image_bytes: usize,
    pub user_memory_bytes: usize,
    pub syscall_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Elf(crate::elf::Error),
    AddressSpace(crate::address_space::Error),
    InvalidState,
    NoSpace,
    InvalidFd,
    OutputTooLong,
    UserEntryUnavailable,
}

#[derive(Debug)]
pub struct NativeRuntime {
    address_space: AddressSpace,
    registers: InitialRegisters,
    heap_next: usize,
    started: bool,
    exited: bool,
}

const MAX_RUNTIME_SLOTS: usize = 32;

struct RuntimeStore(UnsafeCell<[Option<NativeRuntime>; MAX_RUNTIME_SLOTS]>);

unsafe impl Sync for RuntimeStore {}

static RUNTIME_STORE: RuntimeStore =
    RuntimeStore(UnsafeCell::new([const { None }; MAX_RUNTIME_SLOTS]));

impl NativeRuntime {
    pub fn prepare(
        owner: ProcessId,
        plan: &LoadPlan,
        aslr: AslrHook,
        arguments: &[&[u8]],
        environment: &[&[u8]],
    ) -> Result<Self, Error> {
        Self::prepare_internal(owner, None, plan, aslr, arguments, environment)
    }

    #[allow(dead_code)]
    pub fn prepare_image(
        owner: ProcessId,
        image: &[u8],
        plan: &LoadPlan,
        aslr: AslrHook,
        arguments: &[&[u8]],
        environment: &[&[u8]],
    ) -> Result<Self, Error> {
        Self::prepare_internal(owner, Some(image), plan, aslr, arguments, environment)
    }

    fn prepare_internal(
        owner: ProcessId,
        image: Option<&[u8]>,
        plan: &LoadPlan,
        aslr: AslrHook,
        arguments: &[&[u8]],
        environment: &[&[u8]],
    ) -> Result<Self, Error> {
        let mut address_space = AddressSpace::new(owner, aslr).map_err(Error::AddressSpace)?;
        let mut heap_next = 0;
        for index in 0..plan.segment_count {
            let segment = plan.segment(index).ok_or(Error::InvalidState)?;
            let flags = PageFlags {
                user: true,
                readable: segment.flags.readable,
                writable: segment.flags.writable,
                executable: segment.flags.executable,
            };
            let mut page = segment.virtual_start;
            while page < segment.virtual_end {
                if let Err(error) = address_space.map_anonymous(page, flags) {
                    let _ = address_space.destroy();
                    return Err(Error::AddressSpace(error));
                }
                page = page.checked_add(PAGE_SIZE).ok_or(Error::NoSpace)?;
            }
            if let Some(image) = image {
                let file_size = usize::try_from(segment.file_size).map_err(|_| Error::NoSpace)?;
                if file_size != 0 {
                    let file_offset = plan
                        .file_offset(segment.virtual_start, file_size)
                        .ok_or(Error::InvalidState)?;
                    let file_end = file_offset.checked_add(file_size).ok_or(Error::NoSpace)?;
                    address_space
                        .load(
                            segment.virtual_start,
                            image
                                .get(file_offset..file_end)
                                .ok_or(Error::InvalidState)?,
                        )
                        .map_err(Error::AddressSpace)?;
                }
            }
            heap_next = heap_next.max(segment.virtual_end);
        }
        if let Err(error) = address_space.map_stack() {
            let _ = address_space.destroy();
            return Err(Error::AddressSpace(error));
        }
        let stack =
            match plan.build_initial_stack(address_space.stack_top(), arguments, environment) {
                Ok(stack) => stack,
                Err(error) => {
                    let _ = address_space.destroy();
                    return Err(Error::Elf(error));
                }
            };
        if image.is_some() {
            address_space
                .load(stack.string_base(), stack.string_bytes())
                .map_err(Error::AddressSpace)?;
            address_space
                .load_words(stack.stack_pointer, stack.words())
                .map_err(Error::AddressSpace)?;
        }
        let heap_next = address_space
            .heap_base(align_up(heap_next, PAGE_SIZE).ok_or(Error::NoSpace)?)
            .ok_or(Error::NoSpace)?;
        Ok(Self {
            address_space,
            registers: stack.registers(plan.entry),
            heap_next,
            started: false,
            exited: false,
        })
    }

    pub fn start(&mut self) -> Result<InitialRegisters, Error> {
        if self.started || self.exited {
            return Err(Error::InvalidState);
        }
        self.started = true;
        crate::bootlog::ok_fmt(format_args!(
            "native init start entry=0x{:x} stack=0x{:x}",
            self.registers.instruction_pointer, self.registers.stack_pointer
        ));
        Ok(self.registers)
    }

    pub fn activate_for_resumable(&mut self) -> Result<(), Error> {
        if self.started || self.exited || self.address_space.is_active() {
            return Err(Error::InvalidState);
        }
        self.address_space.activate().map_err(Error::AddressSpace)
    }

    pub fn enter_user(&mut self) -> Result<(), Error> {
        self.enter_user_inner(true, true)
    }

    pub fn enter_user_quiet(&mut self) -> Result<(), Error> {
        self.enter_user_inner(false, true)
    }

    #[allow(dead_code)]
    pub fn enter_user_resumable(&mut self, log: bool) -> Result<(), Error> {
        self.enter_user_inner(log, false)
    }

    fn enter_user_inner(&mut self, log: bool, destroy: bool) -> Result<(), Error> {
        if !self.started || self.exited || self.address_space.is_active() {
            return Err(Error::InvalidState);
        }
        let root = self.address_space.root_frame().ok_or(Error::InvalidState)?;
        if log {
            crate::bootlog::ok("native user entry: activating address space");
        }
        let activated = self.address_space.activate();
        activated.map_err(Error::AddressSpace)?;
        if log {
            crate::bootlog::ok("native user entry: address space active");
            crate::bootlog::ok("native user entry: switching TTBR0");
        }
        let switched = crate::arch::switch_to_user(root);
        if !switched {
            crate::arch::restore_kernel_address_space();
            let _ = self.address_space.destroy();
            return Err(Error::UserEntryUnavailable);
        }
        if log {
            crate::bootlog::ok("native user entry: TTBR0 active, entering EL0");
        }
        if !crate::arch::enter_user(self.registers) {
            crate::arch::restore_kernel_address_space();
            let _ = self.address_space.destroy();
            return Err(Error::UserEntryUnavailable);
        }
        crate::arch::restore_kernel_address_space();
        if destroy {
            self.address_space.destroy().map_err(Error::AddressSpace)?;
            self.exited = true;
        }
        Ok(())
    }

    pub fn entry(&self) -> usize {
        self.registers.instruction_pointer
    }

    pub fn registers(&self) -> InitialRegisters {
        self.registers
    }

    pub fn root_frame(&self) -> Option<crate::address::PhysAddr> {
        self.address_space.root_frame()
    }

    pub fn alloc(&mut self, bytes: usize) -> Result<usize, Error> {
        if !self.started || self.exited || bytes == 0 {
            return Err(Error::InvalidState);
        }
        let size = align_up(bytes, PAGE_SIZE).ok_or(Error::NoSpace)?;
        let end = self.heap_next.checked_add(size).ok_or(Error::NoSpace)?;
        if end > self.address_space.heap_limit() {
            return Err(Error::NoSpace);
        }
        let start = self.heap_next;
        let mut page = start;
        while page < end {
            if let Err(error) = self.address_space.map_anonymous(page, PageFlags::USER_RW) {
                while page > start {
                    page -= PAGE_SIZE;
                    let _ = self.address_space.unmap_page(page);
                }
                return Err(Error::AddressSpace(error));
            }
            page += PAGE_SIZE;
        }
        self.heap_next = end;
        Ok(start)
    }

    pub fn write_fd(&self, fd: u32, bytes: &[u8]) -> Result<usize, Error> {
        if fd != 1 && fd != 2 {
            return Err(Error::InvalidFd);
        }
        self.write_serial(bytes)
    }

    pub fn write_serial(&self, bytes: &[u8]) -> Result<usize, Error> {
        if !self.started || self.exited {
            return Err(Error::InvalidState);
        }
        if bytes.len() > MAX_WRITE {
            return Err(Error::OutputTooLong);
        }
        crate::bootlog::ok_fmt(format_args!(
            "native init serial write bytes={}",
            bytes.len()
        ));
        Ok(bytes.len())
    }

    pub fn exit(&mut self, status: i32) -> Result<(), Error> {
        if !self.started || self.exited {
            return Err(Error::InvalidState);
        }
        self.address_space.destroy().map_err(Error::AddressSpace)?;
        self.exited = true;
        crate::bootlog::ok_fmt(format_args!("native init exited status={}", status));
        Ok(())
    }

    pub fn discard(&mut self) -> Result<(), Error> {
        if self.exited {
            return Ok(());
        }
        self.address_space.destroy().map_err(Error::AddressSpace)?;
        self.exited = true;
        Ok(())
    }

    pub fn is_exited(&self) -> bool {
        self.exited
    }
}

fn runtime_index(process: ProcessId) -> Option<usize> {
    let slot = (process.get() & 0xffff) as usize;
    (slot != 0 && slot <= MAX_RUNTIME_SLOTS).then_some(slot - 1)
}

pub fn install(process: ProcessId, runtime: NativeRuntime) -> Result<(), Error> {
    let index = runtime_index(process).ok_or(Error::InvalidState)?;
    crate::arch::without_interrupts(|| unsafe {
        let slots = &mut *RUNTIME_STORE.0.get();
        if slots[index].is_some() {
            return Err(Error::InvalidState);
        }
        slots[index] = Some(runtime);
        Ok(())
    })
}

pub fn start(process: ProcessId) -> Result<InitialRegisters, Error> {
    let index = runtime_index(process).ok_or(Error::InvalidState)?;
    crate::arch::without_interrupts(|| unsafe {
        (&mut *RUNTIME_STORE.0.get())[index]
            .as_mut()
            .ok_or(Error::InvalidState)?
            .start()
    })
}

#[allow(dead_code)]
pub fn enter_resumable(process: ProcessId, log: bool) -> Result<(), Error> {
    let index = runtime_index(process).ok_or(Error::InvalidState)?;
    crate::arch::without_interrupts(|| unsafe {
        (&mut *RUNTIME_STORE.0.get())[index]
            .as_mut()
            .ok_or(Error::InvalidState)?
            .enter_user_resumable(log)
    })
}

pub fn take(process: ProcessId) -> Option<NativeRuntime> {
    let index = runtime_index(process)?;
    crate::arch::without_interrupts(|| unsafe { (&mut *RUNTIME_STORE.0.get())[index].take() })
}

pub fn discard(process: ProcessId) -> Result<(), Error> {
    let Some(mut runtime) = take(process) else {
        return Ok(());
    };
    runtime.discard()
}

pub fn handle_fault(
    process: ProcessId,
    fault: crate::vm::FaultInfo,
) -> Option<crate::vm::FaultResult> {
    let index = runtime_index(process)?;
    crate::arch::without_interrupts(|| unsafe {
        (&mut *RUNTIME_STORE.0.get())[index]
            .as_mut()
            .map(|runtime| runtime.address_space.handle_fault(fault))
    })
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}

pub fn contract_self_check() {
    let _errors = [
        Error::Elf(crate::elf::Error::Truncated),
        Error::AddressSpace(crate::address_space::Error::NoFrames),
        Error::InvalidState,
        Error::NoSpace,
        Error::InvalidFd,
        Error::OutputTooLong,
        Error::UserEntryUnavailable,
    ];
    let image = crate::elf::contract_image(crate::elf::Machine::current());
    let plan = crate::elf::parse(&image, crate::elf::Machine::current(), 0).unwrap();
    let arguments = [b"init".as_slice()];
    let environment: [&[u8]; 0] = [];
    let mut runtime = NativeRuntime::prepare(
        ProcessId::INIT,
        &plan,
        AslrHook::new(19),
        &arguments,
        &environment,
    )
    .unwrap();
    let registers = runtime.start().unwrap();
    assert_eq!(runtime.entry(), plan.entry);
    assert_eq!(registers.instruction_pointer, plan.entry);
    assert_eq!(runtime.registers(), registers);
    assert!(runtime.alloc(PAGE_SIZE + 1).unwrap() >= plan.entry);
    assert_eq!(runtime.write_fd(1, b"init\n").unwrap(), 5);
    assert_eq!(runtime.write_fd(3, b"bad"), Err(Error::InvalidFd));
    assert_eq!(
        runtime.write_serial(&[0; MAX_WRITE + 1]),
        Err(Error::OutputTooLong)
    );
    runtime.exit(0).unwrap();
    assert!(runtime.is_exited());
    assert_eq!(runtime.alloc(1), Err(Error::InvalidState));
    assert_eq!(runtime.exit(1), Err(Error::InvalidState));
}

pub fn sample_profile_self_check() -> SampleMetrics {
    let start = crate::time::ticks();
    let image = crate::elf::contract_image(crate::elf::Machine::current());
    let plan = crate::elf::parse(&image, crate::elf::Machine::current(), 0).unwrap();
    let arguments = [b"norx-sample".as_slice()];
    let environment: [&[u8]; 0] = [];
    let mut runtime = NativeRuntime::prepare(
        ProcessId::INIT,
        &plan,
        AslrHook::new(23),
        &arguments,
        &environment,
    )
    .unwrap();
    runtime.start().unwrap();
    assert_eq!(runtime.write_fd(1, b"norx sample\n").unwrap(), 12);
    runtime.exit(7).unwrap();
    SampleMetrics {
        startup_ticks: crate::time::ticks().wrapping_sub(start),
        image_bytes: image.len(),
        user_memory_bytes: (plan.total_pages + 2) * PAGE_SIZE,
        syscall_count: 2,
    }
}
