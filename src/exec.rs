use crate::address_space::AslrHook;
use crate::address_space::PAGE_SIZE;
use crate::elf::{self, InitialRegisters, Machine};
use crate::process::{ProcessId, ProcessTable};
use crate::user_runtime::{self, NativeRuntime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Elf(elf::Error),
    Runtime(user_runtime::Error),
    Process(crate::process::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResultInfo<'a> {
    pub registers: InitialRegisters,
    pub closed_fds: usize,
    pub interpreter: Option<&'a [u8]>,
}

pub struct ExecRequest<'a> {
    pub owner: ProcessId,
    pub image: &'a [u8],
    pub machine: Machine,
    pub load_bias: usize,
    pub aslr: AslrHook,
    pub arguments: &'a [&'a [u8]],
    pub environment: &'a [&'a [u8]],
    pub requested_interpreter: Option<&'a [u8]>,
}

pub enum Replacement<'a> {
    Committed(NativeRuntime, ResultInfo<'a>),
    RolledBack(NativeRuntime, Error),
}

static mut EXEC_TEST_TABLE: ProcessTable = ProcessTable::new();
static mut EXEC_TEST_IMAGE: [u8; PAGE_SIZE + 4] = [0; PAGE_SIZE + 4];

pub fn replace<'a>(
    mut old: NativeRuntime,
    processes: &mut ProcessTable,
    request: ExecRequest<'a>,
) -> Replacement<'a> {
    let plan = match elf::parse(request.image, request.machine, request.load_bias) {
        Ok(plan) => plan,
        Err(error) => return Replacement::RolledBack(old, Error::Elf(error)),
    };
    let interpreter = match elf::select_interpreter(request.image, request.requested_interpreter) {
        Ok(interpreter) => interpreter,
        Err(error) => return Replacement::RolledBack(old, Error::Elf(error)),
    };
    let mut next = match NativeRuntime::prepare(
        request.owner,
        &plan,
        request.aslr,
        request.arguments,
        request.environment,
    ) {
        Ok(runtime) => runtime,
        Err(error) => return Replacement::RolledBack(old, Error::Runtime(error)),
    };
    if let Err(error) = next.start() {
        return Replacement::RolledBack(old, Error::Runtime(error));
    }
    let closed_fds = match processes.close_on_exec(request.owner) {
        Ok(closed_fds) => closed_fds,
        Err(error) => {
            let _ = next.exit(0);
            return Replacement::RolledBack(old, Error::Process(error));
        }
    };
    if let Err(error) = old.exit(0) {
        let _ = next.exit(0);
        return Replacement::RolledBack(old, Error::Runtime(error));
    }
    let registers = next.registers();
    Replacement::Committed(
        next,
        ResultInfo {
            registers,
            closed_fds,
            interpreter,
        },
    )
}

fn contract_success(
    processes: &mut ProcessTable,
    owner: ProcessId,
    fd: crate::process::FileDescriptor,
    image: &[u8],
    plan: &elf::LoadPlan,
    arguments: &[&[u8]],
    environment: &[&[u8]],
) {
    let mut old =
        NativeRuntime::prepare(owner, plan, AslrHook::new(23), arguments, environment).unwrap();
    old.start().unwrap();
    {
        let Replacement::Committed(mut next, result) = replace(
            old,
            processes,
            ExecRequest {
                owner,
                image,
                machine: Machine::current(),
                load_bias: 0,
                aslr: AslrHook::new(29),
                arguments,
                environment,
                requested_interpreter: Some(b"/lib/requested-loader.so"),
            },
        ) else {
            panic!()
        };
        assert_eq!(result.closed_fds, 1);
        assert_eq!(result.interpreter, Some(&b"/lib/requested-loader.so"[..]));
        assert_eq!(result.registers.instruction_pointer, plan.entry);
        assert_eq!(
            processes.fd_info(owner, fd),
            Err(crate::process::Error::InvalidFd)
        );
        next.exit(0).unwrap();
    }
}

fn contract_rollback(
    processes: &mut ProcessTable,
    owner: ProcessId,
    plan: &elf::LoadPlan,
    arguments: &[&[u8]],
    environment: &[&[u8]],
) {
    let bad_image = [0u8; 64];
    let mut old =
        NativeRuntime::prepare(owner, plan, AslrHook::new(31), arguments, environment).unwrap();
    old.start().unwrap();
    let Replacement::RolledBack(old, error) = replace(
        old,
        processes,
        ExecRequest {
            owner,
            image: &bad_image,
            machine: Machine::current(),
            load_bias: 0,
            aslr: AslrHook::new(37),
            arguments,
            environment,
            requested_interpreter: None,
        },
    ) else {
        panic!()
    };
    assert!(!old.is_exited());
    assert_eq!(error, Error::Elf(elf::Error::BadMagic));
    let mut old = old;
    old.exit(0).unwrap();
}

pub fn contract_self_check() {
    let image: &[u8] = unsafe {
        let image = core::ptr::addr_of_mut!(EXEC_TEST_IMAGE);
        image.write(elf::contract_image(Machine::current()));
        &*image
    };
    let plan = elf::parse(image, Machine::current(), 0).unwrap();
    let arguments = [b"init".as_slice()];
    let environment: [&[u8]; 0] = [];
    let processes = unsafe { &mut *core::ptr::addr_of_mut!(EXEC_TEST_TABLE) };
    let (owner, thread) = processes.create_init().unwrap();
    processes.switch_to(None, thread).unwrap();
    let fd = processes.open_fd(owner, 9, true, false).unwrap();
    processes.set_close_on_exec(owner, fd, true).unwrap();
    contract_success(processes, owner, fd, image, &plan, &arguments, &environment);
    contract_rollback(processes, owner, &plan, &arguments, &environment);
}
