# Single-address-space evaluation

Status: evaluated for the current Norx protection boundary. The decision is
**no-go for mutually untrusted native POSIX processes**. Norx keeps separate
process address spaces as the security boundary. This document does not
authorize a shared-root mode or change the POSIX ABI.

## Scope and threat model

The candidate design would put more than one protection domain under one
translation root. The adversary is a native user process that can execute
arbitrary ring-3/EL0 instructions, choose any user virtual address, retain
stale pointers, race another process's exit/reuse, and attempt to alter its
own protection state. The boundary must protect:

- each process's code, data, stack, heap, and loader state from every other
  process;
- kernel text/data, page tables, scheduler/context state, and device mappings
  from user access;
- instruction fetch as well as data reads and writes; and
- teardown and address reuse, including pointers held across `exec`, exit,
  cancellation, and a generation rollover.

Signals, shared memory, file mappings, IPC, and debugger access are explicit
exceptions. They must be granted by a kernel-owned object with a bounded
lifetime and rights; a common address range or a copied integer is not an
authority grant.

## Existing protection story

Norx already has a real, hardware-enforced *separate-address-space* story:

| Property | x86_64 | AArch64 |
| --- | --- | --- |
| Root owned by a process runtime | `AddressSpace` owns a root frame and mapped frames | same contract |
| Kernel/user split | cloned kernel root plus user mappings with `PTE_USER`; kernel entries remain supervisor-only | cloned TTBR0 root plus AP user descriptors; kernel mappings remain outside the user domain |
| Writable/executable policy | `PTE_WRITABLE` and `PTE_NX`; W+X is rejected before mapping | `AP_USER_RW`/`AP_USER_RO` and `UXN`; W+X is rejected before mapping |
| Domain switch | `switch_to_user` loads CR3; return restores the Norx root | `switch_to_user` loads TTBR0; return restores the kernel root |
| Mapping lifetime | owned leaf/table frames are zeroed, unmapped, and returned on destroy | same bounded ownership and teardown contract |
| Fault boundary | page faults identify user vs kernel access and route stack growth/protection failure | data-abort syndrome and fault address feed the same contract |

The implementation evidence is in `src/address_space.rs`,
`src/arch/x86_64/paging.rs`, and `src/arch/aarch64/paging.rs`. The runtime
activates the selected root before entry and restores the kernel root after
return in `src/user_runtime.rs`. This is the correct boundary for native
processes: a user load from another process's address is not present in the
current CR3/TTBR0 translation tree and therefore faults in hardware.

This protection story is deliberately not the same as a single address
space. If two native processes were mapped as user-accessible leaves under
one root, the hardware would permit either process to read or write the other
process's leaves. ASLR, non-overlapping ranges, a software range check, or a
process ID stored beside a pointer would not change that result.

## Candidate mechanisms

### Separate virtual regions under one root — reject

Static regions are useful for layout and can make accidental pointers less
likely, but they do not isolate hostile native code. The page-table U bit is
permitted for both regions, and native code can issue a load/store to an
address outside its assigned interval. Removing the U bit from the other
region makes it inaccessible to *all* user processes, not selectively
accessible to its owner. This option has no authority-preserving operation to
use during a context switch.

### x86 protection keys (MPK/PKU) — not a native boundary by itself

Protection keys add a hardware access decision for user data pages, but the
PKRU state is writable by user-mode `WRPKRU`. They also do not control
instruction fetch. Consequently, arbitrary native code could enable access to
another domain or execute code from a domain whose data access is disabled.
MPK can become useful only for a narrower design in which all code that runs
with a shared root is verified not to execute `WRPKRU` (and every equivalent
escape), or where a stronger hardware domain-control mechanism is available.
That is a verifier-plus-hardware design, not MPK alone.

