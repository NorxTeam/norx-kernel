# Universal Driver Framework: deep audit

Status: baseline audit and implementation follow-ups complete for the current
`0.1` tree, 2026-08-07.

This is a static audit of the driver, bus-adjacent, MMIO/PIO, interrupt,
DMA, global-state, unsafe-boundary, timeout, capability, and test-hook paths.
The inventory and gap statements below are the baseline captured before the
framework and driver passes. The follow-up sections record the implementation
that now exists and its verification evidence.

## Current boot path

The current path is a fixed bring-up sequence, not discovery:

1. `arch::init` initializes a hard-coded early UART and disables interrupts.
2. `log` selects VGA and/or the bootloader framebuffer.
3. `time::init` reads the architecture counter.
4. `drivers::init` clears a metadata array, reports serial/clock/framebuffer,
   initializes the fixed RAM block layer, and reports its geometry and queue.
5. `vfs::init` mounts ramfs and writes `/hello.txt` through a file handle.
6. Memory, page tables, architecture exception tables, interrupt controller,
   syscall entry, scheduler, and timer are initialized.
7. The serial debugger becomes the only runtime input path.

Evidence: `src/main.rs:28-133`, `src/arch/x86_64.rs:10-13`,
`src/arch/aarch64.rs:9-17`, and `src/drivers/mod.rs:5-27`.

There is no bus enumeration, matching, resource allocator, probe/remove
lifecycle, interrupt registration API, DMA API, or driver-owned error type.
`src/drivers/framework.rs` is a bounded metadata list only.

## Inventory and ownership ledger

| Component | Resource and current owner | Current consumers | Lifetime and gap |
| --- | --- | --- | --- |
| x86 NS16550 | COM1 PIO ports `0x3f8..0x3ff`; raw port access is in `arch` | boot logging, panic logging, serial debugger | Initialized by `arch::init` with a scratch-register presence probe; never released or represented as a resource. |
| aarch64 PL011 | MMIO base `0x0900_0000`; raw register access is in `drivers::serial::pl011` | boot logging, panic logging, serial debugger | Initialized by `arch::init`; address is assumed, not obtained from the DTB, and there is no absent-device state. |
| NS16550 MMIO | `0x1000_0000` helper in `ns16550` | none on supported targets | RISC-V-only dead surface in this tree; no RISC-V target or caller exists. Remove until RISC-V is supported, or retain only with a real platform resource. |
| VGA fallback | Physical text memory `0xb8000`, writer state in `vga::STATE` | `log` during early x86 boot | `log::init` enables it and `log::init_framebuffer` disables it. No ownership or presence probe exists; x86-only fallback is intentionally retained. |
| Firmware framebuffer | Bootloader-provided pointer, size, stride, format; reservation is recorded by `boot` | `framebuffer`, `log::CONSOLE` | Boot parsing validates geometry and reserves the range; console copies the descriptor and never releases it. No mode change, hotplug, or cache/flush contract exists. |
| Clock counter | `rdtsc` on x86; `cntvct_el0`/`cntfrq_el0` on aarch64 | `time`, scheduler, diagnostics | Counter access is architecture-owned. Frequency is unknown on x86; aarch64 programs the virtual counter as a GIC timer PPI at the scheduler rate after DTB discovery. |
| x86 PIC/PIT | PIC ports `0x20/0x21/0xa0/0xa1`, PIT ports `0x40/0x43` | timer fallback and legacy IRQ EOI | `arch::x86_64` owns the remapped fallback path; APIC/HPET is preferred when calibration succeeds. Both routes use the shared IRQ contract. |
| x86 IDT/GDT/TSS | Static IDT, GDT, TSS, and 16 KiB kernel stack | exception and timer handlers, syscall stack setup | `arch::x86_64::tables` owns all storage for the current boot. It is single-CPU static state; per-CPU and teardown semantics are absent. |
| x86 local APIC | APIC MSR and physical MMIO base from `IA32_APIC_BASE` | calibrated timer interrupt path and status command | `arch::x86_64` owns APIC/HPET calibration, vector routing, EOI, and the PIT fallback decision. The MMIO pointer has no explicit mapping/resource validation. |
| aarch64 exception vectors/GIC | Static vector table, EL-specific system registers, and DTB-discovered GICv2/v3 | synchronous syscall/fault entry, IRQ acknowledgement, and generic timer PPI | `arch::aarch64::tables` installs VBAR and uses a dedicated kernel stack; `arch::aarch64::gic` owns distributor/CPU-interface or redistributor setup, ACK/EOI, and timer enable. |
| Block layer | `static mut RAMDISK` fallback plus optional x86_64 modern PCI virtio-blk, fixed request queue | `vfs`, serial `block` diagnostic, FAT32 persistence smoke | `drivers::block` owns geometry, request lifetime, bounded completion, partition discovery, read-only state, backend selection, and write-through cache policy. The RAM backend is volatile; virtio-blk owns its bounded DMA queue and optional flush boundary. |
| Driver registry | Static 32-entry metadata array and length | boot initialization and `drivers` debugger command | `framework` owns storage, but callers ignore the `bool` result. Registration is only a status display and does not own hardware or bind a driver. |
| Physical frames | Static range table and monotonic allocator | x86 paging lazy mapper | `memory` owns allocation; paging returns a frame when a lazy leaf cannot be installed. This is an adjacent memory contract, not a device DMA allocator. |
| Direct map/page tables | x86 static table pool and page-table entries | paging and lazy fault path | `arch::x86_64::paging` owns the pool and mappings. The current interface exposes raw pointers/physical addresses and has no mapping lifetime or concurrency contract. |

The ownership conclusion is deliberately simple: every current “driver” is
kernel-global early-boot state. No resource is shared through a typed handle,
and no caller can ask a driver to quiesce, remove, or rebind.

## Required audit categories

