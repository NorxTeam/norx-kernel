# Norx Wasm module profile

Norx modules use the WebAssembly binary format, version 1, with a strict
Norx profile. The kernel receives a verified module; compilation remains
off-target.

## Module limits and sections

- The module is capped at 256 KiB before validation and has at most 64 linear
  memory pages (4 MiB), one memory, and one table-free instance. The first
  kernel interpreter deliberately instantiates one page; larger modules wait
  for the VM memory model pass.
- Accepted standard sections are `type`, `import`, `function`, `memory`,
  `export`, `code`, and one bounded active `data` segment. `global`, `start`, `table`, `element`,
  `tag`, `data-count`, shared memory, SIMD, threads, exceptions, reference
  types, and floating point are rejected in profile v1.
- A `norx.profile` custom section starts with `NRXV`, major `1`, minor `0`,
  and a zero flags word. Unknown profile versions fail closed.
- The single active data segment must initialize within the declared memory.
  No host pointer or host filesystem path can occur in a module ABI.

## Entry, imports, and exports

- The required export is `norx_main` with type `() -> i32`; its return value is
  the process exit status.
- Only the `norx` import module is allowed. v1 imports are:
  `log(i32 ptr, i32 len) -> i32`, `fd_read(i32 fd, i32 ptr, i32 len) -> i32`,
  `fd_write(i32 fd, i32 ptr, i32 len) -> i32`, and `exit(i32 status) -> ()`.
- The staged interpreter executes `log`; the other signatures are reserved for
  the userspace syscall bridge and a module that calls them is rejected until
  that bridge is enabled.
- Pointers are offsets into the instance's linear memory and are checked for
  overflow and bounds on every import. File descriptors are kernel handles,
  never guest pointers or host descriptors.
- The module may export `memory` for debugging/inspection, but imports may not
  replace it and no mutable guest global is exposed as a capability.

## Linking, dependencies, and debug data

- There is no native relocation or executable code linking in profile v1.
  Function imports are the complete link surface and must match an exact
  versioned signature.
- An optional `norx.deps` custom section contains at most eight basename
  dependencies, each validated by the same `/lib`/`/lib64` VFS-only policy as
  native libraries. The kernel does not inspect `LD_LIBRARY_PATH`, RPATH, or
  host paths.
- An optional `norx.debug` custom section contains bounded function names and
  `(code offset, source file, line)` entries. Debug data is discarded before
  execution and cannot change validation or capabilities.

## Deterministic resource rules

Validation and execution enforce a 256-frame call stack, a 64 KiB value stack,
and a fixed fuel budget supplied by the process policy. Exhausting fuel,
stack, memory, or an import capability produces a named deterministic trap;
cancellation also produces a trap and revokes the instance's handles.

This profile intentionally excludes general WASI until Norx has a stable
userspace filesystem, process, and capability ABI. The format version and
limits are kernel-owned and must be bumped together when the profile changes.
