# Portable and native sample

The staged comparison uses the same high-level calls:

```text
norx_print("norx sample\n")
norx_exit(7)
```

The portable fixture is the Wasm module `CONTRACT` in `src/wasm.rs`. It has a
`norx.profile` v1 section, one active data segment containing the message, and
calls `norx.log(ptr, len)`. The `norx_main() -> i32` result is the equivalent
of `norx_exit(7)`. The verifier and interpreter check the data bounds before
the host call. `fd_read`, `fd_write`, and imported `exit` remain reserved
until the userspace syscall bridge is enabled.

The native fixture is the one-page ET_EXEC image from `elf::contract_image()`
in `src/elf.rs`. `user_runtime::sample_profile_self_check()` loads it, starts
the staged native runtime, performs `write(fd=1, ...)`, and exits with status
7. Native instruction execution is intentionally not claimed yet; the ELF
loader and syscall-facing runtime are the current 2.3/2.4 staged boundary.

Each boot runs both fixtures once and emits one comparison record:

```text
sample compare portable startup_ticks=... module_bytes=... linear_memory=...
host_calls=1 native startup_ticks=... elf_bytes=... user_memory=...
syscalls=2
```

The tick values are comparable only within the same QEMU target and boot. The
portable sample reserves one 64 KiB linear-memory page; the native sample maps
one ELF page plus the two-page initial user stack. The portable surface is one
bounded host call plus an exit result, while the native surface is `write` and
`exit`. `module_bytes`, `elf_bytes`, and mapped user memory are deterministic;
the tick fields are the measured startup baseline for future optimization.