### Drivers and bus helpers

Existing driver implementations are limited to:

- NS16550 PIO on x86 and an unused NS16550 MMIO helper behind the RISC-V cfg;
- PL011 MMIO on aarch64;
- VGA text output and the firmware framebuffer console;
- a RAM-backed block smoke device;
- metadata for serial, clock, display, and block classes.

There are no PCI, virtio, USB, PS/2, audio, network, filesystem, hub, or
generic bus helpers in the current tree. No dead implementation is hidden in
the tracked source: the old universal syscall/user smoke paths and shell were
removed in history and are not part of this inventory.

The registry's `Filesystem` class has no implementation or registration. The
current VFS is a caller of the RAM block smoke API, not a filesystem driver.

### MMIO and PIO

- PIO is centralized only at the x86 `inb`/`outb` wrappers, but callers pass
  literal ports and receive no validation or failure result.
- UART MMIO helpers form raw pointers from `base + register` and use volatile
  access without alignment, mapping, endianness, or resource-range checks.
- PL011 assumes the QEMU virt base instead of consuming a DTB-described
  resource.
- APIC MMIO forms a pointer from the physical APIC base without an explicit
  mapping contract.
- VGA and framebuffer pointers originate in boot parsing and are bounded by
  geometry/size checks, but the lifetime is an unchecked copied raw pointer.

The future contract must make address space, register width, endianness,
barriers, and mapping lifetime part of a `Resource`; raw address arithmetic
must remain inside the architecture/bus implementation.

### Interrupt paths

- x86 installs IDT entries for exceptions, spurious IRQs, and vector 32.
  Timer IRQ work increments statistics, charges the scheduler, and sends a
  legacy PIC EOI in the hard handler (`src/arch/x86_64/tables.rs:204-220`).
- x86 selects the calibrated APIC timer when available and keeps the remapped
  PIT/PIC path as a bounded fallback; both paths feed the shared IRQ contract.
- aarch64 installs exception vectors on the dedicated kernel stack, discovers
  GICv2/v3 from the DTB, acknowledges and EOIs interrupt IDs, and programs the
  virtual generic timer PPI. The scheduler runtime smoke requires positive IRQ
  accounting; polling remains the bounded fallback when discovery or setup is
  unavailable.
- The shared registration table provides line/vector ownership, bounded hard
  handlers, deferred callbacks, and storm coalescing. Per-CPU ownership,
  threaded IRQs, and high-throughput driver callbacks remain out of scope.

The scheduler callback is bounded by the current fixed task array and runs from
the deferred normal-context pass; it must not become the model for
high-throughput drivers.

### DMA

No current driver performs DMA, allocates a DMA buffer, programs a descriptor,
or transfers ownership between CPU and device. This is an explicit “not yet
implemented” result, not an implicit safe assumption. A future `DmaBuffer`
must define physical address, alignment, size, cache maintenance, direction,
ownership, and failure/teardown behavior before any PCI/virtio/USB driver is
added.

### Global mutable state

The audit found global mutable state in the driver registry, RAM disk, console,
VGA writer, boot hand-off, memory allocator, page-table pool, scheduler, and
timekeeping. Early initialization happens before interrupts, but runtime reads
and writes do not consistently state that precondition. In particular:

- `framework::list` is used by the runtime debugger while registration has no
  synchronization or duplicate policy;
- `RAMDISK` is accessed through raw pointers with no lock or single-owner
  contract;
- framebuffer and VGA writers use global state and are not safe for concurrent
  logging;
- page-table and frame allocation state is monotonic and not reclaimable;
- the scheduler explicitly disables interrupts around its current state, which
  is the only clear synchronization boundary in this group.

The first framework implementation should document single-CPU/early-boot
constraints rather than pretend these globals are reusable device objects.

### Unsafe casts and hardware boundaries

Unsafe pointer construction is concentrated in boot parsing, framebuffer/VGA,
UART, APIC, paging, and architecture assembly. The boot parser checks input
sizes and overflow in many paths, but `read_u8/u32/u64` are raw unchecked
dereferences by design. The driver-facing gaps are the fixed UART/APIC bases,
unvalidated MMIO mapping, and copied framebuffer pointers. These must be
covered by the resource contract; spreading pointer validation into every
caller would duplicate the same bug surface.

### Fake capabilities and silent errors

- `drivers::init` now registers serial as `Failed` when the early UART latch is
  unavailable, but clock and framebuffer still need richer probe states. A
  missing framebuffer is only diagnosed later in `main`.
- aarch64 reports the clock driver as ready even though its scheduler timer
  returns `false`; the GIC is explicitly absent.
- all four `framework::register` results are ignored, so capacity failure is
  invisible.
- `vfs::init` ignores the initial RAM-disk write result and `main` prints
  “vfs initialized” regardless of that result.
- the block API returns `bool`, which loses the distinction between invalid
  LBA, unavailable device, timeout, and I/O error.
- serial formatting returns failure to the shared writer, which records the
  first init or transmit-timeout reason and suppresses later UART attempts;
  fallback sinks still receive the same log call.

The future state must be derived from probe results and must distinguish at
least `deferred`, `unsupported`, `busy`, and `failed`, as required by the
Roadmap. A metadata entry must never advertise a capability merely because a
boot path attempted to initialize it.

### Busy loops and timeouts

The audit found unbounded UART waits in the x86 NS16550 and PL011 transmit
paths, plus a dead RISC-V NS16550 path. The dead path was removed; both live
transmit paths now use the shared 4096-poll limit, return failure, and the
shared serial layer suppresses repeated writes after the first hardware
failure.
`halt` and the serial-debugger main loop remain intentionally unbounded control
loops and are not device wait bugs.

### Test hooks and observability

- `sched::self_check` is a boot-time assertion for the scheduler model.
- The serial debugger's `crash` command is an intentional panic-path hook;
  `drivers`, `irq`, `hw`, `mem`, and `paging` are diagnostic views.
