# Norx syscall ABI boundary

The first ABI boundary is architecture-neutral and versioned as `ABI_VERSION =
1` in `src/syscall.rs`. `Number` owns Linux-shaped syscall numbers, `Args`
always carries six 64-bit words, and `Metadata` records argument count and
restart policy. `Errno::return_value()` encodes failures as the unsigned
two's-complement form of a negative errno, so both entry wrappers expose the
same logical return model.

x86_64 enters through `syscall` and returns through `sysretq`. The wrapper saves
the user stack, `rcx`, and `r11`, switches to the TSS-provided kernel stack,
maps Linux's `r10` fourth argument into the SysV `rcx` position, and preserves
the sixth argument as the seventh SysV stack argument. It masks interrupts
through `IA32_FMASK` and refuses initialization if the TSS stack is absent.

aarch64 uses the synchronous exception vector for SVC. It recognizes the SVC
class, maps `x8` to the syscall number and `x0..x5` to the six logical
arguments, calls the same `syscall::dispatch`, restores the saved registers,
and returns with `eret`. Unsupported numbers and currently unimplemented
numbers both return `-ENOSYS` through the common dispatcher.

The first runtime slice now handles `read`, `write`, `exit`, `wait`, `getpid`,
`gettid`, `yield`, `sleep`, and `close` against the bounded process table.
Every process starts with nonblocking fd 0 and writable fds 1/2. `read` polls
serial input and returns `-EAGAIN` when no byte is ready; `write` copies at most
1024 bytes from the active user address space to the kernel console, which fans
out to serial and the available graphical console. Neither call falls back to
host I/O. Filesystem-backed descriptors and blocking device streams remain
follow-up work.

`Timespec` is explicitly two signed 64-bit fields; user pointers and words are
64-bit at this stage. `RestartPolicy` is metadata for the blocking boundary.
The ABI and runtime self-checks verify table size, argument width, error
encoding, structure alignment, and control syscalls during boot; the bootlog
reports the ABI version, table size, negative-error convention, and runtime
checks on both targets.

The current `usercopy` boundary validates null/canonical/overflow/size rules.
x86_64 uses an IDT assembly shim with an active-copy range and recovery RIP;
aarch64 uses the corresponding SVC-vector data-abort path and ELR recovery.
Both return an `EFAULT`-class result for an unmapped copy without entering the
fatal exception path. No syscall currently accepts a user pointer, so the
copy boundary is the only TOCTOU surface; each operation copies into or out of
a caller-owned kernel slice in one bounded operation. Process-owned page
tables and multi-step TOCTOU pinning remain prerequisites for future pointer-
accepting syscalls.

The public C header and ABI smoke example live under the shared
`BoaKernel/userspace` tree: `include/norx/syscall.h` and
`examples/abi_smoke.c`. They are freestanding source artifacts; linking and
user-mode execution remain deferred until the runtime prerequisites listed
above are active.
