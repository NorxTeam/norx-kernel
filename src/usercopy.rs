#![allow(
    dead_code,
    reason = "Staged architecture and fault hooks are not all enabled in bootstrap"
)]

const MAX_COPY: usize = 64 * 1024;

use core::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

#[cfg(test)]
extern crate std;

const MAX_CPUS: usize = 64;
const NO_CPU_ID: u64 = u64::MAX;

struct CpuCopyState {
    cpu_id: AtomicU64,
    active_frame: AtomicPtr<CopyFrame>,
}

impl CpuCopyState {
    const fn new() -> Self {
        Self {
            cpu_id: core::sync::atomic::AtomicU64::new(NO_CPU_ID),
            active_frame: AtomicPtr::new(core::ptr::null_mut()),
        }
    }
}

// CPU IDs are architectural identifiers, not dense array indices: x2APIC and
// MPIDR values may be sparse. The registry gives each online ID one bounded
// private frame slot without requiring a global lock in the fault path.
static COPY_STATES: [CpuCopyState; MAX_CPUS] = [const { CpuCopyState::new() }; MAX_CPUS];

#[repr(C)]
struct CopyFrame {
    faultable_start: u64,
    recovery: u64,
    faultable_end: u64,
}

impl CopyFrame {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn current() -> Self {
        let faultable_start = copy_stub_address();
        let recovery = copy_recovery_address();
        Self {
            faultable_start,
            recovery,
            faultable_end: recovery,
        }
    }

    fn contains(&self, rip: u64) -> bool {
        rip >= self.faultable_start && rip < self.faultable_end
    }
}

fn state_for_cpu_in(states: &[CpuCopyState], cpu_id: usize) -> Option<&CpuCopyState> {
    let cpu_id = u64::try_from(cpu_id).ok()?;
    if cpu_id == NO_CPU_ID {
        return None;
    }
    if let Some(state) = states
        .iter()
        .find(|state| state.cpu_id.load(Ordering::Acquire) == cpu_id)
    {
        return Some(state);
    }
    states.iter().find(|state| {
        state
            .cpu_id
            .compare_exchange(NO_CPU_ID, cpu_id, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    })
}

fn state_for_cpu(cpu_id: usize) -> Option<&'static CpuCopyState> {
    state_for_cpu_in(&COPY_STATES, cpu_id)
}

fn current_cpu_state() -> Option<&'static CpuCopyState> {
    state_for_cpu(crate::arch::cpu_id())
}

fn activate_frame(state: &CpuCopyState, frame: &CopyFrame) -> Result<(), Error> {
    let frame = core::ptr::addr_of!(*frame).cast_mut();
    state
        .active_frame
        .compare_exchange(
            core::ptr::null_mut(),
            frame,
            Ordering::Release,
            Ordering::Acquire,
        )
        .map(|_| ())
        .map_err(|_| Error::Busy)
}

fn deactivate_frame(state: &CpuCopyState) {
    state
        .active_frame
        .store(core::ptr::null_mut(), Ordering::Release);
}

fn with_active_frame<T>(operation: impl FnOnce() -> T) -> Result<T, Error> {
    let state = current_cpu_state().ok_or(Error::CpuUnavailable)?;
    let frame = CopyFrame::current();
    crate::arch::without_interrupts(|| {
        activate_frame(state, &frame)?;
        let result = operation();
        deactivate_frame(state);
        Ok(result)
    })
}

fn fault_recovery_for(state: &CpuCopyState, rip: u64) -> Option<u64> {
    let frame = state.active_frame.load(Ordering::Acquire);
    if frame.is_null() {
        return None;
    }
    // The pointer is published only after the stack frame is initialized and
    // cleared before that frame leaves scope; the fault runs on this CPU.
    let frame = unsafe { &*frame };
    frame.contains(rip).then_some(frame.recovery)
}

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    r#"
    .section .text,"ax"
    .global norx_x86_copy_from_user
norx_x86_copy_from_user:
    mov rcx, rdx
    rep movsb
    xor eax, eax
    ret
norx_x86_copy_from_user_recovery:
    mov eax, 14
    ret
norx_x86_copy_from_user_end:
    nop