- The serial contract self-check covers the failure-reason encoding and poll
  bound; `scripts/qemu-boot.ps1 -SerialDevice none` provides a bounded missing
  COM1 run. Other driver-specific fault injection, malformed descriptor, DMA
  failure, and hot-unplug hooks remain absent.
- The CI workflow covers formatting, two builds, two Clippy runs, and GRUB
  image creation, but does not run QEMU smoke assertions or failure matrices.

These hooks are retained as bring-up observability. They must not become the
only validation path for the stable driver contracts.

## Removed, retained, and replaced

### Already removed from the current tree

- Universal architecture-neutral syscall smoke payloads, the old ABI module,
  user stubs, process/capability smoke paths, and their test-only execution
  routes were removed in `9d2dfcf`.
- The framebuffer shell and its broad command surface were replaced by the
  serial debugger in `dbb5646`.
- Earlier accidental complexity and unused infrastructure were removed in
  `2ba24e2`.

No deleted driver needs to be resurrected for this framework. The historical
paths are evidence for the audit only.

### Retain for the first vertical path

- The serial output path, because it is the only reliable early diagnostic
  channel and is required to report probe order and teardown.
- The x86 VGA fallback and firmware framebuffer console, because they are
  boot diagnostics rather than general display-driver promises.
- The fixed RAM block layer, because it is the smallest VFS test path and
  exposes the queue, geometry, and ownership contract for real media later.
- Architecture-local exception and syscall entry code remains separate, while
  the versioned, architecture-neutral syscall contract is documented in
  `docs/syscall-abi.md`.
- The scheduler self-check and serial debugger diagnostics.

### Replace in later framework work

- The metadata-only `Driver` list was replaced by the fixed-capacity typed
  `Bus`, `Device`, `Driver`, `Resource`, `Irq`, `DmaBuffer`, `DeviceState`, and
  error contract in `src/drivers/framework.rs`.
- Hard-coded UART initialization with a resource-backed serial device and
  bounded I/O.
- The bounded RAM fallback with a full persistent media stack once allocation,
  mount publication, recovery, and namespace requirements justify that scope.
- Direct VFS-to-LBA knowledge with the first mountable filesystem vertical
  path.
- Legacy PIT/PIC fallback remains available behind an explicit interrupt/timer
  source contract; x86 APIC/HPET delivery is now the calibrated default.
- Raw framebuffer assumptions with a display contract once mode selection,
  flush, cursor, damage, and hotplug semantics are needed.

## Audit decisions for the next task

1. Keep the fixed-capacity bus/device contract driven by the first real
   PS/2, PCI, or virtio device path; do not add speculative discovery layers.
2. Keep resource ownership and cleanup at the probe boundary. The lifecycle
   implementation releases resources after failed probe and successful remove
   while preserving resources when a remove callback fails.
3. Keep early UART and framebuffer services explicitly separate from runtime
   drivers; missing optional hardware must become a reported state, not a
   panic or an infinite wait.
4. Introduce no DMA or interrupt API as a placeholder. Add each only with a
   concrete consumer and a QEMU failure test.

### Lifecycle follow-up

The framework now distinguishes `early` and `runtime` driver stages. Probe
errors map to `unsupported`, `deferred`, `busy`, or `failed` states instead of
being treated as a panic condition; probe cleanup releases resources for each
non-ready result. The boot-time contract check covers all four outcomes, and
the serial debugger exposes the stage and state in its `drivers` view.

### Low-level access follow-up

Current serial, APIC, and VGA paths use `src/io.rs`: checked MMIO ranges with
volatile little-endian register access and barriers, plus checked x86 PIO
regions. Raw port instructions are private to the x86 architecture wrapper.
`PhysAddr`/`VirtAddr`, direct-map conversion hooks, DMA direction/alignment
validation, and explicit unsafe cache synchronization cover the address and
ownership contract. No current device owns a DMA queue, so cache-sync calls
remain unused until the first concrete DMA driver; page-table writes and FDT
unaligned parsing remain architecture/boot-parser internals.

### IRQ follow-up

IRQ registration now carries an opaque `RegistrationId` containing the slot,
generation, and `IrqOwner`. Reusing a slot cannot invalidate an old handle;
owner-mismatched teardown is rejected. `Resource::Irq.registration` links a
device resource to that handle, and resource release unregisters it before
clearing the device resource table.

The hard callback is bounded and non-blocking: it may acknowledge device state
and publish bounded atomic/queue work, but it must not run deferred work,
register/unregister IRQs, sleep, allocate, log, or touch mutable driver state
that normal-context code is concurrently updating. The dispatcher marks one
pending bit per registration; `run_deferred()` drains one bounded pass, rejects
reentrant calls, and runs callbacks only outside hard-interrupt context. Timer
and driver callbacks are covered by the boot marker
`NORX_DRIVER_IRQ_CONTRACT_OK v=1 ownership=1 deferred=1 callbacks=1 negative=1`.

The required lifecycle is: prepare immutable IRQ state, register the callback,
publish driver state and its `Resource::Irq`, then unmask hardware. Teardown
masks hardware first, drains or rejects pending deferred work, unregisters the
opaque handle, and only then releases driver state. Shared IRQs, MSI/MSI-X
routing, per-CPU ownership, threaded IRQs, and high-throughput queues remain
deferred until their controller contracts exist.

### Runtime tracing follow-up

The framework keeps a bounded 128-event ring buffer for discover/match/probe,
publish/suspend/resume/quiesce/remove/rebind, and resource add/release. Each
event records device/driver identity, state, resource/IRQ/DMA counts, order,
and success. The serial debugger exposes it through `trace`; hard IRQ paths do
not write the buffer.

### UART follow-up

