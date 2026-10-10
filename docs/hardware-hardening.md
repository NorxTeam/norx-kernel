# Optional hardware memory hardening

This is the S0-100 architecture decision for ARM64 MTE and x86 LAM/MPK. It
does not change the POSIX profile or enable a hardware feature. The current
kernel has no safe feature-probe, per-thread register context, signal/fault
ABI, allocator tag state, or user-mode deployment boundary for these features.

## Decision

The mandatory boundary remains separate user page tables, user/supervisor page
permissions, NX and W^X, checked and bounded usercopy, generation-checked
ownership, and capability authorization. Hardware hardening may reduce bug
impact or improve detection, but it must never be required to make an invalid
pointer, stale handle, unauthorized mapping, or kernel access safe.

| Feature | What it actually provides | Norx decision |
| --- | --- | --- |
| ARM64 MTE | A 4-bit allocation tag for each 16-byte granule, compared with the logical tag in a pointer when the mapping is Normal-Tagged memory. | Optional detection of spatial/temporal memory bugs in selected process-owned mappings. It is not authority, page-table isolation, DMA isolation, or guaranteed temporal safety. |
| x86 LAM | Masking of selected upper linear-address bits so software can carry metadata in a pointer. | Not a protection mechanism. Keep it disabled for kernel pointers and user ABI until every pointer-facing boundary has an explicit normalize/validate rule. |
| x86 MPK/PKU | A 4-bit page key selected in each PTE and per-thread PKRU access/write restrictions for user data accesses. | Optional defense in depth for cooperative, verified same-address-space domains. It does not restrict instruction fetches and user code can change its own PKRU, so it cannot isolate hostile native code by itself. |

The separate-address-space design is therefore still the first production
boundary. A future single-address-space design must provide a verifier and a
non-bypassable transition policy before MPK or a similar mechanism can be
considered sufficient for that design.

## Threat model and invariants

The attacker may supply malformed syscall arguments, tagged or metadata-bearing
pointers, stale handles, malicious native instructions, invalid ELF mappings,
and concurrent/re-entrant requests. The attacker may also run on hardware that
lacks the optional feature. The kernel must continue to enforce these rules in
all cases:

1. User ranges are checked for zero, overflow, alignment, architecture range,
   and ownership before dereference. A tag or LAM metadata bit never grants
   authority and never bypasses `usercopy::validate`.
2. Page-table permissions and address-space ownership remain authoritative.
   W+X, kernel mappings in a user table, stale address-space objects, and stale
   generation-bearing handles remain rejected without the optional feature.
3. Optional state is owned by the thread/address-space object that owns the
   corresponding architectural state. It is saved, restored, reset on exec,
   and revoked before a slot, page, key, or address space is reused.
4. Feature absence, disabled firmware support, unsupported mappings, exhausted
   keys, tag-state allocation failure, and unsupported fault delivery return a
   typed unsupported/error result. They never become a host fallback, fake
   capability, or success-returning stub.
5. A hardening fault is a bounded user fault or process termination with a
   defined diagnostic. It must not corrupt kernel state, release a live frame,
   publish a partial mapping, or panic the kernel because the process supplied
   a bad tag or metadata bit.

## ARM64 MTE

MTE is useful for finding use-after-free and some out-of-bounds accesses: the
pointer's logical tag must match the allocation tag of every 16-byte granule.
The tag space is only four bits, so a reused allocation can receive the same
tag and escape detection. Tag checking is also configurable as synchronous,
asynchronous, or asymmetric; asynchronous reporting is not precise enough for
the primary fault address or rollback decision.

If implemented later, Norx should use this sequence:

1. Probe the architectural feature and firmware configuration at boot. Keep
   the result per CPU and require all CPUs that can run a thread to support the
   selected mode; otherwise use the ordinary page-table path.
2. Add an internal, versioned mapping attribute for Normal-Tagged memory. It
   may be accepted only for anonymous process-owned RAM after the page-table
   mapping and tag storage lifecycle are defined. Do not infer it from a raw
   POSIX `prot` bit or apply it to device/DMA memory, firmware mappings, or
   untrusted filesystem pages.
3. Initialize tags when a page/allocation is published, change the tag before
   reuse, and clear or quarantine tag state before returning a frame. Mapping,
   unmapping, rollback, fork/clone, exec, core dump, and crash recovery must
   account for tags as well as data. A page allocator that only owns 4 KiB
   frames is not by itself an MTE allocation implementation.
4. Start with synchronous checking for an explicitly opted-in Norx extension.
   The fault path must distinguish translation, permission, and tag faults and
   deliver the documented process fault. Asynchronous checking may be a later
   telemetry mode, never the only correctness mode.
5. Keep tagged-pointer syscall acceptance opt-in and scoped to data arguments.
   Any accepted pointer is normalized only for address-range validation; its
   tag is not retained as a capability. Untagged control arguments and all
   pointer arithmetic/overflow checks retain their ordinary rules.

The current aarch64 QEMU path uses `cortex-a57`, and the repository has no MTE
smoke fixture. A future MTE claim therefore needs real MTE-capable hardware or
a QEMU CPU model that demonstrably exposes FEAT_MTE, plus a target test that
proves both the enabled and absent-feature paths.

