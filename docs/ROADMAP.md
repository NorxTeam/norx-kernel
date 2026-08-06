# Norx Roadmap

## Current Track

1. UEFI boot without external bootloader.
2. ExitBootServices, memory map, physical frame allocator.
3. Kernel paging, physical-frame allocation, and lazy page-fault allocation.
4. Interrupt timers and preemptive scheduling.
5. Norx syscall ABI with SCLA activation.
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
- A tiny architecture-neutral RAM block device `norx-ram0` exists for future VFS/filesystem smoke tests.
- Shell diagnostics now include `hw`, `mem`, `vm`, and `block`.

## Current VFS Status

- VFS mounts a tiny in-kernel root over `norx-ram0`.
- `/hello.txt` supports read/write through shell commands `ls`, `cat`, `write`, and `vfs`.
- `/bin/hello` and `/bin/args` are tiny Norx object files with code sections stored in VFS over `norx-ram0`.
- This is a smoke layer for future ext4/zfs/btrfs/exfat adapters, not a real on-disk filesystem yet.

## Current Process Status

- `run` resolves executable paths through VFS, reads the object bytes, validates the header, loads the code section into the user code page, and executes it through the architecture user-mode path.
- `rundebug` keeps the temporary in-kernel entry-id path for smoke tests.
- User program launches now allocate a PID and record owner, state, exit code, runtime ticks, capabilities, path, and code size in a small fixed process table exposed by `ps`, `procs`, `wait <pid>`, and `kill <pid>`.
- Loaded user code now performs buffered `Write(ptr,len)` before `Exit`, so userspace string output reaches the kernel log path on x86_64 and aarch64.
- Loaded user code receives a compact `argc/argv` table in the user stack and can print `argv[0]` through the same `Write(ptr,len)` path.
- Process I/O is now bound to the shell session that launched it: VM-owned commands write to the framebuffer session, serial-owned commands write/read through serial. The current synchronous runner keeps `stdin/stdout/stderr` in a process-scoped I/O guard, with small per-session stdout/stderr rings readable through `stdio tail` and consumable through `stdio read`.
- Executables expose a tiny Norx object header: magic, ABI version, entry id, flags, capabilities, code length.
- Executables now carry capability bitsets in their VFS object header; shell command `sec` shows kernel/user-mode security status.
- x86_64 GDT has ring-3 code/data descriptors and a loaded TSS with RSP0 kernel stack; `userctx` exposes selectors, user pages, and TSS diagnostics.
- x86_64 prepares a user launch context: user code page, user stack page, selectors, kernel transition stack, and a tiny user probe payload.
- x86_64 now clones the firmware PML4 into a Norx-owned CR3 and switches to it after direct-map setup.
- x86_64 can enter the user probe page through `iretq`; diagnostics prove CPL3 RIP/RSP and the probe payload bytes.
- x86_64 `userprobe` now performs a real ring3 -> `int 0x80` -> universal syscall dispatcher -> shell round-trip and returns value `42`.
- x86_64 user syscall diagnostics expose trap count, int80 hits, fast-path hits, last op, and probe fault RIP.
- aarch64 has the shared universal syscall dispatcher, a real `svc #0` kernel smoke path through the exception vector, and `userctx` reports EL0 PC/SP layout.
- aarch64 allocates physical frames for the EL0 probe payload and stack, writes the payload bytes, maps both frames into the shared 44-bit-safe user VA window, and `userprobe` now performs EL0 -> `svc #0` -> kernel -> shell round-trip with value `42`.
- Next step: move process execution out of the synchronous shell call path, using the process table and stdio ring cursors as the first async output collection path.

## Current Syscall Status

