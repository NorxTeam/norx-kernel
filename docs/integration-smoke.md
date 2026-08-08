# Staged userspace integration smoke

The early boot path runs the complete staged integration surface before
starting the serial debugger. The final marker is emitted only after the
preceding checks return successfully:

```text
integrated staged QEMU smoke process/ELF/file/memory/dynamic/VM/fault/permission/teardown checks passed
```

The marker covers these existing contracts:

- VFS initialization performs ramfs file writes/reads, offsets, permissions,
  mount-tree and unmount checks.
- The process contract creates a child and kernel thread, switches threads,
  checks credentials/FDs/capabilities, then exits and waits for the child.
- `AddressSpace` maps executable/read-write pages and the guarded stack,
  rejects W^X and invalid/fault addresses, grows the stack, and tears down all
  mappings.
- ELF parsing and `NativeRuntime` map the native image, build the initial
  stack, exercise the staged FD/serial path, bounded allocation, and exit.
- Dynamic linking validates VFS-only library discovery, a relocation and TLS,
  then runs the one-library/missing-dependency smoke. The Wasm interpreter runs
  the portable sample and deterministic memory/cancellation traps.
- Syscall/usercopy checks cover negative errno, invalid user ranges and the
  architecture-specific entry boundary; process and address-space checks cover
  permission and teardown behavior.

The same marker is checked in graphical QEMU on x86_64 and aarch64. In
addition, the native userspace gate must emit all three success markers on both
architectures:

```text
nordix-rust-smoke: external userspace ELF exited cleanly
nordix-c-runtime: external userspace ELF exited cleanly
nordix-cxx-runtime: external userspace ELF exited cleanly
```

Those markers prove actual static ELF instruction execution, syscall entry,
process creation, address-space activation, and clean exit. They do not claim
that pointer-based userspace `read`/`write`, real dynamic linking, or
asynchronous signal delivery are complete; the staged kernel contracts cover
the corresponding file, loader, signal, and event models until those ABI gates
are opened.

The old universal machine-code/syscall probe is not part of this path. The
remaining `contract_self_check` functions are bounded, typed boot assertions
for individual contracts; they do not accept or dispatch arbitrary guest
bytes. The boot log records this absence explicitly before the normal syscall
entry initialization.

## Negative boundary checks

The same boot pass runs bounded rejection checks before any optional device
probe:

- boot hand-off rejects truncated or malformed Multiboot2 tags on x86_64,
  malformed FDT headers and cell counts on aarch64, and invalid framebuffer
  ranges on both architectures;
- MMIO, PIO, and DMA helpers reject zero-sized, overflowing, unaligned, and
  out-of-range accesses without dereferencing them;
- the driver framework rejects invalid resource ranges and DMA ownership or
  direction, while block, virtio-net, virtio-gpu, and xHCI checks reject bad
  queue or descriptor shapes and unavailable operations;
- FAT32, ext4, btrfs, ELF, dynamic-loader, Wasm, syscall, and usercopy
  contracts exercise malformed metadata, broken images, invalid pointers,
  permission failures, and deterministic traps;
- the interrupt contract drives the bounded storm path and verifies that
  delivery accounting remains consistent under concurrent-style re-entry.

Each group fails the boot assertion immediately; the following serial marker
is emitted only after all groups have returned successfully:

```text
interrupt, MMIO, PIO, and DMA boundary checks passed
boot hand-off and parser boundary checks passed
```

## Serial and framebuffer artifacts

CI keeps the raw serial stream for diagnosis and derives an ANSI/CR-free
`*.stable.log` from it before applying smoke assertions. The stable file is
the automation interface; the raw file remains available when a failure needs
the original terminal control sequences.

The serial debugger begins with the versioned handshake
`NORX_SERIAL_DEBUGGER_READY v=1`; command names and tabular status fields stay
ASCII and versioned so smoke tooling does not need to parse the framebuffer
rendering. The handshake is asserted on the aarch64 run, which reaches the
debugger; x86_64 currently stops earlier in timer bring-up and keeps its early
boot marker as the honest boundary.

Each QEMU run also exposes a QMP endpoint and captures a `screendump` in PPM
format after its required marker. These files are visual regression artifacts,
not pass/fail input: a missing firmware GOP or an intentionally serial-only
architecture may produce an empty or firmware-owned framebuffer while the
serial contract still remains authoritative. Local runs use GTK plus
`ramfb` on aarch64 (or `virtio-vga` on x86_64), so the same display path is
visible during development.