The early serial paths now use a shared 4096-poll bound and report
initialization or transmit failure instead of waiting forever. The x86 NS16550
driver probes its standard scratch register after programming a
validated divisor from the configured clock and baud, enables or disables its
FIFO, and can request the controller's automatic RTS/CTS mode. The aarch64
PL011 driver programs rounded integer and fractional divisors, FIFO mode, and
the PL011 RTS/CTS bits. Both controllers reject zero or unrepresentable clock
and baud configurations before touching hardware.

`SerialConfig` is shared by the two controllers; the x86 `Port` object and
the PL011 `init_with_config` path provide the runtime configuration boundary.
The architecture early-console wrappers use the default configuration and
mark the shared serial service failed after the first bounded I/O failure, so
later logging does not repeatedly spin on a missing or stalled UART. The boot
log emits the versioned `NORX_EARLY_UART_READY` or
`NORX_EARLY_UART_DISABLED` marker after fallback selection. Each architecture
runs a boot-time divisor/FIFO/flow-control contract check before driver
registration.

### PS/2 controller follow-up

The x86-only PS/2 controller driver owns the standard `0x60/0x64` ports and
serializes controller/device commands with a bounded lock and interrupts
disabled during each transaction. Controller commands wait for the input and
output buffer with a finite poll budget. Device commands recognize `ACK` and
`RESEND`, retry `RESEND` at most three times, and return a bounded error to the
diagnostic caller.

Initialization disables both ports, flushes stale output, disables translation
and IRQ bits, validates the controller self-test (`0x55`), tests both ports,
and enables only ports that pass. IRQ1 and IRQ12 are registered through the
shared hard-IRQ boundary; handlers drain at most eight bytes into separate
bounded raw queues, while PIC masks and controller IRQ bits are enabled before
`sti`. A missing or failed controller becomes `unsupported` or `failed` in the
driver view and leaves the serial debugger usable. aarch64 deliberately
registers no PS/2 device and reports the platform as unsupported.

The controller exposes raw queues for the mouse driver without coupling it to
the serial debugger.

### PS/2 keyboard follow-up

The keyboard driver selects scan-code set 2 and enables scanning through the
controller command path. Its bounded poll consumes raw bytes outside hard IRQ
context and translates make/break sequences, `E0` extended keys, and the
multi-byte Pause sequence into physical `KeyCode` values. Modifier and lock
state covers Shift, Ctrl, Alt, GUI, CapsLock, NumLock, and ScrollLock.

Events are delivered through the independent bounded `input` queue as
`KeyEvent { code, pressed, repeat, modifiers, text }`; the serial debugger only
offers a diagnostic drain command. US and Russian layouts are supported, and
the software repeat policy can accept hardware repeats or suppress them. The
keyboard self-check covers make/break, extended keys, Pause, Shift, Russian
text mapping, and repeat suppression.

### PS/2 mouse follow-up

The mouse driver resets to defaults, reads the device ID, negotiates the
sample-rate sequences for wheel and five-button modes, and enables reporting
only after capability selection succeeds. It decodes standard and four-byte
packets with signed movement, wheel direction, left/right/middle buttons, and
optional back/forward buttons. Invalid overflow packets are discarded and the
first-byte bit-3 check resynchronizes after dropped or spurious bytes.

Decoded movement and button transitions are delivered as the same independent
`input::Event::Pointer` stream used by future consumers. Hard IRQ work remains
limited to bounded raw-byte capture; packet decoding and queue publication are
bounded normal-context polling. The mouse self-check covers synchronization,
signed coordinates, button transitions, wheel data, and extra buttons.

### USB xHCI foundation follow-up

The x86 PCI path scans configuration mechanism #1 for an xHCI class device,
validates its 64-bit BAR, reads the capability registers, and performs a
bounded controller halt/reset sequence before setting the slot configuration.
The boot path exposes a small `HostController` contract with controller kind,
capabilities, reset, and connected-port queries; EHCI/OHCI can implement that
contract without changing future USB class drivers. The current xHCI state is
reset and ready for transfer-ring allocation; transfer rings and USB device
enumeration are covered by the following USB pass.

If PCI/xHCI is absent, the driver reports `unsupported` and keeps booting. The
QEMU x86 path adds `qemu-xhci` explicitly; aarch64 logs the current bring-up
limitation instead of claiming a host controller.

### USB transfers and enumeration follow-up

The xHCI runtime allocates page-aligned DMA pages from the physical frame
allocator and maps them through the existing direct map. Command, event,
endpoint, input-context, device-context, and ERST pages use the shared DMA
ownership and cache-sync checks. The event ring is polled in bounded loops;
interrupt-pending bits are acknowledged after each event, and failed or
timed-out transfers issue a bounded Stop Endpoint command before returning an
error.

The first connected port is reset and its speed is decoded before Enable Slot
and Address Device. Control transfers fetch the device and configuration
descriptors, parse the configuration and interrupt-IN endpoint, send
SetConfiguration, and configure the endpoint context. The transfer engine also
provides the same bounded Normal-TRB path for bulk and interrupt transfers;
class-specific HID parsing and event publication are covered by the following
pass, while other USB classes remain deferred.

### USB hub and disconnect follow-up

Hub-class devices are identified from the device and interface descriptors,
configured through the normal control path, and validated with the class hub
descriptor request before the device is retained. The xHCI runtime polls root
port connection state in the existing bounded debugger loop; removal issues
Disable Slot, clears the published device state, resets the endpoint ring, and
logs cleanup failures. A later connection starts the same bounded enumeration
path again, so disconnected devices are not left published.

### USB HID keyboard and mouse follow-up

The HID path requests the report descriptor through the normal control
transfer flow, rejects non-zero report IDs, and bounds parsing to 256
descriptor bytes, 24 fields, eight explicit local usages, and 64 report
bytes. It tracks descriptor logical minima so signed relative mouse values are
decoded correctly instead of treating `0xff` as `255`.

