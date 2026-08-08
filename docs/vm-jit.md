# JIT decision

The first interpreter profile runs a bounded 32-iteration Wasm smoke module
and reports target ticks in the boot log. This is a regression baseline, not a
claim that a JIT is already useful: there is no representative application
workload or measured hot loop yet.

Norx therefore keeps the interpreter-only path for now. A future JIT may be
added only after a real workload shows a material speedup and after the design
covers W^X allocation, architecture permissions, instruction-cache
maintenance, cancellation, and code revocation on both targets.
