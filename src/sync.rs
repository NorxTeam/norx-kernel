use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Maximum bounded wait for a kernel lock. A waiter cannot sleep while local
/// interrupts are masked, so lock contention beyond this limit is fail-stop.
pub const SMP_LOCK_SPIN_LIMIT: usize = 1_000_000;

/// A bounded SMP lock for fixed-capacity kernel state.
///
/// The owner token is the CPU id in the kernel and the host test thread id in
/// tests. Re-entry by that owner is counted so layered kernel helpers can call
/// one another while the state remains serialized. Callers must keep the
/// closure short and must not retain references obtained from it.
pub struct SmpLock<T> {
    owner: AtomicUsize,
    depth: AtomicUsize,
    value: UnsafeCell<T>,
}

impl<T> SmpLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            owner: AtomicUsize::new(0),
            depth: AtomicUsize::new(0),
            value: UnsafeCell::new(value),
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        #[cfg(all(test, not(target_os = "none"), not(target_os = "uefi")))]
        {
            let _guard = self.lock();
            // Host tests do not execute privileged interrupt instructions.
            unsafe { f(&mut *self.value.get()) }
        }
        #[cfg(not(all(test, not(target_os = "none"), not(target_os = "uefi"))))]
        {
            crate::arch::without_interrupts(|| {
                let _guard = self.lock();
                unsafe { f(&mut *self.value.get()) }
            })
        }
    }

    fn lock(&self) -> SmpGuard<'_, T> {
        let token = owner_token();
        if self.owner.load(Ordering::Acquire) == token {
            self.depth.fetch_add(1, Ordering::Relaxed);
            return SmpGuard { lock: self, token };
        }
        let mut spins = 0;
        loop {
            if self
                .owner
                .compare_exchange(0, token, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                self.depth.store(1, Ordering::Relaxed);
                return SmpGuard { lock: self, token };
            }
            if spins == SMP_LOCK_SPIN_LIMIT {
                panic!("SMP lock contention");
            }
            spins += 1;
            core::hint::spin_loop();
        }
    }
}

unsafe impl<T: Send> Sync for SmpLock<T> {}

struct SmpGuard<'a, T> {
    lock: &'a SmpLock<T>,
    token: usize,
}

impl<T> Drop for SmpGuard<'_, T> {
    fn drop(&mut self) {
        debug_assert_eq!(self.lock.owner.load(Ordering::Relaxed), self.token);
        if self.lock.depth.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.lock.owner.store(0, Ordering::Release);
        }
    }
}

fn owner_token() -> usize {
    #[cfg(all(test, not(target_os = "none"), not(target_os = "uefi")))]
    {
        use core::hash::{Hash, Hasher};
        use std::collections::hash_map::DefaultHasher;

        let mut hasher = DefaultHasher::new();
        std::thread::current().id().hash(&mut hasher);
        let token = (hasher.finish() as usize).wrapping_add(1);
        return if token == 0 { 1 } else { token };
    }
    #[cfg(not(all(test, not(target_os = "none"), not(target_os = "uefi"))))]
    {
        crate::arch::cpu_id().saturating_add(1).max(1)
    }
}

#[cfg(all(test, not(target_os = "none"), not(target_os = "uefi")))]
mod tests {
    use super::SmpLock;
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn serializes_reentry_and_foreign_threads() {
        let lock = Arc::new(SmpLock::new(0usize));
        let barrier = Arc::new(Barrier::new(2));
        let worker_lock = Arc::clone(&lock);
        let worker_barrier = Arc::clone(&barrier);
        let worker = thread::spawn(move || {
            worker_barrier.wait();
            for _ in 0..10_000 {
                worker_lock.with(|value| *value += 1);
            }
        });
        barrier.wait();
        for _ in 0..10_000 {
            lock.with(|value| {
                *value += 1;
                lock.with(|value| *value += 1);
            });
        }
        worker.join().unwrap();
        assert_eq!(lock.with(|value| *value), 30_000);
    }
}