Boot keyboard reports publish modifier-aware key press/release events through
the same `input::Event::Key` queue used by PS/2. Mouse reports publish signed
X/Y/wheel movement and button transitions through the same
`input::Event::Pointer` queue. The xHCI runtime keeps one interrupt transfer
pending at a time, polls completion in bounded normal context, stops the
endpoint on timeout, and clears HID state during disconnect cleanup. Keyboard
and mouse descriptor self-checks run during driver initialization.

### USB serial and storage class follow-up

The xHCI class pass keeps endpoint ownership in the same bounded runtime
lifetime as HID. CDC ACM devices are recognized from their interface classes;
the QEMU FTDI-compatible serial device (`0403:6001`) is an explicit vendor
quirk. CDC line coding/control-line requests and FTDI baud/modem-control
requests configure 115200 8N1, and all progress or failure is emitted through
the kernel bootlog.

Mass-storage devices are recognized only for the USB BOT path currently
needed by this roadmap item. The driver configures bulk IN/OUT rings, submits
a bounded CBW, reads an INQUIRY response, validates the matching CSW, and
retains no endpoint after disconnect. Block geometry, READ CAPACITY, and block
read/write are deliberately deferred to the later block-layer task; they are
not advertised by this class probe.

### AC'97 audio follow-up

The audio path uses the QEMU-compatible Intel AC'97 controller discovered by
the shared PCI bus-0 helper. It validates and enables the PIO BARs, checks the
codec-ready bit, records codec vendor and extended-audio identifiers, enables
variable-rate PCM, and configures a bounded stereo signed-16-bit 48 kHz
format.

The playback ring owns one 32-bit-addressable BDL and four zeroed 4 KiB DMA
buffers. `submit_pcm` accepts only bounded interleaved stereo samples, syncs
the buffer through the common DMA contract, and restarts the playback stream.
Polling acknowledges buffer-complete state and restarts after FIFO, last-valid,
or halted status so underrun/overrun recovery remains bounded. The
`audio silence` debugger command exercises the PCM submission path.

No AC'97 controller, unavailable codec, invalid BAR, or failed DMA setup is a
boot-stopping condition: the driver is registered as unsupported/failed and
the bootlog explicitly reports that the deterministic silent fallback is
active. The aarch64 build registers the same fallback status without touching
x86 PIO or PCI code.

### Virtio-net follow-up

The first network device is the QEMU-compatible legacy virtio-pci net device.
PCI discovery filters for vendor `1af4`, so q35's unrelated built-in e1000e
does not become a false virtio probe failure. The driver negotiates MAC and
link-status features, reports the six-byte MAC and link state, and keeps
checksum offload disabled so protocol code owns checksum calculation.

RX and TX use separate legacy virtqueues with bounded device queue sizes and
32-bit DMA addresses. RX descriptors remain device-owned until
`receive_packet` copies a validated frame into the caller's buffer, then the
descriptor is reposted. TX remains kernel-owned until a used-ring completion;
`transmit_packet` rejects oversized frames, pads short Ethernet frames, and
returns `Busy` while the descriptor is in flight. Queue memory and packet
buffers use the shared DMA sync contract.

The legacy IRQ line is registered with a bounded hard handler that only reads
and acknowledges the device ISR; deferred work handles link and TX completion
in normal context. If IRQ registration is unavailable, the driver remains
usable through the same bounded polling path and emits a bootlog warning.
Absent virtio-net, unsupported features, or an unsupported BAR leave the
driver deferred with an explicit bootlog message.

### Block layer follow-up

The former direct RAM-disk sector helper is now a fixed-capacity block layer.
It publishes sector geometry, accepts aligned multi-sector requests, queues
requests behind `RequestId<'a>` ownership, and completes them through a bounded
poll/wait path. Read and write wrappers remain for the current VFS, while the
submit/wait API is available for the next filesystem vertical. Invalid ranges,
unaligned sizes, read-only writes, queue saturation, and bounded timeouts have
explicit typed errors.

The RAM backend initializes a small MBR partition table and discovers bounded
valid entries, falling back to a whole-disk partition when no valid table is
present. On x86_64, a modern PCI virtio-blk device can replace that backend
after VERSION_1 and F_FLUSH negotiation. Its one split queue is capped
at eight descriptors, uses separate page-aligned DMA regions and one-sector bounce buffers,
and validates used-ring count, descriptor, length, and request-status fields
before publishing completion. A detected but failed device is reported and
then falls back explicitly to the volatile RAM disk.

Cache policy is explicit and currently write-through; selecting write-back
returns `Unsupported` until a page cache owns dirty data. The serial `block`
command reports geometry, queue, completions, partitions, read-only state, and
cache mode, with `ro`, `rw`, `flush`, `write-through`, and explicit
write-back rejection controls.

Without attached media, the x86_64 QEMU bootlog reports `sector=512 sectors=32 queue=8
partitions=1 readonly=false cache=write-through`; the aarch64 path remains
RAM-backed and architecture neutral. The persistent smoke attaches a fixed FAT32
image and reports the virtio geometry before the same block contract checks.

### Virtio-blk and fixed-file persistence follow-up

`src/drivers/virtio_blk.rs` is deliberately a bounded polling backend rather
than a general storage subsystem. It discovers the modern PCI capabilities,
negotiates `VERSION_1` and optional block flush support, accepts read-only
media, and exposes sector reads, writes, and flushes through the block layer.
The driver has a contract self-check for queue geometry and malformed used-ring
responses; `scripts/run.sh` exposes it through `QEMU_BLOCK_IMAGE`.