"#
);

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    r#"
    .global norx_aarch64_copy_from_user
norx_aarch64_copy_from_user:
    cbz x2, 2f
1:
    ldrb w3, [x1], #1
    strb w3, [x0], #1
    subs x2, x2, #1
    b.ne 1b
2:
    mov w0, #0
    ret
norx_aarch64_copy_from_user_recovery:
    mov w0, #14
    ret
norx_aarch64_copy_from_user_end:
    nop
"#
);

#[cfg(target_arch = "x86_64")]
extern "C" {
    fn norx_x86_copy_from_user(destination: *mut u8, source: *const u8, length: usize) -> u64;
    fn norx_x86_copy_from_user_recovery();
}

#[cfg(target_arch = "aarch64")]
extern "C" {
    fn norx_aarch64_copy_from_user(destination: *mut u8, source: *const u8, length: usize) -> u64;
    fn norx_aarch64_copy_from_user_recovery();
}

#[cfg(target_arch = "x86_64")]
const USER_LIMIT: u64 = 0x0000_8000_0000_0000;
#[cfg(target_arch = "aarch64")]
const USER_LIMIT: u64 = 0x0000_1000_0000_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Null,
    NonCanonical,
    Overflow,
    TooLarge,
    Unmapped,
    Busy,
    CpuUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserRange {
    pub address: u64,
    pub length: usize,
}

pub fn validate(address: u64, length: usize) -> Result<UserRange, Error> {
    if address == 0 {
        return Err(Error::Null);
    }
    if length > MAX_COPY {
        return Err(Error::TooLarge);
    }
    let end = address.checked_add(length as u64).ok_or(Error::Overflow)?;
    if address >= USER_LIMIT || end > USER_LIMIT {
        return Err(Error::NonCanonical);
    }
    Ok(UserRange { address, length })
}

pub fn copy_from_user(address: u64, output: &mut [u8]) -> Result<usize, Error> {
    validate(address, output.len())?;
    let result = with_active_frame(|| unsafe {
        copy_stub(output.as_mut_ptr(), address as *const u8, output.len())
    })?;
    if result == 0 {
        Ok(output.len())
    } else {
        Err(Error::Unmapped)
    }
}

