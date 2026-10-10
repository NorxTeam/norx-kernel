# Nordix Roadmap

Nordix is the future assembled distribution name. The projects below remain
general-purpose components that implement the shared platform ABI; they are
not utilities created only for one distribution branding.

This is the shared roadmap for everything under `BoaKernel`. It is kept next
to the repositories so the kernel, utilities, forks, and future system
components follow one plan.

This document is also the release-gate source of truth. A checkbox is not
evidence that a production boundary exists: the section status, gate matrix,
target matrix, and linked tests decide whether a dependency may be enabled.

## Architecture and priority decision (2026-10-10)

Native Norx/Nordix is **ARM64 only**. Native x86/x86_64 is unsupported and
existing x86 implementation/tests are archival diagnostics, not a supported
release target. x86 and x86_64 applications may become translated guests on
ARM64 through `wli`; guest instruction sets never imply a native kernel port.
This decision overrides earlier two-architecture checklist wording below;
historical `[x]` evidence is retained as history, not current release scope.
All new native SDK, utility, driver, CI and release gates target aarch64.

`wli` is the first priority standalone utility port. Work on its bounded
ARM64 native entrypoint and parser proceeds now, alongside the kernel fixes
it needs. Guest execution remains blocked on real kernel/process/ABI gates;
a launcher plan cannot stand in for a Linux personality or a translator port.

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

## Compatibility target

Norx/Nordix targets maximum practical POSIX compatibility as a first-class
platform goal. The implementation should cover the useful POSIX surface in
layers: processes and sessions, file descriptors and pathname semantics,
permissions, `exec`, signals, threads, `termios`/PTY, clocks and timers,
standard streams, libc, shell behavior, utilities, locale/UTF-8, and the
portable parts of sockets and networking.

POSIX compatibility means a documented, tested Norx profile with predictable
extensions; it does not mean copying Linux internals or promising that every
Linux/glibc binary will run unchanged. Norx-specific syscalls, capabilities,
package metadata, and service IPC must live behind versioned namespaces and
must not silently change the POSIX behavior of the base profile. Each POSIX
claim needs a conformance test, an explicit unsupported list, and a target
matrix entry before it becomes a release dependency.

## Status vocabulary and release gates

During the migration from the original checklist, the old `[x]` marks in
Sections 1--5 are historical scope markers. The authoritative status is the
section status below plus the gates in Section 0.1. New work must use these
meanings:

- `[x]` — complete for the stated target and evidence level; no sentence in
  the same item may say that a required part remains open.
- `[~]` — scaffolded, host-tested, staged, or QEMU-smoked, but not a complete
  production/security boundary.
- `[ ]` — not complete or blocked by a prerequisite.
- `HOST`, `STAGED`, `QEMU`, `SECURITY`, and `RELEASE` are evidence tags; they
  do not replace a gate and must not be confused with production readiness.

The roadmap uses four release profiles: `bootstrap` (kernel plus static
handoff), `single-user` (real target execution and POSIX core), `secure-multi`
(authorization, persistence, services, and recovery), and `desktop`
(graphics, network, audio, packages, and applications). A later profile may
not consume an earlier profile merely because a fixture or host test passes.

## Open implementation tracks

### 0. Foundation fixes

Status: `[~]` audit backlog is open. This section is the current critical path;
no later section may claim a secure release until G0--G4 are closed.

#### 0.0 Security and correctness audit backlog (2026-08-27)

Scope: all Norx-owned code in `norx-kernel`, `userspace`, `coreutils`, `getty`,
`login`, `passwd`, `userdb`, `quickinit`, and `toolchain`. The curl checkout was
also checked; its Norx delta is provenance/sync metadata, not a custom runtime
implementation. These items are audit findings, not claims that the existing
smoke tests provide a security boundary. Do not mark an item complete until the
negative test, the fix, and the relevant QEMU path all exist.

##### P0 — release blockers

- [ ] **P0-01 — Per-CPU syscall/return state.** `norx-kernel/src/arch/x86_64/syscall.rs:52-98,105-168,192-245` and `src/arch/aarch64/syscall.rs:15-98,103-176` keep syscall arguments, user-return stacks, switch state, and contexts in process-global `static mut` storage. SMP or re-entrant entry lets one CPU overwrite another CPU's control-flow state. Replace with per-CPU/per-thread frames, explicit ownership, and an SMP/re-entry stress test.
- [ ] **P0-02 — Real filesystem authorization.** `norx-kernel/src/vfs.rs:434-442,1504-1510,1583-1587,1889-1915` has no inode owner/group and checks only whether any read/write mode bit is set; `chmod`/`fchmod` do not authorize the caller. A `0600` file is therefore not private, and any process can change modes. Add uid/gid or capability ownership, directory traversal/write checks, and actor-aware open/chmod/unlink/rename tests.
- [ ] **P0-03 — Unauthenticated root recovery.** `login/norx/src/lib.rs:204-232` creates uid/gid 0 plus `CAP_SESSION_ADMIN` solely when a caller reports `database_available == false`. Make recovery depend on a kernel-owned maintenance mode and a physical/cryptographic recovery proof; the service must not be able to self-declare database failure.
- [ ] **P0-04 — Account database recovery trusts attacker-selected paths.** `userdb/norx/src/lib.rs:845-876,878-920` accepts arbitrary committed/temp paths and promotes any parseable temp file when the committed file is missing/corrupt. On a filesystem-backed adapter this is an account takeover path. Bind the operation to a kernel/storage primitive with fixed same-directory names, no-follow/exclusive creation, owner binding, generation/nonce, integrity authentication, and crash-recovery tests.

##### P1 — must fix before SMP, persistence, or untrusted services

