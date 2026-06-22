# Boa Roadmap

## Current Track

1. UEFI boot without external bootloader.
2. ExitBootServices, memory map, physical frame allocator.
3. Kernel paging, heap, and lazy page-fault allocation.
4. Interrupt timers and preemptive scheduling.
5. Boa syscall ABI with SCLA activation.
6. Process loader, address spaces, VFS, drivers, userspace services.

## Architecture Notes To Explore

- Single Address Space OS: one large 64-bit virtual address space with unique regions per process. Avoid page-table switches where safe. Needs a real protection story first: hardware tags, MPK-like domains, verifier, or hybrid isolation.
- Lazy allocation by default: reserve virtual ranges first, allocate physical frames on page fault. Requires page fault dispatch, zero-fill policy, and OOM behavior.
- PaX-style hardening: NX stack/heap where possible, W^X, ASLR, guard pages, strict executable mappings, hardened loader.
- Memory tagging: ARM64 MTE where available. On x86, investigate LAM, MPK, and software tag checks. Treat tags as optional hardware acceleration, not the only safety boundary.
- Thread-migrating IPC: evaluate after processes and scheduler exist. Keep service ABI compatible with regular message passing so this can be optimized in later.
- Channels instead of UNIX signals: prefer shared-memory ring buffers and event queues. Signals should not be the primary async primitive.
- Capabilities instead of root: process authority comes from unforgeable kernel capabilities, not global superuser identity.
- Driver crash isolation: long term, move drivers to userspace services and restart them independently. Kernel drivers should remain early-boot/minimal.
- Safe kernel bytecode/JIT: eBPF or WASM-like verified programs for filters and syscall hooks. Needs verifier before JIT.
- Black box ring buffer: persistent warm-reboot log region for last kernel events and panic context.
- Hot-reloading modules: only after module ABI, capabilities, and quiescence rules exist.

## Near-Term Rule

Do not build speculative versions of these ideas before the required lower layer exists. Leave interfaces shaped so they are possible later.

## Current VM Status

- x86_64 has a working first lazy fault path for a small kernel-resident page pool.
- x86_64 lazy pages are backed by real physical frames through the direct map.
- aarch64 keeps the shared VM API but needs real syndrome/fault-address exception entry before lazy mapping is enabled there.
- Paging now has an architecture-neutral status API and shell command `paging`.
- x86_64 maps the first 16 MiB at `0xffff800000000000` and exposes `dmaptest`.
- aarch64 direct-map base is planned but not active.

## Current Scheduler/Timer Status

- The scheduler has runtime state and is charged at 100 Hz.
- x86_64 has a real PIT/PIC IRQ0 scheduler timer.
- aarch64 uses `cntfrq_el0` polling until GIC + generic timer interrupt wiring exists.
- Next step: move x86_64 from legacy PIT/PIC to APIC/HPET calibration and add aarch64 GIC timer IRQ.

## Current Input Status

- Serial input is always active for the serial shell session.
- x86_64 PS/2 keyboard uses IRQ1 with a small scancode ring buffer and polling fallback.
- aarch64 needs USB HID or platform keyboard input wiring.

## Current Interrupt Status

- x86_64 counts timer, keyboard, spurious interrupts, and fatal exceptions.
- Shell command `irq` exposes interrupt counters for smoke testing.
- aarch64 interrupt counters exist but stay zero until GIC/timer IRQ entry is wired.

## Current Hardware/Block Status

- x86_64 detects/enables local APIC through CPUID + IA32_APIC_BASE MSR and LAPIC MMIO SVR, while IRQ routing still uses legacy PIC/PIT.
- aarch64 reports GIC as deferred until controller discovery and MMIO setup exist.
- A tiny architecture-neutral RAM block device `boa-ram0` exists for future VFS/filesystem smoke tests.
- Shell diagnostics now include `hw`, `mem`, `vm`, and `block`.

## Current VFS Status

