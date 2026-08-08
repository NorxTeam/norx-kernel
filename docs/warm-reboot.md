# Warm-reboot records

The boot path keeps a four-entry, checksummed record ring. Records contain the
boot state, sequence, error kind, error code, and two context arguments.
`crash::fatal` appends panic context before reporting and halting; normal boot
appends `BOOTING`, `CHECKPOINT`, and `READY` records at the corresponding
lifecycle boundaries.

The durable backend is four EFI non-volatile variables, one per ring slot. A
slot update is followed by a full-sector read-back before the commit is
reported as successful. The x86_64 Multiboot2 header requests the EFI64
system-table tag, so the GRUB/UEFI path can use that backend on both supported
targets. EFI calls are serialized with interrupts masked and restore the
architecture context before returning. On AArch64, the firmware VBAR and
SP_EL0 are restored around each call; TTBR0 is switched only when both firmware
and runtime tables are available. x86_64 also keeps the newest record in
battery-backed CMOS as a fallback for late panic paths when firmware runtime
services are unavailable.

The in-kernel RAM disk is deliberately not treated as persistent storage: it
is reinitialized on every boot. A successful warm-log commit therefore means
that EFI NVRAM or the x86 CMOS fallback accepted the record, never merely that
a volatile memory copy was updated.

To repeat the persistent firmware path, keep the vars file outside the rebuilt
image: `QEMU_VARS=build/warm-vars.fd ./scripts/run.sh aarch64`. For a visible
AArch64 QEMU window, add `-display gtk -device ramfb`; without a display
device QEMU opens a serial/parallel console window instead of the framebuffer.
A second QEMU process with the same vars file must report the previous
`READY`, `BOOTING`, or `PANIC` record with empty stderr. QEMU 10's QMP
`system_reset` reloads the pflash device state before the in-process test can
observe the write, so it is not treated as persistence evidence; process
restart is the reliable QEMU smoke boundary.

The graphical AArch64 smoke covers a clean boot, a second boot reporting
`previous boot completed`, and a serial-debugger `crash` followed by a boot
reporting the panic code and arguments. The x86_64 smoke reaches the durable
pre-architecture checkpoint; the current QEMU run remains blocked later in
the existing timer bring-up, before the normal READY path.

If neither durable backend is available, initialization and every later
status/panic commit report failure while the serial report remains
authoritative. No volatile RAM-disk write is counted as persistence.