`src/fat32.rs` opens a writable view over the real block callbacks and updates
only the existing fixed `/NORX.PST` chain. It does not allocate clusters or
create arbitrary files. The CI image generator pre-seeds the file, and the
two-boot smoke asserts the sequence survives a QEMU process restart. FAT32
allocation, mount-tree publication, ext4/btrfs write paths, journal recovery,
and a complete persistent POSIX namespace remain later roadmap work.

### Ramfs follow-up

The first writable filesystem is now an in-memory ramfs mounted through the
VFS API. It has a fixed inode table, directory child slots, regular-file data,
bounded path parsing, file handles with borrow-safe offsets, owner-style mode
checks (`0644` files and `0755` directories), and explicit errors for invalid
paths, capacity, permissions, directories, and open handles. `mkdir`, `open`,
`read_handle`, `write_handle`, `seek`, `chmod`, `rename`, `unlink`, and
`remove_dir` are the core operations; the existing serial `ls`, `cat`, and
`write` commands are compatibility wrappers over handles.

Mount and unmount are explicit. Unmount refuses while a handle, dentry, child
mount, or namespace root is live, and the boot self-check exercises nested
directory creation, offset readback, hard-link lifetime, stale-handle
rejection, type-aware traversal, mountpoint protection, duplicate-target
rejection, namespace visibility, permission denial, rename, unlink, busy
unmount, clean unmount, and remount before creating `/hello.txt`. Persistence,
page cache, and on-disk filesystem formats remain separate follow-up work.

The x86_64 and aarch64 QEMU bootlogs reach `ramfs /hello.txt readback passed`,
the ramfs/VFS self-check messages, and `vfs initialized` without an exception.
The architecture-neutral implementation passes both target builds; repository
wide clippy still reports pre-existing warnings outside this VFS pass.

### FAT32 follow-up

`src/fat32.rs` now provides the first on-disk filesystem vertical. `Mount::open`
validates the FAT32 BPB, derives bounded data geometry, walks FAT cluster
chains, resolves both short names and checked UTF-16 long names, and reads
regular files through the block-sector contract. The boot path probes the active
block backend read-only and leaves ramfs as the writable root when no FAT32
volume is present. When a persistent virtio-blk image is attached, the bounded
`/NORX.PST` check uses `Mount::open_rw`, the existing fixed chain, and
the block flush boundary.

`Mount::open_rw` adds a deliberately bounded safe-write path: it rewrites data
inside an existing cluster chain and commits the file size in its directory
entry only after the data sectors succeed. It rejects read-only mounts and
files larger than their existing chain; cluster allocation and file creation
remain a later extension. The fixture self-check covers BPB validation, long
name lookup, read-only rejection, directory-size update, readback, and the
no-space guard.

The normal no-media boot reports `FAT32 parser, long-name, and safe-write
checks passed`, the expected absent-volume fallback, and `kernel initialization
complete` on x86_64 and aarch64. The separate x86_64 media run reports the
versioned `/NORX.PST` write and readback markers. All unsupported-device and
fallback messages remain in the normal kernel boot log.

### Ext4 staged follow-up

`src/ext4.rs` adds a read-only ext4 mount path after the FAT32 probe. It checks
the superblock geometry, group descriptor size, inode layout, extents feature,
and journal recovery boundary before exposing a volume. The current staged
reader supports depth-0 extents, inode reads, bounded directory records,
regular-file reads, mode-based read permission checks, and root/path lookup.
Journaled volumes are accepted only as read-only when recovery is not required;
write support and journal replay remain explicitly outside this stage.

The fixture self-check covers a journaled read-only volume, root and regular
inode lookup, extent-backed file reads, directory-mode rejection, permission
metadata, short buffer handling, and write rejection. The x86_64 and aarch64
QEMU bootlogs both report `ext4 superblock, extent, directory, permission, and
journal checks passed`, then cleanly fall back with `ext4 volume absent; ramfs
remains the writable root` and reach `kernel initialization complete`.

### Btrfs staged follow-up

`src/btrfs.rs` adds a conservative read-only Btrfs bootstrap. It verifies the
CRC32C superblock, parses the system chunk array, maps logical addresses through
the chunk tree, validates checksummed tree blocks, traverses bounded internal
and leaf nodes, and discovers root items for subvolumes and snapshots. The
reader exposes path/stat and inline-file reads with mode checks; writes,
compression, regular data extents, non-CRC checksums, RAID profiles, recovery,
and unsupported incompatibility flags return explicit errors.

The fixture contains an internal root-tree node, a chunk tree, a default
subvolume, a snapshot root, and a checksum-protected inline file. Its self-check
also corrupts the superblock and selects an unsupported checksum type to verify
negative paths. Both QEMU bootlogs report `btrfs superblock, checksum, tree, and
subvolume checks passed`, then log `btrfs volume absent; ramfs remains the
writable root` and complete kernel initialization on x86_64 and aarch64.

### VFS mount API follow-up

`src/vfs.rs` now owns the common bounded mount contract: `MountId` and
`NamespaceId` identify a mount tree, `mount_in_namespace` resolves targets,
and `lookup_in_namespace` returns a generation-checked dentry whose release is
required before unmount. Mount nodes carry source, parent, flags, and
propagation metadata. Read-only mounts reject all mutating operations, mount
points cannot be removed or renamed while attached, duplicate targets are
rejected, and unmount rejects the root, child mounts, open handles, and live
dentries. Namespace destruction is transactional with respect to child mounts
and live references. Namespace creation gets an independent root mount over
the same validated ramfs backend; the fixed-capacity model is deliberate.
Process pathname operations currently select the root namespace; attaching a
process namespace and adding mount namespace syscalls remain a later ABI
follow-up rather than an undocumented claim of process isolation.

The serial debugger exposes `vfs` inspection plus `vfs mount <target>`,
`vfs umount <id>`, `vfs lookup <path>`, and `vfs namespace`. Thus mount sources
and targets are observable and recoverable through the normal serial path, not
through test-only calls. FAT32, ext4, and btrfs keep their staged read-only
parsers and are the next adapters to attach to this shared mount backend.

