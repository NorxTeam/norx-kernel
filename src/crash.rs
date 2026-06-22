use crate::{error::KernelError, uefi::RawFramebuffer};

static mut FRAMEBUFFER: Option<RawFramebuffer> = None;

pub fn init(raw: RawFramebuffer) {
    unsafe {
        FRAMEBUFFER = Some(raw);
    }
}

pub fn fatal(error: KernelError) -> ! {
    crate::error::report(error);

    if let Some(raw) = unsafe { FRAMEBUFFER } {
        let mut fb = crate::framebuffer::init(raw);
        fb.crash(error);
    }

    crate::arch::halt()
}
