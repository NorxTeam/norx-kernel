# Staged dynamic linking

Norx accepts bounded `ET_DYN`/PIE images with an explicit page-aligned load
bias. `PT_INTERP` is validated and exposed by the exec replacement contract;
`PT_DYNAMIC` is parsed into a bounded plan containing `DT_NEEDED`, strings,
symbols, SysV/GNU hash metadata, RELA entries, and `PT_TLS`.

The plan supports the relative relocation formula `B + A` and identifies the
architecture-specific symbolic relocation classes. REL entries, malformed
tables, missing hashes, unsafe names, oversized metadata, and unsupported
relocations fail closed. TLS file/memory sizes and alignment are bounded.

Library lookup is deliberately VFS-only: the staged policy searches
`/lib`, accepts a single basename, and never interprets host
filesystem paths, environment variables, or untrusted path components.

The boot smoke contract runs a main image with one `libdep.so`, resolves an
undefined `foo`, applies the symbolic `S + A` check, rejects `missing.so`,
checks the VFS path, and exits the staged runtime with status zero on both
architectures. It is intentionally a kernel contract until user-mode CPU
execution is available.

Actual shared-object page mapping, relocation writes into user memory, TLS
register hand-off, and executing a standalone user-mode dynamic linker remain
follow-up work after VM/user mappings are available.