Both final QEMU bootlogs report the mount-tree self-check, `vfs initialized`,
and `kernel initialization complete`; all probe fallbacks and unsupported
features remain in the normal kernel boot log.

### Driver failure matrix follow-up

The common driver framework now runs one bounded matrix before device probes.
It covers absent devices, probe timeout, malformed descriptors, DMA failure,
hot-unplug through `quiesce` followed by `remove`, and an interrupt storm of
4096 hard-dispatches coalesced into one deferred pass. The typed framework
errors drive the same state transitions used by real drivers, and resource
release is checked after failed probes and removal. The serial `drivers` and
`trace` commands expose the resulting state/resource lifecycle; the bootlog
reports the matrix result and scenario names.

The x86_64 and aarch64 QEMU logs both report
`driver failure matrix passed scenarios=6` and reach
`kernel initialization complete` with empty QEMU stderr. Device-specific
hardware loops remain bounded by descriptor, port, queue, or explicit poll
limits; their architecture-specific modules stay behind the appropriate
`cfg` boundary.

### First stable driver contract

The first stable driver contract is the fixed-capacity API in
`src/drivers/framework.rs`: `Bus` owns discovered `Device` IDs, `Device` owns
validated MMIO/PIO/IRQ/DMA resources, and `DriverOps` owns probe/suspend/resume/
quiesce/remove callbacks. `probe` has bounded state transitions and releases
resources on every non-ready result; `hot_unplug` performs the explicit
quiesce/remove teardown path and refuses invalid ownership or state changes.
The 128-event trace ring plus serial `drivers` and `trace` commands are the
diagnostic surface for state and resource lifetime.

Initialization loops in the concrete drivers are bounded by fixed descriptor,
port, queue, or poll budgets. Architecture-specific code is limited to the
serial/PCI/PS/2/USB module boundaries and the `arch` layer; shared framework
types and state transitions are architecture-neutral. This contract is now
documented and frozen before the userspace device-service work begins.

The migration gate and first service candidates are recorded in
`docs/service-boundary.md`. Until user instruction execution, capability
grants, and a service supervisor are active, no driver is relabeled as a
userspace service merely for roadmap progress.

### Display contract follow-up

The display path now has an explicit versioned contract around the
firmware-provided framebuffer. `NORX_DISPLAY_MODE_CONTRACT_OK v=1` defines the
guest mode as the `RawFramebuffer` geometry and marks the current
`ModePolicy::FirmwareFixed` policy. The host window is presentation-only: its
scaling or resize cannot change guest mode. A future GOP or virtio-gpu mode
change must use a controller-owned transaction that prepares the new backing,
sets the scanout, publishes the new mode, and rolls back on failure.

Discovery validates geometry, publishes the current mode through a mode-list
API, and permits selecting that mode without pretending that a firmware
framebuffer can switch hardware modes. Mode IDs are looked up by value rather
than treated as array indexes, so future mode lists can use opaque stable IDs.

Damage regions are bounded and validated against the active mode; `flush`
consumes the queue and records the flush boundary for the current directly
mapped framebuffer. Cursor state has the same bounds contract. EDID and
hotplug are represented as explicit unsupported/no-event results until a
display controller can provide them. The serial `display` command exposes
`modes`, `mode`, `damage`, `flush`, `cursor`, and `edid` for bring-up checks.

The x86_64 virtio-gpu path supports both legacy PIO discovery and modern PCI
common/notify capabilities. It negotiates only `VIRTIO_F_VERSION_1`, creates a
bounded control queue, and queries the scanout through `GET_DISPLAY_INFO`.
When a valid 32-bit firmware framebuffer is present, the driver binds it as a
single contiguous backing entry and submits the minimal 2D sequence:
`RESOURCE_CREATE_2D`, `RESOURCE_ATTACH_BACKING`, `SET_SCANOUT`,
`TRANSFER_TO_HOST_2D`, and `RESOURCE_FLUSH`. 3D, virgl, multiple resources,
and mode switching are intentionally deferred; the firmware framebuffer stays
the fallback when the optional GPU path is absent or fails. The guest
framebuffer rectangle must fit completely in the device scanout; a smaller
scanout returns a mode mismatch instead of silently changing guest geometry.
Diagnostics report `guest=WxH` and `scanout=WxH` separately, with
`mode-switch=false`.

### Minimal network stack follow-up

The x86_64 network stack now owns Ethernet framing, IPv4 header validation,
Internet checksums, ARP resolution, ICMP echo diagnostics, and bounded UDP and
TCP transport state. DHCP runs during initialization and binds the lease,
gateway, and DNS server through the QEMU user-net path. DNS supports bounded A
record queries; IPv6, fragmentation, retransmission windows, TCP congestion
control, and hardware checksum offload remain explicit follow-up work.

The socket-facing surface is intentionally a fixed-capacity kernel API:
`udp_send`/`udp_receive`, `tcp_connect`/`tcp_send`/`tcp_receive`, `ping`, and
`dns_query`. A single diagnostic UDP socket and TCP connection keep ownership
simple while the kernel has no userspace process/socket table. ARP misses return
`WouldBlock` after queuing one request, and all RX/TX work is bounded by the
serial-debugger polling pass; malformed frames, unavailable routes, busy
connections, and timeout retries return explicit errors and bootlog messages.

The x86_64 network-stack smoke reached DHCP offer/request/lease binding with
`ip=10.0.2.15 gateway=10.0.2.2 dns=10.0.2.3`, answered an ICMP echo through
QEMU user-net, queued UDP, and received a forwarded UDP datagram through the
serial `net udp recv` API. A host TCP listener completed the diagnostic TCP
SYN/SYN-ACK/ACK handshake and exchanged `hello`/`world` payloads. All these
states and failures were visible in the kernel bootlog or serial debugger. A
forwarded synthetic DNS response completed an `example.com` A-record query and
reported `dns-result=Some(Ipv4Addr([1, 2, 3, 4]))`; the live QEMU resolver was
also allowed to time out through the bounded error path. The malformed pre-fix
IPv4 contract is covered by `net::contract_self_check`.

