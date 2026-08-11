# Norx process and task model

This is the contract for the first userspace process layer. It is deliberately
separate from the current scheduler smoke tasks: `sched::Task` remains a
kernel scheduling probe. User threads now have a bounded architecture context
for cooperative syscall-boundary switching; timer preemption and the remaining
blocking/wakeup policy are still follow-up work.

## Fixed identities and ownership

| Object | Identity | Initial bound | Owner |
| --- | --- | ---: | --- |
| Process | `ProcessId(u32)` | 32 | process table |
| Thread | `ThreadId(u32)` | 64 | user thread: one process; kernel thread: kernel |
| Scheduler task | `TaskId(u32)` | 64 | exactly one thread |
| File descriptor | `Fd(u32)` | 32 per process | process FD table |
| Open file description | generation-tagged slot | 64 global | VFS/mount handle |

IDs are never used as array indexes without a bounds and generation check.
Process and thread slots are reclaimed only after the exit/wait and join
ownership rules release every reference. The init process has a reserved PID;
its parent is `None`, and all orphaned children are reparented to init before
their exit record becomes waitable.

## State machines

Processes move through `Creating -> Running -> Exiting -> Zombie -> Reaped`.
Threads move through `Created -> Ready -> Running -> Blocked/Sleeping ->
Exited`. `ThreadKind::User` is attached to a process; `ThreadKind::Kernel` has
no userspace owner. A process is `Running` while it owns at least one
non-exited user thread; `Exiting` prevents new threads and file descriptors. A
zombie keeps only its exit status, resource-usage snapshot, parent linkage, and
wait reference.

The scheduler selects `TaskId`, while the task owns a `ThreadId`; a thread
owns the saved register context and a user thread points at one process
address space. The process-table contract charges running threads, makes
block/wakeup and preemption explicit, and validates a switch only from
`Running` to `Ready` and `Ready` to `Running`. x86_64 and AArch64 now save the
syscall-boundary register frame and install the selected process address space
before returning to user mode. Timer preemption, blocking wait queues, and
floating-point/TLS context remain deferred. No process state is modified from
a hard IRQ handler.

## File descriptors and VFS

An FD table entry contains an open-file-description reference, descriptor flags
(`close_on_exec`, `nonblocking`), and the access mode. `dup`-style operations
share the open-file description and offset; `close` drops one reference. The
VFS mount/dentry reference is held by the description, so unmount remains
`Busy` until all descriptions and dentries are released. `exec` closes only
entries with `close_on_exec`; process exit closes the entire table before the
zombie record is published.

The first implementation is fixed-capacity and does not expose host paths.
Descriptors are the only process-visible route to VFS handles; raw `FileHandle`
slots never cross the syscall boundary.

## Credentials and events

Credentials contain real/effective/saved UID and GID values plus a fixed
capability bitset (`Mount`, `RawIo`, `NetAdmin`, `NetRaw`, `MemoryMap`, and
`DeviceAdmin`). Authorization checks consume the effective credentials and the
operation's capability requirement; UID 0 without the capability is denied,
so there is no implicit global-root escape hatch. A process owns a bounded
signal/event bitmap and a pending queue. Delivery is recorded as pending state
and occurs at a safe return or blocking boundary, never by running arbitrary
user code in interrupt context.

## Parent/child and exit/wait

`exit(status)` atomically marks the calling thread exited, closes process FDs,
detaches address-space ownership, and publishes one wait record after all
threads are gone. `wait` matches a direct child (or an explicitly supported
wildcard), consumes exactly one zombie record, and returns the PID plus status.
Waking a waiter and reparenting an orphan happen under the same process-table
lock. Invalid parent relationships, double reap, and wait without a matching
child return explicit errno values.

The current boot path creates the reserved PID 1 process before entering the
embedded quickinit ELF. PID 1 may spawn and reap the bounded bootstrap child;
after its deterministic hand-off exit, the kernel reactivates the init thread
to continue the ordered boot log and recovery path. The design still
intentionally excludes fork/clone and copy-on-write until the address-space
contract is expanded. It also excludes signals that require user stack
construction until the safe user-memory path is tied to process-owned page
tables.

The trust boundary and future kernel-mediated capability-transfer rules are
defined in `docs/security-boundary.md`; this process model's credentials are
ambient authorization, not transferable resource handles.
