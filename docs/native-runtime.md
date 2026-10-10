# Native runtime boundary

`user_runtime::NativeRuntime` is the bounded first runtime around an ELF
`LoadPlan`. It maps each validated PT_LOAD page into a process-owned
`AddressSpace`, creates the guarded stack/register image, exposes an explicit
`start()` boundary, provides a bounded heap-page allocator, routes stdout/stderr
through the kernel serial bootlog, and releases all mappings on `exit()`.

The fixed runtime store has one slot per process slot, but each entry also
stores the complete generational `ProcessId`. The low bits select a bounded
slot; every start, resumable entry, fault, take, and discard operation then
requires an exact full-ID match. Reaping advances the process generation, so
an exited process ID cannot operate on a later process's runtime or address
space. Installation also checks that the runtime's address-space owner is the
same full ID. Store access uses the shared reentrant per-CPU-token SMP lock
with local IRQ masking and a bounded fail-stop contention limit; address-space
destruction happens after `take()` releases the store lock.

The boot self-check uses a static init-style ELF fixture and verifies start,
serial/FD output, allocation, invalid-FD/oversized-output rejection, and clean
exit on both targets. The representative external fixtures now also execute in
real user mode on graphical QEMU for x86_64 and AArch64:

- Rust exercises startup parsing, `alloc::Vec`, `getpid`, `gettid`, `yield`, and
  `exit`;
- C exercises the freestanding runtime, atomics/threading boundary, allocator
  boundary, and `exit`;
- C++ exercises placement new, DSO-filtered/idempotent `__cxa_finalize`, and a
  normal exit that drains the bounded C/C++ callback registry; its destructor
  and C `atexit` handlers emit separate markers before the process exits.

The kernel-side staged smoke remains the evidence for process creation, ELF
loading, VFS file I/O, dynamic-loader metadata, signal/event models, W^X,
fault boundaries, and teardown. Pointer-based userspace `read`/`write`, real
dynamic linking, and asynchronous signal delivery remain explicit follow-up
ABI gates; these fixtures do not claim them prematurely.
