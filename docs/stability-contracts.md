# Norx stability contracts

This document is the release-gate index for the current bring-up. A row is
considered stable only for the scope stated here; “stable” means that ownership
and failure behavior are explicit and exercised by the bounded contract or
QEMU smoke, not that the subsystem is production-ready.

## Contract vocabulary

- **Owner** names the subsystem that allocates, publishes, and releases the
  state. A consumer may borrow a descriptor or resource only through that
  owner’s typed interface.
- **Failure** names the observable result for the current path. Optional
  hardware must become `unsupported`, `deferred`, `busy`, or `failed` rather
  than silently becoming a fake device or stopping boot.
- **ABI** includes wire, file, register, or kernel-facing contracts. Internal
  Rust types are not promised to userspace unless the row says they are
  versioned.
- **Hardware** is the tested QEMU or firmware scope. It is not a claim about
  every machine that happens to implement the same standard.

## Release-gate matrix

| Domain | Owner and lifetime | Failure behavior | ABI / contract | Supported hardware and smoke | Explicitly unsupported or deferred |
| --- | --- | --- | --- | --- | --- |
| Boot hand-off | `boot` owns the copied `BootInfo`; reserved ranges remain reserved for the kernel lifetime. | Malformed Multiboot2/FDT, invalid EFI data, or an invalid framebuffer descriptor halts before normal initialization. | x86_64 GRUB Multiboot2; aarch64 GRUB EFI plus DTB; framebuffer geometry and overflow rules are typed. | QEMU `q35`/OVMF and `virt`/AArch64 UEFI. | Runtime mode switching, hotplug, and arbitrary bootloader extensions are not promised. |
| Logging and console | `drivers::serial` owns UART I/O; `log` fans output to serial, VGA, and an optional firmware framebuffer. | Init failure or a transmit timeout marks the UART failed permanently for this boot; each transmit waits at most 4096 status polls, and later writes return immediately while fallback sinks continue. | ASCII serial stream; `NORX_EARLY_UART_READY v=1` and `NORX_EARLY_UART_DISABLED v=1 reason=init|tx-timeout` plus `NORX_SERIAL_DEBUGGER_READY v=1` are smoke interfaces. | x86_64 NS16550 COM1 (including scratch-register presence probe), x86 VGA/virtio-GOP, and aarch64 PL011 on QEMU `virt`. | AArch64 uses the fixed QEMU `virt` PL011 address; arbitrary MMIO absence cannot be safely probed until DTB resource ownership exists. No framebuffer keyboard shell; ARM QEMU may have no GOP and therefore uses serial-only fallback. |
| MMIO, PIO, and DMA | `io` validates ranges; the driver framework owns registered resources and DMA direction/owner metadata. | Zero, overflow, alignment, range, and invalid DMA ownership return `false` or a typed driver error before dereference. | Width-specific little-endian MMIO/PIO methods and `DmaBuffer` ownership contract. | x86 PIO and PCI MMIO; MMIO/DMA boundary checks on both targets. | IOMMU isolation, device-specific cache coherency, and arbitrary physical mappings are not implemented. |
| Memory and paging | `memory` owns the static physical range table and frame allocation; architecture paging owns page tables and mappings. | Exhaustion returns `None` or a boot failure; invalid mappings, W+X, and fault addresses are rejected. x86_64 maps the complete allocator-visible physical limit through the direct map before lazy faults are enabled; failed leaf installation returns the frame. | Internal page/address types; no userspace mapping ABI is frozen beyond the documented address-space contract. | x86_64 direct map, CR3 switching, real fault-entry recovery, volatile lazy-page touch, PTE inspection, and boundary checks; aarch64 table setup and TTBR0 boundary checks. | aarch64 lazy fault mapping, general mapping teardown, ASLR entropy, and IOMMU-backed allocation remain deferred. |
| IRQ, timer, and scheduler | Architecture code owns interrupt delivery; each registration has an opaque generation-bearing owner handle; deferred work is drained by the normal-context poller; scheduler owns task accounting. | Unavailable hardware timer falls back to bounded polling; duplicate ownership, stale handles, hard-context registration, reentrant deferred calls, and interrupt storms are rejected or accounted. | `RegistrationId`, `IrqOwner`, `Resource::Irq.registration`, bounded hard callbacks, coalesced deferred callbacks, and teardown-before-resource-release. No driver may run unbounded hard-IRQ work or mutate shared runtime state from hard context. | x86 APIC/HPET and legacy PIT fallback inspection under QEMU; aarch64 GICv2/v3 discovery, generic-timer PPI ACK/EOI, 100 Hz accounting, and `NORX_DRIVER_IRQ_CONTRACT_OK v=1` under `virt`. | Shared IRQ, MSI/MSI-X routing, threaded IRQs, per-CPU ownership, and high-throughput queues are deferred. |
| Driver framework | `framework` owns `Bus`, `Device`, `Driver`, `Resource`, IRQ, and DMA lifetimes from probe through remove. | Probe can report `deferred`, `unsupported`, `busy`, or `failed`; cleanup is required on failed probe and teardown. | Fixed-capacity typed framework contract and serial trace rows; resource ownership is explicit. | QEMU PCI/virtio/xHCI matrix on x86_64; absence/failure matrix on both targets. | Dynamic allocation, arbitrary rebind concurrency, and hardware-specific drivers outside the listed QEMU devices are not promised. |
| UART and PS/2 input | UART drivers own register programming; PS/2 controller serializes commands and publishes keyboard/mouse events. | Bounded ACK/RESEND and controller retries return timeout/failure; absent PS/2 is unsupported without stopping boot. | NS16550, PL011, scan-set-2, and device-independent input event contracts. | x86_64 QEMU COM1 and PS/2 keyboard/mouse; aarch64 PL011 early console. | PS/2 on aarch64, arbitrary keyboard layouts beyond the current set, and hotplug policy are deferred/unsupported. |
| USB | xHCI owns controller rings and endpoint DMA; the service supervisor owns the risky runtime endpoint after hand-off. | Reset/enumeration/transfer timeouts and malformed descriptors fail the device and revoke resources. | Bounded host-controller, USB descriptor, HID report, and class-transfer contracts. | QEMU xHCI with HID, hub, FTDI-compatible serial, and mass-storage fixtures on x86_64. | EHCI/OHCI, xHCI on the current aarch64 bring-up, isochronous audio, and general userspace USB ABI are deferred. |
| Display | Boot `RawFramebuffer` is reserved by `boot`; `display` owns the guest mode/status/damage state and never frees firmware memory. | Invalid mode or damage returns a typed error; a scanout smaller than the guest framebuffer is rejected instead of cropped or scaled; absent framebuffer leaves serial active; unsupported EDID/hotplug is explicit. | Versioned `NORX_DISPLAY_MODE_CONTRACT_OK v=1`: guest mode is the firmware `RawFramebuffer` geometry, `ModePolicy::FirmwareFixed` is the current policy, host-window scaling is external presentation, and virtio scanout geometry is separate. Future controller changes must be transactional. | Firmware framebuffer and basic 2D virtio-gpu on x86_64 QEMU with `virtio-vga`; the smoke records guest and scanout geometry separately. | 3D acceleration, firmware mode changes, EDID parsing, hotplug events, aarch64 virtio-gpu, and controller-owned mode transactions are deferred. |
| Block and filesystems | Block layer owns request buffers/queue completion; each filesystem owns its parsed superblock, mounts, and file handles. | Invalid request, timeout, read-only write, malformed metadata, and unsupported feature return typed errors; absent volumes fall back to ramfs. | Sector geometry, `RequestId<'a>`, partition, cache, VFS mount, and read-only filesystem contracts. | RAM block smoke, ramfs, FAT32/ext4/btrfs fixtures on both targets. | The RAM disk is not persistent; FAT32/ext4/btrfs writes, journaling recovery, and production block devices are deferred. |
| VFS and file handles | VFS owns mount tree, namespace, dentry/inode references, file offsets, and unmount validation; process tables own bounded descriptor references. | Invalid/generation-stale handles, permissions, type-aware path traversal, read-only mounts, mountpoint mutation, duplicate targets, and unsafe unmounts return errors without leaked references. | Internal VFS API plus bounded process-local `open`/`pipe`/`dup2`/`close`/`mkdir`/`rmdir`/`unlink`/`rename`/`link`/`stat`/`read_dir`; `/hello.txt` and `/lib` remain fixture paths, not a host-filesystem ABI. | In-kernel ramfs with bounded mount, namespace, descriptor, metadata, hard-link, and directory lifecycle smoke; CI asserts `/hello.txt` readback on x86_64 and aarch64. | Process-attached mount namespaces, persistent storage namespaces, symlink nodes, and a complete POSIX filesystem ABI are deferred. |
| Networking and audio | PCI/virtio or AC'97 drivers own DMA rings; network/audio state is published through typed status and bounded queues. | Absent, malformed, or unsupported device returns deferred/unsupported; link, DMA, underrun, and queue failures are recoverable. | Ethernet/ARP/IPv4/UDP/TCP diagnostic contract and PCM ring contract; no stable socket ABI yet. | x86_64 QEMU legacy virtio-net and AC'97 fixtures; both have failure-path checks. | aarch64 network/audio drivers, IPv6, full DHCP/DNS policy, and userspace sockets/PCM are deferred. |
| Syscalls and usercopy | Syscall entry owns register decoding; `usercopy` validates ranges; process/address-space code owns user mappings. | Unknown calls return `-ENOSYS`; invalid pointers/ranges return negative errno; no unchecked user pointer is dereferenced. | Versioned ABI v2, Linux-shaped numbers, six-word args, 64-bit types, negative errno returns, bounded stdio/filesystem records, and process-local descriptors. | x86_64 `syscall/sysret` and aarch64 SVC boundary, including real user-buffer console/file I/O and bounded argv/env, self-check under graphical QEMU. | Unbounded streams, symlink/locale compatibility, and a glibc ABI are deferred. |
| Processes and services | Process table owns PID/TID, FD, credentials, wait records, and task state; service supervisor owns restart/revoke lifecycle. | Failed prepare rolls back; exit publishes one wait record; crash/restart revokes resources before re-entry. | Fixed-capacity process, FD, capability, IPC, and service contracts; no arbitrary guest execution path. | Staged kernel-runtime and user-entry checks on x86_64/aarch64 QEMU. | Full isolated process creation, scheduler preemption, capability transfer to userspace, and service IPC grants remain deferred. |
| ELF, exec, and dynamic loading | Loader owns validated image plan and mapped pages until commit; `exec` keeps the old runtime until replacement succeeds. | Truncation, bad headers, overlap, W+X, invalid entry, dependency, or relocation errors roll back without partial publication. | ELF64 `PT_LOAD`/`PT_INTERP`/`PT_DYNAMIC`, auxv, stack/register image, and staged ET_DYN contract. | Static ELF fixtures and one-library dynamic smoke on both targets. | Full native user instruction execution, TLS ABI compatibility, arbitrary shared libraries, and glibc compatibility are deferred. |
| Wasm VM | Verifier owns module validation; interpreter owns linear memory, fuel, handles, and cancellation for each run. | Invalid control flow, bounds, stack/fuel, import, handle, or cancellation returns a deterministic trap and releases run state. | Versioned Norx VM/Wasm module format documented in `vm-format.md`; host calls use bounded handles. | Interpreter and portable sample self-check on both QEMU targets. | JIT, arbitrary WASI, raw kernel pointers, and unbounded host imports are unsupported. |
| IPC and security boundary | IPC objects own queues/rings and waiter references; capability code owns authority transfer; namespaces own mount visibility. | Full/empty/closed/revoked transitions and unauthorized operations return typed errors; blocking is bounded and non-spinning. | Fixed-capacity channel, shared-memory ring, event queue, capability, credential, and namespace contracts. | Boot self-checks and staged service lifecycle on both targets. | Cross-process capability grants, persistent shared-memory ABI, and signal-first IPC are deferred. |
| Warm reboot and panic context | `crash` owns the four-record ring and checksum validation; EFI variables or x86 CMOS are backing stores; serial is fallback. | Corrupt records are ignored; failed durable writes are reported and panic context remains serial-only; panic always halts. | 64-byte record format, states `BOOTING/READY/PANIC/CHECKPOINT`, and four-slot sequence ordering. | OVMF/UEFI variable persistence across QEMU process restart; x86 CMOS fallback. | RAM disk persistence, cross-firmware variable migration, and guaranteed persistence after power loss are not promised. |

