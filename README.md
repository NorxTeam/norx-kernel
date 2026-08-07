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
- JetBrains Mono console font and allocation-free black console panic output
  with detailed diagnostics.
- Serial-only, line-oriented `serial-debugger`; there is no framebuffer
  keyboard shell.
- Basic memory, paging, timer, scheduler, interrupt, VFS, and driver
  infrastructure.
- Architecture-local syscall entry stubs reserved for a future userspace ABI;
  no cross-architecture syscall contract is defined yet.

The current VFS is only a smoke layer backed by an in-kernel RAM block device
and contains `/hello.txt`. Norx is not production-ready. See the
[roadmap](docs/ROADMAP.md) for the current implementation status.

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
