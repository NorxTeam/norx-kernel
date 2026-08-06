const ENOSYS: u64 = 38;

pub fn init() {}

#[no_mangle]
extern "C" fn norx_aarch64_syscall_rust(
    _op: u64,
    _a0: u64,
    _a1: u64,
    _a2: u64,
    _a3: u64,
    _a4: u64,
    _a5: u64,
) -> u64 {
    0u64.wrapping_sub(ENOSYS)
}
