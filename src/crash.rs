use crate::error::KernelError;

pub fn fatal(error: KernelError) -> ! {
    crate::error::report(error);
    crate::arch::halt()
}
