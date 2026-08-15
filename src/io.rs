use core::sync::atomic::{fence, Ordering};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MmioRegion {
    base: usize,
    size: usize,
}

impl MmioRegion {
    /// Creates a region for an already mapped virtual address.
    ///
    /// # Safety
    /// The caller must ensure that the complete range is mapped to device
    /// memory for the lifetime of the region.
    pub unsafe fn new(base: usize, size: usize) -> Option<Self> {
        if size == 0 || base.checked_add(size).is_none() {
            return None;
        }
        Some(Self { base, size })
    }

    fn address(self, offset: usize, width: usize) -> Option<usize> {
        if !width.is_power_of_two() {
            return None;
        }
        let end = offset.checked_add(width)?;
        if end > self.size {
            return None;
        }
        let address = self.base.checked_add(offset)?;
        (address % width == 0).then_some(address)
    }

    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn read_u8(self, offset: usize) -> Option<u8> {
        let address = self.address(offset, 1)?;
        fence(Ordering::SeqCst);
        let value = unsafe { (address as *const u8).read_volatile() };
        fence(Ordering::SeqCst);
        Some(value)
    }

    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn read_u16_le(self, offset: usize) -> Option<u16> {
        let address = self.address(offset, 2)?;
        fence(Ordering::SeqCst);
        let value = unsafe { (address as *const u16).read_volatile() };
        fence(Ordering::SeqCst);
        Some(u16::from_le(value))
    }

    pub fn read_u32_le(self, offset: usize) -> Option<u32> {
        let address = self.address(offset, 4)?;
        fence(Ordering::SeqCst);
        let value = unsafe { (address as *const u32).read_volatile() };
        fence(Ordering::SeqCst);
        Some(u32::from_le(value))
    }

    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn read_u64_le(self, offset: usize) -> Option<u64> {
        let address = self.address(offset, 8)?;
        fence(Ordering::SeqCst);
        let value = unsafe { (address as *const u64).read_volatile() };
        fence(Ordering::SeqCst);
        Some(u64::from_le(value))
    }

    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn write_u8(self, offset: usize, value: u8) -> bool {
        let Some(address) = self.address(offset, 1) else {
            return false;
        };
        fence(Ordering::SeqCst);
        unsafe { (address as *mut u8).write_volatile(value) };
        fence(Ordering::SeqCst);
        true
    }

    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn write_u16_le(self, offset: usize, value: u16) -> bool {
        let Some(address) = self.address(offset, 2) else {
            return false;
        };
        fence(Ordering::SeqCst);
        unsafe { (address as *mut u16).write_volatile(value.to_le()) };
        fence(Ordering::SeqCst);
        true
    }

    pub fn write_u32_le(self, offset: usize, value: u32) -> bool {
        let Some(address) = self.address(offset, 4) else {
            return false;
        };
        fence(Ordering::SeqCst);
        unsafe { (address as *mut u32).write_volatile(value.to_le()) };
        fence(Ordering::SeqCst);
        true
    }

    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    pub fn write_u64_le(self, offset: usize, value: u64) -> bool {
        let Some(address) = self.address(offset, 8) else {
            return false;
        };
        fence(Ordering::SeqCst);
        unsafe { (address as *mut u64).write_volatile(value.to_le()) };
        fence(Ordering::SeqCst);
        true
    }
}

pub fn contract_self_check() {
    assert!(unsafe { MmioRegion::new(0, 0) }.is_none());
    assert!(unsafe { MmioRegion::new(usize::MAX, 1) }.is_none());
    let region = unsafe { MmioRegion::new(0x1000, 8) }.expect("valid MMIO contract range");
    assert!(region.read_u16_le(1).is_none());
    assert!(region.read_u32_le(8).is_none());

    let address = crate::address::VirtAddr::new(0x1000);
    assert!(!unsafe { dma_for_device(address, 0) });
    assert!(!unsafe { dma_for_cpu(crate::address::VirtAddr::new(usize::MAX), 1) });

    #[cfg(target_arch = "x86_64")]
    {
        assert!(PioRegion::new(0xffff, 2).is_none());
        let ports = PioRegion::new(0x3f8, 8).expect("valid PIO contract range");
        assert!(ports.read_u8(8).is_none());
        assert!(ports.read_u16(1).is_none());
        assert!(ports.read_u32(5).is_none());
    }
}