- VFS mounts a tiny in-kernel root over `boa-ram0`.
- `/hello.txt` supports read/write through shell commands `ls`, `cat`, `write`, and `vfs`.
- `/bin/hello` and `/bin/args` are tiny Boa object files with code sections stored in VFS over `boa-ram0`.
- This is a smoke layer for future ext4/zfs/btrfs/exfat adapters, not a real on-disk filesystem yet.

## Current Process Status

- `run` resolves executable paths through VFS, reads the object bytes, validates the header, loads the code section into the user code page, and executes it through the architecture user-mode path.
- `rundebug` keeps the temporary in-kernel entry-id path for smoke tests.
- User program launches now allocate a PID and record owner, state, exit code, runtime ticks, capabilities, path, and code size in a small fixed process table exposed by `ps` and `procs`.
- Loaded user code now performs buffered `Write(ptr,len)` before `Exit`, so userspace string output reaches the kernel log path on x86_64 and aarch64.
- Loaded user code receives a compact `argc/argv` table in the user stack and can print `argv[0]` through the same `Write(ptr,len)` path.
- Process I/O is now bound to the shell session that launched it: VM-owned commands write to the framebuffer session, serial-owned commands write/read through serial. The current synchronous runner keeps `stdin/stdout/stderr` in a process-scoped I/O guard, with small per-session stdout/stderr rings readable through `stdio tail` and consumable through `stdio read`.
- Executables expose a tiny Boa object header: magic, ABI version, entry id, flags, capabilities, code length.
- Executables now carry capability bitsets in their VFS object header; shell command `sec` shows kernel/user-mode security status.
- x86_64 GDT has ring-3 code/data descriptors and a loaded TSS with RSP0 kernel stack; `userctx` exposes selectors, user pages, and TSS diagnostics.
- x86_64 prepares a user launch context: user code page, user stack page, selectors, kernel transition stack, and a tiny user probe payload.
- x86_64 now clones the firmware PML4 into a Boa-owned CR3 and switches to it after direct-map setup.
- x86_64 can enter the user probe page through `iretq`; diagnostics prove CPL3 RIP/RSP and the probe payload bytes.
- x86_64 `userprobe` now performs a real ring3 -> `int 0x80` -> universal syscall dispatcher -> shell round-trip and returns value `42`.
- x86_64 user syscall diagnostics expose trap count, int80 hits, fast-path hits, last op, and probe fault RIP.
- aarch64 has the shared universal syscall dispatcher, a real `svc #0` kernel smoke path through the exception vector, and `userctx` reports EL0 PC/SP layout.
- aarch64 allocates physical frames for the EL0 probe payload and stack, writes the payload bytes, maps both frames into the shared 44-bit-safe user VA window, and `userprobe` now performs EL0 -> `svc #0` -> kernel -> shell round-trip with value `42`.
- Next step: move process execution out of the synchronous shell call path, using the process table and stdio ring cursors as the first async output collection path.

## Current Syscall Status

- SCLA constants and activation registers are documented for x86_64 and aarch64.
- A Boa universal syscall dispatcher exists for activation, clock, byte/buffer read/write, and exit; loaded user code exercises session-routed buffered write, `argv[0]` output, and exit on both supported architectures.
- Syscall dispatch accepts capability sets and denies unauthorized operations.
- x86_64 configures `syscall/sysret` MSRs (`STAR`, `LSTAR`, `FMASK`) and has an entry stub wired to the universal dispatcher.
- x86_64 syscall entry now switches from user RSP to the TSS/RSP0 kernel stack before calling Rust code, then restores user RSP for `sysretq`.
- x86_64 also has a DPL3 `int 0x80` gate stub for the same universal dispatcher and uses a high user VA region outside firmware identity maps.
- Shell command `syscalltrap` exercises the x86_64 trap handler path from kernel smoke; `userprobe` proves a tiny CPL3 trap round-trip through the same universal dispatcher.
- aarch64 has an arch-local dispatcher/status layer, same-EL SVC entry, EL0 code/stack page mappings, and a working EL0 SVC probe round-trip.
