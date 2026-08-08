const MAX_COPY: usize = 64 * 1024;

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
static COPY_ACTIVE: AtomicBool = AtomicBool::new(false);

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    r#"
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
    #[cfg(target_arch = "x86_64")]
    {
        COPY_ACTIVE.store(true, Ordering::Release);
        let result = unsafe {
            norx_x86_copy_from_user(output.as_mut_ptr(), address as *const u8, output.len())
        };
        COPY_ACTIVE.store(false, Ordering::Release);
        if result == 0 {
            Ok(output.len())
        } else {
            Err(Error::Unmapped)
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        COPY_ACTIVE.store(true, Ordering::Release);
        let result = unsafe {
            norx_aarch64_copy_from_user(output.as_mut_ptr(), address as *const u8, output.len())
        };
        COPY_ACTIVE.store(false, Ordering::Release);
        if result == 0 {
            Ok(output.len())
        } else {
            Err(Error::Unmapped)
        }
    }
}

pub fn copy_to_user(address: u64, input: &[u8]) -> Result<usize, Error> {
    validate(address, input.len())?;
    #[cfg(target_arch = "x86_64")]
    {
        COPY_ACTIVE.store(true, Ordering::Release);
        let result =
            unsafe { norx_x86_copy_from_user(address as *mut u8, input.as_ptr(), input.len()) };
        COPY_ACTIVE.store(false, Ordering::Release);
        if result == 0 {
            Ok(input.len())
        } else {
            Err(Error::Unmapped)
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        COPY_ACTIVE.store(true, Ordering::Release);
        let result =
            unsafe { norx_aarch64_copy_from_user(address as *mut u8, input.as_ptr(), input.len()) };
        COPY_ACTIVE.store(false, Ordering::Release);
        if result == 0 {
            Ok(input.len())
        } else {
            Err(Error::Unmapped)
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub fn handles_fault(rip: u64) -> bool {
    if !COPY_ACTIVE.load(Ordering::Acquire) {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    let (start, end) = (
        norx_x86_copy_from_user as *const () as usize as u64,
        norx_x86_copy_from_user_recovery as *const () as usize as u64,
    );
    #[cfg(target_arch = "aarch64")]
    let (start, end) = (
        norx_aarch64_copy_from_user as *const () as usize as u64,
        norx_aarch64_copy_from_user_recovery as *const () as usize as u64,
    );
    rip >= start && rip < end
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub fn recovery_address() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        norx_x86_copy_from_user_recovery as *const () as usize as u64
    }
    #[cfg(target_arch = "aarch64")]
    {
        norx_aarch64_copy_from_user_recovery as *const () as usize as u64
    }
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
}
