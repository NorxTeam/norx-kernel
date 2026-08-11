# Norx Kernel

[![CI](../../actions/workflows/ci.yml/badge.svg)](../../actions/workflows/ci.yml)

Norx is an experimental Rust kernel and OS bring-up project. The current
boot path is GRUB-based:

- `x86_64-unknown-none` is loaded through GRUB Multiboot2;
- `aarch64-unknown-uefi` is loaded by GRUB's EFI chainloader and receives its
  DTB through the UEFI configuration table or `boot/norx.dtb`.

## What exists now

- Norx ASCII-art startup screen with ordered, colored status logs.
- GRUB hand-off for memory regions, reserved ranges, modules, command line,
  architecture information, and optional framebuffer.
- x86_64 VGA text fallback followed by a framebuffer-console takeover using the
  bootloader's active resolution and full pixel surface.
- Terminus 6x12 Linux-console bitmap font rendered at 2x scale (12x24 cells)
  and allocation-free black console panic output with detailed diagnostics.
- Serial-only, line-oriented `serial-debugger`; there is no framebuffer
  keyboard shell.
- Basic memory, paging, timer, scheduler, interrupt, VFS, and driver
  infrastructure.
- A versioned cross-architecture syscall boundary with Linux-shaped numbers,
  six-word arguments, negative errno returns, and x86_64/aarch64 entry
  wrappers; user processes and process-owned user pages are not implemented.
- The fixed-capacity process/thread/FD/credentials/exit-wait model is documented
  in `docs/process-model.md`; the embedded quickinit bootstrap runs as PID 1
  during the ordered userspace hand-off and returns to the kernel recovery path.

The current VFS is a bounded mount-tree smoke layer backed by an in-kernel RAM
filesystem and contains `/hello.txt`. Norx is not production-ready. See the
[shared roadmap](../ROADMAP.md) for the current implementation status and
future system work. The current ownership, failure, ABI, hardware, and
unsupported-case release gates are indexed in
[`docs/stability-contracts.md`](docs/stability-contracts.md).

## Requirements

- Rustup. The repository pins nightly Rust and both kernel targets in
  `rust-toolchain.toml`.
- A POSIX shell (`sh`), Cargo, and the GRUB tools `grub-mkstandalone` and
  `grub-file`.
- QEMU with the matching EDK2 firmware files in its share directory.

On Windows, run the shell script from WSL or another POSIX-compatible shell.
The script can also use a custom firmware directory through `QEMU_SHARE`.

## Build

```sh
cargo fmt --check
cargo build --target x86_64-unknown-none
cargo build --target aarch64-unknown-uefi
cargo clippy --target x86_64-unknown-none -- -D warnings
cargo clippy --target aarch64-unknown-uefi -- -D warnings
```

## Build and run in QEMU

```sh
./scripts/run.sh x86_64
./scripts/run.sh aarch64
```

The script builds a GRUB EFI system partition under `build/<arch>/esp` and
starts QEMU with serial output attached to the terminal. Set
`PROFILE=release` for a release build or `MODE=build` to create the partition
without starting QEMU:

```sh
PROFILE=release ./scripts/run.sh x86_64
MODE=build ./scripts/run.sh aarch64
```

For aarch64, set `NORX_DTB` to use a board-provided DTB. If it is unset, the
script generates a QEMU `virt` DTB. `QEMU_MACHINE` can override the default
QEMU machine, and `QEMU_SHARE` can point to the directory containing the EDK2
firmware and variable files.

On Windows, use the compact PowerShell wrapper when an ESP and UEFI vars file
already exist:

```powershell
.\scripts\qemu-boot.ps1 x86_64 -TimeoutSeconds 60 -SerialLog build\qemu-x86.log
.\scripts\qemu-boot.ps1 aarch64 -TimeoutSeconds 60 -SerialLog build\qemu-arm.log
.\scripts\qemu-boot.ps1 x86_64 -Interactive
```

Pass `-Esp`, `-Vars`, or `-Marker 'kernel initialization complete'` when using
another fixture. A timeout returns code `124` and preserves the serial log.

### `nsh` interactive smoke

After building the pinned Rust userspace fixtures, build the kernel with
`RUN_NSH_SMOKE=1 REQUIRE_USERSPACE_FIXTURE=1`, refresh both ESP kernel images,
and run the prompt-driven shell harness:

```sh
python3 ../toolchain/scripts/build-rust-userspace.py
RUN_NSH_SMOKE=1 REQUIRE_USERSPACE_FIXTURE=1 PROFILE=release MODE=build ./scripts/run.sh x86_64
RUN_NSH_SMOKE=1 REQUIRE_USERSPACE_FIXTURE=1 PROFILE=release MODE=build ./scripts/run.sh aarch64
./scripts/nsh-smoke.sh x86_64
./scripts/nsh-smoke.sh aarch64
```

The harness waits for every prompt and checks quoting, partial-line Ctrl-C
interruption, parser errors, command lookup, pipeline, redirection,
assignment, background-job, `source`, `exec`, and the explicit clean-exit
marker. It stops when the shell reports that clean exit; the kernel's later
service-supervisor checks are outside this shell-specific scenario.
`QEMU_MEMORY`, `QEMU_ACCEL`, and `QEMU_MINIMAL_DEVICES` are optional
environment overrides for constrained hosts; the default device and memory
profile remains the normal graphical QEMU smoke profile.

## Serial debugger

After boot, use the serial console. Type `help` to print the current command
list:

```text
help time sched irq hw mem paging drivers uname vfs ls cat write crash halt
```

`crash` intentionally enters the kernel panic path; `halt` stops the CPU.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for branch and verification rules.

## License

Norx Kernel is licensed under the GNU General Public License v3.0 only. See
`LICENSE`.
