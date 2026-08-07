# Norx Roadmap

## Current Track

1. GRUB hand-off with architecture-specific entry contracts.
2. GRUB memory map, reserved regions, framebuffer, modules, and command line.
3. Kernel paging, physical-frame allocation, and lazy page-fault allocation.
4. Interrupt timers and preemptive scheduling.
5. Architecture-local syscall entry points and a deliberately unspecified ABI.
6. Process loader, address spaces, VFS, and userspace services after the ABI is
   designed from real requirements.

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
- Paging now has an architecture-neutral status API and serial-debugger command
  `paging`.
- x86_64 maps the first 16 MiB at `0xffff800000000000`; the direct-map smoke
  read is internal paging support, not a debugger command.
- aarch64 direct-map base is planned but not active.

## Current Scheduler/Timer Status

- The scheduler has runtime state and is charged at 100 Hz.
- x86_64 has a real PIT/PIC IRQ0 scheduler timer.
- aarch64 uses `cntfrq_el0` polling until GIC + generic timer interrupt wiring exists.
- Next step: move x86_64 from legacy PIT/PIC to APIC/HPET calibration and add aarch64 GIC timer IRQ.

## Current Input Status

- Serial input is always active for the serial-debugger session.
- The framebuffer keyboard path and PS/2 input queue were removed with the
  kernel shell; future interactive input must be added as an explicit debugger
  or userspace feature.

## Current Interrupt Status

- x86_64 counts timer, spurious interrupts, and fatal exceptions.
- Serial-debugger command `irq` exposes interrupt counters for diagnostics.
- aarch64 interrupt counters exist but stay zero until GIC/timer IRQ entry is wired.

## Current Hardware/Block Status

- x86_64 detects/enables local APIC through CPUID + IA32_APIC_BASE MSR and LAPIC MMIO SVR, while IRQ routing still uses legacy PIC/PIT.
- aarch64 reports GIC as deferred until controller discovery and MMIO setup exist.
- A tiny architecture-neutral RAM block device `norx-ram0` exists for future VFS/filesystem smoke tests.
- Serial-debugger diagnostics include `hw`, `mem`, `vm`, and `block` while they
  remain useful during the current bring-up.

## Current VFS Status

- VFS mounts a tiny in-kernel root over `norx-ram0`.
- `/hello.txt` supports read/write through serial-debugger commands `ls`, `cat`,
  `write`, and `vfs`.
- No executable objects are stored in VFS; the root currently contains only the
  text smoke file.
- This is a smoke layer for future ext4/zfs/btrfs/exfat adapters, not a real on-disk filesystem yet.

## Current Process Status

- x86_64 now clones the firmware PML4 into a Norx-owned CR3 and switches to it after direct-map setup.
- No process loader, executable format, process table, user payload, or
  capability model is active. These are postponed until a real userspace
  design exists.

## Current Syscall Status

- No SCLA or universal cross-architecture dispatcher is part of the kernel.
- x86_64 keeps the hardware `syscall/sysret` entry and a TSS-backed kernel stack;
  an unassigned operation returns `ENOSYS`.
- aarch64 keeps the exception-vector SVC entry; an unassigned operation returns
  `ENOSYS`.
- Userspace memory, process execution, capabilities, and the syscall numbers
  remain intentionally undefined until their actual consumers exist.

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

- [x] Removed `src/heap.rs`: the former UEFI entry initialized a static 64 KiB
  buffer while consuming 16 physical frames and never connecting those frames
  to the buffer. The replacement is to defer a real heap until an allocator
  has an actual kernel caller.
- [x] Hardened the firmware → framebuffer boundary in the boot hand-off
  parser. Unsupported pixel formats, null bases, invalid stride/geometry,
  overflowed sizes, and undersized buffers are rejected before
  `framebuffer::Fb::put_pixel` creates a slice or writes pixels.
- [x] Hardened the boot memory-map → physical allocator boundary. Entry size,
  map bounds, page count overflow, range overflow, and the fixed range-table
  limit are checked; adjacent ranges are merged and skipped ranges are
  reported instead of being counted as allocatable memory.
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

