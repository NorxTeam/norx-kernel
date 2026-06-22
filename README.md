# Boa Kernel

[![CI](../../actions/workflows/ci.yml/badge.svg)](../../actions/workflows/ci.yml)

Boa is an experimental Rust kernel for a UEFI-first operating system. It boots
without an external bootloader and currently targets `x86_64-unknown-uefi` and
`aarch64-unknown-uefi`.

## Current Scope

- Direct UEFI entry and framebuffer console.
- Serial logging and interactive shell sessions.
- Basic memory, paging, timer, scheduler, and interrupt infrastructure.
- Early user-mode execution path with a small Boa syscall ABI.
- Minimal process I/O routing through per-session `stdin`, `stdout`, and
  `stderr` channels.

## Requirements

- Rustup. The repository pins nightly Rust and UEFI targets in
  `rust-toolchain.toml`.
- QEMU with EDK2 firmware files available in QEMU's share directory.

## Build

```sh
cargo build --target x86_64-unknown-uefi
cargo build --target aarch64-unknown-uefi
```

## Run

```sh
./scripts/run.sh x86_64
./scripts/run.sh aarch64
```

Set `PROFILE=release` for a release build or `MODE=build` to create the EFI
system partition without starting QEMU.

## Status

Boa is not production-ready. The public tree is intended for kernel development,
architecture experiments, and reproducible boot tests.