- SCLA constants and activation registers are documented for x86_64 and aarch64.
- A Norx universal syscall dispatcher exists for activation, clock, byte/buffer read/write, and exit; loaded user code exercises session-routed buffered write, `argv[0]` output, and exit on both supported architectures.
- Syscall dispatch accepts capability sets and denies unauthorized operations.
- x86_64 configures `syscall/sysret` MSRs (`STAR`, `LSTAR`, `FMASK`) and has an entry stub wired to the universal dispatcher.
- x86_64 syscall entry now switches from user RSP to the TSS/RSP0 kernel stack before calling Rust code, then restores user RSP for `sysretq`.
- x86_64 also has a DPL3 `int 0x80` gate stub for the same universal dispatcher and uses a high user VA region outside firmware identity maps.
- Shell command `syscalltrap` exercises the x86_64 trap handler path from kernel smoke; `userprobe` proves a tiny CPL3 trap round-trip through the same universal dispatcher.
- aarch64 has an arch-local dispatcher/status layer, same-EL SVC entry, EL0 code/stack page mappings, and a working EL0 SVC probe round-trip.

## Norx Kernel Patch Backlog

These are the next implementation tasks. Do not treat the current smoke-test
paths below as permanent architecture decisions.

### 1. Audit the kernel and remove accidental complexity

- [x] Inspect the boot path, memory and paging code, IRQ/timer setup,
  scheduler, VFS, process loader, syscall ABI, drivers, logging, and panic
  handling end to end.
- [x] Find workarounds, dead code, duplicated state, misleading abstractions,
  architecture leaks, unsafe assumptions, and code that exists only for an
  obsolete smoke test.
- [x] Record each finding with its root cause, affected callers, risk, and the
  smallest correct replacement before changing it.
- [x] Remove speculative interfaces and test-only state once the dependent
  paths are removed.

#### Audit outcome (2026-08-06)

- [x] Removed `src/heap.rs`: `main::efi_main` initialized a static 64 KiB
  buffer while consuming 16 physical frames and never connecting those frames
  to the buffer. The replacement is to defer a real heap until an allocator
  has an actual kernel caller.
- [x] Hardened the UEFI → framebuffer boundary in
  `uefi::gop_framebuffer`. Unsupported pixel formats, null bases, invalid
  stride/geometry, overflowed sizes, and undersized buffers are rejected
  before `framebuffer::Fb::put_pixel` creates a slice or writes pixels.
- [x] Hardened the UEFI → physical allocator boundary in
  `uefi::memory_map`/`memory::load_ranges`. Descriptor size, map bounds, page
  count overflow, range overflow, and the fixed range-table limit are checked;
  adjacent ranges are merged and skipped ranges are reported instead of being
  counted as allocatable memory.
- [x] Fixed a mutable-global alias in `log::handle_ansi`: `CSI J` now resets
  the borrowed `Console` directly, and the reset clears the pending ANSI
  parser state as well.
- [x] Made `paging::init_norx_cr3` stop when its writable mapping cannot be
  established instead of continuing into an unsafe page-table write.
- [x] Removed a misleading non-fatal `error::report` call for a missing
  framebuffer, made the x86-only compiler feature conditional, and removed
  dead pixel-format handling. Both targets now pass format, Clippy with
  `-D warnings`, and debug builds.

#### Deferred findings with owners

- GRUB must replace the `efi_main`/`SystemTable`/GOP/ExitBootServices contract
  before UEFI-specific workarounds can be removed; this is task 2.
- x86 user-mode setup still marks firmware-owned transition pages as user
  accessible, and aarch64 page-table writes still rely on identity-mapped
  frames. These are protection-boundary issues requiring the new boot contract
  and syscall/user-mode decision, so they belong to tasks 2–3 rather than a
  partial local workaround.
- The VFS object binaries, universal syscall probes, synchronous process path,
  framebuffer shell, and their diagnostic state are intentional remaining
  smoke infrastructure. They are tracked for deletion in tasks 3–5; removing
  them during this audit would leave a cross-task half-migration.
- UART writes still wait indefinitely when the configured device is absent;
  the serial-debugger rewrite must define the early-console failure policy in
  task 4.

### 2. Replace the current boot path with GRUB

- [ ] Replace the current direct UEFI-first entry path with a GRUB boot
  contract.
- [ ] The repository currently has no Limine configuration; this task means
  replacing the existing boot entry design with GRUB, not deleting a present
  Limine integration.
