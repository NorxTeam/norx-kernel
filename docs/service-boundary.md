# Userspace driver-service boundary

The risky-driver migration is gated until three prerequisites are real, not
just named:

1. A user process owns an address space, has an architecture context switch,
   and can execute a bounded service loop.
2. The driver framework owns DMA buffers and IRQ/MMIO resources with explicit
   revoke/quiesce completion; no device resource is handed out as a raw value.
3. A supervisor owns service lifecycle, restart budget, crash detection,
   endpoint revocation, and rebind/teardown ordering.

## Intended first migration

The first candidates are restartable or failure-prone runtime drivers:

| Service | Kernel keeps | Service receives |
| --- | --- | --- |
| xHCI/HID | PCI discovery, IOMMU/DMA validation, IRQ routing, quiesce/revoke | endpoint and transfer-ring handles |
| virtio-net | PCI transport, DMA ownership, link reset | RX/TX rings and network capability |
| AC'97 audio | PCI/MMIO and DMA ownership | PCM ring and stream-control capability |
| block/storage | partition/media discovery and request broker | bounded block queue and sector capability |

Serial, early framebuffer, page-table, interrupt, and supervisor plumbing stay
kernel-owned during the first migration because they are boot or isolation
dependencies. A service cannot retain a resource after `quiesce`; the
supervisor waits for in-flight completions, revokes the IPC/DMA handles, then
unbinds and releases the framework resources before restart.

## Current status

The framework already has typed device states, resource ownership checks,
DMA-direction validation, bounded callbacks, quiesce/remove transitions, a
trace ring, and a fail-closed `revoke` path for failed removal. The staged
supervisor now validates a non-init user thread, owns a generation-checked
service handle, stops the process, detects an already-exited service, applies
a fixed restart budget, and verifies that DMA/MMIO resources are gone even
when the driver removal callback fails. The IPC layer supplies bounded queues
and waiters.

The bounded native service image now has a real instruction/context-switch
entry and syscall exit on both x86_64 and AArch64. On x86_64, the xHCI adapter
registers its real MMIO and DMA lease with the framework, runs the complete
supervisor lifecycle in QEMU (start, quiesce, stop/revoke, clean restart,
crash recovery/revoke, and final restart), and leaves the controller running
under the supervisor. AArch64 validates the same user-entry boundary and
correctly gates this adapter because the platform has no xHCI controller.

The bounded service payload currently exits after proving the transition; it
does not yet perform USB transfer I/O from userspace or receive raw
capability-handle syscalls. The xHCI transfer engine therefore remains a
kernel runtime behind the service lifecycle until that separate ABI pass is
implemented. The handoff item is checked for this first bounded migration;
direct user-driver I/O remains an explicit follow-up rather than an implied
completion.
