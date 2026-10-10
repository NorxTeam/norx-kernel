# Norx ABI and driver platform matrix

This is the platform support ledger for the kernel boundary. It is the
source of truth for what a target may consume; `README.md`,
`docs/stability-contracts.md`, and the roadmap may link to it but must not
imply broader support than the cells below.

The active native target is ARM64 (`aarch64-unknown-uefi`). Both console
profiles and three evidence/deployment stages are tracked below. Retained
x86 source does not imply native support; x86 guest execution belongs to
`WLI`. Historical x86 matrix cells below are explicitly unsupported:

- **staged** means the bounded contract, fixture, or failure path runs during
  kernel initialization or a target build;
- **QEMU** means the target-specific boot or device smoke is an intended,
  documented test entrypoint and has the scope stated in the notes;
- **real-device** means a physical machine or board with that hardware. The
  current release profile deliberately makes no real-device claim.

Each cell is `staged / QEMU / real-device`:

| Code | Meaning |
| --- | --- |
| `R` | Ready for the bounded scope named by the row. |
| `P` | Partial: fixture, fallback, diagnostic slice, or negative-only evidence; not a complete POSIX/device promise. |
| `U` | Explicitly unsupported in this target/profile. This is an expected result, not a silent fallback. |
| `D` | Deferred: the ABI or implementation is not enabled and must not be inferred from a lower layer. |

`R` never means production-ready in general. It means only that the stated
bounded contract has the stated evidence. Real-device `U` is intentional: no
physical platform is in the current support contract, and a matching PCI,
UART, framebuffer, or firmware implementation must not be assumed to work.

## Target and console profiles

The graphical profile adds a firmware framebuffer or QEMU display frontend;
serial remains the authoritative diagnostic and failure channel in both
profiles. A graphical profile does not provide a framebuffer keyboard shell,
general mode switching, or a graphical login ABI.

| Target/profile | staged | QEMU | real-device | Required boundary |
| --- | --- | --- | --- | --- |
| x86_64 / serial | `U` | `U` | `U` | Retained historical code; native x86 builds and CI are retired. |
| x86_64 / graphical | `U` | `U` | `U` | Retained historical code; native x86 builds and CI are retired. |
| aarch64 / serial | `R` | `R` | `U` | GRUB EFI, DTB hand-off, PL011, serial debugger. |
| aarch64 / graphical | `P` | `P` | `U` | Optional UEFI GOP/ramfb; serial fallback remains valid when no framebuffer is supplied. |

`QEMU` evidence uses the direct ARM EFI serial smoke (`scripts/arm-efi-smoke.py`).
The optional `scripts/run.sh aarch64` launcher requires GRUB. Native x86 is
outside the release target matrix; foreign binary support needs separate
`WLI` backend evidence.

## ABI and boundary matrix

The rows include current versioned contracts, stable internal boundaries that
drivers or userspace consume, and the missing prerequisites whose absence
must remain visible. The source column is the contract owner; the evidence
column names the existing self-check or smoke entrypoint rather than claiming
that an external repository is part of this kernel.