- [ ] Define and validate the GRUB hand-off for memory map, framebuffer,
  command line, boot modules, and architecture information.
- [ ] Remove UEFI-only boot workarounds that are no longer needed and keep the
  kernel entry layer small and architecture-specific where required.
- [ ] Update build scripts, documentation, CI, and local run targets for the
  GRUB boot image and supported architectures.

### 3. Remove universal-syscall binary smoke paths

- [ ] Remove the embedded `/bin/hello` and `/bin/args` machine-code payloads,
  `object_code`, and their architecture-specific byte arrays.
- [ ] Remove VFS sectors and object generation used only to store those test
  binaries.
- [ ] Remove the universal syscall/SCLA demonstration path if the audit shows
  it is not part of the intended kernel ABI.
- [ ] Remove the related loader, probe, capability, status, and diagnostic
  code instead of leaving partial compatibility shims.
- [ ] Keep only the real architecture-local syscall entry points and the
  minimum ABI needed by the future userspace design.

### 4. Replace the kernel shell with `serial-debugger`

- [ ] Remove the framebuffer keyboard shell and its role as a system control
  interface.
- [ ] Keep a minimal serial-only input/output loop for kernel diagnostics.
- [ ] Rename shell-facing types, functions, messages, and documentation to
  `serial-debugger`.
- [ ] Make command parsing line-oriented, deterministic, and safe during early
  boot and failure handling.
- [ ] Keep only commands that are explicitly useful for inspecting or
  recovering the kernel.

### 5. Delete obsolete test commands and debugger baggage

- [ ] Review every current command and remove anything not needed by the
  serial debugger.
- [ ] Candidates for removal include `dmaptest`, `syscalltrap`, `sclatest`,
  `stdio`, `lazytest`, `block`, `vfs`, `run`, `rundebug`, and `runuser`, plus
  their backing code and state.
- [ ] Remove test-only counters, probe payloads, fake process paths, embedded
  binaries, smoke-only VFS data, and obsolete diagnostic formatting.
- [ ] Keep a small intentional diagnostic set for boot state, memory, paging,
  interrupts, processes, logs, and explicit panic testing.
- [ ] Re-run the audit after deletion so no command references or dead
  dependencies remain.

### 6. Implement the Norx boot presentation

- [ ] Add a compact ASCII-art `Norx` title as the first visible boot output.
- [ ] Emit ordered kernel startup logs for each initialization stage.
- [ ] Use consistent colored status markers in the style of `[    OK    ]`,
  with matching failure and warning states.
- [ ] Separate kernel initialization, hardware discovery, diagnostics, and
  the future operating-system boot stage.
- [ ] Leave an explicit extension point for the future OS bootloader without
  claiming that an unimplemented stage has completed.
- [ ] Keep the same essential startup information available through serial
  output when framebuffer output is unavailable.

### 7. Replace the console font with JetBrains Mono

- [ ] Locate the already-downloaded JetBrains Mono font asset and verify its
  format and license before bundling it.
- [ ] Convert the required glyph range to the bitmap representation used by
  the Norx console.
- [ ] Replace the current `noto-sans-mono-bitmap` dependency and generated
  font data with the JetBrains Mono asset.
- [ ] Preserve the existing low-level renderer contract unless the font size
  requires a measured layout change.
- [ ] Verify ASCII-art alignment, boot logs, serial-debugger output, and panic
  text at the target resolution.

### 8. Redesign the kernel panic screen

- [ ] Use a dark-gray background with a centered sad face `:(`.
- [ ] Display a prominent centered `KERNEL PANIC` heading.
- [ ] Render the detailed error description below it, including error kind,
  architecture, address/register data, ticks/time, boot stage, and relevant
  subsystem state.
- [ ] Continue emitting the critical panic information to serial output for
  headless debugging.
- [ ] Keep the panic renderer allocation-free and safe when the heap,
  interrupts, or normal logging path are unavailable.
- [ ] Test the layout on both supported architectures and with deliberately
  triggered panic paths.