Reference: [Intel SDM, protection keys](https://cdrdv2-public.intel.com/868137/325462-089-sdm-vol-1-2abcd-3abcd-4.pdf),
which documents PKRU's user-mode read/write controls and the fact that
protection keys do not affect instruction fetch.

### AArch64 MTE — defense in depth, not an authority domain

MTE detects mismatched allocation and pointer tags. It is valuable for
spatial/temporal memory-safety bugs, but it is not an owner identity or a
per-process access-control list. Native code can create and manipulate tagged
pointers, tags are not a substitute for an unforgeable capability, and
asynchronous tag faults are not precise enough to define syscall or rollback
semantics. MTE therefore cannot be the sole boundary between co-located
untrusted POSIX processes. It may harden either the existing separate spaces
or a future verified compartment.

Reference: [Arm's MTE user guide](https://developer.arm.com/community/arm-community-blogs/b/operating-systems-blog/posts/new-mte-user-guide),
which distinguishes precise synchronous faults from imprecise asynchronous
reporting and describes the tag lock/key model.

### A verifier — viable only for a different execution profile

The existing Wasm direction is the viable verifier option: a module has
bounded linear memory, no raw kernel pointers, a fixed call/import table,
bounded stack/fuel, and revocable handles. If the verifier is complete for
the shipped instruction set and the interpreter never exposes an unchecked
host pointer, multiple Wasm instances can share a kernel translation root
without making native POSIX memory co-location sound.

The verifier must reject raw address arithmetic that escapes linear memory,
indirect calls outside verified tables, unbounded resource requests, and any
instruction or import that can alter the domain-control state. A verifier
failure is a load-time error; it must never fall back to native execution.

### Hybrid — recommended future direction

Keep native POSIX processes on separate CR3/TTBR0 roots. Co-locate only a
separately specified verified Wasm profile, optionally using MPK or MTE as
defense in depth where the hardware and target policy support it. Use
explicit kernel-owned shared-memory capabilities for data exchange instead
of making all user pages mutually visible.

This preserves POSIX pointer semantics, `fork`/copy-on-write and `exec`
rollback options, signal-frame construction, and generation-based teardown
without making those contracts depend on a verifier or optional CPU feature.

## Required invariants for any future co-location

No implementation may enable a shared-root mode until all of these are true:

1. A hardware access check is applied to every user data access and every
   instruction-fetch path that can cross a compartment boundary.
2. The active domain is kernel-selected on every entry, return, interrupt,
   fault, and scheduler switch; user code cannot widen it.
3. Domain metadata, region ownership, shared-memory grants, and generation
   state are kernel-owned and bounded. An address, pkey, tag, or process ID is
   not accepted as proof of ownership.
4. Exit, `exec`, cancellation, timeout, and fault teardown revoke the domain
   before its pages or domain identifier can be reused. TLB/cache/tag state
   is invalidated on every CPU that could run the old context.
5. A failed map, domain switch, verifier load, or resource charge rolls back
   all earlier allocations and leaves the old process runnable or explicitly
   terminated according to the syscall contract.
6. Native and verified execution profiles are versioned separately. A
   verifier, MPK/MTE hardening path, or unsupported CPU feature must not
   silently change native POSIX behavior or fall back to host I/O.

## Admission and verification gate

The first implementation of a co-located profile must carry target evidence,
not just host tests:

- x86_64 and AArch64 feature detection with an explicit unsupported result;
- cross-domain instruction-fetch, read, write, and atomic-access negatives;
- malicious `WRPKRU`/tag manipulation and attempted domain widening;
- interrupt, nested fault, syscall re-entry, and concurrent CPU switch tests;
- stale pointer and generation-reuse tests after exit, `exec`, cancellation,
  and timeout;
- verifier malformed-input/fuzz/no-panic tests, including overflow and
  resource-exhaustion rollback;
- TLB/cache/tag invalidation tests across CPUs; and
- real QEMU or hardware execution of both the allowed and denied paths.

Until those tests exist, the only accepted implementation is the existing
separate-address-space path. There is no single-address-space ABI, feature
flag, fallback, or test-only capability in this repository.

## Decision

S0-098 is complete as an architecture evaluation: **do not adopt a single
address space for native POSIX processes**. The current hardware boundary is
stronger and simpler—separate page-table roots—and is the only boundary that
the current x86_64/AArch64 runtime can claim. Revisit co-location only as the
hybrid verified-Wasm profile after the gate above is implemented and tested.