The x86_64 display smoke reported `NORX_DISPLAY_MODE_CONTRACT_OK v=1`
and `display ready mode=0 1280x800 pitch=5120`. On boots that reach the serial
debugger, mode listing/selection, damage queueing and flush, cursor on/off,
and the explicit EDID unsupported result can be exercised. The
aarch64 UEFI smoke reported the explicit no-framebuffer fallback and still
reached `kernel initialization complete` with serial diagnostics available.
The current x86_64 QEMU `virtio-vga` configuration reports a guest framebuffer
larger than its advertised scanout, so the strict path returns `ModeMismatch`
and keeps the firmware display fallback. A matched scanout is required before
claiming a successful 2D virtio-gpu display backend; this mismatch is an
intentional negative contract case, not host-window scaling.

## Verification

Passed on this tree:

```text
cargo fmt --check
cargo build --target x86_64-unknown-none
cargo build --target aarch64-unknown-uefi
cargo clippy --target x86_64-unknown-none -- -D warnings
cargo clippy --target aarch64-unknown-uefi -- -D warnings
```

Direct QEMU boot smoke also passed on both architectures using the existing
UEFI ESPs: x86_64 reached the serial debugger; aarch64 reached the serial
debugger after loading `/boot/norx.dtb` with `-cpu cortex-a57`. The lifecycle
self-check runs during driver initialization on both boots, including the UART
divisor/FIFO/flow-control checks. Both bounded serial paths reached the prompt
without a panic or initialization halt. GRUB image creation through
`scripts/run.sh` was not run because the local machine lacks
`grub-mkstandalone` and `grub-file`; the script now carries the aarch64 CPU and
DTB hand-off required by the QEMU path. The existing untracked
`assets/fonts/Terminus-u12n.bdf` was outside this audit and was left unchanged.

The x86_64 QEMU PS/2 check reported `controller=true`, both ports discovered,
and `irq-registered=true irq-enabled=true`; the serial-debugger reset command
received a keyboard ACK. The aarch64 QEMU fallback reached the debugger with
no PS/2 initialization path or panic. A QEMU `sendkey a` event was delivered
as independent press/release events, and switching the diagnostic layout to
Russian produced `text=Some('ф')`. The x86 boot log reported controller and
 keyboard readiness plus `ps/2 IRQ1/IRQ12 routing enabled`.

The x86 QEMU USB check used `qemu-xhci` with `usb-kbd` attached and reported
PCI `00:03.0`, vendor/device `0x1b36/0x000d`, BAR `0xc000004000`, 64 slots,
8 ports, 16 interrupters, and 64-bit addressing. Bootlog evidence covered
port reset at high speed, slot/address completion, both descriptors (including
configuration length 34), SetConfiguration, endpoint configuration, and
`devices=1 configured=1 vid=0x0627 pid=0x0001`; the kernel reached the serial
debugger without a panic. The aarch64 boot log reported the
unsupported-host-controller fallback explicitly.

The QEMU hub smoke reported a full-speed hub with `hub-ports=8` after reading
its class descriptor. QMP `device_del` produced the bootlog warning
`xHCI USB device disconnected; removing slot`; adding the keyboard back
produced `xHCI USB connection detected; enumerating device` followed by
`xHCI USB device re-enumerated`.

The x86 HID keyboard smoke reached `xHCI USB HID keyboard report-bytes=8`,
published `hid=keyboard`, and reached `kernel initialization complete`
without a panic. The separate HID mouse smoke reached
`xHCI USB HID mouse report-bytes=4`, published `hid=mouse`, and completed
enumeration without a panic; QEMU's USB mouse input device did not reach the
final boot-log line before the bounded smoke window, so that result is not
counted as a full-boot assertion. The parser self-check covers the mouse
event path independently.

The x86 serial-class smoke attached QEMU's always-plugged FTDI-compatible
`usb-serial` device and reached `class=serial`,
`serial-protocol=ftdi`, and `xHCI USB serial configured protocol=ftdi` before
kernel initialization completed without a panic. The storage hotplug smoke
used a bounded 1 MiB raw backing file and reached endpoint configuration,
`xHCI USB mass-storage inquiry ok bytes=36`, CSW validation, and
`xHCI USB device re-enumerated`; all diagnostics appeared in the kernel
bootlog. Full block I/O remains intentionally outside this roadmap item.

The x86 fallback smoke reached `AC'97 audio controller not found; silent
fallback active`; the AC'97 smoke added QEMU's `8086:2415` controller and
reached codec `8384:7600`, 48 kHz stereo S16 playback, a four-entry 4096-byte
DMA ring, and `running=true` in the kernel bootlog. A serial debugger smoke
submitted `audio silence` and reported `running=true recoveries=0`. The
aarch64 UEFI smoke reached `AC'97 audio unsupported on aarch64; silent
fallback active` and completed kernel initialization without a panic.

The x86 virtio-net smoke used QEMU's legacy `virtio-net-pci` with a user-mode
backend and reached `1af4:1000`, MAC `52:54:00:12:34:56`, `link=true`,
`irq=11`, `interrupts=true`, and 256-entry RX/TX queues in the kernel bootlog.
The serial debugger `net tx` command queued a 60-byte frame; a follow-up
`net` status reported `tx=1 busy=false` without a panic. The q35 no-network
smoke ignored the built-in e1000e, logged `virtio-net controller not found;
networking deferred`, and completed initialization. The aarch64 UEFI smoke
reported the explicit unsupported-network fallback and reached the debugger.