- [ ] **P1-01 — Usercopy fault state is global.** `norx-kernel/src/usercopy.rs:7,95-145` uses one `COPY_ACTIVE` flag; one concurrent copy can clear it while another is still in the faultable copy stub. Make fault recovery per-CPU and tied to the active copy frame, then test concurrent mapped/unmapped copies.
- [ ] **P1-02 — Thread generations are discarded by the architecture layer.** `norx-kernel/src/arch/x86_64/syscall.rs:344-357` and `src/arch/aarch64/syscall.rs:238-251` index contexts using only the low 16-bit slot, while `ThreadId` generations live in the high bits (`src/process.rs:44-50`). `CONTEXT_VALID` is never cleared on reap, so a recycled slot can appear to have the old thread's context. Store and validate the complete generational id and revoke context on thread destruction.
- [ ] **P1-03 — Kernel shared tables are not uniformly SMP-safe.** `norx-kernel/src/vfs.rs:995-999,2572-2579`, `src/pipe.rs:144,232-233`, `src/memory.rs:8-17`, and `src/drivers/network.rs:353-453` mutate global state with no global lock (IRQ masking is not an SMP lock). Introduce one documented lock/ownership policy and race tests for VFS, pipes, frame allocation, and network state.
- [ ] **P1-04 — Runtime slots also lose process generations.** `norx-kernel/src/user_runtime.rs:38-43,295-319` uses an `UnsafeCell` store protected only by local IRQ masking and maps a process to a slot using only its low bits. Add an SMP lock and a generational slot key; prove that an exited process cannot access a recycled runtime/address space.
- [ ] **P1-05 — Frame free accepts non-owned physical memory.** `norx-kernel/src/memory.rs:109-122` accepts any aligned frame above 1 MiB and only rejects duplicates in the return cache. A bad internal caller can inject firmware/kernel/device frames into the allocator and corrupt page tables or data. Track allocation ownership and reject frames outside an allocated range.
- [ ] **P1-06 — Persistent file handles are path aliases, not file objects.** `norx-kernel/src/vfs.rs:1533-1535,1671-1707,1722-1765` stores a persistent path and re-stats/reopens it for every read/write. Rename-and-recreate can redirect an existing fd to a different inode. Store a stable backend inode/extent identity or enforce an immutable-handle contract, with rename/reopen race tests.
- [ ] **P1-07 — Boot/firmware parsers dereference unchecked handoff pointers.** `norx-kernel/src/boot.rs:1209-1225` implements `read_u8/u32/u64` as unconditional raw reads; structural checks occur after reads and EFI function pointers are trusted from firmware tables. Add bounded headers/tables, checked arithmetic, pointer-region validation, and malformed Multiboot/FDT/EFI fixtures.
- [ ] **P1-08 — Timed-out block requests can release live DMA state.** `norx-kernel/src/drivers/block.rs:324-347` resets a queued request after a poll timeout even though a lower driver may still own its buffer/descriptor. Quiesce or cancel before reuse, poison request generations, and add delayed-completion tests before asynchronous drivers are enabled.
- [ ] **P1-09 — DMA contracts do not prove buffer ownership.** `norx-kernel/src/drivers/framework.rs:73-90` validates alignment/range but relies on a caller-supplied owner and physical address. Make DMA buffers allocator-issued, device-scoped, and lifetime-tracked; add an IOMMU/trusted-device decision for PCI/virtio.
- [ ] **P1-10 — Service authority is represented by forgeable value structs.** `userdb/norx/src/lib.rs:65-95,317-330,1144-1150`, `login/norx/src/lib.rs:78-105,293-327`, and `passwd/norx/src/lib.rs:66-124` accept copied uid/capability fields as the authorization input. Keep kernel checks, but replace service-to-service authority with an opaque kernel-issued session/capability handle and revalidation tests; these structs must never be treated as proof.
- [ ] **P1-11 — User creation bypasses the password policy.** `userdb/norx/src/lib.rs:163-184,483-549` validates the policy only in `change_password`; `create_user` hashes arbitrary bytes directly. Reject empty/oversized/NUL/newline passwords before hashing and cap hashing work, with creation-path tests.
- [ ] **P1-12 — Locked-user sentinel can become a real password.** `userdb/norx/src/lib.rs:374-391,552-615` unlocks an account by only clearing `FLAG_LOCKED`, while `create_locked_user` stores a fixed known `LOCKED_PASSWORD_HASH`. Use an explicit `NO_PASSWORD` state or force a password reset before unlock; test unlock, login, and reset transitions.
- [ ] **P1-13 — Getty accepts replayable control events.** `getty/norx/src/lib.rs:318-337,386-390` uses a bare boolean login result and identity-free `session_closed`; `Restart` returns to prompting without revoking an active session/token. Add terminal/session nonce correlation, reject stale events, and make restart terminate/revoke the prior session.
- [ ] **P1-14 — PID1 config parsing has no input/resource budget.** `quickinit/src/lib.rs:90-205,287` allows unbounded source lines, services, strings, arrays, environment, and capability names before creating `Vec`/`String` values. Enforce byte/count/dependency/argument limits before allocation and test a memory-DoS corpus.
- [ ] **P1-15 — Build output and tool identity are not trusted.** `toolchain/scripts/build.py:25-42,85-99` accepts arbitrary `NORX_*`/`RUSTC` paths, only substring-checks tool versions, validates ELF by report text (`elf64` plus `entry`), and copies to destinations with `shutil.copy2` non-atomically and symlink-following. Pin realpaths/digests, validate machine/ABI/segments/entry/W^X/interpreter, and publish atomically into a confined rootfs.
- [ ] **P1-16 — Freestanding allocators recurse forever on contention.** `coreutils/norx/src/lib.rs:289-323` and `userspace/rust/src/lib.rs:561-597` retry `compare_exchange` by recursively calling `alloc` while advertising `Sync`. Use a bounded iterative retry/fail path or a real synchronized allocator, then run concurrent allocation tests.
- [ ] **P1-17 — Startup ABI trusts argc and pointer arithmetic.** `coreutils/norx/src/lib.rs:339-374`, `userspace/rust/src/lib.rs:554-559`, and `userspace/runtime/src/startup/start.c:7-12` dereference the initial stack and derive argv/envp without bounds, NULL-termination, or a maximum argc check; C also narrows argc to `int`. Define a versioned bounded startup block and reject malformed stacks before any pointer walk.
- [ ] **P1-18 — `cp` has a destructive TOCTOU window.** `coreutils/norx/src/lib.rs:926-975` stats source/destination, then opens/truncates the destination by path. A rename/replacement race can overwrite the wrong target, and a later read error leaves a truncated file. Use fd-relative/no-follow checks and copy to a same-directory temp file followed by atomic rename.
- [ ] **P1-19 — C++ local-static guard is not a valid state machine.** `userspace/runtime/src/cxx/cxxabi.cpp:25-40` returns the CAS result directly; a second thread observing guard state `1` can proceed as if initialization completed. Implement pending/initialized/aborted states with wait/recursion handling, or explicitly disable thread-safe statics until threads exist.
- [ ] **P1-20 — Getty ABI fields are not validated at the trust boundary.** `getty/norx/src/lib.rs:104-118` slices `username` using caller-provided `username_length` without checking it against the fixed array. A malformed handoff can panic the login path. Validate every length/reserved field before conversion and fuzz the handoff ABI.
- [ ] **P1-21 — Storage commit can succeed and still report failure.** `userdb/norx/src/lib.rs:855-876` returns an error if `unlock()` fails after replace/sync have already committed the new database. A retry can run against stale in-memory state or produce an ambiguous commit. Return a distinct `CommittedButUnlockFailed` state and make callers reload/reconcile before retrying.

##### P2 — correctness and hardening backlog