| ABI or boundary | Source / owner | x86_64 serial | x86_64 graphical | aarch64 serial | aarch64 graphical | Evidence / limitation |
| --- | --- | --- | --- | --- | --- | --- |
| Boot hand-off: Multiboot2 / UEFI + DTB | `src/boot.rs`, `src/arch/` | `U/U/U` | `U/U/U` | `R/R/U` | `R/R/U` | Boot parser self-check; `scripts/run.sh`; malformed tags, FDT, EFI data, and framebuffer ranges fail closed. |
| MMIO, PIO, DMA resource boundary | `src/io.rs`, `src/drivers/framework.rs` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | `framework::contract_self_check`; no IOMMU, device-scoped allocator, or real-device cache contract. |
| IRQ registration and deferred-work boundary | `src/irq.rs`, `src/drivers/framework.rs` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | `NORX_DRIVER_IRQ_CONTRACT_OK`; shared IRQ, MSI/MSI-X, threaded IRQ, and per-CPU ownership are deferred. |
| Fixed-capacity driver lifecycle | `src/drivers/framework.rs` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | `matrix_self_check`; probe failure, absent, timeout, malformed, DMA failure, hot-unplug, and interrupt-storm cases. |
| Syscall ABI v2 | `src/syscall.rs`, `docs/syscall-abi.md` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | x86_64 `syscall/sysretq`, AArch64 SVC, six 64-bit arguments, negative errno, bounded usercopy; no glibc ABI. |
| Native userspace C/Rust ABI | `docs/userspace-abi.md`, `src/user_runtime.rs` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | Static Norx runtime and bounded external fixtures; pointer I/O, full threads, and dynamic execution remain limited. |
| Process, FD, credentials, exit/wait | `src/process.rs`, `docs/process-model.md` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | Staged process/user-entry checks and QEMU fixtures; fork/clone, timer preemption, and complete blocking are deferred. |
| Address-space and mapping contract | `src/address_space.rs`, `docs/address-space.md` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | W^X, guard stack, bounds, teardown, and architecture boundary checks; general user mapping syscalls are deferred. |
| ELF64 load-plan ABI | `src/elf.rs`, `docs/elf-loader.md` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | Static ELF fixtures and malformed-image rejection; actual arbitrary guest execution is not implied by parsing. |
| `exec` replacement transaction | `src/exec.rs`, `docs/exec.md` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | Commit/rollback, interpreter selection, close-on-exec, and teardown checks; old runtime remains until commit. |
| Staged dynamic linking / TLS metadata | `src/dynamic.rs`, `docs/dynamic-linking.md` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | One-library metadata/relocation smoke; shared-object mapping, loader execution, and glibc compatibility are deferred. |
| Serial terminal ABI v1 | `src/tty.rs`, `docs/terminal-abi.md` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | One controlling serial TTY, process groups, bounded window metadata; no framebuffer keyboard shell or general PTY. |
| Serial debugger protocol v1 | `src/serial_debugger.rs` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | Versioned ASCII command/diagnostic stream; serial remains authoritative even in graphical profiles. |
| Login/session ABI | `docs/login-abi.md`, syscall extensions 410–411 | `U/U/U` | `U/U/U` | `P/D/U` | `P/D/U` | Fixed `SessionSpec` and credential checks are documented; login consumers and real authentication are separate gates. |
| Userdb ABI v1 | `docs/userdb-abi.md` | `U/U/U` | `U/U/U` | `P/D/U` | `P/D/U` | Header/staged contract only; no kernel owner-aware VFS or credential-handle transfer. |
| Kernel resource capability handles v1 | `src/capability.rs`, `docs/security-boundary.md` | `U/U/U` | `U/U/U` | `P/D/U` | `P/D/U` | Generation-checked opaque handles and rights are staged; public transfer syscalls and a complete service ABI are not enabled. |
| Wait queue / blocking boundary | `src/wait.rs`, `src/sync.rs` | `U/U/U` | `U/U/U` | `P/D/U` | `P/D/U` | Bounded owner/token/deadline state exists; scheduler parking, futex syscalls, and full cancellation integration remain deferred. |
| Signal model | `src/signal.rs`, `src/process.rs` | `U/U/U` | `U/U/U` | `P/D/U` | `P/D/U` | Masks, dispositions, pending state, and frame tokens are bounded; user-stack delivery and complete restart semantics remain deferred. |
| VFS, file handles, mount namespace | `src/vfs.rs`, syscall v2 | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | Ramfs and bounded persistent readers; process-attached namespaces, symlinks, and complete POSIX filesystem semantics are deferred. |
| Block and filesystem adapter boundary | `src/drivers/block.rs`, `src/fat32.rs`, `src/ext4.rs`, `src/btrfs.rs` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | RAM block path on both targets; x86_64 virtio-blk/FAT32 persistence is a bounded optional vertical. |
| devfs/sysfs ABI v1 | `src/devicefs.rs` | `U/U/U` | `U/U/U` | `P/D/U` | `P/D/U` | Bounded virtual device registry and hotplug event records are staged; mount/syscall publication and package-independent `/dev`/`/sys` are not enabled. |
| Display mode contract v1 | `src/drivers/display.rs`, `docs/integration-smoke.md` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | Firmware-fixed guest mode; host scaling is external; EDID, hotplug, and controller-owned mode changes are deferred. |
| Driver-service hand-off | `src/service.rs`, `docs/service-boundary.md` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | x86_64 xHCI lifecycle is the first bounded service path; AArch64 validates user entry and explicitly gates xHCI. |
| IPC channels, rings, events | `src/ipc.rs`, `docs/ipc.md` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | Fixed-capacity queue, ownership, close, revoke, and blocked-result self-checks; public IPC syscalls/capability grants are deferred. |
| Security and capability boundary | `docs/security-boundary.md`, `src/process.rs` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | Kernel-side pointer/ID/path/credential checks; transferable resource handles and complete actor-aware VFS authorization are not complete. |
| Norx Wasm module profile v1 | `src/wasm.rs`, `docs/vm-format.md` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | Interpreter, fuel, bounds, cancellation, and deterministic trap checks; JIT, general WASI, and raw pointers are unsupported. |
| Network diagnostic API | `src/net.rs`, `src/drivers/network.rs` | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | x86_64 bounded virtio-net/QEMU user-net diagnostic path; no stable userspace socket ABI, and no AArch64 network driver. |
| Audio/PCM diagnostic boundary | `src/drivers/audio.rs` | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | x86_64 AC'97 bounded PCM probe; no stable userspace PCM ABI and no AArch64 audio driver. |
| Userdb storage envelope v1 | `src/userdb.rs`, `docs/userdb-abi.md` | `U/U/U` | `U/U/U` | `P/D/U` | `P/D/U` | Fixed kernel-owned paths, owner tuple, generation, nonce, MAC, and recovery promotion are staged; persistent QEMU integration is separate. |
| Package metadata/generation v1 | `src/package.rs` | `U/U/U` | `U/U/U` | `P/D/U` | `P/D/U` | Bounded signed metadata, artifact digest, prepare/commit/rollback state; no production package-manager or real-media backend. |
| Warm-reboot and panic records | `src/crash.rs`, `docs/warm-reboot.md` | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | EFI variables on both paths and x86 CMOS fallback; power-loss durability and arbitrary persistent namespaces are not promised. |
| Recovery/watchdog/core-dump records v1 | `src/recovery.rs` | `U/U/U` | `U/U/U` | `P/D/U` | `P/D/U` | Separate bounded recovery journal, watchdog tokens, core metadata, and package rollback records; durable target evidence is not enabled. |

