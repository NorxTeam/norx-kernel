# Portable VM evaluation

The comparison is for a kernel-resident interpreter with bounded resources,
capability-based imports, deterministic traps, and no compiler runtime in the
kernel.

| Criterion | Wasm/WASI | LLVM IR/bitcode | Custom register VM |
|---|---|---|---|
| Verifier complexity | High but standardized; use a strict profile | Very high; IR is not a safe deployment ABI | Low to medium if types, control flow, and limits are explicit |
| Memory isolation | Linear memory plus checked table/call rules | Must build a complete safety layer around IR | Direct bounds checks in the verifier/interpreter |
| Syscall imports | Natural explicit imports; avoid unrestricted WASI | ABI and target semantics leak into imports | Small native import table is straightforward |
| Binary size | Compact mature encoding | Large metadata and toolchain assumptions | Smallest format for Norx-only modules |
| Debugging | Existing names/source-map tooling | Excellent off-target tooling | Must define symbols and traces ourselves |
| Determinism | Good with a restricted WASI/import profile | Difficult around undefined behavior and target-dependent lowering | Strong by construction |
| Cross-architecture portability | Mature and proven | Requires careful target-independent subset | Strong, but every producer must target Norx |

## Decision

LLVM IR is an off-target compilation representation, never a kernel execution
format. A custom register VM is the smallest implementation, but it would
also require a new compiler, assembler, debugger, and ecosystem before it can
carry useful programs.

The recommended first implementation is a strict Wasm core profile with
Norx-owned imports instead of general WASI: bounded linear memory, no threads,
no floating point initially, no dynamic code generation, fixed fuel/stack
limits, and handles rather than raw pointers. This retains existing Wasm
tooling while keeping the kernel surface smaller than full WASI.

The next 2.5 tasks must freeze the exact module subset, import ABI, verifier
limits, binary sections, debug metadata, and dependency rules before any
interpreter code is added.