#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PioRegion {
    base: u16,
    size: u16,
}

#[cfg(target_arch = "x86_64")]
impl PioRegion {
    pub fn new(base: u16, size: u16) -> Option<Self> {
        if size == 0 || base as u32 + size as u32 > 0x1_0000 {
            return None;
        }
        Some(Self { base, size })
    }

    fn port(self, offset: usize) -> Option<u16> {
        if offset >= self.size as usize {
            return None;
        }
        self.base.checked_add(offset as u16)
    }

    pub fn base(self) -> u16 {
        self.base
    }

    fn port_width(self, offset: usize, width: usize) -> Option<u16> {
        if offset.checked_add(width)? > self.size as usize {
            return None;
        }
        self.base.checked_add(offset as u16)
    }

    pub fn read_u8(self, offset: usize) -> Option<u8> {
        self.port(offset).map(crate::arch::port_read)
    }

    pub fn write_u8(self, offset: usize, value: u8) -> bool {
        let Some(port) = self.port(offset) else {
            return false;
        };
        crate::arch::port_write(port, value);
        true
    }

    pub fn read_u16(self, offset: usize) -> Option<u16> {
        if offset & 1 != 0 {
            return None;
        }
        self.port_width(offset, 2).map(crate::arch::port_read_u16)
    }

    pub fn write_u16(self, offset: usize, value: u16) -> bool {
        if offset & 1 != 0 {
            return false;
        }
        let Some(port) = self.port_width(offset, 2) else {
            return false;
        };
        crate::arch::port_write_u16(port, value);
        true
    }

    pub fn read_u32(self, offset: usize) -> Option<u32> {
        if offset & 3 != 0 {
            return None;
        }
        self.port_width(offset, 4).map(crate::arch::port_read_u32)
    }

    pub fn write_u32(self, offset: usize, value: u32) -> bool {
        if offset & 3 != 0 {
            return false;
        }
        let Some(port) = self.port_width(offset, 4) else {
            return false;
        };
        crate::arch::port_write_u32(port, value);
        true
    }
}

#[allow(dead_code)]
/// Synchronizes a mapped buffer before handing it to a device.
///
/// # Safety
/// The caller must guarantee that the virtual range is mapped, accessible, and
/// owned by the DMA buffer for the duration of the operation.
pub unsafe fn dma_for_device(address: crate::address::VirtAddr, length: usize) -> bool {
    let Some((start, end, line)) = dma_range(address, length) else {
        return false;
    };
    #[cfg(target_arch = "x86_64")]
    {
        let _ = (start, end, line);
        fence(Ordering::SeqCst);
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let mut current = start;
        while current < end {
            core::arch::asm!("dc cvac, {}", in(reg) current, options(nostack));
            current += line;
        }
        core::arch::asm!("dsb ish", options(nostack));
    }
    true
}

#[allow(dead_code)]
/// Invalidates a mapped buffer after a device has written it.
///
/// # Safety
/// The caller must guarantee that the virtual range is mapped, accessible, and
/// owned by the DMA buffer for the duration of the operation.
pub unsafe fn dma_for_cpu(address: crate::address::VirtAddr, length: usize) -> bool {
    let Some((start, end, line)) = dma_range(address, length) else {
        return false;
    };
    #[cfg(target_arch = "x86_64")]
    {
        let _ = (start, end, line);
        fence(Ordering::SeqCst);
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let mut current = start;
        while current < end {
            core::arch::asm!("dc ivac, {}", in(reg) current, options(nostack));
            current += line;
        }
        core::arch::asm!("dsb ish", options(nostack));
    }
    true
}

fn dma_range(address: crate::address::VirtAddr, length: usize) -> Option<(usize, usize, usize)> {
    if length == 0 {
        return None;
    }
    let line = cache_line_size();
    let start = address.value() & !(line - 1);
    let end = address.value().checked_add(length)?.checked_add(line - 1)? & !(line - 1);
    Some((start, end, line))
}

#[cfg(target_arch = "x86_64")]
fn cache_line_size() -> usize {
    64
}

#[cfg(target_arch = "aarch64")]
fn cache_line_size() -> usize {
    let ctr: u64;
    unsafe {
        core::arch::asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack));
    }
    4usize << (ctr as usize & 0xf)
}