### ABI prerequisites not yet published

These are intentionally in the matrix even though they have no public header
or syscall number. A lower-level self-check must not be read as evidence that
one of these boundaries exists.

| Missing ABI | staged | QEMU | real-device | Required before |
| --- | --- | --- | --- | --- |
| Signals, masks, dispositions, delivery frames, restart, process groups | `D` | `D` | `U` | POSIX process control and shell job control. |
| PTY, controlling terminal, `termios`, window size, foreground group | `D` | `D` | `U` | Interactive shell and graphical/session login. |
| Monotonic/realtime clocks, correction, timezone ownership | `D` | `D` | `U` | Timers, login policy, TLS, and reproducible builds. |
| Process mapping/heap syscalls with guard, W^X, ASLR, COW, quota, unmap | `D` | `D` | `U` | General user processes and dynamic runtimes. |
| Stable userspace sockets, routing, DNS, and network management | `D` | `D` | `U` | Network applications and daemons. |

## Driver matrix

This inventory is derived from every module under `src/drivers/`, including
nested serial and USB modules. Architecture-specific `cfg` boundaries are
listed as `U` instead of being omitted. The display, block, and serial rows
include their fallback behavior so an absent optional device cannot be
mistaken for a supported replacement.

| Driver / module | Bus or resource | x86_64 serial | x86_64 graphical | aarch64 serial | aarch64 graphical | Owner, failure, and evidence |
| --- | --- | --- | --- | --- | --- | --- |
| `framework` | platform/PCI/virtio/USB/PS/2 resources | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | Fixed-capacity lifecycle, resource validation, IRQ/DMA ownership, trace ring; `src/drivers/framework.rs::matrix_self_check`. |
| `drivers` init/orchestrator | platform driver registry | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | `src/drivers/mod.rs` binds probe results to named fallback states; registration failure is returned to the boot caller. |
| `serial` common service | platform UART failure latch | `U/U/U` | `U/U/U` | `R/R/U` | `R/R/U` | `src/drivers/serial/mod.rs` provides the shared `SerialConfig`, bounded I/O, and one-way failure state around the selected backend. |
| `serial::ns16550` | x86 PIO COM1 | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | Scratch-register probe and bounded 4096-poll TX; ARM does not compile or claim this backend. |
| `serial::pl011` | AArch64 MMIO PL011 | `U/U/U` | `U/U/U` | `R/R/U` | `R/R/U` | Fixed QEMU `virt` base, bounded TX failure latch; DTB resource discovery and physical hardware are not claimed. |
| `display` firmware framebuffer | boot-owned framebuffer | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | `RawFramebuffer` remains boot-owned; absent framebuffer leaves serial active; no mode change/hotplug. |
| `display` `virtio-gpu` | x86 PCI MMIO/PIO + DMA | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | Control-queue diagnostics only. Raw firmware framebuffer scanout fails closed with `DmaUnavailable`: its physical memory lacks allocator-issued DMA ownership. An owned scanout buffer and refresh/drain lifetime are prerequisites; software framebuffer fallback remains available. No 3D or ARM backend. |
| `block` RAM disk | kernel memory | `U/U/U` | `U/U/U` | `P/P/U` | `P/P/U` | Volatile 512-byte-sector backend, fixed queue and partition view; it is never a real-media claim. |
| `virtio_blk` | x86 modern PCI + split-queue DMA | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | Optional VERSION_1/FLUSH backend with bounded polling; QEMU `QEMU_BLOCK_IMAGE` persistence only. |
| `pci` | x86 PCI config mechanism #1 | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | Bus-0 class/vendor scan and BAR validation; no AArch64 PCI host-controller resource path. |
| `ps2` controller | x86 PIO `0x60/0x64` | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | Bounded command/ACK/RESEND, IRQ ownership, and raw queues; absent ARM controller is explicitly unsupported. |
| `keyboard` | PS/2 raw queue | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | Scan-set-2 parser, layouts, repeat policy, bounded input queue; no ARM PS/2 path. |
| `mouse` | PS/2 raw queue | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | Bounded packet parser and pointer events; no ARM PS/2 path. |
| `usb::xhci` | x86 PCI MMIO + DMA/IRQ | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | Reset, rings, bounded enumeration, disconnect, and service lifecycle; ARM host controller is explicitly unsupported. |
| `usb::hid` | xHCI interrupt endpoint | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | Keyboard/mouse report parser and descriptor limits; only the x86 xHCI path feeds it. |
| `audio` AC'97 | x86 PCI PIO + DMA | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | Bounded stereo S16/48 kHz PCM ring and silent fallback; no ARM backend or stable PCM userspace ABI. |
| `network` virtio-net | x86 legacy PCI PIO + DMA/IRQ | `U/U/U` | `U/U/U` | `U/U/U` | `U/U/U` | Bounded Ethernet/IPv4/ARP/UDP/TCP diagnostic path; no ARM driver, IPv6, or stable socket ABI. |

## Verification entrypoints and ownership rules

- **Staged:** `cargo build --target aarch64-unknown-uefi`, strict ARM clippy,
  and the bounded contract self-checks reached during ARM initialization.
- **QEMU:** `scripts/arm-efi-smoke.py --log build/arm-efi.serial.log`; ARM serial
  assertions define its bounded pass. Graphical and persistence gates remain open.
- **Serial evidence:** normalized serial logs are authoritative for pass/fail;
  QEMU screenshots and host-window size are visual artifacts only.
- **Real-device:** no command is listed because the support state is `U` for
  every current row. A real-device entry may change only after a resource
  description, failure path, architecture build, and hardware smoke are
  added for the exact board/controller; it must not be inferred from QEMU.

The invariant is monotonic ownership: boot owns hand-off memory, each driver
owns its registers/queues/DMA until quiesce and release, the framework owns
resource state, and the ABI owner validates all user-visible lengths, IDs,
paths, and generations. Optional hardware reports `U`, `D`, or `P` through
the typed path and keeps the named fallback; it never becomes a fake ready
device.