- The GRUB entry layer is now in place; the remaining page-table concerns are
  tracked below and are no longer hidden behind a firmware-specific entry API.
- The old x86 user-mode path and aarch64 identity-mapped page-table concerns
  remain historical protection-boundary findings. User execution is now
  intentionally unimplemented; revisit these when a real user ABI is added.
- The VFS object binaries, universal syscall probes, synchronous process path,
  framebuffer shell, and their diagnostic state were removed in tasks 3–5.
  The remaining VFS and syscall code is intentionally minimal until those
  subsystems receive a real OS-level design.
- UART writes still wait indefinitely when the configured device is absent;
  the serial-debugger rewrite must define the early-console failure policy in
  task 4.

### 2. Replace the current boot path with GRUB

- [x] Replace the current direct UEFI-first entry path with a GRUB boot
  contract.
- [x] The repository currently has no Limine configuration; this task means
  replacing the existing boot entry design with GRUB, not deleting a present
  Limine integration.
- [x] Define and validate the GRUB hand-off for memory map, framebuffer,
  command line, boot modules, and architecture information.
- [x] Remove UEFI-only boot workarounds that are no longer needed and keep the
  kernel entry layer small and architecture-specific where required.
- [x] Update build scripts, documentation, CI, and local run targets for the
  GRUB boot image and supported architectures.

#### GRUB hand-off outcome (2026-08-06)

- [x] Added `src/boot.rs` with an x86_64 Multiboot2 parser. It validates the
  loader magic and bounded tag structure, copies the command line, collects
  usable memory, records modules, validates a 32-bit RGB/BGR framebuffer, and
  reserves the kernel image, Multiboot information block, and modules before
  the physical allocator sees the map.
- [x] Added the aarch64 GRUB EFI contract. GRUB uses `chainloader` to start the
  `aarch64-unknown-uefi` kernel image; the EFI entry reads the standard DTB
  configuration table and falls back to `\\boot\\norx.dtb` through the EFI
  Simple File System protocol. The existing FDT parser still owns memory,
  initrd, command-line, and optional framebuffer extraction.
- [x] Kept the x86_64 freestanding Multiboot2 entry and added an aarch64 UEFI
  entry with loaded-image reservation and GOP framebuffer detection. The ARM
  Linux `Image` path was removed from the runnable GRUB contract because GRUB's
  ARM64 `linux` loader requires a Linux EFI stub, which Norx is not.
- [x] Replaced the UEFI run path with `scripts/run.sh`, standalone GRUB EFI
  images, architecture-specific `grub.cfg`, QEMU firmware-variable handling,
  CI image builds, and updated contribution/build documentation.
- [x] Verified x86_64 Multiboot2 and aarch64 UEFI builds plus Clippy with
  `-D warnings`. Full GRUB image execution is wired into CI; the local Windows
  environment lacks `grub-mkstandalone`, `grub-file`, and `xorriso`, so the
  runtime smoke used the official Debian GRUB EFI binaries and QEMU's FAT
  volume fallback.

### 3. Remove universal-syscall binary smoke paths

- [x] Remove the embedded `/bin/hello` and `/bin/args` machine-code payloads,
  `object_code`, and their architecture-specific byte arrays.
- [x] Remove VFS sectors and object generation used only to store those test
  binaries.
- [x] Remove the universal syscall/SCLA demonstration path because it is not
  part of the intended kernel ABI.
- [x] Remove the related loader, probe, capability, status, and diagnostic
  code instead of leaving partial compatibility shims.
- [x] Keep only the real architecture-local syscall entry points and the
  minimum ABI needed by the future userspace design.

#### Universal-syscall cleanup outcome (2026-08-06)

- [x] Deleted the embedded executable objects, architecture-specific machine
  code, object headers, process loader, synchronous process table, user-mode
  probes, capability set, SCLA constants, and stdio rings.
- [x] Reduced the RAM VFS to `/hello.txt`; its one-byte sector length is now
  bounded to 255 bytes instead of pretending a 511-byte file fits.
- [x] Kept only architecture-local `syscall/sysret` and SVC entry stubs. They
  do not expose an ABI yet and return `ENOSYS` for every unassigned operation.
