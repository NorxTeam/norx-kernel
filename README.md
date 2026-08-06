# Norx Kernel

[![CI](../../actions/workflows/ci.yml/badge.svg)](../../actions/workflows/ci.yml)

Norx is an experimental Rust kernel booted by GRUB. It currently targets
`x86_64-unknown-none` through GRUB's Multiboot2 EFI hand-off and
`aarch64-unknown-none-softfloat` through GRUB's ARM64 Linux-image/FDT hand-off.

## Current Scope

- GRUB hand-off with memory regions, reserved ranges, modules, command line,
  architecture information, and optional framebuffer.
- Serial logging and interactive shell sessions.
- Basic memory, paging, timer, scheduler, and interrupt infrastructure.
- A text-only VFS smoke layer over the built-in RAM block device.
- Architecture-local syscall entry stubs reserved for the future userspace ABI;
  no cross-architecture syscall contract is claimed yet.

## Requirements

- Rustup. The repository pins nightly Rust and freestanding targets in
  `rust-toolchain.toml`.
- GRUB's `grub-mkstandalone` and `grub-file` tools.
- An `objcopy` compatible with LLVM binary output for the ARM64 image.
- QEMU with EDK2 firmware files available in QEMU's share directory.

## Build

```sh
cargo build --target x86_64-unknown-none
cargo build --target aarch64-unknown-none-softfloat
```

## Run

```sh
./scripts/run.sh x86_64
./scripts/run.sh aarch64
```

The script builds a GRUB EFI system partition under `build/<arch>/esp` and
starts QEMU with it. Set `PROFILE=release` for a release build or `MODE=build`
to create the partition without starting QEMU. The ARM64 path converts the
freestanding ELF into the Linux `Image` layout expected by GRUB.

## Status

Norx is not production-ready. The public tree is intended for kernel development,
architecture experiments, and reproducible boot tests.

## License

Norx Kernel is licensed under the GNU General Public License v3.0 only. See
`LICENSE`.
