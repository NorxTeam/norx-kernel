 # Nordix Roadmap

Nordix is the future assembled distribution name. The projects below remain
general-purpose components that implement the shared platform ABI; they are
not utilities created only for one distribution branding.

This is the shared roadmap for everything under `BoaKernel`. It is kept next
to the repositories so the kernel, utilities, forks, and future system
components follow one plan.

## Direction

Norx follows Linux-proven principles where they remove design risk: a stable
file-descriptor and syscall model, a VFS with mounts, an explicit
bus/device/driver model, ELF userspace, capability-aware permissions, and
small composable kernel subsystems. This is compatibility with mature ideas,
not an attempt to copy the whole Linux kernel.

- Keep architecture-specific entry code and hardware details behind narrow
  interfaces.
- Prefer a boring, inspectable kernel core over speculative abstractions.
- Treat resource ownership, error paths, teardown, and partial hardware
  failure as first-class design requirements.
- Use native Linux-like formats and conventions when they have been proven in
  practice; do not preserve the old universal-binary probes.
- Implement one complete vertical path before adding another abstraction.
- Keep early boot drivers minimal; move restartable and risky drivers out of
  the kernel when a userspace service model exists.
- Every new subsystem needs a QEMU smoke path, serial diagnostics, and a
  failure test before it becomes a dependency for later work.

## Open implementation tracks

### 0. Foundation fixes

### Memory and virtual memory

- [x] Finish and verify the x86_64 lazy fault path backed by real physical
  frames through the direct map.
- [x] Implement real aarch64 exception entry with fault syndrome and fault
  address handling before enabling lazy mapping there.
- [x] Finish the architecture-neutral page-fault contract, zero-fill policy,
  out-of-memory behavior, guard pages, and mapping teardown.
- [x] Define the protection boundary for user address spaces before adding process
  execution. Keep W^X, NX, ASLR, and strict executable mappings in scope.

### Scheduling and interrupts

  - [x] Verify scheduler runtime state and 100 Hz accounting as a tested
  foundation.
- [x] Replace x86_64 legacy PIT/PIC scheduler routing with
  APIC/HPET calibration and a clean interrupt-time source.
- [x] Finish aarch64 GICv2/v3 discovery, generic timer IRQ wiring,
  interrupt acknowledgement/EOI, and timer accounting.
- [x] Define interrupt ownership, deferred work, and safe driver callbacks before
  adding high-throughput device queues.

### Early console and input

- [x] Resolve the exact display-mode contract for future GOP/virtio-gpu mode
  changes instead of treating host-window scaling as a guest resize.
- [x] Define the failure policy for an absent or stalled early UART; early logging
  must not wait forever when the configured device is missing.

### Storage and VFS baseline

- [x] Complete and verify the bounded mount tree and namespace-aware path
  lookup over the in-kernel RAM filesystem, including `/hello.txt`.
- [x] Add the bounded x86_64 persistent-storage vertical: modern virtio-blk,
  a pre-seeded FAT32 `/NORX.PST` record, and a two-boot CI readback smoke.
- [ ] Replace staged read-only FAT32, ext4, and btrfs readers with persistent
  storage, descriptor inheritance across concurrent `spawn2`, and the complete
  POSIX filesystem ABI; retain bounded process-local tables for `open`, `pipe`,
  `dup2`, and `close`.
  - [x] Route staged spawn finalization and rollback through an explicit
    `{parent, child, thread}` transaction, including process-group setup.
  - [x] Make FAT32, ext4, and btrfs volume probes consume a bounded active
    block partition view instead of assuming raw LBA zero.
  - [x] Publish bounded `lseek`, `fstat`, and `fchmod` calls for VFS handles in
    the Rust and C syscall interfaces.
  - [x] Add bounded FAT32 cluster allocation/free-chain updates for existing
    files, including mirrored-FAT consistency checks and grow/shrink fixtures.
  - [x] Introduce an explicit read-only `PersistentMount` reader boundary for
    partition-backed FAT32, ext4, and btrfs probes without copying into RAMFS.
  - [x] Add bounded persistent metadata (`stat`/`read_dir`) for FAT32, ext4, and
    btrfs, including malformed-directory rejection and bounded name conversion.
  - [x] Dispatch mounted persistent paths through VFS `lookup`, read-only
    `open`/`read`, `lseek`, `stat`/`fstat`, and `read_dir` without RAMFS imports.
  - [x] Connect persistent `fsync`/`sync_path` to the block flush boundary and
    return typed read-only/unsupported errors for backends and mutations that
    remain outside the bounded writable FAT32 slice.
  - [x] Auto-mount the first available read-only persistent volume at
    `/storage` and run bounded VFS lookup/read_dir/open/read/seek/fsync/reopen
    smoke with unmount rollback on failure.
  - [x] Add explicit `spawn2` arbitrary open-FD inheritance with rollback,
    `FD_CLOEXEC` preservation, and bounded descriptor-capacity checks.
  - [x] Publish bounded `fcntl(F_GETFD/F_SETFD)` support for `FD_CLOEXEC` in
    the Rust and C syscall interfaces.
  - [x] Add an explicit writable FAT32 VFS mount path for bounded existing-file
    rewrites plus short/LFN create, mkdir/rmdir, unlink, directory-chain growth,
    and same-directory rename; retain typed unsupported boundaries outside it.
  - [x] Exercise the writable FAT32 path from the boot mount lifecycle itself:
    mount with writable flags when the block device is writable, create/truncate
    a VFS file, write/fsync/seek/read it back, reopen it, and unmount with a
    durable-boundary check.
  - [x] Keep persistent hard links honest: route the bounded FAT32 boundary
    through VFS and return typed `ENOTSUP` while inode/nlink alias consistency
    is not implemented; cover malformed paths and no-entry-creation fixtures.
  - [x] Add a journal-clean ext4 writer boundary for existing depth-0 extent
    rewrites with bounded JBD2 ordering, rollback, and recovery rejection;
    retain typed unsupported allocation and directory mutation.
  - [x] Add a btrfs COW transaction boundary with generation/checksum ordering
    and rollback fixtures, while keeping allocation/root publication/recovery
    mutations read-only until the allocator is implemented.
  - [x] Route persistent `rmdir` through the VFS backend and add an x86 QEMU
    storage-image argument for exercising a real virtio-blk FAT32 path.
  - [x] Serialize process-runtime table access with a reentrant SMP lock keyed
    by the current CPU, with bounded contention failure and lock self-checks.
  - [x] Harden the C filesystem ABI smoke with layout, flags, errno, syscall
    numbering, and unevaluated wrapper-shape assertions on both target triples.
  - [x] Load bounded user ELF images from the mounted VFS `/storage` path with
    stat-before/after, direct reads for images larger than the legacy 4 KiB
    handle buffer, parse/prepare/discard boot coverage, and shared loader
    routing for legacy and resumable spawn paths; keep embedded fixtures as a
    compatibility fallback for bring-up and non-runtime tests.
  - [x] Prove a real external userspace `spawn2` fixture on x86_64 and
    aarch64 with inherited open descriptors, concurrent child transaction and
    `wait_status` behavior, and negative errno/bounds checks.
- [ ] Replace the smoke-only storage path with the mountable filesystem stack in
  the Universal Driver Framework track below.

### Architecture research kept for later

- [ ] Evaluate a single-address-space design only after a real hardware-enforced
  protection story exists: separate regions, MPK/MTE-like domains, a verifier,
  or a hybrid model.
- [ ] Treat lazy allocation as the default mapping policy, with explicit reserve,
  commit, zero-fill, and OOM semantics.
- [ ] Investigate ARM64 MTE and x86 LAM/MPK as optional hardening, never as the
  only safety boundary.
- [ ] Prefer shared-memory channels, ring buffers, and event queues over signals
  as the primary asynchronous IPC primitive.
- [ ] Use capabilities for authority instead of a single global root identity.
- [ ] Keep driver crash isolation, verified eBPF/WASM-like filters, persistent
  warm-reboot logs, and hot-reloadable modules as post-foundation work.

### 1. Universal Driver Framework

Build one coherent framework that can describe a PS/2 controller and a
complex PCI/virtio device without forcing every driver through architecture or
bus-specific hacks. The framework must be powerful enough for real hardware,
but small enough that a new driver is mostly probe logic and device policy.

#### 1.1 Deep audit and contract design

- [x] Audit every existing driver, bus helper, MMIO/PIO access, interrupt
  path, DMA assumption, global state, unsafe cast, fake capability, and test
  hook. Record what is removed, what is retained, and which caller owns each
  resource.
- [x] Remove dead drivers, duplicated register definitions, architecture leaks,
  fake devices, busy loops without a timeout, and error paths that silently
  continue after failed initialization.
- [x] Define the core objects: `Bus`, `Device`, `Driver`, `Resource`, `Irq`,
  `DmaBuffer`, `DeviceState`, and typed driver errors. Keep ownership explicit
  and make probe/remove/rebind behavior deterministic.
- [x] Define the lifecycle: discover, match, probe, publish, suspend/resume,
  quiesce, remove, and failed-probe cleanup. Use automatic resource cleanup
  where it prevents leaks, but do not hide hardware ordering requirements.
- [x] Separate early-boot services from normal runtime drivers. A driver must
  be able to report `deferred`, `unsupported`, `busy`, and `failed` without
  turning a missing optional device into a kernel panic.
- [x] Standardize MMIO, PIO, port I/O, endianness, register barriers, cache
  maintenance, DMA ownership, physical/virtual address conversion, and
  alignment checks for x86_64 and aarch64.
- [x] Define interrupt delivery for legacy IRQ, MSI/MSI-X, GIC, and threaded or
  deferred work. No driver may perform unbounded work in a hard IRQ handler.
- [x] Add bus/device/driver tracing to the serial-debugger without returning
  to a framebuffer shell. Include probe order, resources, IRQs, DMA, and
  teardown state.

#### 1.2 Serial and low-level input drivers

- [x] Finish UART support for the current early console and runtime serial
  devices, including timeout behavior, FIFO use, baud configuration, and
  RTS/CTS flow control where hardware exposes it.
- [x] Implement a PS/2 controller driver with command serialization, ACK and
  RESEND handling, bounded retries, controller self-test, port discovery,
  IRQ routing, and a clean fallback when no controller exists.