- [x] Removed shell commands and diagnostics that existed only to exercise the
  deleted path: `sec`, `userctx`, `userprobe`, `procs`, `ps`, `wait`, `kill`,
  `run`, `rundebug`, `runuser`, `syscalls`, `syscalltrap`, `sclatest`, and
  `stdio`.
- [x] Verified both supported target builds, both Clippy runs with `-D warnings`,
  formatting, diff whitespace, and a source scan with no universal syscall or
  embedded-binary references remaining.
- The next userspace implementation must begin with an explicit ABI and page
  ownership design; no compatibility layer is intentionally carried forward.

### 4. Replace the kernel shell with `serial-debugger`

- [x] Remove the framebuffer keyboard shell and its role as a system control
  interface.
- [x] Keep a minimal serial-only input/output loop for kernel diagnostics.
- [x] Rename shell-facing types, functions, messages, and documentation to
  `serial-debugger`.
- [x] Make command parsing line-oriented, deterministic, and safe during early
  boot and failure handling.
- [x] Keep only commands that are explicitly useful for inspecting or
  recovering the kernel.

#### Serial-debugger outcome (2026-08-06)

- [x] Replaced the dual framebuffer/serial session shell with one
  `serial_debugger` loop that reads and writes only through the serial driver.
- [x] Removed the framebuffer keyboard input module, PS/2 scancode queue, IRQ1
  handler, and keyboard-only counters.
- [x] The parser accepts one bounded line, handles CR/LF and backspace, ignores
  unsupported control bytes, caps arguments at eight, and never writes past its
  128-byte line buffer.
- [x] Kept diagnostics for time, scheduler, IRQs, hardware, memory, paging,
  drivers, VFS, file inspection/recovery, explicit crash, and halt.
- [x] Verified the serial-debugger rename has no remaining shell/input-session
  references in source; obsolete command deletion is complete in task 5.

### 5. Delete obsolete test commands and debugger baggage

- [x] Review every current command and remove anything not needed by the
  serial debugger.
- [x] Remove the former `dmaptest`, `lazytest`, and `block` debugger commands and
  VFS/shell diagnostics that do not help recover the kernel, plus their backing
  code and state.
- [x] Remove remaining test-only counters, fake paths, smoke-only VFS data, and
  obsolete diagnostic formatting.
- [x] Keep a small intentional diagnostic set for boot state, memory, paging,
  interrupts, future process state, logs, VFS file recovery, and explicit panic
  testing.
- [x] Re-run the audit after deletion so no command references or dead
  dependencies remain.

#### Debugger cleanup outcome (2026-08-06)

- [x] Deleted `dmaptest`, `lazytest`, and the block-device `Device` test view;
  the RAM block device remains only as the VFS backing store.
- [x] Removed the unused VM probe/status API, keyboard counters, and all old
  shell command text. The serial-debugger command set is now explicit and
  bounded instead of falling through to fake process execution.
- [x] Kept only commands with a current inspection or recovery use: help, time,
  scheduler/IRQ/hardware/memory/paging/driver state, uname, VFS listing and
  file recovery, explicit crash, and halt.
- [x] Verified both builds, both Clippy runs with `-D warnings`, formatting,
  whitespace, and a source scan for removed command names and shell/input
  paths.

### 6. Implement the Norx boot presentation

- [x] Add the compact block-art `Norx` title as the first visible boot output.
- [x] Emit ordered kernel startup logs for each initialization stage.
- [x] Use consistent colored status markers in the style of `[  OK  ]`,
  with matching failure and warning states.
- [x] Separate kernel initialization, hardware discovery, diagnostics, and
  the future operating-system boot stage.
- [x] Leave an explicit extension point for the future OS bootloader without
  claiming that an unimplemented stage has completed.
- [x] Initialize an x86_64 VGA text fallback before framebuffer takeover and
  mirror early logs to both console paths.
- [x] Reinitialize the framebuffer console from the bootloader's actual
  width, height, stride, and pixel format so it fills the supplied surface.
- [x] Keep the same essential startup information available through serial
  output when framebuffer output is unavailable.

#### Boot presentation outcome (2026-08-06)

