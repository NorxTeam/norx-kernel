# Initial userspace ABI and libc strategy

Norx will start with a small, statically linked Norx libc layer over the
versioned Linux-shaped syscall ABI. This is an intentional scope decision:
Norx does not promise glibc, musl, or general Linux binary compatibility until
the required process, filesystem, memory, signal, and dynamic-linking APIs
exist.

## ABI contract

- `ABI_VERSION = 1` remains the userspace-visible ABI version.
- Syscalls use six 64-bit logical arguments and return either a non-negative
  result or the unsigned representation of a negative errno.
- The libc syscall shim owns architecture details: x86_64 `syscall` register
  placement and AArch64 `svc`/`x8` placement are not exposed to applications.
- Applications use the native x86_64 SysV or AArch64 AAPCS64 C ABI. The kernel
  does not emulate a foreign calling convention.
- Pointers are validated and copied at the kernel boundary; libc does not pass
  kernel pointers or host filesystem paths.

## First libc surface

The first layer will wrap the runtime slice already defined by the kernel:
`exit`, `wait`, `getpid`, `gettid`, `yield`, `sleep`, and `close`. `errno`,
fixed-width types, bounded string/memory helpers, and a small process/fd API
will be provided as ordinary userspace code. `read` and `write` wrappers stay
reserved until process-owned mappings and user-buffer pinning are implemented;
they must not silently fall back to host I/O.

The initial binaries are static and use the existing bounded native runtime.
`execve`, `mmap`/heap growth, signals, threads, and dynamic linking are added
to the libc surface only when their kernel contracts are present.

## Compatibility boundary

This strategy supports Norx-native programs and keeps the syscall ABI stable
without carrying a compatibility layer in the kernel. Bounded staged
`ET_DYN`/PIE metadata, relocation, TLS, and VFS-only library-search contracts
now exist; actual shared-object mapping and dynamic-linker execution remain
separate follow-up work. No glibc compatibility claim is made before those
items pass QEMU smoke tests on both supported architectures.

The frozen ABI header and a supported-call smoke example are published in the
shared `BoaKernel/userspace` tree.