- [ ] **P2-01 — Login lockout is global, not scoped.** `login/norx/src/lib.rs:149-200,239-264` shares one lockout/next-attempt across all usernames and terminals; one user's failures throttle every user, while any success clears the same state. Key policy state by account plus terminal/source and test parallel sessions.
- [ ] **P2-02 — Authentication leaks account state.** `login/norx/src/lib.rs:178-196` and `userdb/norx/src/lib.rs:296-315` distinguish missing, disabled, locked, expired, and bad-password paths and skip the verifier for missing/disabled users. Normalize externally visible errors and verifier/timing behavior.
- [ ] **P2-03 — Session environment/path validation is incomplete.** `login/norx/src/lib.rs:90-105,329-338` blocks only `LD_` variables and checks lexical paths; it does not enforce a complete loader-variable allowlist or kernel-resolved no-follow/confinement semantics. Validate an explicit environment allowlist and bind shell/cwd to kernel path capabilities before exec.
- [ ] **P2-04 — Userdb path and hash parsers accept non-canonical values.** `userdb/norx/src/lib.rs:1025-1035,1072-1095` permits `.`/`..` path components and checks only base64-looking characters/minimum lengths rather than canonical decoded salt/digest sizes. Canonicalize/reject special components and parse Argon2 parameters and decoded lengths strictly.
- [ ] **P2-05 — `read_sources` invokes callbacks with partial input.** `coreutils/norx/src/lib.rs:1130-1177` records a `read_all` error but still calls the transformation callback with incomplete data. Make failed reads fail closed and add short-read/error injection tests.
- [ ] **P2-06 — `find` has output/node budgets but no complete traversal budget.** `coreutils/norx/src/lib.rs:1028-1072` limits depth only; a wide tree can grow output and temporary allocations until the fixed heap aborts. Bound visited nodes, output bytes, and per-command work; detect cycles via stable inode identity.
- [ ] **P2-07 — Runtime libc is not thread-correct and sleep semantics are wrong.** `userspace/runtime/src/libc/unistd.c:7-11,53-69` uses process-global errno, ignores `tv_nsec`, and returns the requested seconds on sleep errors. Make errno TLS when threads land, validate nanoseconds, propagate interruption/error remainder, and test edge cases.
- [ ] **P2-08 — Syscall structs are not version/size negotiated.** `norx-kernel/src/syscall.rs:375-461,731-955` copies fixed-size `SpawnSpec`/`SessionSpec`/delegation structs while mostly ignoring `reserved` fields. Add ABI version and declared size checks, require reserved bytes zero, and reject future/partially initialized layouts deterministically.
- [ ] **P2-09 — Kernel `read_dir` trusts backend name lengths.** `norx-kernel/src/syscall.rs:1201-1208` slices the local name array using `entry.name_length`; a malformed backend entry can panic the syscall path. Validate count, name length, UTF-8/terminators, and backend invariants before indexing.
- [ ] **P2-10 — RAMFS create failure leaks the newly linked inode.** `norx-kernel/src/vfs.rs:1551-1588` creates a node before permission/read-only checks and does not roll it back on the later failure path. Make create/open transactional and add repeated failing `O_CREAT` capacity tests.
- [ ] **P2-11 — Mount security flags are declarative only.** `norx-kernel/src/vfs.rs:288-320` defines `no_exec`, `no_suid`, and `no_dev`, but the audit found no enforcement at exec, credential, or device-open boundaries. Either implement and test the policy or remove the flags until they are real.
- [ ] **P2-12 — Quickinit stop/restart state is race-prone.** `quickinit/src/lib.rs:577-629,669-765` has no child generation/epoch, manually stopped `Always` services can be restarted when their old child exits, and `now_ms + backoff` is unchecked. Add kill/reap state, generation matching, checked deadlines, and stale-exit tests.
- [ ] **P2-13 — Rust syscall mirror is behind the C/kernel ABI.** `userspace/rust/src/lib.rs:16-51` omits `Lseek`, `Fstat`, `Fchmod`, and `Fcntl` present in `norx-kernel/src/syscall.rs:57-59` and `userspace/include/norx/syscall.h:49-52`. Generate or compile-check one canonical ABI definition.
- [ ] **P2-14 — Dynamic loader trust inherits mutable `/lib`.** `norx-kernel/src/dynamic.rs:789-806` restricts names and roots but does not authenticate library contents or bind them to immutable inode/version metadata. Define signed/hashed loader inputs and enforce ownership/permissions before enabling dynamic user services.
- [ ] **P2-15 — Parser arithmetic needs fuzz coverage.** `norx-kernel/src/dynamic.rs:833-850` and `src/elf.rs:553-565` contain offset helpers whose `offset + width` arithmetic and bounded slice assumptions should be made checked at the helper boundary. Add malformed ELF/dynamic-image fuzzing with a no-panic contract.
- [ ] **P2-16 — Getty prompt/session deadlines are caller-driven.** `getty/norx/src/lib.rs:286-308,331-337,340-354` accepts input based on phase but does not reject an expired prompt unless `tick()` was called; `session_closed` also leaves failed-attempt state in place. Enforce deadlines in every state-changing API, reset attempts on a verified close, and test skipped-tick/stale-close cases.
- [ ] **P2-17 — Coreutils path construction allocates before enforcing the path limit.** `coreutils/norx/src/lib.rs:487-512` copies attacker-controlled `PWD` and raw input into a `Vec` before checking `MAX_PATH`; it also leaves lexical aliases unresolved. Bound each component/copy first and use kernel canonical/no-follow path resolution.

##### P3 — low-severity contract debt

- [ ] **P3-01 — C++ destructors are silently discarded.** `userspace/runtime/src/cxx/cxxabi.cpp:12-23` reports successful `__cxa_atexit` without storing callbacks. Either implement bounded exit cleanup or document/guard the unsupported ABI surface.
- [ ] **P3-02 — Small fixed heaps turn ordinary output into process aborts.** `coreutils/norx/src/lib.rs:289-330` and bounded command paths allocate output/input without a uniform command-wide budget. Return `ENOMEM`/short failure consistently instead of relying on allocator abort behavior.
- [ ] **P3-03 — Invariant failures still panic in production paths.** Examples include `quickinit/src/lib.rs:603,762,774` and `norx-kernel/src/vfs.rs:2987-2989`; replace externally reachable `expect`/“loop exhausted means descendant” behavior with typed error handling and invariant tests.

Audit gate: P0 items block any claim of multi-user security. P1 items block SMP,
DMA, persistent writable storage, and untrusted service execution. Every closed
item must carry a regression test, a negative/fuzz case where applicable, and a
QEMU smoke or documented reason why the path is host-only.

#### 0.1 Mandatory gates and dependency graph

These gates are the transition criteria for the whole roadmap. They are not
extra documentation: a later stage is blocked until the corresponding gate is
closed. A bounded fixture may be kept for bring-up, but it must be labelled
`[~]` and cannot satisfy a release gate by itself.

- [ ] **G0 — secure kernel core.** Close P0-01..P0-04 and the P1 items that
  affect SMP, usercopy, physical-frame ownership, file handles, boot parsing,
  DMA, and syscall/ABI trust boundaries. Add race, malformed-input, and
  negative tests before enabling untrusted services or multi-user claims.
- [ ] **G1 — canonical platform ABI.** Generate C and Rust bindings from one
  versioned ABI definition. Require size/version/reserved-field validation,
  stable errno and restart rules, explicit ownership of handles, and a
  compatibility CI job for every kernel/userspace change.
- [ ] **G2 — real target execution.** Provide isolated process address spaces,
  `exec`, `mmap`/`munmap`/`brk`/`mprotect`, a real process heap, blocking wait
  primitives, signal delivery/return, thread identity, deterministic teardown,
  and resource revocation on exit. Static smoke ELFs do not close this gate.
- [ ] **G3 — secure storage and namespaces.** Implement actor-aware uid/gid or
  capability authorization, stable file objects, no-follow path traversal,
  atomic same-directory replacement, durable commit/recovery semantics,
  mount-flag enforcement, and a versioned `/cfg`/`/users`/`/dev`/`/sys`
  layout. Persistent writable storage is blocked until this gate closes.
- [ ] **G4 — services and devices.** Define kernel-issued opaque capability and
  session handles, revocation, IPC lifetime, devfs/hotplug ownership, DMA
  lifetime, device isolation, and CPU/memory/IO limits. Userspace drivers and
  service restart are blocked until failure ownership is enforceable.
- [ ] **G5 — POSIX single-user profile.** Pass the declared POSIX profile for
  process, files, libc, `termios`/PTY, signals, time, threads, shell, and core
  utilities on x86_64 and aarch64. Every unsupported POSIX area must be
  documented and tested as unsupported rather than silently stubbed.
- [ ] **G6 — package and recovery trust.** Ship a static package bootstrap,
  trusted-key root, signed metadata, artifact verification, immutable store,
  atomic generations, rollback, and boot recovery. Source builds require a
  sandbox with network/resource policy; they are not part of bootstrap.
- [ ] **G7 — graphical/network/audio release.** Only after G0--G6, freeze the
  display, socket, audio, Wayland, application-registry, and sandbox contracts
  and run the complete target matrix. Browser and compatibility runtimes are
  consumers of this gate, never prerequisites for it.

Dependency order:

```text
G0 → G1 → G2 → G3 → G4 → G5 → G6 → G7
│     │     │     │     │     │     │
│     │     │     │     │     │     └─ desktop, browser, large ports
│     │     │     │     │     └─ Gamma profiles, signatures, rollback
│     │     │     │     └─ quickinit services, devfs, IPC, quotas
│     │     │     └─ userdb, persistent VFS, account/session tools
│     │     └─ runtime, quickinit bootstrap, nsh target execution
│     └─ all public C/Rust headers and syscall consumers
└─ low-level drivers may bring up in parallel, but remain untrusted
```

#### 0.2 Kernel prerequisites currently missing from the stage boundaries

