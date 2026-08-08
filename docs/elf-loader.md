# Norx bounded ELF64 loader

`src/elf.rs` parses a bounded ELF64 image into a `LoadPlan`. It validates the
magic/class/endianness/version, object type, target machine, header sizes,
program-header bounds, PT_LOAD alignment, file ranges, `filesz <= memsz`, user
address limits, segment overlap, entry-point coverage, and W+X permissions.
Each segment records page-aligned virtual bounds, file bytes, zero-fill bytes,
and read/write/execute permissions without trusting host paths or allocating
unbounded memory.

The plan also builds a bounded initial stack with argc/argv/envp, `AT_PAGESZ`,
`AT_ENTRY`, and `AT_NULL` auxiliary-vector entries, then exposes the initial
stack pointer and instruction-pointer register state. The ASLR value is an
explicit load-bias input; ET_DYN requires a page-aligned bias.

The parser and stack builder run as boot self-checks on both targets. The
x86_64 register image uses SysV `RFLAGS=0x202`; aarch64 keeps its entry state
local with EL0 flags and `x0=argc`, `x1=argv`, and `x2=envp`. Mapping
the plan into a process-owned address space and entering user mode are kept
separate: they require the architecture context-switch and user-page-table
integration that follows this contract.

Negative coverage includes bad magic, truncated file data, overlapping PT_LOAD
segments, non-canonical user ranges, W+X flags, invalid entry points, and
oversized argv/env vectors.
