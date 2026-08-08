# Kernel/user trust boundary and capability transfer

## Trust boundary

The kernel is the authority for physical memory, page tables, interrupts,
devices, VFS objects, process identities, and scheduling. A user process is
untrusted input at every syscall, loader, VM host call, and IPC endpoint.

| Boundary input | Kernel rule |
| --- | --- |
| User pointer/length | validate canonicality, overflow, range, and ownership before copying |
| Process/thread/FD ID | validate slot and generation; never use the ID as a raw array index |
| VFS path | resolve only inside the process namespace; never consult a host path |
| Device/resource handle | use an owning, generation-checked kernel object with explicit rights |
| Capability request | authorize effective credentials; UID 0 alone never bypasses the bitset |
| Wasm import | expose only the versioned host signature and bounded instance handles |

Kernel pointers, physical addresses, page-table objects, device registers,
and mutable kernel references never cross this boundary. `copy_from_user` and
`copy_to_user` are the only pointer-facing primitives; a future blocking
operation must pin or copy its user data before it can sleep.

## Two capability layers

1. `Credentials.capabilities` is an ambient, kernel-owned authorization set
   for coarse operations such as `Mount`, `RawIo`, `NetAdmin`, `NetRaw`,
   `MemoryMap`, and `DeviceAdmin`. It is checked against effective credentials;
   real/saved UID and GID values do not grant an implicit bypass.
2. A resource capability is a generation-tagged handle to one kernel object
   (for example an IPC endpoint, VFS description, device, or shared-memory
   region) plus a rights mask. It is the only transferable authority. The
   integer visible to a process is an opaque table slot, not a kernel pointer
   or a global object address.

## Transfer protocol

The future IPC grant operation follows this fixed rule:

1. The sender presents an owned handle and an explicit subset of its rights.
2. The kernel verifies the sender has a transfer right and that the requested
   subset is no wider than the sender's current rights.
3. The kernel creates a new receiver-owned handle with
   `sender_rights & requested_rights`; it never aliases the sender's table
   slot or trusts a user-supplied object address.
4. Close, process exit, exec close-on-exec, endpoint revocation, and generation
   rollover invalidate the corresponding handle. In-flight operations retain
   a kernel reference until completion, then release it.

Ambient credentials are not transferred through IPC. A service receives only
the specific resource handles and rights it needs; privilege escalation,
UID-only grants, arbitrary handle duplication, and raw pointer transfer are
denied. The first IPC implementation must add the transfer-right check and
audit trail before exposing grants to userspace.

## Current staged boundary

The kernel currently enforces the credential checks, process/FD ownership,
usercopy validation, W^X address-space ownership, VFS namespace rules, and
Wasm handle revocation. IPC endpoints, shared-memory grants, and a public
transfer syscall are intentionally deferred to the next section of the
Roadmap; until then no capability-transfer API is advertised.