- [ ] Add a real wait queue/futex-like primitive with cancellation, timeout,
  wake-one/wake-all, ownership, and no-spinning tests. Polling loops do not
  count as blocking semantics.
- [ ] Define the complete signal model: masks, dispositions, pending state,
  delivery frames, restart behavior, process-group signals, `SIGCHLD`, and
  teardown/revocation ordering.
- [ ] Define PTY, session, controlling-terminal, `termios`, window-size, and
  foreground-process-group ABI before calling shell job control complete.
- [ ] Define monotonic and realtime clocks, conversion/overflow rules,
  interruption semantics, clock correction, timezone data ownership, and the
  userspace time ABI before login, TLS, timed services, or reproducible builds.
- [ ] Publish process mapping and heap syscalls with guard-page, W^X, ASLR,
  lazy-fault, copy-on-write, quota, and unmap teardown tests.
- [ ] Define devfs/sysfs and hotplug event ownership. `/dev` must not be a
  package archive payload, and `/sys` must not expose raw kernel pointers.
- [ ] Define crash/core-dump, watchdog, reboot, and recovery records together
  with package-generation rollback; a warm-reboot log alone is not recovery.
- [ ] Add a platform matrix for every ABI and driver: x86_64/aarch64,
  serial/graphical, staged/QEMU/real-device, and explicitly unsupported.
- [ ] Maintain a repository manifest mapping every roadmap component to its
  actual path, upstream remote/commit, owner, package recipe, test entrypoint,
  and source-of-truth status. A roadmap name such as `nsh`, `gamma`, or `nde`
  must not imply that a repository exists when only a generated fixture does.

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

Status: `[~]` low-level QEMU bring-up and bounded contracts exist, but the
framework is not a production ownership boundary until G0 and G4 close. In
particular, DMA, per-CPU state, asynchronous completion, device isolation, and
the aarch64 support matrix remain gated.

Build one coherent framework that can describe a PS/2 controller and a
complex PCI/virtio device without forcing every driver through architecture or
bus-specific hacks. The framework must be powerful enough for real hardware,
but small enough that a new driver is mostly probe logic and device policy.

#### 1.1 Deep audit and contract design

- [~] Audit every existing driver, bus helper, MMIO/PIO access, interrupt
  path, DMA assumption, global state, unsafe cast, fake capability, and test
  hook. Record what is removed, what is retained, and which caller owns each
  resource.
- [~] Remove dead drivers, duplicated register definitions, architecture leaks,
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
- [~] Standardize MMIO, PIO, port I/O, endianness, register barriers, cache
  maintenance, DMA ownership, physical/virtual address conversion, and
  alignment checks for x86_64 and aarch64.
- [~] Define interrupt delivery for legacy IRQ, MSI/MSI-X, GIC, and threaded or
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

