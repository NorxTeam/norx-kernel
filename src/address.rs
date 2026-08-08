#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysAddr(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtAddr(usize);

impl PhysAddr {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    pub fn is_aligned(self, alignment: usize) -> bool {
        alignment.is_power_of_two() && self.0.is_multiple_of(alignment as u64)
    }

    pub fn checked_add(self, bytes: usize) -> Option<Self> {
        self.0.checked_add(bytes as u64).map(Self)
    }
}

impl VirtAddr {
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    pub const fn value(self) -> usize {
        self.0
    }

    pub fn is_aligned(self, alignment: usize) -> bool {
        alignment.is_power_of_two() && self.0.is_multiple_of(alignment)
    }

    pub fn checked_add(self, bytes: usize) -> Option<Self> {
        self.0.checked_add(bytes).map(Self)
    }
}