## x86 LAM

LAM changes the canonicality check and masks selected upper bits before paging
translation. With LAM48, user metadata occupies bits 62:48 and bit 47 must be
zero; with LAM57, metadata occupies bits 62:57. The mask is a pointer
representation feature, not a permission check, tag comparison, bounds check,
or lifetime check.

LAM is consequently not an S0-100 hardening boundary. If Norx ever uses it,
the safe contract is:

- enable it only for explicitly opted-in user threads and never for supervisor
  pointers, page-table roots, physical addresses, MMIO, or kernel return state;
- strip/validate metadata at every syscall, fault, IPC, loader, VFS, and
  usercopy boundary before comparing ranges or using an address as an object;
- reject metadata-bearing pointers for control arguments unless that argument's
  versioned ABI explicitly permits them;
- save/restore the thread's LAM mode with the address-space context and clear it
  on exec; and
- test 4-level/5-level paging interactions, non-canonical inputs, overflow,
  aliasing of differently tagged representations, and context-switch races.

Until that contract exists, the existing canonical user range is the correct
policy and LAM stays disabled. In particular, no upper pointer bits may be
used as a capability, generation, or bounds token merely because the CPU can
mask them.

## x86 MPK/PKU

MPK associates one of 16 four-bit keys with a user page-table entry. PKRU is a
32-bit, thread-local register with access-disable and write-disable bits for
each key. The permissions apply to data accesses, not instruction fetches,
and user code can execute the instructions that change its own PKRU. MPK can
therefore make accidental cross-domain data access fail cheaply, but a native
attacker that can execute arbitrary code in the same address space can change
PKRU or use an executable gadget to defeat the policy.

A future MPK implementation must be a defense-in-depth layer over page-table
permissions and a trusted runtime/verifier:

- reserve key 0 for the default domain and allocate the remaining keys from a
  bounded address-space-owned pool; exhaustion is `Unsupported`/`NoSpace`;
- retain ordinary PTE read/write/execute permissions as the upper bound and
  never model execute-only or code integrity as a PKRU property;
- save and restore PKRU as part of every thread context switch, initialize it
  to deny-by-default for non-default keys, and sanitize it on syscall/fault
  return, exec, thread creation, and thread destruction;
- permit key changes only through a versioned Norx extension with an explicit
  domain right. Do not expose a raw `WRPKRU`-equivalent ABI to unverified native
  code or use MPK as a substitute for capabilities;
- serialize PTE key changes with TLB invalidation and address-space ownership,
  and roll back the PTE and key allocation together on failure; and
- test instruction-fetch bypass, malicious `WRPKRU`, per-thread divergence,
  stale key reuse, interrupt/syscall entry, and migration across CPUs.

MPK is not needed for the current separate page tables. It should be revisited
only when a concrete same-address-space runtime has a verifier, a controlled
instruction set, and a complete thread-state/context-switch contract.

## ABI and verification gate

There is no POSIX-visible MTE, LAM, or MPK ABI in this revision. A future
Norx-only extension must be versioned separately from the POSIX base and must
report capability rather than pretending that every target supports it. The
minimum gate for enabling any option is:

| Gate | Required evidence |
| --- | --- |
| Probe | CPUID/ID-register and firmware checks are bounded, immutable after boot, and tested on supported and absent-feature CPUs. |
| Ownership | Per-thread/per-address-space state has an owner, generation/lifetime rule, rollback path, and CPU migration rule. |
| Boundary | Existing pointer, page-table, W^X, capability, and usercopy checks still pass with the feature disabled and with malformed metadata/tags. |
| Faults | Synchronous negative tests cover invalid tags, invalid metadata, permission faults, stale state, and re-entry without kernel panic or leaked frames. |
| Concurrency | SMP/context-switch tests prove no thread observes another thread's MTE control, LAM mode, or PKRU. |
| Lifecycle | Exec, exit/reap, mapping teardown, key/tag reuse, cancellation, and failed-commit rollback are covered. |
| Target | x86_64 and aarch64 builds remain clean; QEMU/hardware smoke proves enabled and fallback behavior. |
| POSIX | The base profile is unchanged; the extension is discoverable, opt-in, and returns a typed unsupported result when unavailable. |

The current repository satisfies the ordinary page-table/usercopy portion of
this gate but not the optional-feature rows. That is an intentional deferment,
not an implementation claim.

## Primary references

- [Arm MTE Linux ABI and fault modes](https://cdn.kernel.org/doc/html/latest/arch/arm64/memory-tagging-extension.html)
- [Arm tagged-address ABI](https://www.kernel.org/doc/html/latest/arch/arm64/tagged-pointers.html)
- [Intel LAM programming reference, section 6](https://cdrdv2-public.intel.com/812218/architecture-instruction-set-extensions-programming-reference.pdf)
- [Intel SDM, protection-key rights registers and PKU](https://cdrdv2-public.intel.com/874249/253668-090-sdm-vol-3a.pdf)
- [Linux protection-keys overview](https://cdn.kernel.org/doc/html/latest/core-api/protection-keys.html)