- [x] Implement PS/2 keyboard support for scan-code translation, modifiers,
  layouts, repeat policy, and event delivery without coupling it to the
  serial-debugger.
- [x] Implement PS/2 mouse support for packet decoding, buttons, wheel,
  optional extra buttons, synchronization recovery, and a device-independent
  pointer event stream.

#### 1.3 USB

- [x] Start with a QEMU-compatible xHCI host controller and design the host
  controller interface so EHCI/OHCI can be added without changing USB class
  drivers.
- [x] Implement port reset, speed detection, device and configuration
  descriptors, endpoint setup, control/bulk/interrupt transfers, transfer
  timeouts, cancellation, and DMA-safe buffers.
- [x] Add hub enumeration and disconnect handling before exposing devices to
  userspace.
- [x] Add USB HID keyboard and mouse drivers using the same event interfaces
  as PS/2 devices. Keep HID report parsing bounded and descriptor-driven.
- [x] Add serial and storage class support only after enumeration and transfer
  lifetime rules are stable.

#### 1.4 Audio, networking, and graphics

- [x] Add a QEMU-friendly audio path first (AC'97 or Intel HDA), with codec
  discovery, DMA ring handling, underrun/overrun recovery, a small PCM API,
  and a deterministic silent fallback when no audio device exists.
- [x] Add virtio-net as the first network driver, then cover e1000 or another
  simple PCI device. Define RX/TX ownership, queues, interrupts, checksum
  handling, link state, and packet lifetime before building protocols.
- [x] Build the minimum network stack needed for useful networking: Ethernet,
  ARP, IPv4, ICMP diagnostics, UDP, TCP, DHCP, DNS, and a socket-facing API.
  Keep IPv6 and advanced offloads behind explicit follow-up tasks.
- [x] Replace the current display assumption with a basic display driver
  contract: framebuffer discovery, mode list, mode selection, EDID, damage
  regions, flush, cursor, and hotplug notification.
- [x] Support the current firmware framebuffer and a simple virtio-gpu path
  first. Do not attempt 3D acceleration yet; keep the interface extensible for
  future complex GPUs without embedding a vendor driver in the early kernel.

#### 1.5 Block devices, filesystems, and mounts

- [x] Split the current RAM block smoke device into a block layer with sector
  geometry, queueing, completion, timeout, partition discovery, read-only
  mode, and explicit cache/writeback ownership.
- [x] Implement `ramfs` as the first real writable filesystem, with inodes,
  directories, regular files, file offsets, permissions, unlink/rename, and
  mount/unmount tests.
- [x] Implement FAT32 for EFI/QEMU volumes: BPB validation, cluster chains,
  long names, directory updates, read-only boot mode first, then safe writes.
- [x] Implement ext4 in staged form: superblock/group validation, extents,
  inode and directory reads, journaling boundaries, permissions, and a
  read-only mount before write support.
- [x] Implement btrfs only after the block, page-cache, checksum, and VFS
  contracts are stable. Start with a safe read-only mount, tree traversal,
  checksums, snapshots/subvolumes discovery, and explicit unsupported-feature
  errors.
- [x] Define the VFS mount API: mount tree, mount namespaces, path lookup,
  dentry/inode lifetime, mount flags, propagation rules, and unmount safety.
  Every filesystem must mount through the same interface.
- [x] Add mount sources and targets to the serial-debugger for inspection and
  recovery; no filesystem command should become a hidden test-only path.

#### 1.6 Framework completion criteria

- [x] Run the full driver matrix on QEMU x86_64 and aarch64 with absent-device,
  timeout, hot-unplug, malformed-descriptor, DMA-failure, and interrupt-storm
  tests.
- [x] Ensure every driver has bounded initialization, explicit teardown, a
  serial diagnostic view, and no architecture-specific code outside the
  appropriate bus/arch layer.
- [x] Document the public driver contracts and freeze the first stable version
  before adding high-level userspace device services.

### 2. Linux-like syscalls, processes, libraries, and execution

Replace the removed universal syscall probes with a real Linux-shaped userspace
foundation. Linux conventions are the default because they are well-tested,
well-documented, and compatible with existing toolchains; Norx keeps its own
implementation and security model.

#### 2.1 Audit and ABI boundary

- [x] Audit all remaining syscall entry code, register handling, TSS/kernel
  stacks, SVC vectors, error returns, pointer conversions, and stale comments.
  Delete obsolete universal bins, probe state, and compatibility names rather
  than carrying a second ABI.
- [x] Define an architecture-neutral syscall table and architecture-local
  entry wrappers. x86_64 should use the Linux-like register convention around
  `syscall/sysret`; aarch64 should use an SVC convention with the same logical
  argument and return model.
  - [x] Isolate the x86_64 syscall-entry stack from the ordinary kernel stack;
    verify resumable user exits and clean ELF returns without the former
    `rip=0x8` invalid-op failure.
- [x] Define syscall numbering/versioning, negative error returns, `errno`
  translation, restart rules, 32/64-bit types, time types, pointer widths,
  and structure alignment before publishing userspace headers.
- [x] Implement safe `copy_from_user`/`copy_to_user`, range validation,
  overflow checks, page-fault recovery, and protection against TOCTOU in every
  syscall that accepts userspace memory.

#### 2.2 Processes, threads, and address spaces

- [x] Design the process, thread, task, file-descriptor table, credentials,
  signal/event state, parent/child relationship, and exit/wait model.
- [x] Implement a minimal process address space with user/kernel separation,
  page-table ownership, guard pages, stack growth policy, ASLR hooks, W^X, and
  deterministic teardown.
- [x] Add kernel threads and user threads as separate concepts, with scheduler
  accounting, blocking/wakeup, preemption, and safe context switching.
- [x] Start with `exit`, `wait`, `getpid`, `gettid`, `yield`, `sleep`, and a
  small file-descriptor API. Add fork-like semantics only after address-space
  cloning and copy-on-write have a tested design.
- [x] Define capabilities and credentials before exposing privileged device,
  mount, memory, or network operations. Do not make a global root identity the
  only authorization mechanism.

#### 2.3 ELF loader and simple native binaries

- [x] Implement a bounded ELF64 loader for x86_64 first: header/program-header
  validation, `PT_LOAD` mapping, permissions, zero-fill, entry-point checks,
  stack construction, auxiliary vector, and initial register state.
- [x] Add a minimal native userspace runtime that can start a statically linked
  `init`-style binary, write to serial or a file descriptor, allocate memory,
  and exit cleanly.
- [x] Add aarch64 ELF loading only after the common loader contract is stable;
  keep architecture-specific relocations, entry state, and ABI details local.
- [x] Implement `execve`-like replacement with argument/environment copying,
  close-on-exec descriptors, interpreter selection, and failure rollback.
- [x] Add negative tests for malformed headers, overlapping segments,
  non-canonical addresses, W+X mappings, truncated files, oversized argv/env,
  and invalid entry points.

#### 2.4 Libraries and dynamic linking

- [x] Choose the initial userspace ABI and libc strategy: a small Norx libc
  layer with Linux-shaped calls, or an explicitly supported existing libc
  subset. Do not promise glibc compatibility before the required kernel APIs
  exist.
- [x] Support `ET_DYN`/PIE and shared objects in staged form: interpreter path,
  `PT_INTERP`, `PT_DYNAMIC`, symbol lookup, relocations, TLS, and library search
  policy.
- [x] Define `/lib` or a Norx equivalent through the VFS mount tree;
  library discovery must not depend on host filesystem paths.
- [x] Add a dynamic-linker smoke program with one shared library, one failing
  dependency, relocation checks, and clean process exit.

#### 2.5 Portable low-level virtual machine

- [x] Compare Wasm/WASI, LLVM bitcode/IR, and a small custom register VM against
  Norx requirements: verifier complexity, memory isolation, syscall imports,
  binary size, debugging, deterministic execution, and cross-architecture
  portability.
- [x] Do not embed the full LLVM toolchain in the kernel by default. LLVM may
  be used off-target as the compiler pipeline; the kernel should receive a
  compact verified module or native ELF, not a huge compiler runtime.
- [x] Select one module format and define its versioning, sections, types,
  imports/exports, relocation/linking model, debugging metadata, and library
  dependency rules.
- [x] Implement an interpreter and verifier first. Validate control flow,
  integer operations, memory bounds, stack depth, resource limits, and syscall
  capabilities before execution.
- [x] Add a VM memory model with isolated linear memory, bounded host calls,
  handles instead of raw kernel pointers, cancellation, and deterministic
  traps.
- [x] Evaluate the optional JIT gate after the interpreter is correct and
  profiled; the current bounded profile keeps Norx interpreter-only until a
  representative hot loop justifies generated code obeying W^X, architecture
  permissions, cache maintenance, and revocation rules.
- [x] Provide one portable sample program and one native ELF sample using the
  same high-level library calls, then compare startup cost, memory use, and
  syscall surface.

#### 2.6 Syscall and userspace completion criteria

- [x] Freeze the first documented syscall ABI and publish headers/examples in
  a separate userspace/toolchain area under the shared `BoaKernel` directory.
- [x] Exercise process creation, ELF loading, file I/O, memory mapping,
  dynamic linking, VM execution, faults, permissions, and process teardown in
  QEMU on both supported architectures.
- [x] Ensure no universal machine-code payload, architecture-neutral syscall
  probe, or test-only execution path remains in the kernel after the new path
  is active.

### 3. Protection, IPC, and service boundaries

- [x] Define the kernel/user trust boundary and capability transfer model.
- [x] Add channels, shared-memory rings, event queues, and blocking semantics
  as the primary IPC path; keep signal-like notifications secondary.
- [x] Move restartable or risky drivers toward userspace services once process
  isolation, DMA ownership, and a service supervisor exist.
- [x] Add persistent warm-reboot logs and panic context after the storage and
  mount layers can reserve a reliable backing region.

### 4. Validation and release discipline

- [x] Keep x86_64 and aarch64 builds, Clippy, formatting, GRUB image creation,
  and QEMU smoke tests in CI.
- [x] Add negative tests for every parser and hardware boundary: malformed
  boot tags, invalid descriptors, bad filesystems, broken ELF files, invalid
  syscall pointers, DMA failures, and interrupt races.
- [x] Keep serial-debugger output stable enough for automated smoke assertions,
  while treating framebuffer screenshots as visual regression artifacts.
- [x] Before declaring a subsystem stable, document its ownership model,
  failure behavior, ABI, supported hardware, and explicit unsupported cases.

### 5. Standalone projects and userspace

Every project in this section lives in its own repository under
`BoaKernel/REPONAME/`. Projects must consume the documented Norx ABI instead
of reaching into kernel internals. Each repository needs its own README,
license and contribution rules, reproducible cross-builds, unit/host tests,
QEMU smoke coverage with stable serial markers, and an installation layout
that can be assembled into a Norx system image. The dependency order below is
intentional: toolchains and libraries come first, then PID 1 and the shell
parser, then shell-intrinsic commands, account/session/security tools, the
standalone command set, and individually owned daemons and utilities.

#### 5.1 `toolchain` — C, C++, and Rust toolchains

- [x] Choose the initial strategy for C/C++ and Rust: maintained fork,
  upstream cross-compilation targets, or a small Norx-specific compiler
  layer. Record which components remain upstream and which are patched.
- [x] Define the Norx target triples, object format, calling convention,
  syscall ABI, startup files, linker scripts, relocation rules, TLS model,
  stack alignment, atomics, floating-point policy, and x86_64/aarch64
  feature sets.
- [x] Build a reproducible cross-toolchain with compiler, assembler, linker,
  debugger support, target headers, and a sysroot for both supported
  architectures. Keep host tools separate from target libraries.
- [x] Port the base C/C++ runtime and system libraries: headers, `libc`,
  `libm`, threading/atomics, C++ ABI, exception/unwind policy, startup and
  termination code, and the supported `libstdc++` or `libc++` subset.
- [x] Port the Rust target support: `core`, `alloc`, `compiler_builtins`,
  panic/runtime policy, `std` where justified, linker integration, build
  scripts, and a stable Norx userspace crate/API layer.
- [x] Compile and run representative C, C++, and Rust programs on graphical
  QEMU, including syscalls, process creation, ELF/file and dynamic-loader
  kernel gates, signal/event model checks, and clean exit on both architectures.
  The external fixtures prove real static ELF execution and clean exit; the
  staged kernel smoke remains the evidence for file I/O, dynamic-loader,
  signal, and event contracts that are not yet exposed as pointer-based
  userspace APIs.
- [x] Publish versioned SDK/sysroot artifacts and compatibility rules so
  quickinit, nsh, coreutils, and future applications can upgrade without
  silently changing the ABI. `toolchain/sdk.toml` freezes SDK 0.1.0, target
  manifests carry ABI and file hashes, and `publish-sdk.py` emits deterministic
  per-target release archives plus an index.

#### 5.1.8 Repository ownership and generated boundaries

- [x] Make `userspace` the only source of truth for public ABI headers, the
  freestanding C/C++ runtime, the Rust userspace API, and their source
  fixtures. Keep target specs, linker scripts, build orchestration, compiler
  pins, and SDK metadata in `toolchain`.
- [x] Remove the tracked duplicate header tree from `toolchain/sysroot`.
  Build scripts now stage the generated sysroot under the ignored
  `toolchain/build/sysroot` directory directly from `userspace` sources.
- [x] Remove the duplicate root-level `userspace` SDK recipe. Runtime and Rust
  packages keep recipes beside their sources; SDK packaging has one canonical
  recipe in `toolchain/gamma.toml`.
- [x] Add a CI guard that rejects tracked generated sysroot/build files and
  verifies every published SDK header against its canonical `userspace` source.
- [x] Document the package dependency graph and release hand-off from
  `userspace` sources through `toolchain`, runtime packages, and the final SDK
  before adding more language runtimes.
- [x] Establish the upstream-port policy for external projects: create a
  maintained Git fork of the upstream repository and add only the Norx port,
  integration, packaging, and compatibility changes required for the target
  system. Do not rewrite mature upstream projects from scratch.
- [x] Preserve the upstream remote, tags, license notices, and provenance in
  every fork. Keep Norx changes in an isolated branch or reviewable patch
  queue so upstream updates can be merged or rebased without losing local
  work.
- [x] Record the upstream URL, exact upstream commit/tag, local patch series,
  build options, license/trademark obligations, and generated artifact hashes
  in the repository documentation and Gamma recipe/lock metadata.
- [x] Add a repeatable upstream-sync procedure and CI job for each maintained
  fork: fetch a new upstream revision, reapply or rebase the Norx patches,
  run host/QEMU smoke tests, report conflicts, and retain the last known-good
  revision for rollback.

#### 5.2 `quickinit` — userspace init and service supervisor

Progress: the standalone `quickinit` repository now contains a host-testable
policy core for strict init configuration, dependency validation, startup
 ordering, restart/backoff limits, timeouts, signal forwarding, bounded
 structured logs, recovery mode, and supervisor CLI parsing. The kernel now
 exposes the bounded process/exec/wait handoff needed to run the bootstrap as
 PID 1; the remaining service configuration, signal, and storage interfaces
 stay open for the following checkboxes. Real Rust/C/C++ ELFs write through
 user pointers to the graphical/serial console on both supported architectures.

Progress: a separate `quickinit/bootstrap` no-std package now builds a pinned
external ELF for x86_64 and aarch64. The kernel embeds it ahead of the other
 userspace fixtures and the graphical QEMU smoke shows its write/getpid/wait/
 spawn/exit contract as PID 1, followed by child reaping and kernel-context
 restoration, on both architectures. The bootstrap remains intentionally
 smaller than the future service supervisor, but the first PID 1 handoff is
 now the boot contract.

Progress: quickinit is a userspace boot supervisor, separate from the kernel
and analogous to systemd/initd. Graphical boot renders a centered, stable
Unicode overlay (`─ │ ┌ ┐ └ ┘`) with a title, current stage, square progress
bar, and percentage. The backing console restores lines around the window
without whole-screen clearing; the overlay remains active from quickinit
through the full OS boot, changes to the crash state on failure, and will
close only when control is transferred into the future shell. Both graphical
QEMU smokes reach the serial debugger without panic or `INFO` output.

- [x] Keep the quickinit overlay stable from boot start through system-ready;
  reserve closure for the future shell hand-off.
- [x] Standardize component markers as `[   OK   ] component: status` (with
  matching `WARN`/`FAIL` results) for quickinit and userspace fixtures.

- [x] Start `quickinit` as the first userspace process after the kernel,
  continue the kernel log without losing sequence/order information, reap
  children, and expose a deterministic boot failure mode.
- [x] Define the init configuration format, environment, working directory,
  standard streams, capabilities, resource limits, dependencies, startup
  ordering, readiness notification, and shutdown behavior. `quickinit` now
  validates the strict bounded contract, merges global/per-service
  environments, orders dependency-safe startup, gates notification readiness,
  and applies configured shutdown signals and timeout actions; target syscall
  application remains scoped to the following kernel ABI work.
- [x] Implement autostart for programs, foreground/background processes,
  services and daemons, with restart policies, crash limits, timeouts,
  dependency failures, and clean signal forwarding. `quickinit` now models
  explicit autostart/manual start, foreground/background mode, service/daemon/
  oneshot lifecycles, `Stopping` and timeout escalation, intentional-stop
  restart suppression, dependency blocking, and best-effort signal fan-out;
  process groups and target signal/launch syscalls remain a separate kernel
  ABI boundary.
- [x] Provide a CLI for inspecting and changing log levels, listing service
  state, starting/stopping/restarting services, viewing exit causes, and
  requesting a controlled shutdown or reboot. `quickinit` now exposes a
  deterministic parser, typed dispatcher, stable status/log renderers, and
  backend-error propagation; transport and target console wiring remain ABI
  integration work.
- [x] Add service logging with stable timestamps/sequence IDs, bounded log
  retention, serial fallback, and an explicit policy for unavailable storage
  or a stalled child. `quickinit` now bounds memory/serial records, exposes
  `on_storage_unavailable = serial|continue|halt`, accepts storage availability
  from the target adapter, and logs the timeout/stop/kill escalation policy
  exactly once per transition.
- [x] Launch the first shell service (or recovery shell) only after the
  required mounts, device services, and logging path are ready; keep a
  recovery mode available when normal configuration is invalid. `quickinit`
  now validates `[boot]` gate references, holds the normal shell until all
  gates and storage are ready, provides an explicit recovery-shell launch,
  and defaults malformed configuration to a safe shell recovery mode.
- [x] Test PID 1 behavior under malformed configuration, missing binaries,
  rapid crashes, orphan reaping, signal races, dependency cycles, shutdown,
  and kernel panic/reboot log hand-off. The quickinit host suite now has 25
  focused policy tests for these cases; existing x86_64/aarch64 graphical
  QEMU smokes cover PID 1 hand-off, child reaping, deterministic failure, and
  kernel log continuation.

#### 5.3 `norxshell` (`nsh`) — interactive shell

- [x] Choose the implementation base: a focused native shell or a carefully
  scoped fork of an existing shell. Keep the first version small enough to
  audit and define compatibility goals before copying Bash or fish behavior.
  `nsh/README.md` selects a native Rust `no_std`/`alloc` shell over
  a hosted Bash/fish fork and freezes the initial POSIX-shaped subset,
  unsupported behavior, and ABI gates.
- [x] Implement command parsing, quoting, escaping, variables, environment
  inheritance, command lookup, exit status, sequences, conditionals,
  pipelines, redirections, here-documents, and background jobs. The initial
  host-tested `nsh` parser/expansion layer now preserves these
  constructs and exposes a narrow `CommandRunner` execution hook; real command
  lookup/process/fd application remains gated on the Norx userspace ABI.
- [x] Keep `nsh` focused on parsing, expansion, command lookup, pipeline
  orchestration, terminal interaction, dispatch, and Linux-like shell
  builtins. The shell-intrinsic set remains inside `nsh`: `cd`, `export`,
  `set`/`unset`, `exit`, `umask`, `jobs`, `fg`, `bg`, `wait`, `source`, and
  `exec`. These commands must run in the shell process because they change its
  cwd, environment, exit state, process groups, or image. `pwd`, `echo`,
  `env`, `true`, `false`, and similar commands also have standalone utility
  implementations, as in Linux; the current host-tested builtin state and
  typed job/source/exec requests remain the `nsh` implementation boundary.
- [x] Provide line editing, history, completion, prompts, startup files,
  terminal resize handling, UTF-8 policy, and safe behavior when no
  interactive terminal is available. `nsh` now provides bounded
  line input, history navigation, deterministic completion/prompt/resize,
  UTF-8/NUL validation, and explicit interactive/non-interactive startup
  policy; target keyboard/TTY wiring remains an ABI hook.
- [x] Define process groups, job control, terminal ownership, signal
  forwarding, interrupt handling, and the behavior of pipelines that fail
  part-way through. `nsh` now provides a host-tested `JobControl`
  contract with process-group/terminal-owner state, group signals, interrupt
  forwarding, completion, and deterministic `pipefail` status; Norx process
  groups and TTY ownership remain the backend hook.
- [x] Add script-mode tests and interactive QEMU smoke tests for quoting,
  redirection, pipelines, job control, errors, interrupted commands, and
  clean shell exit. Script-mode integration coverage now exists in
  `nsh/tests/script_mode.rs`; a freestanding `nsh` ELF is now built
  and staged for both targets, and `norx-kernel/scripts/nsh-smoke.sh` now
  delegates prompt-paced serial driving to `nsh-smoke.py`, including explicit
  ABI-boundary regression cases. The v1 process-I/O extension ABI is now
  synchronized across kernel, Rust, C, staged headers, and docs; bounded pipe,
  process-group, `dup2`, VFS truncate/append, pipe endpoint lifetime, and
  serial controlling-TTY ownership primitives have self-checks. The kernel now
  routes bounded `open`/`read`/`write`/`close`/`dup2`/`pipe` and synchronous
  `wait_status`; the external `spawn2` fixture now proves inherited open
  descriptors, concurrent child transaction/wait behavior, and negative
  errno/bounds checks on x86_64 and aarch64. Arbitrary argv/environment
  transfer, blocking pipe waits, and concurrent/background execution remain
  explicitly incomplete. The
  prompt-driven QEMU harness now verifies both x86_64 and aarch64 through
  quoting, partial-line Ctrl-C, redirection, deterministic unsupported
  pipeline/assignment/background/job-control errors, parser and lookup
  failures, source/exec backend errors, and clean shell exit.

#### 5.4 Userland command, account, session, and privilege utilities

Every project below is a separate repository under `BoaKernel/`. A repository
may ship several tightly related binaries, but each binary still needs its own
owner, CLI contract, tests, installation path, and QEMU smoke marker. `nsh`
must launch these programs through the documented userspace ABI rather than
embedding their implementations.

The exception is the Linux-like shell builtin set listed in 5.3: those
implementations stay in `nsh` because a child ELF cannot modify its parent
shell's cwd, environment, process group, or exit state.

##### 5.4.1 `coreutils` — base command-line utilities

- [x] Define the coreutils scope and shared command/error/output conventions;
  keep shell process state and privilege transitions out of this project.
- [x] Implement the first bootable set: `cat`, `echo`, `env`, `ls`, `pwd`,
  `mkdir`, `rmdir`, `cp`, `mv`, `rm`, `touch`, `ln`, `stat`, `find`, `grep`,
  `head`, `tail`, `sort`, `wc`, `true`, `false`, and `sleep`. `cd` is not a
  coreutils binary because it must change the parent shell's working directory.
- [x] Keep system-facing commands out of coreutils. `ps`/`kill`,
  `mount`/`umount`, `dmesg`/`logctl`, reboot/shutdown, and
  hardware/storage/network diagnostics belong to the separately owned
  repositories in 5.5; `service` remains part of quickinit's supervisor CLI.
- [x] Establish pathname, permissions, symlink, locale/UTF-8, buffering,
  exit-status, error-message, and signal conventions shared by all commands.
- [x] Support static and dynamically linked builds as the toolchain permits,
  keep binaries small for the base image, and avoid hidden host-filesystem
  dependencies.
- [x] Add per-command unit tests, malformed-input tests, pipeline tests, and
  a minimal root-filesystem smoke image that runs the commands through `nsh`.

##### 5.4.2 `userdb` — users, groups, credentials, and password policy

- [x] Define the account/group record format, lookup API, password-hash policy,
  credential inheritance, capability mapping, disabled/locked states, and
  ownership of mutable account data.
- [x] Provide bounded parsing, atomic updates, corruption recovery, migration
  rules, and explicit behavior when persistent storage is unavailable.
- [x] Publish the ABI consumed by `login`, `passwd`, `userctl`, `su`, `sudo`,
  `getty`, `quickinit`, and session utilities; never expose password material
  through shell environment or serial diagnostics.
- [x] Add host tests, malformed-database tests, permission tests, lockout and
  recovery tests, and staged QEMU persistence smoke coverage.

##### 5.4.3 `getty` — CLI greeter and terminal session launcher

- [x] Implement serial/TTY discovery, terminal initialization, login prompt,
  bounded retry/lockout behavior, session limits, and clean hand-off to
  `login`; keep authentication and account mutation in `userdb`/`login`.
- [x] Support recovery-console and unavailable-terminal policies without
  turning a missing optional console into a kernel panic.
- [x] Define ownership of the terminal, controlling process group, window
  size, signal forwarding, and shutdown/restart behavior with `quickinit`.
- [x] Add interactive serial and graphical QEMU smoke tests with stable
  greeter, timeout, failed-login, and successful-handoff markers.

##### 5.4.4 `login` — account authentication and shell session setup

- [x] Authenticate through `userdb`, enforce disabled/locked/expiry policy,
  apply credentials and capabilities, and start the selected user shell with
  a clean environment, cwd, umask, limits, and standard streams.
- [x] Define failure delays, retry limits, audit records, recovery login, and
  behavior when the account database or terminal backend is unavailable.
- [x] Keep `login` separate from `getty`: `getty` owns the terminal prompt,
  while `login` owns authentication and session construction.
- [x] Add host tests plus QEMU tests for success, bad credentials, locked
  users, environment setup, shell exit, and session teardown.

##### 5.4.5 `passwd` — password management utility

- [x] Implement password change, confirmation, policy validation, old-password
  verification, privileged reset, lock/unlock, and safe atomic persistence
  through `userdb`.
- [x] Keep hashes, prompts, error messages, and audit output out of command
  arguments, environment variables, core dumps, and serial logs.
- [x] Add malformed-input, policy-boundary, concurrent-update, interrupted
  write, and QEMU user-session tests.

##### 5.4.6 `userctl` — user and group administration utility

- [x] Provide separate commands or subcommands for creating, modifying,
    disabling, deleting, and listing users/groups, with explicit capability and
    ownership checks.
- [x] Define UID/GID allocation, primary and supplementary groups, home
    directory policy, default shell selection, and rollback on partial failure.
- [x] Add permission, duplicate-ID, malformed-record, storage-failure, and
    QEMU administrative-session tests.

##### 5.4.7 `sudo` — delegated privilege utility

- [ ] Define a least-privilege policy format, command/path matching, argument
  rules, environment filtering, authentication timeout, audit records, and
  capability transfer through the Norx credentials ABI.
- [ ] Keep `sudo` separate from both `login` and `su`; it delegates a bounded
  command, does not create a new login session, and must not rely on a global
  root identity as its only authorization mechanism.
- [ ] Add policy parser, denial/allow, environment-injection, path-race,
  audit-failure, signal, and QEMU smoke tests.

##### 5.4.8 `su` — user-switching utility

- [ ] Implement authenticated user switching with explicit session/cwd/env/
  group semantics and a controlled shell hand-off.
- [ ] Define when root or a capability may bypass password checks, prevent
  ambient capability leakage, and preserve an auditable parent/child session
  relationship.
- [ ] Add success, denial, locked-account, environment, signal, teardown, and
  QEMU smoke tests.

##### 5.4.9 `session-utils` — identity and session inspection

- [ ] Implement standalone `id`, `whoami`, `groups`, `who`, and related
  read-only session inspection commands using the documented credentials and
  session APIs.
- [ ] Keep `logout` as an `nsh` builtin because it terminates the current
  shell/session; do not implement it as a child binary that cannot affect its
  parent.
- [ ] Add bounded output, permission, stale-session, malformed-record, and
  QEMU session-listing tests.

#### 5.5 Individually owned daemons and utility repositories

There is deliberately no umbrella repository or package. Every entry below is
a separate repository under `BoaKernel/`, with its own ABI
contract, configuration, privilege boundary, tests, QEMU markers, and package
output. Related binaries may share one repository only when they have the same
owner and lifecycle.

##### 5.5.1 `syslogd` — logging daemon and log utilities

- [ ] Define the bounded log record format, timestamps, sequence IDs, sinks,
  retention, crash hand-off, serial fallback, and unavailable-storage policy.
- [ ] Implement `syslogd` plus its owned `dmesg`/`logctl` utilities with
  capability checks and stable machine-readable output.
- [ ] Add malformed-record, full-buffer, stalled-sink, restart, persistence,
  and QEMU boot/log-continuation tests.

##### 5.5.2 `devd` — device discovery daemon

- [ ] Define device enumeration, hotplug, ownership, capability transfer,
  driver/service binding, timeout, and unsupported-device behavior.
- [ ] Implement the first userspace device manager without duplicating kernel
  driver logic or making optional hardware failure fatal to boot.
- [ ] Add absent-device, malformed-descriptor, hot-unplug, restart, and QEMU
  serial-marker tests.

##### 5.5.3 `mountd` — mount manager and mount utilities

- [ ] Define the mount request ABI, namespace/capability checks, mount table,
  failure rollback, unmount safety, and configuration ownership.
- [ ] Implement `mountd` plus `mount`/`umount` utilities; keep filesystem
  parsing in filesystem libraries and the kernel VFS contract.
- [ ] Add read-only, missing-device, malformed-filesystem, busy-unmount,
  crash-recovery, and both-architecture QEMU tests.

##### 5.5.4 `timed` — timekeeping daemon

- [ ] Define monotonic/realtime ownership, clock correction limits, timezone
  data, persistence, and behavior when hardware or network time is absent.
- [ ] Implement `timed` and a small inspection/control CLI with bounded
  privilege and no silent host-clock fallback.
- [ ] Add drift, invalid-update, restart, unavailable-source, and QEMU tests.

##### 5.5.5 `netd` — network configuration daemon

- [ ] Define interface/address/route ownership, DHCP/DNS configuration,
  socket/IPC API, capability policy, restart behavior, and safe defaults.
- [ ] Implement `netd` only after the Norx socket ABI and virtio-net path are
  stable; keep protocol code in reusable userspace libraries.
- [ ] Add link loss, malformed lease, unavailable network, restart, and QEMU
  serial/network smoke tests.

##### 5.5.6 `netutils` — network command-line utilities

- [ ] Implement interface/address inspection, route inspection, DHCP/DNS
  control, `ping`, and basic remote/debug transport as standalone commands.
- [ ] Keep credentials, timeouts, cancellation, output, and exit statuses
  explicit; never require a shell script or host networking.
- [ ] Add malformed-packet, timeout, permission, interrupted-request, and
  graphical QEMU tests.

##### 5.5.7 `procutils` — process and resource inspection utilities

- [ ] Implement `ps`, `kill`, system information, and resource-monitoring
  commands over the documented process/credentials ABI.
- [ ] Define visibility, capability checks, signal restrictions, stable output,
  and behavior for exited or inaccessible processes.
- [ ] Add permission, stale-process, malformed-record, signal, and QEMU tests.

##### 5.5.8 `storage-utils` — filesystem and disk maintenance utilities

- [ ] Implement filesystem checks, disk inspection, image mounting helpers,
  crash-log export, and safe read-only diagnostics.
- [ ] Keep write/repair operations capability-gated, transactional where
  possible, and separate from the kernel's filesystem implementations.
- [ ] Add corrupt-image, truncated-volume, read-only, interrupted-repair,
  and both-architecture QEMU tests.

##### 5.5.9 `recoveryctl` — recovery and diagnostics utility

- [ ] Implement a bounded recovery/diagnostics CLI for boot failures,
  unsupported hardware, failed mounts, crashed daemons, and log export.
- [ ] Define recovery capabilities, safe-mode behavior, serial-only fallback,
  and hand-off back to quickinit without embedding a second supervisor.
- [ ] Add malformed-configuration, missing-storage, crashed-process, and
  deterministic recovery QEMU tests.

##### 5.5.10 `powerctl` — reboot and shutdown utility

- [ ] Implement `reboot`, `shutdown`, and controlled power-state requests with
  capability checks, confirmation policy, timeout, and failure reporting.
- [ ] Coordinate shutdown ordering through quickinit; this repository must not
  become a second service manager.
- [ ] Add denial, active-process, timeout, repeated-request, and QEMU tests.

#### 5.6 `curl` — network client and libcurl fork

- [ ] Select and pin the upstream curl/libcurl baseline, document the Norx
  patches, license obligations, supported protocols, and the minimum feature
  set for the first release.
- [ ] Port libcurl to the Norx socket, DNS, file, time, process, and signal
  APIs; define blocking, timeout, cancellation, proxy, and connection reuse
  behavior without depending on host filesystem or shell tools.
- [ ] Provide the `curl` CLI with HTTP/HTTPS download and upload, headers,
  redirects, authentication, proxy configuration, resume, progress output,
  exit statuses, and safe handling of malformed responses.
- [ ] Integrate the selected TLS and certificate-validation backend, including
  the Norx trust-store layout, hostname validation, time errors, and explicit
  unsupported protocol/cipher behavior.
- [ ] Add host tests, malformed-input tests, local HTTP/HTTPS fixture tests,
  timeout and interrupted-transfer tests, and graphical QEMU smoke coverage
  with stable serial assertions.

#### 5.7 `git` — version-control fork

- [ ] Select and pin the upstream Git baseline, separate portable core code
  from host-specific helpers, and record the compatibility and licensing
  changes required for Norx.
- [ ] Publish the ported `git` command-line client as a separate Gamma output
  with the standard user-facing commands, stable exit statuses, pager/editor
  integration, credential handling, and a documented unsupported-command list.
- [ ] Port Git's filesystem, process, memory, time, threading, terminal,
  environment, path, and signal layers to the Norx userspace ABI; remove
  hidden dependencies on host shell commands and platform paths.
- [ ] Bring up the core local workflow: repository creation, object storage,
  index, add, commit, status, log, diff, branch, tag, checkout, merge, and
  integrity checks with bounded failure behavior.
- [ ] Add remote transport through libcurl for HTTPS and define the SSH or
  other secure transport plan, credential storage policy, proxy behavior, and
  certificate verification rules.
- [ ] Port configuration, hooks, pager/editor integration, path quoting,
  locale/UTF-8 handling, locking, crash recovery, and concurrent repository
  access only after the core repository format is stable.
- [ ] Add repository corruption, truncated pack/index, lock contention,
  interrupted write, malformed ref, network failure, and permission tests;
  exercise clone/fetch/push and local workflows in graphical QEMU.

#### 5.8 `micro` — terminal editor fork

- [ ] Pin the upstream micro baseline and decide whether the first Norx build
  uses a ported Go runtime/toolchain or a staged native rewrite; document the
  decision instead of making Go support an implicit kernel dependency.
- [ ] Port terminal, PTY, filesystem, process, signal, time, environment,
  Unicode, and terminal-size handling to Norx with a clear fallback for a
  non-interactive console.
- [ ] Implement the essential editor workflow: open/save, buffers, undo/
  redo, search/replace, selections, syntax highlighting, key bindings,
  mouse input, tabs/splits, and safe recovery from write failures.
- [ ] Define configuration, themes, plugins, shell-command integration, and
  external editor behavior; keep optional extensions disabled when their
  runtime or security model is unavailable.
- [ ] Add tests for empty/large/UTF-8 files, invalid encodings, read-only and
  full filesystems, interrupted saves, terminal resize, signal handling, and
  crash recovery, followed by interactive graphical QEMU smoke tests.

#### 5.9 `gamma` — package manager and system profiles

Repository: `BoaKernel/gamma/`.

Gamma combines a simple pacman-like command line with immutable packages,
declarative system configuration, reproducible builds, atomic generations, and
rollback. The initial implementation must keep the package manager independent
from any one repository host and must use the documented platform userspace
ABI.

##### 5.9.1 Nordix filesystem layout

- [ ] Freeze the first system layout and make it part of the userspace ABI:

  ```text
  /cfg/       system and application configuration plus app data
  /users/     user homes, identities, and per-user data
  /bin/       user-facing executables and command entry points
  /lib/       shared libraries and runtime data for the default ABI
  /sys/       immutable system state and kernel-visible system data
  /gamma/     Gamma store, profiles, generations, and package-manager state
  /boot/      boot files, kernels, initramfs, and boot configuration
  /dev/       devfs and device nodes
  ```

- [ ] Define which paths are package-owned, user-owned, generated, mutable,
  or kernel-provided. Package installation must not silently overwrite user
  data under `/cfg/` or `/users/`.
- [ ] Define architecture and ABI conventions for the single 64-bit `/lib/`,
  executable lookup through `/bin/`, Gamma activation through `/gamma/`,
  system integration through `/sys/`, and boot integration through `/boot/`.
- [ ] Keep `/dev/` outside normal package payloads; device nodes and devfs
  state are created by the kernel and device services, not copied from an
  archive.

##### 5.9.2 Package model and file format

- [ ] Define the `.gpk` archive format: version, compression, canonical file
  ordering, metadata encoding, regular files, directories, symlinks, modes,
  owners, timestamps, capabilities, checksums, and signature placement.
- [ ] Reject path traversal, absolute archive escapes, duplicate paths,
  malformed metadata, unsupported file types, oversized entries, and hashes
  that do not match the payload before extraction or activation.
- [ ] Separate the package recipe from the installed package manifest. A
  recipe describes how to build; the embedded manifest describes the exact
  ready-to-install artifact and its file ownership.
- [ ] Define the package manifest schema with name, version, release, target
  architecture, ABI, license, source identity, outputs, file list, runtime
  dependencies, build dependencies, test dependencies, optional dependencies,
  provided interfaces, conflicts, replacements, configuration files, mutable
  data, services, checksums, and signatures.
- [ ] Support one source repository producing multiple independently declared
  outputs, such as `git`, `git-docs`, and `git-completion`. Build and publish
  each output as its own logical package; optionally provide a meta-package
  that depends on a selected group.
- [ ] Define package versions, version constraints, epochs/releases where
  required, ABI compatibility, architecture qualifiers, and deterministic
  dependency error messages.

##### 5.9.3 Recipes, system configuration, and lock files

- [ ] Define `gamma.toml` as the source recipe format. It must support native
  packages, imported source archives, Git sources, monorepos, subdirectories,
  patches, build systems, staging directories, multiple outputs, and target
  conditions without embedding host paths.
- [ ] Keep desired system state separate from package recipes in
  `/cfg/gamma/system.toml`. The system file declares channels, target ABI,
  requested packages, profiles, service choices, and system options.
- [ ] Define `/cfg/gamma/gamma.lock` as the resolved closure: exact package
  versions, artifact hashes, repository metadata hashes, Git commit SHAs,
  source hashes, selected outputs, and target architecture.
- [ ] Make lock updates explicit. A branch or tag may select a Git source in a
  recipe, but every reproducible build and installed generation must record a
  concrete commit SHA in the lock file.
- [ ] Add schema versions, migration rules, validation, useful diagnostics,
  and a dry-run representation of the dependency and activation plan.

##### 5.9.4 Package sources

- [ ] Install verified local artifacts with `g -i ./package.gpk`; the archive
  must contain a complete manifest and must not need source files to install.
- [ ] Install prebuilt packages from configured repositories with a signed
  index, target/ABI selection, dependency metadata, mirrors, channels, and
  reproducible artifact hashes.
- [ ] Support Git recipes with the coordinates `git/pkgname` and
  `git/author/pkgname`, plus an explicit `git+https://...` form for arbitrary
  hosts. Resolve coordinates through a provider or source index rather than
  assuming one hosting service forever.
- [ ] Locate `gamma.toml` at a repository root or selected `subdir`, resolve
  monorepo outputs by package name, verify the selected commit, and preserve
  the source URL, commit, and hash in the lock file.
- [ ] Distinguish source builds from binary installs in diagnostics and cache
  successful source builds as normal `.gpk` artifacts in the local store.

##### 5.9.5 Store, profiles, and generations

- [ ] Implement an immutable content-addressed store under `/gamma/`, for
  example `/gamma/store/<hash>-<name>-<version>/`, with no in-place
  mutation after verification.
- [ ] Implement system and per-user profiles under `/gamma/profiles/`.
  A profile selects exact store paths and projects their executables and
  libraries into `/bin/`, `/lib/`, and other approved public paths.
- [ ] Keep active generation selection atomic. A failed install or interrupted
  update must leave the previous generation usable.
- [ ] Record generation manifests, dependency closures, activation time,
  source provenance, and the previous generation for rollback and auditing.
- [ ] Support explicit rollback, profile inspection, package verification,
  orphan detection, and garbage collection that never removes live or pinned
  generations.
- [ ] Define boot integration so `/boot/` can select a known-good system
  generation and recover when the newest generation fails before userspace.

##### 5.9.6 CLI and transactions

- [ ] Provide `gamma` and `g` aliases with the initial commands:

  ```text
  g -i <package|file|source>   install and resolve a package
  g -r <package>              remove from the active profile
  g -l [package]              list installed packages or package files
  g -s <query>                search configured indexes
  g -u [package]              update selected packages and metadata
  g -U                        reconcile and upgrade the whole system
  ```

- [ ] Provide readable long options for scripts and administration, including
  `--install`, `--remove`, `--list`, `--search`, `--update`, `--upgrade-system`,
  `--info`, `--dry-run`, `--verify`, `--rollback`, and `--gc`.
- [ ] Make install, remove, and update operations transactions: resolve the
  full dependency graph, verify all inputs, stage files, check conflicts,
  build the next profile, and switch it only after every step succeeds.
- [ ] Make `g -U` reconcile `/cfg/gamma/system.toml` and `gamma.lock`, report
  changes before applying them, and support an explicit lock refresh rather
  than silently changing source revisions.
- [ ] Keep removed packages in the store until no profile, user profile,
  rollback generation, or pinned development environment references them.
- [ ] Add progress, structured serial logs, machine-readable errors, exit
  statuses, cancellation, and safe recovery from interrupted transactions.

##### 5.9.7 Dependencies, conflicts, and ownership

- [ ] Implement runtime, build, test, optional, and target-specific
  dependencies with transitive resolution and cycle diagnostics.
- [ ] Implement `provides`, `conflicts`, `replaces`, and ABI capability
  matching for libraries and alternative implementations.
- [ ] Maintain an ownership index for every active file. Reject collisions by
  default and require an explicit, validated alternative-path rule for shared
  generated data.
- [ ] Keep configuration and app data under `/cfg/`, user data under
  `/users/`, and package-managed immutable payloads under `/gamma/`.
  Upgrades must preserve user-edited configuration unless the user explicitly
  requests a migration.
- [ ] Replace arbitrary install/uninstall scripts with a small declarative
  action system for known tasks such as service registration, certificate
  refresh, cache rebuilds, and boot entry updates.
- [ ] Define service descriptors for quickinit, capability requirements,
  startup ordering, restart policy, configuration paths, and safe behavior
  when a service or optional hardware feature is unavailable.

##### 5.9.8 Repositories and trust

- [ ] Define repository configuration under `/cfg/gamma/`, including names,
  URLs, channels, architecture filters, priority, mirrors, and trusted keys.
- [ ] Define signed repository indexes containing package versions, targets,
  dependencies, artifact hashes, sizes, provenance, and signatures.
- [ ] Verify repository metadata before resolution and verify package
  signatures and payload hashes before entering the store.
- [ ] Define trusted local, development, and production modes. Unsigned Git
  packages may be used for development only through an explicit opt-in and
  must be clearly marked in the generation metadata.
- [ ] Add key rotation, revoked-key handling, expired metadata, mirror
  disagreement, offline-cache, and repository rollback behavior.

##### 5.9.9 Reproducible and isolated builds

- [ ] Build Git-sourced packages in a sandbox with a declared target sysroot,
  toolchain, environment, inputs, output directory, and resource limits.
- [ ] Disable network access during builds by default; permit declared source
  fetches only in the fetch phase and record every fetched input hash.
- [ ] Stage all output under a temporary destination, generate the file list
  from the result, normalize timestamps where possible, and reject undeclared
  output or host-path leakage.
- [ ] Cache successful builds by source commit, recipe hash, dependency
  closure, toolchain identity, target ABI, and build options.
- [ ] Add reproducibility checks by rebuilding the same lock closure and
  comparing manifests and payload hashes.

##### 5.9.10 Bootstrap, tests, and QEMU validation

- [ ] Define the bootstrap path for installing Gamma into the first Norx
  userspace without requiring Gamma to already be installed. Keep a minimal
  static bootstrap artifact and a documented hand-off to normal profiles.
- [ ] Add host tests for TOML/schema validation, dependency solving, version
  constraints, Git coordinates, archive safety, path normalization, file
  ownership, signatures, lock files, profiles, and garbage collection.
- [ ] Add negative tests for missing dependencies, cycles, conflicts, bad
  signatures, corrupt indexes, malformed `.gpk` files, path traversal,
  duplicate files, truncated payloads, failed Git fetches, dirty sources,
  interrupted writes, failed activation, and rollback.
- [ ] Run the package-manager bootstrap and full userland transaction in
  graphical QEMU on x86_64 and aarch64 with `-display gtk`; ARM runs must
  include `-device ramfb` so the framebuffer is visible rather than opening
  only a serial/parallel console window.
- [ ] Assert stable serial markers for fetch, resolve, build, verify, activate,
  rollback, and completed-system states, and retain framebuffer screenshots as
  visual regression artifacts.
- [ ] Test degraded boots with missing repositories, unavailable disks,
  invalid configuration, unsupported architecture, absent network, crashed
  services, and a failed newest system generation.

#### 5.10 Graphical platform and desktop environment

##### 5.10.1 Wayland and Qt foundation

- [ ] Choose a standard Wayland-compatible architecture instead of creating a
  private display protocol. Put Norx-specific protocol extensions in the
  `nwayland` package while keeping the standard Wayland client model
  compatible.
- [ ] Keep `nwayland` inside the `nde` source repository as one of its
  independently built Gamma outputs. Do not create a second repository for
  the Wayland layer.
- [ ] Port and package the Qt foundation required by the desktop profile:
  `qtbase`, Qt GUI/Quick, Qt Wayland, input, fonts, image codecs, networking,
  accessibility, and the Norx platform backend. Each dependency must be its
  own package with an ABI/version contract.
- [ ] Implement the Norx Wayland backend for outputs, seats, input, shared
  buffers, damage, clipboard, drag-and-drop, text input, decorations,
  fullscreen, scaling, and multi-monitor behavior.
- [ ] Add security boundaries for client surfaces, input focus, clipboard,
  screenshots, screen capture, privileged protocol extensions, and crashed
  clients before enabling third-party graphical applications.
- [ ] Run standard Wayland client fixtures and Qt Quick smoke programs on
  x86_64 and aarch64 in graphical QEMU with stable serial markers and
  framebuffer screenshots.

##### 5.10.2 `nde` repository and package outputs

- [ ] Define `nde` as one source repository containing multiple Gamma
  packages rather than one inseparable binary. The initial outputs are
  `nwayland`, `ndesession`, `nwidgets`, `npanel`, `nlauncher`, `nsettings`,
  `ncontrol`, `ngreeter`, `nthemes`, `nicons`, and shared desktop APIs.
- [ ] Add the following independently installable outputs to the `nde`
  repository: `nwayland`, `ndesession`, `nwidgets`, `npanel`, `nlauncher`,
  `nsettings`, `ncontrol`, `ngreeter`, `nthemes`, `nicons`, and `kitty`, plus
  the clock/calendar, pinned-and-running-apps, and application-list widgets.
- [ ] Implement the desktop session around `nwayland`: compositor/window
  management, workspace and virtual-desktop state, focus policy, stacking,
  tiling/floating behavior, animations, screenshots, lock screen, and
  session shutdown.
- [ ] Implement configurable panels with replaceable panel layouts, applets,
  menus, notification area, task switcher, clocks, launchers, and widgets.
  Widgets must have versioned APIs, resource limits, and permission scopes.
- [ ] Implement `ngreeter` as the graphical login/session greeter with
  user selection, authentication handoff, session selection, accessibility,
  keyboard navigation, network/power actions, recovery mode, and safe
  shutdown/restart. The GUI must never handle password verification itself;
  authentication remains behind the userspace login/authentication API.
- [ ] Port and package upstream Kitty as the system terminal with Wayland/Qt
  integration, GPU rendering, font and emoji support, tabs, panes, clipboard,
  keyboard shortcuts, shell integration, crash recovery, and `nsh` as the
  default shell where configured. Do not create a second terminal emulator.
- [ ] Implement `nsettings` as the graphical application for all system
  and NDE settings: display, input, themes, fonts, wallpaper, panels,
  widgets, shortcuts, notifications, power, accessibility, applications,
  privacy, network, Wi-Fi, VPN, audio, and session behavior.
- [ ] Implement the clock/calendar widget with locale, timezone, 12/24-hour
  format, date format, accessibility, notification integration, and multiple
  monitor/panel placement support.
- [ ] Implement the pinned-and-running-apps panel widget using the application
  registry, stable `app_id` values, `.link` files, compositor surface state,
  workspace state, launch indicators, close/switch actions, drag-to-reorder,
  and multi-monitor behavior.
- [ ] Implement the application-list utility and launcher widget with search,
  categories, icons, keyboard navigation, recent/favorite applications,
  launch actions, MIME/default-app visibility, and safe `app_id`-based launch.
- [ ] Implement the Control Center widget with quick controls for Wi-Fi,
  network, VPN, audio, Bluetooth, display brightness, night mode, power,
  notifications, and privacy, plus deep links into `nsettings`.
- [ ] Implement graphical system configuration for display, input, themes,
  fonts, wallpaper, panels, widgets, shortcuts, notifications, power,
  accessibility, default applications, and privacy settings.
- [ ] Keep the desktop configuration declarative, schema-versioned, validated,
  recoverable, and separate from package-managed immutable payloads. A bad
  desktop configuration must fall back to a safe session.
- [ ] Define the IPC contracts between compositor, shell, panels, widgets,
  settings, launcher, notifications, and lock screen. A crashed widget or
  panel must not take down the compositor or the whole session.
- [ ] Add host tests for configuration and layout state, malformed widget
  manifests, permission checks, recovery mode, and deterministic settings
  migration, followed by full graphical QEMU session tests.

##### 5.10.3 Application manifests and graphical registration

- [ ] Make Gamma package manifests the source of truth for installed
  applications. Every application receives a stable reverse-domain `app_id`
  and declares its executable, name, icon, categories, MIME/protocol
  handlers, terminal requirement, actions, capabilities, and supported
  architectures.
- [ ] Keep the Gamma package coordinate separate from user-facing application
  metadata: for example, package `nsettings` may register the localized
  application name `Settings`, while `app_id`, executable ownership, icons,
  and capabilities remain stable registry data.
- [ ] Validate application entries during package install and generation
  activation: IDs must be unique, executables and icons must be owned by the
  package, capabilities must be declared, and malformed entries must not
  enter the active registry.
- [ ] Generate the application registry atomically from the active Gamma
  profile. NDE must resolve launches by `app_id`, never by an untrusted
  arbitrary shell command or copied executable path.
- [ ] Support MIME types, URI schemes, default applications, desktop/menu
  categories, per-user overrides, launch actions, startup behavior, and
  capability prompts through the same registry.
- [ ] Keep application icons in package-owned icon themes with stable names,
  multiple resolutions, fallback icons, and no host filesystem lookup.
- [ ] Add registry rebuild, package removal, duplicate-ID, missing-icon,
  missing-executable, capability-denial, and rollback tests in QEMU.

##### 5.10.4 `.link` desktop shortcut format

- [ ] Define `.link` as the Norx desktop shortcut format for the desktop,
  panels, menus, and other user-facing locations. It is separate from the
  package application manifest and references an `app_id`.
- [ ] Use a versioned, human-readable TOML representation with at least
  `version`, `app_id`, optional display title/icon override, and arguments.
  The format must not allow an arbitrary `exec` field or implicit shell code.
- [ ] Define shortcut validation, relative/absolute placement rules, user and
  system ownership, icon fallback, stale-link handling, URI argument `%u`/
  `%U` expansion, and migration between format versions.
- [ ] Make NDE create, edit, move, duplicate, and remove `.link` files
  graphically while preserving user edits and refusing links to unregistered
  or revoked applications.
- [ ] Add parser, path traversal, malformed-TOML, stale-app, capability,
  multi-argument, and graphical drag/drop tests.

##### 5.10.5 Gecko browser integration

- [ ] Select Floorp as the initial Gecko/Firefox product baseline. Keep the
  upstream source reference at `https://github.com/Floorp-Projects/Floorp`
  and review its MPL-2.0 and trademark obligations before creating a Norx
  fork with its own product name.
- [ ] Keep the browser in its own repository and package. It consumes
  `nwayland`, Qt/desktop services only where needed, the userspace ABI,
  networking, storage, fonts, graphics, sandboxing, and the application
  registry; it is not part of the `nde` source repository.
- [ ] Keep Gecko and the Firefox platform as close as possible to an
  upstream ESR baseline. Store Norx platform changes as a small, reviewable
  patch set instead of copying unrelated browser branding or desktop code.
- [ ] Implement the Norx platform ports for process sandboxing, IPC, threads,
  timers, files, sockets, TLS, graphics, input, font rendering, audio,
  accessibility, crash reporting, and Wayland surfaces before attempting a
  full browser build.
- [ ] Ship no custom browser UI or engine work before the graphical platform,
  Qt packages, application registry, and security boundaries have passed
  their smoke gates. The first browser milestone is a pinned upstream-based
  build with explicit unsupported features.
- [ ] Add browser package registration, MIME/URI handlers, icon assets,
  download isolation, profile storage, permission prompts, crash recovery,
  update/rollback behavior, and graphical QEMU smoke tests.

##### 5.10.6 GPU, rendering APIs, and acceleration

- [ ] Define `ndrm` as the versioned Norx kernel/userspace GPU and display
  interface. Keep it separate from the OpenGL and Vulkan implementations and
  document the ABI before adding hardware-specific drivers.
- [ ] Implement the kernel display path for framebuffer fallback, outputs,
  modesetting, EDID, planes, hardware cursor, vblank, page flipping, monitor
  hotplug, suspend/resume, and power management.
- [ ] Implement the kernel GPU path for memory objects, buffer sharing,
  command submission, fences/timeline synchronization, GPU address spaces,
  IOMMU isolation, timeouts, reset/recovery, and per-process resource limits.
- [ ] Implement the first software-rendered graphical profile with a stable
  framebuffer path and Mesa llvmpipe so `nwayland`, Qt, and NDE can run before
  hardware acceleration is available.
- [ ] Port Mesa as an upstream Git fork with a small Norx patch queue. Package
  its EGL, OpenGL, OpenGL ES, Gallium, software renderer, and Norx driver
  integration separately from `nde`; do not implement a private OpenGL stack.
- [ ] Port the Khronos Vulkan Loader as an upstream Git fork and package the
  loader, driver-discovery manifests, validation layers, and Vulkan tools
  separately. Add Norx Vulkan ICDs only after the `ndrm` ABI is stable.
- [ ] Package SPIR-V tools and shader compilers needed by Vulkan, Mesa, Qt,
  and applications. Record compiler versions and shader build inputs for
  reproducible packages.
- [ ] Implement an `nwayland` OpenGL ES/EGL renderer first, with explicit
  buffer import, damage, synchronization, color format, scaling, and fallback
  behavior. Add a Vulkan renderer later without making Vulkan mandatory for
  the first desktop profile.
- [ ] Integrate Qt RHI with the Norx graphics stack and support runtime
  selection between OpenGL and Vulkan. Qt Quick/NDE applications must have a
  deterministic software-rendering fallback and clear unsupported-feature
  diagnostics.
- [ ] Add QEMU graphics stages in order: software framebuffer, virtio-gpu 2D,
  virtio-gpu with virglrenderer for OpenGL, and a Vulkan path through the
  supported virtio-gpu Venus/rutabaga backend. Keep each stage independently
  testable on x86_64 and aarch64.
- [ ] Add the first hardware driver matrix for supported open drivers, starting
  with virtio-gpu and then documented Intel/AMD targets. Record unsupported
  GPUs, firmware requirements, feature levels, and known recovery failures;
  proprietary drivers are not a prerequisite for the initial profile.
- [ ] Add a separate video-acceleration layer for browser and multimedia
  workloads, with software decode fallback, hardware decode capability
  discovery, format negotiation, isolation, and `libva` or the selected Norx
  equivalent as a separately maintained upstream port.
- [ ] Define GPU security policy: clients must not access raw GPU devices,
  command buffers, display outputs, or another client's buffers without an
  explicit capability. Enforce quotas, reset crashed clients, and contain
  malformed shaders and command streams.
- [ ] Add graphics smoke and conformance tests for framebuffer output, EGL,
  OpenGL ES, OpenGL, Vulkan instance/device discovery, shader compilation,
  Qt Quick, compositor rendering, buffer sharing, synchronization, resize,
  multi-monitor, suspend/resume, GPU reset, and software fallback.
- [ ] Keep OpenCL, WebGPU, ray tracing, and general GPU compute outside the
  initial desktop gate; document them as later packages after stable 3D
  rendering, Vulkan security, and driver recovery are available.

#### 5.11 Network platform and Internet access

- [ ] Implement the kernel network subsystem and hardware drivers for
  loopback, Ethernet, IPv4, IPv6, ARP/NDP, TCP, UDP, ICMP, sockets, routing,
  firewall hooks, MTU, link state, power management, and hotplug.
- [ ] Define a versioned kernel-to-userspace network control ABI for links,
  addresses, routes, neighbors, DNS state, and firewall policy; expose
  asynchronous link and address events to userspace.
- [ ] Create a standalone `netd` daemon repository for interface lifecycle,
  DHCPv4/DHCPv6, IPv6 SLAAC/RA, routes, network profiles, and policy
  application. `netd` must not become a generic services umbrella.
- [ ] Create a standalone `wifid` daemon repository for Wi-Fi scanning,
  association, WPA2/WPA3 authentication, roaming, and saved network profiles.
- [ ] Create separate `dnsd`/resolver and network utility packages for DNS,
  `netctl`, `ping`, `traceroute`, `tcpdump`, `curl`, and VPN control; keep
  privileged operations behind capabilities and daemon IPC.
- [ ] Add HTTPS/TLS certificate-store integration, proxy configuration,
  hostname resolution, URI handling, and per-user network permissions.
- [ ] Expose network management to NDE through a versioned IPC API for
  interfaces, Wi-Fi, VPN, DNS, metered connections, and connection sharing;
  the GUI must not manipulate kernel sockets directly.
- [ ] Add QEMU and hardware smoke tests for loopback, Ethernet, DHCP, IPv6,
  DNS, HTTPS, link loss/recovery, Wi-Fi, firewall policy, and clean daemon
  restart.

#### 5.12 Audio platform and PipeWire integration

- [ ] Implement the kernel audio subsystem and drivers for HDA and USB Audio,
  including PCM playback/capture, mixer controls, DMA, hotplug, power
  management, and device capabilities.
- [ ] Select PipeWire as the system audio/media graph instead of designing a
  custom audio server; keep PipeWire, its libraries, SPA plugins, and tools as
  separately versioned Gamma packages.
- [ ] Port/adapt the PipeWire daemon to the Norx userspace ABI, including
  processes, threads, timers, sockets, shared-memory buffers, real-time
  scheduling, permissions, crash recovery, and configuration paths.
- [ ] Port WirePlumber as the PipeWire session/policy manager; define device
  discovery, default devices, profiles, routing, per-application volume,
  permissions, and automatic recovery policies.
- [ ] Package PipeWire client compatibility layers separately, including
  native PipeWire, PulseAudio compatibility, and JACK compatibility where
  the dependency and security review allows it.
- [ ] Create standalone `audio-utils` packages for device inspection,
  playback/capture, volume, routing, diagnostics, and PipeWire graph control;
  do not add a second competing audio server.
- [ ] Add Bluetooth audio integration after the base Bluetooth stack is
  available, covering A2DP, HFP/HSP, device profiles, suspend/resume, and
  reconnect behavior.
- [ ] Expose audio device and policy controls to NDE through PipeWire and
  WirePlumber APIs; panels and settings must not access kernel audio devices
  directly.
- [ ] Add QEMU and hardware smoke tests for HDA, USB Audio, playback,
  capture, mixer changes, multiple applications, hotplug, Bluetooth, graph
  recovery, permissions, and clean session shutdown.

#### 5.13 Ported application and utility portfolio

##### 5.13.1 Base archive, text, and inspection utilities

- [ ] Port `libarchive` as an upstream Git fork and publish separate Gamma
  outputs for `libarchive`, `bsdtar`, `bsdcat`, `bsdcpio`, and `bsdunzip`.
- [ ] Port `zstd`, `gzip`, `xz`, and `bzip2` as maintained upstream forks with
  separate libraries, command-line tools, compression tests, and package
  format integration.
- [ ] Port `less` as the default pager with terminal capability detection,
  large-file behavior, encoding support, search, and safe pipe handling.
- [ ] Port the required `findutils` tools, including `find`, `xargs`, and
  `locate`, with bounded traversal, path safety, and Norx filesystem support.
- [ ] Port `jq`, `ripgrep`, `file`, and `tree` as independent utility packages
  with predictable exit statuses and machine-readable modes where available.
- [ ] Port OpenSSL or the selected compatible TLS/crypto implementation, plus
  `ca-certificates` and `tzdata` data packages, with update and provenance
  handling in Gamma.
- [ ] Add host and QEMU tests for archive safety, decompression limits, Unicode
  paths, large files, broken pipes, permissions, interrupted writes, and
  corrupted input recovery.

##### 5.13.2 Remote access, networking, and system daemons

- [ ] Port OpenSSH as a separate upstream fork with `ssh`, `sshd`, `scp`, and
  `sftp` packages. Integrate authentication, capabilities, host keys, logging,
  sandboxing, and Norx sockets without replacing the existing login model.
- [ ] Port `rsync` as a separate utility with safe path handling, resume,
  remote authentication, bandwidth limits, and Gamma/user-profile isolation.
- [ ] Evaluate `iwd` and `wpa_supplicant` as upstream bases for `wifid`; select
  one implementation, keep the Norx integration in a patch queue, and do not
  implement WPA authentication from scratch.
- [ ] Port `unbound` as the validating DNS resolver backend for `dnsd`, with
  local policy, DNSSEC, cache isolation, per-user policy, and safe fallback.
- [ ] Port `chrony` as the initial time-synchronization daemon with secure
  source selection, clock discipline, suspend/resume handling, and degraded
  offline behavior.
- [ ] Port BlueZ as the Bluetooth stack and integrate its audio/device profiles
  with PipeWire, `wifid`, permissions, pairing storage, and recovery policy.
- [ ] Port CUPS as the printing stack with isolated printer discovery, driver
  packages, job permissions, spool ownership, and graphical NDE integration.
- [ ] Port WireGuard tools and integrate them with `netd`, capability checks,
  key storage, profile activation, and rollback-safe network configuration.
- [ ] Evaluate Avahi as an optional mDNS/zeroconf package after the core DNS,
  network, and trust boundaries are stable; it must remain a standalone daemon.
- [ ] Add integration tests for SSH, DNS, time sync, Wi-Fi recovery, Bluetooth
  pairing, printing, VPN activation, revoked credentials, and offline startup.

##### 5.13.3 Development and diagnostics

- [ ] Port Python as a separate runtime package with a Norx build profile,
  standard-library coverage, TLS/filesystem/socket support, reproducible
  bytecode behavior, and explicit third-party module policy.
- [ ] Port CMake, Ninja, Meson, and `pkg-config` as separate build-tool
  packages with Norx toolchain files, sysroot discovery, target triples, and
  offline/reproducible build modes.
- [ ] Port GDB or LLDB, choosing one initial debugger, with Norx target
  support, syscall/thread/register inspection, core dumps, and QEMU remote
  debugging.
- [ ] Port `tmux` as a terminal multiplexer with `nsh` integration, PTY,
  process-group, resize, clipboard, and session-recovery support.
- [ ] Port `make` if required by upstream build graphs; keep it separate from
  CMake, Ninja, and Meson rather than making one build-system umbrella.
- [ ] Implement and package a Norx `strace`-like syscall tracer using the
  documented tracing ABI, with capability restrictions and bounded output.
- [ ] Add build and diagnostic tests for cross-compilation, offline sources,
  debugger attach, signal/thread inspection, PTY recovery, and malformed
  compiler/build metadata.

##### 5.13.4 Desktop applications and Wayland utilities

- [ ] Port Dolphin as the primary graphical file manager. Keep Dolphin and
  required KDE Frameworks as separate upstream forks/packages, integrate its
  application manifest and MIME handlers, and do not create a second NDE file
  manager for the initial profile.
- [ ] Port FeatherPad as the lightweight Qt text editor with Settings,
  MIME registration, safe recovery, file watching, encoding detection, and
  NDE theme/icon integration.
- [ ] Port mpv as the initial media player/library with PipeWire audio,
  `nwayland` video output, Mesa acceleration, hardware-decoding fallback,
  subtitles, safe URL handling, and package-owned codecs/configuration.
- [ ] Port `btop` as the default interactive system monitor with Norx process,
  memory, CPU, storage, network, GPU, and audio metrics where the relevant
  APIs are available. Keep `btop` separate from NDE widgets.
- [ ] Port `grim`, `slurp`, and `wl-clipboard` as separate Wayland utilities
  for screenshots, region selection, clipboard copy/paste, and NDE shortcut
  integration, with screenshot/capture capabilities enforced by `nwayland`.
- [ ] Port Ark as the graphical archive manager and connect it to the
  `libarchive`, compression, MIME, `.link`, and application-registry packages.
- [ ] Defer a standalone PDF viewer such as `qpdfview` from the initial
  portfolio because the browser already provides PDF viewing; revisit only if
  offline documents, annotations, or system-wide PDF integration require it.
- [ ] Add graphical QEMU tests for file operations, archive extraction,
  editor recovery, media playback, screenshots, clipboard, process metrics,
  MIME/default-app registration, and application rollback.

##### 5.13.5 Large and optional applications

- [ ] Port LibreOffice only after the Qt/graphics, font, printing, filesystem,
  clipboard, document-format, and internationalization foundations are stable.
- [ ] Keep Thunderbird or another separate Gecko-based mail client as a
  deferred optional application, not an initial-profile requirement. Revisit
  it only after the browser platform port, TLS, sandbox, notifications, and
  profile storage are production-ready.
- [ ] Keep Blender and OBS Studio as deferred optional applications, not
  desktop release gates. Revisit them after Vulkan/OpenGL, PipeWire, video
  acceleration, GPU reset recovery, and media codecs pass their release gates.
- [ ] Create one user-facing `ncompat` compatibility utility instead of
  exposing separate Wine and Proton products. The same package must provide
  both a CLI and a graphical manager with application registration, icons,
  `.link` creation, logs, diagnostics, and per-application profiles.
- [ ] Maintain Wine and Proton as separate upstream forks/backends inside the
  `ncompat` project. Keep Wine/Proton, DXVK, VKD3D-Proton, runtime files, and
  licensing boundaries independently updateable; unify them at the manager
  and user-profile level rather than merging their source trees.
- [ ] Implement the Windows/Win32 backend for `ncompat` with prefixes,
  runtime selection, DLL/component management, graphics and audio routing,
  controller/input mapping, filesystem integration, sandbox permissions,
  crash logs, and rollback-safe updates.
- [ ] Implement a Linux software compatibility backend for `ncompat`: Linux
  ELF identification/loading, a versioned Linux syscall compatibility layer,
  Linux shared-library/runtime profiles, filesystem and IPC translation,
  capability boundaries, and clear unsupported-feature diagnostics.
- [ ] Provide an isolated Linux runtime fallback for applications requiring
  unsupported kernel features, using a verified Linux userland and the least
  heavyweight available isolation backend. The GUI and CLI must expose which
  backend is used and keep its files, network, devices, and updates isolated.
- [ ] Define initial `ncompat` CLI operations for backend/runtime discovery,
  install/import, prefix or Linux-runtime creation, launch, stop, remove,
  logs, diagnostics, repair, update, rollback, and application registration.
- [ ] Add an NDE GUI for searching, installing, configuring, launching,
  repairing, and removing Windows and Linux software. It must support per-app
  backend selection, runtime versions, environment overrides, file/URI
  associations, sandbox permissions, GPU/audio options, and safe recovery.
- [ ] Add compatibility tests for Windows GUI/console applications, Linux ELF
  applications, shared libraries, signals, threads, files, sockets, graphics,
  audio, clipboard, input, process isolation, upgrades, rollback, and failure
  diagnostics before enabling `ncompat` in the default image.
- [ ] Evaluate OCI runtimes such as `runc` or `crun`, then Podman or another
  container manager, only after namespaces, capabilities, cgroups-equivalent
  limits, networking, storage, and image verification are available.
- [ ] Evaluate additional browsers, game runtimes, and launchers as separate
  packages; each must pass the application registry, sandbox, update,
  rollback, and security review.

#### 5.14 Cross-project release gates

- [ ] Freeze a versioned userspace ABI and SDK manifest consumed by all
  standalone repositories; CI must reject incompatible sysroot or syscall
  changes without an explicit migration.
- [ ] Build a minimal bootable userland image containing `quickinit`, `nsh`,
  `coreutils`, and the required libraries for both architectures.
- [ ] Build a separate graphical image profile containing `nwayland`,
  `nde`, the required Qt packages, application registry support, and a
  pinned browser only after the graphical security gates pass.
- [ ] Verify the full chain `kernel → quickinit → getty/login → nsh → utility`
  in graphical QEMU, with stable serial assertions and framebuffer screenshots
  stored as visual regression artifacts.
- [ ] Verify the graphical chain `kernel → quickinit → nwayland → nde →
  application`, including app registration, `.link` activation, panel/widget
  recovery, window focus, clipboard, input, and clean session shutdown.
- [ ] Document ownership, permissions/capabilities, failure recovery,
  upgrade/rollback, supported hardware, unsupported cases, and debugging
  procedures for every shipped project.