## Cross-cutting release rules

1. A caller must have one named owner for every resource, buffer, mapping,
   queue entry, and reference. If ownership cannot be stated, the interface
   stays internal and is not promoted to a stable ABI.
2. Optional hardware must be probed through a bounded path. Absence is a
   normal result and must leave the fallback named in the matrix usable.
3. Every public failure result is checked by its caller. A failed probe or
   partial commit may not publish a device, mapping, mount, process, or
   service endpoint.
4. The boot contract suite is the common integration gate. Its serial markers
   are asserted from normalized logs; framebuffer screenshots are retained
   for visual review and never replace serial failure evidence.
5. Any feature not listed under a row's supported scope is intentionally
   unsupported until it gets a typed contract, a failure test, and a QEMU or
   hardware-specific smoke path.

## Verification references

- [driver audit and ownership ledger](driver-audit.md)
- [syscall ABI](syscall-abi.md) and [userspace ABI](userspace-abi.md)
- [process model](process-model.md), [IPC](ipc.md), and [security boundary](security-boundary.md)
- [ELF loader](elf-loader.md), [exec](exec.md), and [dynamic linking](dynamic-linking.md)
- [integration smoke, negative boundaries, and artifacts](integration-smoke.md)
- [warm-reboot persistence and panic context](warm-reboot.md)

The matrix is updated whenever a subsystem changes ownership, exposes a new
ABI, gains hardware coverage, or changes an unsupported-case policy. A roadmap
checkbox must not be marked complete solely because the happy path boots.
