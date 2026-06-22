#[derive(Clone, Copy)]
pub struct Set(u64);

impl Set {
    pub const NONE: Self = Self(0);
    pub const CLOCK: Self = Self(1 << 0);
    pub const LOG_WRITE: Self = Self(1 << 1);
    pub const PROCESS_SPAWN: Self = Self(1 << 2);
    pub const VFS_READ: Self = Self(1 << 3);
    pub const VFS_WRITE: Self = Self(1 << 4);

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits & ((1 << 5) - 1))
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

pub const KERNEL: Set = Set::CLOCK
    .union(Set::LOG_WRITE)
    .union(Set::PROCESS_SPAWN)
    .union(Set::VFS_READ)
    .union(Set::VFS_WRITE);

pub fn names(set: Set, mut f: impl FnMut(&'static str)) {
    if set.contains(Set::CLOCK) {
        f("clock");
    }
    if set.contains(Set::LOG_WRITE) {
        f("log-write");
    }
    if set.contains(Set::PROCESS_SPAWN) {
        f("process-spawn");
    }
    if set.contains(Set::VFS_READ) {
        f("vfs-read");
    }
    if set.contains(Set::VFS_WRITE) {
        f("vfs-write");
    }
}