- [~] Add a QEMU-friendly audio bring-up path first (AC'97 or Intel HDA), with codec
  discovery, DMA ring handling, underrun/overrun recovery, a small PCM API,
  and a deterministic silent fallback when no audio device exists.
- [~] Add virtio-net as the first network driver, then cover e1000 or another
  simple PCI device. Define RX/TX ownership, queues, interrupts, checksum
  handling, link state, and packet lifetime before building protocols.
- [~] Build the minimum diagnostic network stack: Ethernet, ARP, IPv4, ICMP,
  UDP, TCP, DHCP, and DNS. Keep the stable socket ABI, IPv6, routing,
  firewall, and advanced offloads behind Section 5.11 and G1/G5.
- [~] Replace the current display assumption with a basic display driver
  contract: framebuffer discovery, mode list, mode selection, EDID, damage
  regions, flush, cursor, and hotplug notification.
- [~] Support the current firmware framebuffer and a simple virtio-gpu path
  first. Do not attempt 3D acceleration yet; keep the interface extensible for
  future complex GPUs without embedding a vendor driver in the early kernel.

#### 1.5 Block devices, filesystems, and mounts

- [x] Split the current RAM block smoke device into a block layer with sector
  geometry, queueing, completion, timeout, partition discovery, read-only
  mode, and explicit cache/writeback ownership.
- [~] Implement `ramfs` as the first bounded writable filesystem fixture, with inodes,
  directories, regular files, file offsets, permissions, unlink/rename, and
  mount/unmount tests.
- [~] Implement FAT32 for EFI/QEMU volumes: BPB validation, cluster chains,
  long names, directory updates, read-only boot mode first, then safe writes.
- [~] Implement ext4 in staged form: superblock/group validation, extents,
  inode and directory reads, journaling boundaries, permissions, and a
  read-only mount before write support.
- [~] Implement btrfs only after the block, page-cache, checksum, and VFS
  contracts are stable. Start with a safe read-only mount, tree traversal,
  checksums, snapshots/subvolumes discovery, and explicit unsupported-feature
  errors.
- [~] Define the VFS mount API: mount tree, mount namespaces, path lookup,
  dentry/inode lifetime, mount flags, propagation rules, and unmount safety.
  Every filesystem must mount through the same interface.
- [x] Add mount sources and targets to the serial-debugger for inspection and
  recovery; no filesystem command should become a hidden test-only path.

#### 1.6 Framework completion criteria

- [ ] Run the full driver matrix on QEMU x86_64 and aarch64 with absent-device,
  timeout, hot-unplug, malformed-descriptor, DMA-failure, and interrupt-storm
  tests.
- [ ] Ensure every driver has bounded initialization, explicit teardown, a
  serial diagnostic view, and no architecture-specific code outside the
  appropriate bus/arch layer.
- [ ] Document the public driver contracts and freeze the first stable version
  before adding high-level userspace device services.

### 2. Linux-like syscalls, processes, libraries, and execution

Status: `[~]` the syscall and ELF paths are staged and partially QEMU-tested;
G1 and G2 remain open. Host policy tests and embedded fixtures must not be
reported as complete POSIX process, thread, memory, or dynamic-linking support.

Replace the removed universal syscall probes with a real Linux-shaped userspace
foundation. Linux conventions are the default because they are well-tested,
well-documented, and compatible with existing toolchains; Norx keeps its own
implementation and security model.

#### 2.1 Audit and ABI boundary

- [~] Audit all remaining syscall entry code, register handling, TSS/kernel
  stacks, SVC vectors, error returns, pointer conversions, and stale comments.
  Delete obsolete universal bins, probe state, and compatibility names rather
  than carrying a second ABI.
- [~] Define an architecture-neutral syscall table and architecture-local
  entry wrappers. x86_64 should use the Linux-like register convention around
  `syscall/sysret`; aarch64 should use an SVC convention with the same logical
  argument and return model.
  - [x] Isolate the x86_64 syscall-entry stack from the ordinary kernel stack;
    verify resumable user exits and clean ELF returns without the former
    `rip=0x8` invalid-op failure.
- [~] Define syscall numbering/versioning, negative error returns, `errno`
  translation, restart rules, 32/64-bit types, time types, pointer widths,
  and structure alignment before publishing userspace headers.
- [ ] Implement safe `copy_from_user`/`copy_to_user`, range validation,
  overflow checks, page-fault recovery, and protection against TOCTOU in every
  syscall that accepts userspace memory.

#### 2.2 Processes, threads, and address spaces

- [~] Design the process, thread, task, file-descriptor table, credentials,
  signal/event state, parent/child relationship, and exit/wait model.
- [~] Implement a minimal process address space with user/kernel separation,
  page-table ownership, guard pages, stack growth policy, ASLR hooks, W^X, and
  deterministic teardown.
- [ ] Add kernel threads and user threads as separate concepts, with scheduler
  accounting, blocking/wakeup, preemption, and safe context switching.
- [~] Start with `exit`, `wait`, `getpid`, `gettid`, `yield`, `sleep`, and a
  small file-descriptor API. Add fork-like semantics only after address-space
  cloning and copy-on-write have a tested design.
- [~] Define capabilities and credentials before exposing privileged device,
  mount, memory, or network operations. Do not make a global root identity the
  only authorization mechanism.

#### 2.3 ELF loader and simple native binaries

- [~] Implement a bounded ELF64 loader for x86_64 first: header/program-header
  validation, `PT_LOAD` mapping, permissions, zero-fill, entry-point checks,
  stack construction, auxiliary vector, and initial register state.
- [~] Add a minimal native userspace runtime that can start a statically linked
  `init`-style binary, write to serial or a file descriptor, allocate memory,
  and exit cleanly.
- [~] Add aarch64 ELF loading only after the common loader contract is stable;
  keep architecture-specific relocations, entry state, and ABI details local.
- [~] Implement `execve`-like replacement with argument/environment copying,
  close-on-exec descriptors, interpreter selection, and failure rollback.
- [ ] Add negative tests for malformed headers, overlapping segments,
  non-canonical addresses, W+X mappings, truncated files, oversized argv/env,
  and invalid entry points.

#### 2.4 Libraries and dynamic linking

- [~] Choose the initial userspace ABI and libc strategy: a small Norx libc
  layer with Linux-shaped calls, or an explicitly supported existing libc
  subset. Do not promise glibc compatibility before the required kernel APIs
  exist.
- [ ] Support `ET_DYN`/PIE and shared objects in staged form: interpreter path,
  `PT_INTERP`, `PT_DYNAMIC`, symbol lookup, relocations, TLS, and library search
  policy.
- [~] Define `/lib` or a Norx equivalent through the VFS mount tree;
  library discovery must not depend on host filesystem paths.
- [ ] Add a dynamic-linker smoke program with one shared library, one failing
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

- [ ] Freeze the first documented syscall ABI and publish headers/examples in
  a separate userspace/toolchain area under the shared `BoaKernel` directory.
- [ ] Exercise process creation, ELF loading, file I/O, memory mapping,
  dynamic linking, VM execution, faults, permissions, and process teardown in
  QEMU on both supported architectures.
- [ ] Ensure no universal machine-code payload, architecture-neutral syscall
  probe, or test-only execution path remains in the kernel after the new path
  is active.

#### 2.7 POSIX compatibility profile

The default userspace profile is intended to be maximally practically POSIX
compatible. This is a compatibility target, not a claim of formal
certification or transparent execution of arbitrary Linux/glibc binaries.
Linux-shaped syscall names and conventions may be used, but POSIX behavior,
error semantics, cancellation, inheritance, and observable ordering are the
compatibility contract.

- [ ] Publish a versioned POSIX profile with a requirements matrix for
  processes, sessions, file descriptors, pathname resolution, permissions,
  `exec`, signals, threads, `termios`, PTY, clocks, timers, libc, locale,
  shell behavior, utilities, and the portable socket surface.
- [ ] Map each POSIX requirement to a kernel ABI, userspace implementation,
  conformance test, target matrix entry, and an explicit unsupported or
  implementation-defined result. No stub returning success may be presented
  as POSIX support.
- [ ] Complete the process model needed by POSIX: `fork`-like or documented
  spawn semantics, `exec`, inheritance/close-on-exec, `waitpid`-like waits,
  exit status, orphan/reparent behavior, process groups, sessions, and
  deterministic resource teardown.
- [ ] Complete the file and directory model: `openat`/dirfd semantics,
  stable file descriptions, offsets, `stat`/`fstat`, links, rename atomicity,
  `fcntl`, `umask`, ownership, mode checks, no-follow policy, append,
  `fsync`, and durable error reporting.
- [ ] Complete the thread and synchronization model: pthread-compatible
  creation/join/detach, TLS, mutexes, condition variables, rwlocks, once,
  cancellation, atomics, scheduling attributes, and C11/C++ interoperability.
- [ ] Complete signals and terminal behavior: dispositions, masks, pending
  signals, `SIGCHLD`, interruption/restart rules, `sigaction`, `kill`,
  `termios`, PTY allocation, controlling terminals, foreground groups, and
  terminal-size changes.
- [ ] Complete time and locale foundations: monotonic/realtime clocks,
  nanosleep and interruption remainder, timers, timezone data, locale/UTF-8
  policy, numeric limits, and deterministic behavior when wall time is absent.
- [ ] Build a POSIX conformance suite from small independent C fixtures and
  shell scripts. Run it in host emulation where useful and in target QEMU on
  both architectures; record deviations instead of hiding them in fixtures.
- [ ] Keep Norx capabilities, service handles, package metadata, and IPC in
  namespaced extensions. They may restrict or extend POSIX operations only
  through documented error/feature checks and must never make authority
  depend on forgeable user-provided structs.

### 3. Protection, IPC, and service boundaries

Status: `[~]` contracts and staged lifecycle checks exist, but G0, G3, and G4
are open. No service may receive authority from copied UID/capability structs,
and no risky driver may move to userspace until ownership and revocation are
enforced.

- [ ] Define the kernel/user trust boundary and capability transfer model.
- [ ] Add channels, shared-memory rings, event queues, and blocking semantics
  as the primary IPC path; keep signal-like notifications secondary.
- [ ] Move restartable or risky drivers toward userspace services once process
  isolation, DMA ownership, and a service supervisor exist.
- [~] Add persistent warm-reboot logs and panic context after the storage and
  mount layers can reserve a reliable backing region.

### 4. Validation and release discipline

Status: `[~]` CI and bounded negative tests exist, but the section is not a
release gate until every open Section 0 finding is mapped to a regression,
negative/fuzz, and target test. The old broad `[x]` entries are therefore
scoped evidence, not a production sign-off.

- [~] Keep x86_64 and aarch64 builds, Clippy, formatting, GRUB image creation,
  and QEMU smoke tests in CI.
- [ ] Add negative tests for every parser and hardware boundary: malformed
  boot tags, invalid descriptors, bad filesystems, broken ELF files, invalid
  syscall pointers, DMA failures, and interrupt races.
- [~] Keep serial-debugger output stable enough for automated smoke assertions,
  while treating framebuffer screenshots as visual regression artifacts.
- [ ] Before declaring a subsystem stable, document its ownership model,
  failure behavior, ABI, supported hardware, and explicit unsupported cases.

### 5. Standalone projects and userspace

Status: `[~]` repository policy and staged fixtures exist, but the standalone
projects are consumers of G1--G6, not replacements for those gates. The order
below is revised to make the static bootstrap, POSIX profile, package trust,
and target integration explicit.

Every project in this section lives in its own repository under
`BoaKernel/REPONAME/`. Projects must consume the documented Norx ABI instead
of reaching into kernel internals. Each repository needs its own README,
license and contribution rules, reproducible cross-builds, unit/host tests,
QEMU smoke coverage with stable serial markers, and an installation layout
that can be assembled into a Norx system image. The dependency order below is
intentional but gated: toolchain and static runtime first, then G2 target
execution and the POSIX profile, then the minimal PID 1 handoff and shell,
then storage-backed account/session tools, the Gamma bootstrap and package
trust, and only after that individually owned daemons, graphics, networking,
audio, and applications. A host-testable policy core or generated fixture is
not a substitute for its target ABI gate.

#### 5.1 `toolchain` — C, C++, and Rust toolchains

Status: `[~]` reproducible compiler/sysroot and static fixtures exist, but the
full POSIX runtime, threads, floating point, dynamic linking, and target
execution remain gated by G1, G2, and the POSIX profile in Section 2.7.

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
- [~] Port the base C/C++ runtime and system libraries: headers, `libc`,
  `libm`, threading/atomics, C++ ABI, exception/unwind policy, startup and
  termination code, and the supported `libstdc++` or `libc++` subset.
- [~] Port the Rust target support: `core`, `alloc`, `compiler_builtins`,
  panic/runtime policy, `std` where justified, linker integration, build
  scripts, and a stable Norx userspace crate/API layer.
- [~] Compile and run representative C, C++, and Rust programs on graphical
  QEMU, including syscalls, process creation, ELF/file and dynamic-loader
  kernel gates, signal/event model checks, and clean exit on both architectures.
  The external fixtures prove real static ELF execution and clean exit; the
  staged kernel smoke remains the evidence for file I/O, dynamic-loader,
  signal, and event contracts that are not yet exposed as pointer-based
  userspace APIs.
- [~] Publish versioned SDK/sysroot artifacts and compatibility rules so
  quickinit, nsh, coreutils, and future applications can upgrade without
  silently changing the ABI. `toolchain/sdk.toml` freezes SDK 0.1.0, target
  manifests carry ABI and file hashes, and `publish-sdk.py` emits deterministic
  per-target release archives plus an index.
- [ ] Expand the runtime toward the declared POSIX profile: `dirent`,
  `fcntl`, `stat`, `unistd`, `termios`, signals, pthread/TLS, condition
  variables, clocks/timers, locale, `errno`, cancellation, and standards-based
  error semantics. Every temporarily unsupported call must fail explicitly
  with the documented result and have a conformance fixture.
- [ ] Gate C++ ABI, Rust `std`, dynamic linking, floating-point state, and
  hosted-library ports on the corresponding kernel contracts. A compile/link
  success is not runtime support until the target QEMU conformance test passes.

#### 5.1.1 Repository ownership and generated boundaries

Status: `[~]` repository ownership and generated-output rules are documented;
the canonical ABI, source-repository inventory, and per-target compatibility
checks still need to be enforced as part of G1.

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

Status: `[~]` the host policy core and a minimal static bootstrap are present;
full target PID 1, filesystem-backed configuration, signal/process-group
control, blocking waits, service execution, and recovery remain open behind
G2--G4.

Progress: the standalone `quickinit` repository contains a host-testable
policy core for strict init configuration, dependency validation, startup
ordering, restart/backoff limits, timeouts, signal forwarding, bounded
structured logs, recovery mode, and supervisor CLI parsing. The static
bootstrap proves only the currently bounded handoff; the target process,
filesystem, signal, and storage interfaces remain open. Real Rust/C/C++ ELF
fixtures writing through user pointers are evidence for G2 subpaths, not a
complete service supervisor.

The separate `quickinit/bootstrap` no-std package builds a pinned external ELF
for x86_64 and aarch64. Its first handoff is a bootstrap contract only; it must
not be promoted to the full PID 1 contract until G2 provides real spawn/exec,
blocking wait, signals, inherited descriptors, and deterministic revocation.

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

- [~] Start `quickinit` as the first userspace process after the kernel,
  continue the kernel log without losing sequence/order information, reap
  children, and expose a deterministic boot failure mode.
- [~] Define the init configuration format, environment, working directory,
  standard streams, capabilities, resource limits, dependencies, startup
  ordering, readiness notification, and shutdown behavior. `quickinit` now
  validates the strict bounded contract, merges global/per-service
  environments, orders dependency-safe startup, gates notification readiness,
  and applies configured shutdown signals and timeout actions; target syscall
  application remains scoped to the following kernel ABI work.
- [~] Implement autostart for programs, foreground/background processes,
  services and daemons, with restart policies, crash limits, timeouts,
  dependency failures, and clean signal forwarding. `quickinit` now models
  explicit autostart/manual start, foreground/background mode, service/daemon/
  oneshot lifecycles, `Stopping` and timeout escalation, intentional-stop
  restart suppression, dependency blocking, and best-effort signal fan-out;
  process groups and target signal/launch syscalls remain a separate kernel
  ABI boundary.
- [~] Provide a CLI for inspecting and changing log levels, listing service
  state, starting/stopping/restarting services, viewing exit causes, and
  requesting a controlled shutdown or reboot. `quickinit` now exposes a
  deterministic parser, typed dispatcher, stable status/log renderers, and
  backend-error propagation; transport and target console wiring remain ABI
  integration work.
- [~] Add service logging with stable timestamps/sequence IDs, bounded log
  retention, serial fallback, and an explicit policy for unavailable storage
  or a stalled child. `quickinit` now bounds memory/serial records, exposes
  `on_storage_unavailable = serial|continue|halt`, accepts storage availability
  from the target adapter, and logs the timeout/stop/kill escalation policy
  exactly once per transition.
- [ ] Launch the first shell service (or recovery shell) only after the
  required mounts, device services, and logging path are ready; keep a
  recovery mode available when normal configuration is invalid. `quickinit`
  now validates `[boot]` gate references, holds the normal shell until all
  gates and storage are ready, provides an explicit recovery-shell launch,
  and defaults malformed configuration to a safe shell recovery mode.
- [~] Test PID 1 behavior under malformed configuration, missing binaries,
  rapid crashes, orphan reaping, signal races, dependency cycles, shutdown,
  and kernel panic/reboot log hand-off. The quickinit host suite now has 25
  focused policy tests for these cases; existing x86_64/aarch64 graphical
  QEMU smokes cover PID 1 hand-off, child reaping, deterministic failure, and
  kernel log continuation.

#### 5.3 `norxshell` (`nsh`) — interactive shell

Status: `[~]` the parser, expansion, job-control model, and smoke harness are
host-tested/staged. Target command execution, arbitrary argv/environment
transfer, blocking pipelines, background jobs, PTY, and kernel process-group
ownership remain open behind G2 and the POSIX profile.

Ownership decision: the current workspace has no separate top-level `nsh`
repository; it has `userspace` fixtures and `norx-kernel` smoke scripts. Before
calling this project complete, either create `BoaKernel/nsh` or explicitly
make `userspace` the source of truth and remove the misleading standalone-repo
claim.

- [~] Choose the implementation base: a focused native shell or a carefully
  scoped fork of an existing shell. Keep the first version small enough to
  audit and define compatibility goals before copying Bash or fish behavior.
  The ownership document must select a native Rust `no_std`/`alloc` shell over
  a hosted Bash/fish fork and freeze the initial POSIX-shaped subset,
  unsupported behavior, and ABI gates.
- [~] Implement command parsing, quoting, escaping, variables, environment
  inheritance, command lookup, exit status, sequences, conditionals,
  pipelines, redirections, here-documents, and background jobs. The initial
  host-tested `nsh` parser/expansion layer now preserves these
  constructs and exposes a narrow `CommandRunner` execution hook; real command
  lookup/process/fd application remains gated on the Norx userspace ABI.
- [~] Keep `nsh` focused on parsing, expansion, command lookup, pipeline
  orchestration, terminal interaction, dispatch, and Linux-like shell
  builtins. The shell-intrinsic set remains inside `nsh`: `cd`, `export`,
  `set`/`unset`, `exit`, `umask`, `jobs`, `fg`, `bg`, `wait`, `source`, and
  `exec`. These commands must run in the shell process because they change its
  cwd, environment, exit state, process groups, or image. `pwd`, `echo`,
  `env`, `true`, `false`, and similar commands also have standalone utility
  implementations, as in Linux; the current host-tested builtin state and
  typed job/source/exec requests remain the `nsh` implementation boundary.
- [~] Provide line editing, history, completion, prompts, startup files,
  terminal resize handling, UTF-8 policy, and safe behavior when no
  interactive terminal is available. `nsh` now provides bounded
  line input, history navigation, deterministic completion/prompt/resize,
  UTF-8/NUL validation, and explicit interactive/non-interactive startup
  policy; target keyboard/TTY wiring remains an ABI hook.
- [~] Define process groups, job control, terminal ownership, signal
  forwarding, interrupt handling, and the behavior of pipelines that fail
  part-way through. `nsh` now provides a host-tested `JobControl`
  contract with process-group/terminal-owner state, group signals, interrupt
  forwarding, completion, and deterministic `pipefail` status; Norx process
  groups and TTY ownership remain the backend hook.
- [~] Add script-mode tests and interactive QEMU smoke tests for quoting,
  redirection, pipelines, job control, errors, interrupted commands, and
  clean shell exit. Script-mode integration coverage must be owned by the
  selected `nsh` source repository; the current freestanding `nsh` ELF is
  only a staged fixture built by the userspace/toolchain path, and
  `norx-kernel/scripts/nsh-smoke.sh` now
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

Status: `[~]` host tests, bounded adapters, and staged command fixtures exist,
but the account/session stack is blocked by G0/G3/G5. Coreutils and account
tools cannot be release-complete while filesystem authorization, startup ABI,
atomic storage, recovery authentication, session handles, and POSIX process/
terminal semantics remain open.

Every project below is a separate repository under `BoaKernel/`. A repository
may ship several tightly related binaries, but each binary still needs its own
owner, CLI contract, tests, installation path, and QEMU smoke marker. `nsh`
must launch these programs through the documented userspace ABI rather than
embedding their implementations.

The exception is the Linux-like shell builtin set listed in 5.3: those
implementations stay in `nsh` because a child ELF cannot modify its parent
shell's cwd, environment, process group, or exit state.

##### 5.4.1 `coreutils` — base command-line utilities

- [~] Define the coreutils scope and shared command/error/output conventions;
  keep shell process state and privilege transitions out of this project.
- [~] Implement the first bootable set: `cat`, `echo`, `env`, `ls`, `pwd`,
  `mkdir`, `rmdir`, `cp`, `mv`, `rm`, `touch`, `ln`, `stat`, `find`, `grep`,
  `head`, `tail`, `sort`, `wc`, `true`, `false`, and `sleep`. `cd` is not a
  coreutils binary because it must change the parent shell's working directory.
- [~] Keep system-facing commands out of coreutils. `ps`/`kill`,
  `mount`/`umount`, `dmesg`/`logctl`, reboot/shutdown, and
  hardware/storage/network diagnostics belong to the separately owned
  repositories in 5.5; `service` remains part of quickinit's supervisor CLI.
- [~] Establish pathname, permissions, symlink, locale/UTF-8, buffering,
  exit-status, error-message, and signal conventions shared by all commands.
- [~] Support static and dynamically linked builds as the toolchain permits,
  keep binaries small for the base image, and avoid hidden host-filesystem
  dependencies.
- [~] Add per-command unit tests, malformed-input tests, pipeline tests, and
  a minimal root-filesystem smoke image that runs the commands through `nsh`.

##### 5.4.2 `userdb` — users, groups, credentials, and password policy

- [~] Define the account/group record format, lookup API, password-hash policy,
  credential inheritance, capability mapping, disabled/locked states, and
  ownership of mutable account data.
- [~] Provide bounded parsing, atomic updates, corruption recovery, migration
  rules, and explicit behavior when persistent storage is unavailable.
- [~] Publish the ABI consumed by `login`, `passwd`, `userctl`, `su`, `sudo`,
  `getty`, `quickinit`, and session utilities; never expose password material
  through shell environment or serial diagnostics.
- [~] Add host tests, malformed-database tests, permission tests, lockout and
  recovery tests, and staged QEMU persistence smoke coverage.

##### 5.4.3 `getty` — CLI greeter and terminal session launcher

- [~] Implement serial/TTY discovery, terminal initialization, login prompt,
  bounded retry/lockout behavior, session limits, and clean hand-off to
  `login`; keep authentication and account mutation in `userdb`/`login`.
- [~] Support recovery-console and unavailable-terminal policies without
  turning a missing optional console into a kernel panic.
- [~] Define ownership of the terminal, controlling process group, window
  size, signal forwarding, and shutdown/restart behavior with `quickinit`.
- [~] Add interactive serial and graphical QEMU smoke tests with stable
  greeter, timeout, failed-login, and successful-handoff markers.

##### 5.4.4 `login` — account authentication and shell session setup

- [~] Authenticate through `userdb`, enforce disabled/locked/expiry policy,
  apply credentials and capabilities, and start the selected user shell with
  a clean environment, cwd, umask, limits, and standard streams.
- [~] Define failure delays, retry limits, audit records, recovery login, and
  behavior when the account database or terminal backend is unavailable.
- [~] Keep `login` separate from `getty`: `getty` owns the terminal prompt,
  while `login` owns authentication and session construction.
- [~] Add host tests plus QEMU tests for success, bad credentials, locked
  users, environment setup, shell exit, and session teardown.

##### 5.4.5 `passwd` — password management utility

- [~] Implement password change, confirmation, policy validation, old-password
  verification, privileged reset, lock/unlock, and safe atomic persistence
  through `userdb`.
- [~] Keep hashes, prompts, error messages, and audit output out of command
  arguments, environment variables, core dumps, and serial logs.
- [~] Add malformed-input, policy-boundary, concurrent-update, interrupted
  write, and QEMU user-session tests.

##### 5.4.6 `userctl` — user and group administration utility

Status: `[~]` smoke/build integration exists, but no standalone `userctl`
repository is present in the current workspace. Create the repository or make
its ownership explicit before treating the `[x]` checklist below as a shipped
project.

- [~] Provide separate commands or subcommands for creating, modifying,
    disabling, deleting, and listing users/groups, with explicit capability and
    ownership checks.
- [~] Define UID/GID allocation, primary and supplementary groups, home
    directory policy, default shell selection, and rollback on partial failure.
- [~] Add permission, duplicate-ID, malformed-record, storage-failure, and
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

Status: `[ ]` portfolio definitions exist, but the planned repositories are
not yet present in this workspace. Create each source-of-truth repository (or
record an explicit ownership exception) before assigning implementation
completion. Every daemon also depends on the relevant G4 service boundary,
POSIX profile, and target ABI rather than only on a host policy test.

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

Status: `[ ]` design only. The current workspace has no standalone `gamma`
repository; `toolchain/gamma.toml` is a staging recipe, not the package
manager. Gamma is a platform gate and must be brought forward operationally:
the static bootstrap is needed before broad package/application work, while
remote repositories and source builds wait for G3, G4, and G6.

Gamma combines a simple pacman-like command line with immutable packages,
declarative system configuration, reproducible builds, atomic generations, and
rollback. The initial implementation must keep the package manager independent
from any one repository host and must use the documented platform userspace
ABI.

##### 5.9.0 Bootstrap-first milestone

- [ ] Create the `gamma` source repository or record the exact repository
  ownership exception and keep `toolchain/gamma.toml` limited to packaging
  metadata.
- [ ] Define a minimal static `gamma-bootstrap` artifact that can install or
  verify the first local package without Gamma already being present. It must
  have a fixed input format, bounded memory/disk use, no host-path fallback,
  deterministic diagnostics, and a documented hand-off to normal Gamma.
- [ ] Define development, recovery, and production trust modes before package
  installation. Development unsigned packages must never be accepted by a
  production profile accidentally.
- [ ] Establish the first trusted key/root-of-trust hand-off from boot or
  maintenance mode to Gamma, including key rotation, revocation, offline
  recovery, and a failed-verification path that cannot activate a generation.
- [ ] Build a static bootstrap image containing only the kernel, required
  storage/runtime pieces, quickinit bootstrap, and the minimal package
  verifier; prove it on both target architectures before installing a full
  userland profile.

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
- [ ] Publish a migration table for current fixture paths such as `/storage`,
  `/hello.txt`, `/etc`, and `/home` to the final layout. Every compatibility
  alias must have an owner, lifetime, no-follow rule, and removal version;
  packages must never infer layout from a host directory.

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
- [ ] Implement the minimum build-sandbox primitives before enabling source
  builds: filesystem/network namespace or equivalent confinement, CPU/memory/
  IO quotas, read-only source inputs, writable output staging, syscall/device
  policy, cancellation, and cleanup after a crashed build.
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

Status: `[ ]` blocked until G4--G7. The first graphical profile requires a
software framebuffer fallback, input/seat/PTY semantics, shared-buffer
ownership, package application registry, login/session lifecycle, and client
isolation. Section 1.4's framebuffer/virtio-gpu smoke is only low-level
bring-up; it does not satisfy the desktop contract.

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

Status: `[ ]` the current low-level virtio-net work is a diagnostic bring-up,
not a stable socket/network-management platform. This section owns the
production socket ABI, routing, DNS, TLS, firewall, daemon, and recovery gates
and therefore depends on G1, G4, G5, and G6.

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

Status: `[ ]` AC'97/HDA smoke support in Section 1.4 is not a production audio
ABI. PipeWire depends on real processes, threads, timers, shared memory,
permissions, device discovery, and recovery; those prerequisites must close
before this section can enter the desktop release profile.

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

##### 5.13.4a WLI priority compatibility vertical

Status: `[~]` HOST classifier/plan and ARM64 compile-only native CLI are the
first implementation stage. There is no Norx Linux personality, translated
execution, Wine/Proton port, guest isolation, GUI, or production claim.
Source of truth: [NorxTeam/wli](https://github.com/NorxTeam/wli),
initial pushed revision `bb0c2652bd784582a21845ab9b73b03f6f3ab0bd`;
package/classifier license MIT. Native CLI `diagnostics`/`inspect-fd` uses
Norx startup/fstat/read/write; guest execution is still unsupported.

- [~] Provide bounded `no_std` ELF/PE architecture identification, input and
  path limits, explicit backend discovery/launch plans, and fail-closed `run`.
  Compile an ARM64 static native diagnostics/inspection CLI using the
  existing versioned Norx userspace runtime; compilation is not execution.
- [ ] Execute the static ARM64 CLI in QEMU under real userspace, checking
  startup, inherited fd input, stdout/exit, bad input, and actor permissions.
- [ ] Establish a kernel-owned, versioned Linux ABI personality inspired by
  FreeBSD Linuxulator. First prove static AArch64 Linux hello/exit with a
  documented whitelist, ELF/auxv, syscall conventions, fd/process ownership,
  and default unsupported errors. No userspace binary may self-declare a
  Linuxulator capability or bypass Norx authorization.
- [ ] Port a pinned FEX backend to ARM64 Norx or its Linux personality and
  demonstrate an x86_64 static hello/exit guest. Audit mmap/mprotect, Linux
  signals, clone/futex/threads/TLS, host libraries, CPU FP/CRC requirements,
  strong x86 memory ordering, executable-memory authority, instruction cache,
  quotas, cancellation, and crash cleanup before enabling dynamic guests.
  FEX is the first candidate; Box64 remains an alternative evaluation.
- [ ] Package an authenticated x86 guest rootfs only after path confinement,
  dynamic-loader ownership, runtime identity and update/rollback pass.
- [ ] Integrate ARM64 Wine with FEX WoW64/ARM64EC, first a console PE fixture;
  keep Wine's Unix host adaptation and per-prefix isolation explicit.
- [ ] Integrate separately versioned Proton, DXVK/VKD3D-Proton and WLI GUI
  after Vulkan, audio, sockets, input and application isolation pass. Do not
  claim Steam/game compatibility from a console fixture. Keep Linux VM
  fallback deferred until a real virtualization/isolation backend exists.

Primary upstream references checked 2026-10-10:
[FEX](https://github.com/FEX-Emu/FEX),
[FEX Wine backend](https://fex-emu.com/FEX-2501/),
[Box64](https://github.com/ptitSeb/box64),
[Valve Proton](https://github.com/ValveSoftware/Proton),
[Wine source](https://gitlab.winehq.org/wine/wine), and
[FreeBSD Linuxulator](https://docs.freebsd.org/en/books/handbook/linuxemu/).
These upstream Linux/Unix implementations are port candidates, not available
Norx features; source/library and licensing boundaries remain independent.

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
- [ ] Create one user-facing `wli` compatibility utility instead of
  exposing separate Wine and Proton products. The same package must provide
  both a CLI and a graphical manager with application registration, icons,
  `.link` creation, logs, diagnostics, and per-application profiles.
- [ ] Maintain Wine and Proton as separate upstream forks/backends inside the
  `wli` project. Keep Wine/Proton, DXVK, VKD3D-Proton, runtime files, and
  licensing boundaries independently updateable; unify them at the manager
  and user-profile level rather than merging their source trees.
- [ ] Implement the Windows/Win32 backend for `wli` with prefixes,
  runtime selection, DLL/component management, graphics and audio routing,
  controller/input mapping, filesystem integration, sandbox permissions,
  crash logs, and rollback-safe updates.
- [ ] Implement a Linux software compatibility backend for `wli`: Linux
  ELF identification/loading, a versioned Linux syscall compatibility layer,
  Linux shared-library/runtime profiles, filesystem and IPC translation,
  capability boundaries, and clear unsupported-feature diagnostics.
- [ ] Provide an isolated Linux runtime fallback for applications requiring
  unsupported kernel features, using a verified Linux userland and the least
  heavyweight available isolation backend. The GUI and CLI must expose which
  backend is used and keep its files, network, devices, and updates isolated.
- [ ] Define initial `wli` CLI operations for backend/runtime discovery,
  install/import, prefix or Linux-runtime creation, launch, stop, remove,
  logs, diagnostics, repair, update, rollback, and application registration.
- [ ] Add an NDE GUI for searching, installing, configuring, launching,
  repairing, and removing Windows and Linux software. It must support per-app
  backend selection, runtime versions, environment overrides, file/URI
  associations, sandbox permissions, GPU/audio options, and safe recovery.
- [ ] Add compatibility tests for Windows GUI/console applications, Linux ELF
  applications, shared libraries, signals, threads, files, sockets, graphics,
  audio, clipboard, input, process isolation, upgrades, rollback, and failure
  diagnostics before enabling `wli` in the default image.
- [ ] Evaluate OCI runtimes such as `runc` or `crun`, then Podman or another
  container manager, only after namespaces, capabilities, cgroups-equivalent
  limits, networking, storage, and image verification are available.
- [ ] Evaluate additional browsers, game runtimes, and launchers as separate
  packages; each must pass the application registry, sandbox, update,
  rollback, and security review.

#### 5.14 Cross-project release gates

These gates are repeated at every release profile, not deferred until the end
of the project. The first ABI and bootstrap gates must close before target
userspace and package-dependent projects are accepted; the final bullets add
the secure-multi and desktop evidence.

- [ ] Freeze a versioned userspace ABI and SDK manifest consumed by all
  standalone repositories; CI must reject incompatible sysroot or syscall
  changes without an explicit migration.
- [ ] Publish a `bootstrap` profile containing only the kernel, early storage,
  static runtime, quickinit bootstrap, and recovery diagnostics. It must boot
  without Gamma or the network and must have a deterministic rollback path.
- [ ] Publish a `single-user` POSIX profile and run the conformance suite for
  process, files, libc, signals, time, terminal, shell, and core utilities on
  aarch64. Mark every deviation explicitly.
- [ ] Publish a `secure-multi` profile only after G0--G6: authenticated account
  recovery, actor-aware permissions, persistent storage, service revocation,
  signed generations, and degraded/offline boot must all be exercised.
- [ ] Build a minimal bootable userland image containing `quickinit`, `nsh`,
  `coreutils`, and the required libraries for aarch64.
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