- [x] `bootlog::title()` prints the white Norx ASCII art before the first status
  line. The framebuffer is prepared before that output, so the title reaches
  both serial and the visible console when a framebuffer exists.
- [x] x86_64 starts its early console in VGA text memory, then switches the
  visible path to the full GRUB framebuffer and disables VGA writes; VGA stays
  available as a fallback when no framebuffer is supplied.
- [x] Startup output now follows a linear order: GRUB hand-off, framebuffer,
  clock, drivers, VFS, serial-debugger, memory, architecture tables, interrupts,
  syscall entry, paging, VM, scheduler, timer, and completed initialization.
- [x] Console overflow scrolls the existing log by one line instead of clearing
  the framebuffer, preserving the boot history through serial-debugger startup.
- [x] Added a white bouncing-arrow loader that pulses during long memory and
  paging checks. Every boot action now starts with that loader and replaces it
  with its final `OK`, `WARN`, `INFO`, or `FAILED` status on the same line.
- [x] Deferred OS hand-off is marked as `WARN`; informational diagnostics stay
  white and framebuffer RGB/BGR modes preserve their intended colors.
- [x] Status labels use fixed-width colored markers with the requested `[  OK  ]`
  style; the final line explicitly reports the future OS bootloader hand-off as
  deferred.
- [x] Verified both supported target builds, both Clippy runs with `-D warnings`,
  formatting, and diff whitespace.

### 7. Use a Linux-console bitmap font

- [x] Use Terminus 4.49 normal 8×16, matching the fixed-size bitmap style used
  by a traditional Linux virtual console.
- [x] Convert the required Basic Latin glyph range to the bitmap
  representation used by the Norx console.
- [x] Keep the renderer allocation-free and free of any runtime font parser.
- [x] Verify ASCII-art alignment, boot logs, serial-debugger output, and panic
  text at the target resolution.

#### Font outcome (2026-08-07)

- [x] Bundled `assets/fonts/Terminus-u16n.bdf` and its `Terminus-OFL.txt` license.
- [x] Generated crisp Basic Latin 8×16 bitmap glyphs with a 16px line height.
- [x] Added embedded UTF-8 handling and block glyphs for the title's `▄`, `█`,
  and `▀` characters without adding a runtime font parser.
- [x] Added framebuffer SGR support for bold text; status brackets stay gray
  and regular while the status labels retain their individual colors.
- [x] Removed the Noto dependency and lockfile entries; the kernel now embeds
  only the generated Terminus bitmap data and needs no runtime font parser.
- [x] Verified both supported target builds, both Clippy runs with `-D warnings`,
  formatting, and diff whitespace.

### 8. Redesign the kernel panic screen

- [x] Keep the framebuffer console black and continue panic output after the
  startup log without status prefixes.
- [x] Emit a plain `:( KERNEL PANIC` line followed by detailed error data,
  including error kind, architecture, address/register data, and ticks/time.
- [x] Continue emitting the complete panic information to serial output for
  headless debugging.
- [x] Keep the panic path allocation-free by reusing the existing boot-log and
  serial output path.
- [x] Capture runtime framebuffer output on x86_64 and exercise deliberate
  panic paths on both supported architectures.

#### Panic-output outcome (2026-08-07)

- [x] Removed the separate centered panic renderer; panic output now continues
  in the same black framebuffer console as the startup log.
- [x] Added unprefixed panic heading, diagnostics, page-fault details, and the
  final `SYSTEM HALTED` state; startup `FAIL`/`INFO` markers remain unchanged.
- [x] Reused the same bounded, allocation-free serial and framebuffer logging
  path for headless and visual diagnostics.

#### Runtime verification outcome (2026-08-07)

- [x] x86_64 booted through the clean GRUB EFI configuration, printed the Norx
  title and ordered startup log, exposed an `800x600` framebuffer, accepted
  `help`/`crash` through the serial-debugger, and produced a QEMU framebuffer
  capture with the appended plain panic log and `SYSTEM HALTED` line.
- [x] aarch64 booted through GRUB `chainloader` into the UEFI kernel, loaded
  `norx.dtb` through EFI file services, reached completed initialization, accepted
  `help`/`crash`, and emitted the full panic report with `arch: aarch64`.