pub fn copy_to_user(address: u64, input: &[u8]) -> Result<usize, Error> {
    validate(address, input.len())?;
    let result = with_active_frame(|| unsafe {
        copy_stub(address as *mut u8, input.as_ptr(), input.len())
    })?;
    if result == 0 {
        Ok(input.len())
    } else {
        Err(Error::Unmapped)
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub fn fault_recovery(rip: u64) -> Option<u64> {
    fault_recovery_for(current_cpu_state()?, rip)
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn copy_stub_address() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        norx_x86_copy_from_user as *const () as usize as u64
    }
    #[cfg(target_arch = "aarch64")]
    {
        norx_aarch64_copy_from_user as *const () as usize as u64
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn copy_recovery_address() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        norx_x86_copy_from_user_recovery as *const () as usize as u64
    }
    #[cfg(target_arch = "aarch64")]
    {
        norx_aarch64_copy_from_user_recovery as *const () as usize as u64
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn copy_stub(destination: *mut u8, source: *const u8, length: usize) -> u64 {
    norx_x86_copy_from_user(destination, source, length)
}

#[cfg(target_arch = "aarch64")]
unsafe fn copy_stub(destination: *mut u8, source: *const u8, length: usize) -> u64 {
    norx_aarch64_copy_from_user(destination, source, length)
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub fn recovery_address() -> u64 {
    copy_recovery_address()
}

pub fn contract_self_check() {
    assert_eq!(validate(0, 1), Err(Error::Null));
    assert_eq!(validate(USER_LIMIT, 1), Err(Error::NonCanonical));
    assert_eq!(validate(u64::MAX, 1), Err(Error::Overflow));
    assert_eq!(validate(0x1000, MAX_COPY + 1), Err(Error::TooLarge));
    let mut output = [0u8; 1];
    let unmapped = USER_LIMIT - 0x1000;
    assert_eq!(copy_from_user(unmapped, &mut output), Err(Error::Unmapped));
    assert_eq!(copy_to_user(unmapped, b"x"), Err(Error::Unmapped));
    #[cfg(target_arch = "aarch64")]
    {
        let fault = crate::arch::tables::last_usercopy_fault().expect("aarch64 usercopy fault");
        let exception_class = (fault.exception.syndrome >> 26) & 0x3f;
        assert!(matches!(exception_class, 0x24 | 0x25));
        assert_eq!(fault.exception.fault_address, unmapped);
        assert_eq!(fault.recovery_address, recovery_address());
        let copy_start = norx_aarch64_copy_from_user as *const () as u64;
        assert!(fault.exception.return_address >= copy_start);
        assert!(fault.exception.return_address < fault.recovery_address);
    }
}

/// Exercise the actual user page tables after paging has been initialized.
/// The early boundary check above intentionally runs before those tables exist.
pub fn mapped_copy_contract_self_check() {
    let mut space = crate::address_space::AddressSpace::new(
        crate::process::ProcessId::INIT,
        crate::address_space::AslrHook::new(23),
    )
    .expect("usercopy test address space");
    #[cfg(target_arch = "x86_64")]
    let hint = 0x4000_0000_0000;
    #[cfg(target_arch = "aarch64")]
    let hint = 0x0800_0000_0000;
    let address = space
        .aslr_base(
            hint,
            crate::address_space::PAGE_SIZE * 2,
            crate::address_space::PAGE_SIZE,
        )
        .expect("usercopy test address");
    space
        .map_anonymous(address, crate::address_space::PageFlags::USER_RW)
        .expect("mapped usercopy page");
    let initial = *b"mapped-copy";
    space.load(address, &initial).expect("load usercopy page");
    space.activate().expect("activate usercopy page");
    let root = space.root_frame().expect("usercopy root frame");
    assert!(crate::arch::switch_to_user(root));

    let mut output = [0; 11];
    assert_eq!(
        copy_from_user(address as u64, &mut output),
        Ok(initial.len())
    );
    assert_eq!(output, initial);
    assert_eq!(copy_to_user(address as u64, b"rewritten!"), Ok(10));
    let mut rewritten = [0; 10];
    assert_eq!(copy_from_user(address as u64, &mut rewritten), Ok(10));
    assert_eq!(&rewritten, b"rewritten!");

    let unmapped = address + crate::address_space::PAGE_SIZE;
    assert_eq!(
        copy_from_user(unmapped as u64, &mut [0; 1]),
        Err(Error::Unmapped)
    );
    assert_eq!(copy_to_user(unmapped as u64, b"x"), Err(Error::Unmapped));

    crate::arch::restore_kernel_address_space();
    space
        .destroy()
        .expect("destroy usercopy test address space");
}

#[cfg(test)]
mod tests {
    use super::{
        activate_frame, deactivate_frame, fault_recovery_for, state_for_cpu_in, CopyFrame,
        CpuCopyState,
    };
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn concurrent_mapped_and_unmapped_frames_are_cpu_local() {
        let states = Arc::new([CpuCopyState::new(), CpuCopyState::new()]);
        let mapped: &'static CopyFrame = &*Box::leak(Box::new(CopyFrame {
            faultable_start: 0x1000,
            recovery: 0x2000,
            faultable_end: 0x2000,
        })) as &'static CopyFrame;
        let unmapped: &'static CopyFrame = &*Box::leak(Box::new(CopyFrame {
            faultable_start: 0x3000,
            recovery: 0x4000,
            faultable_end: 0x4000,
        })) as &'static CopyFrame;
        let ready = Arc::new(Barrier::new(2));
        let mapped_cleared = Arc::new(Barrier::new(2));

        let mapped_states = Arc::clone(&states);
        let mapped_ready = Arc::clone(&ready);
        let mapped_cleared_barrier = Arc::clone(&mapped_cleared);
        let mapped_thread = thread::spawn(move || {
            let state = &mapped_states[0];
            activate_frame(state, mapped).unwrap();
            mapped_ready.wait();
            assert_eq!(
                fault_recovery_for(state, mapped.faultable_start),
                Some(0x2000)
            );
            deactivate_frame(state);
            mapped_cleared_barrier.wait();
        });

        let unmapped_states = Arc::clone(&states);
        let unmapped_ready = Arc::clone(&ready);
        let unmapped_cleared = Arc::clone(&mapped_cleared);
        let unmapped_thread = thread::spawn(move || {
            let state = &unmapped_states[1];
            activate_frame(state, unmapped).unwrap();
            unmapped_ready.wait();
            unmapped_cleared.wait();
            assert_eq!(
                fault_recovery_for(state, unmapped.faultable_start),
                Some(0x4000)
            );
            assert_eq!(fault_recovery_for(state, mapped.faultable_start), None);
            deactivate_frame(state);
        });

        mapped_thread.join().unwrap();
        unmapped_thread.join().unwrap();
    }

    #[test]
    fn sparse_cpu_ids_claim_distinct_bounded_slots() {
        let states = Arc::new([
            CpuCopyState::new(),
            CpuCopyState::new(),
            CpuCopyState::new(),
            CpuCopyState::new(),
        ]);
        let ready = Arc::new(Barrier::new(4));
        let handles = [0x20usize, 0x8000_0000, 0x1000_0000, 0x7f]
            .into_iter()
            .map(|cpu_id| {
                let states = Arc::clone(&states);
                let ready = Arc::clone(&ready);
                thread::spawn(move || {
                    ready.wait();
                    state_for_cpu_in(&states, cpu_id).unwrap() as *const CpuCopyState as usize
                })
            })
            .collect::<Vec<_>>();
        let first = handles[0].join().unwrap();
        let second = handles[1].join().unwrap();
        let third = handles[2].join().unwrap();
        let fourth = handles[3].join().unwrap();
        assert_ne!(first, second);
        assert_ne!(first, third);
        assert_ne!(first, fourth);
        assert_ne!(second, third);
        assert_ne!(second, fourth);
        assert_ne!(third, fourth);
        assert_eq!(
            state_for_cpu_in(&states, 0x20).unwrap() as *const _ as usize,
            first
        );
        assert!(state_for_cpu_in(&states, u64::MAX as usize).is_none());
    }

    #[test]
    fn malformed_ranges_are_rejected_without_wraparound() {
        let addresses = [0, 1, super::USER_LIMIT - 1, super::USER_LIMIT, u64::MAX];
        let lengths = [0, 1, super::MAX_COPY, super::MAX_COPY + 1, usize::MAX];
        for address in addresses {
            for length in lengths {
                let result = super::validate(address, length);
                if address == 0
                    || length > super::MAX_COPY
                    || address
                        .checked_add(length as u64)
                        .is_none_or(|end| address >= super::USER_LIMIT || end > super::USER_LIMIT)
                {
                    assert!(result.is_err(), "accepted malformed range");
                }
            }
        }
    }

    #[test]
    fn inactive_and_out_of_range_frames_never_recover() {
        let state = CpuCopyState::new();
        let frame = CopyFrame {
            faultable_start: 0x1000,
            recovery: 0x2000,
            faultable_end: 0x2000,
        };
        assert_eq!(fault_recovery_for(&state, 0x1000), None);
        activate_frame(&state, &frame).unwrap();
        assert_eq!(fault_recovery_for(&state, 0x0fff), None);
        assert_eq!(fault_recovery_for(&state, 0x1000), Some(0x2000));
        assert_eq!(fault_recovery_for(&state, 0x2000), None);
        deactivate_frame(&state);
        assert_eq!(fault_recovery_for(&state, 0x1000), None);
    }

    #[test]
    fn nested_copy_cannot_replace_the_active_frame() {
        let state = CpuCopyState::new();
        let outer = CopyFrame {
            faultable_start: 0x1000,
            recovery: 0x2000,
            faultable_end: 0x2000,
        };
        let inner = CopyFrame {
            faultable_start: 0x3000,
            recovery: 0x4000,
            faultable_end: 0x4000,
        };
        activate_frame(&state, &outer).unwrap();
        assert_eq!(activate_frame(&state, &inner), Err(super::Error::Busy));
        assert_eq!(fault_recovery_for(&state, 0x1000), Some(0x2000));
        assert_eq!(fault_recovery_for(&state, 0x3000), None);
        deactivate_frame(&state);
    }
}
