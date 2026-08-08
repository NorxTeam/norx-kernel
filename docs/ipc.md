# IPC primitives

Norx uses bounded kernel-owned IPC primitives as the primary service boundary:

- `Channel` copies messages into a fixed four-slot queue. Send and receive
  return `Full`, `Empty`, or `Closed` without spinning; the corresponding
  wait queue records a waiter and returns `Blocked`.
- `SharedRing` models a four-slot shared-memory data path with fixed 64-byte
  records. Only its producer may publish and only its consumer may consume.
  The kernel checks process identity, queue bounds, output size, and revocation.
- `EventQueue` carries typed `(kind, data)` completion/events separately from
  payload channels. It has bounded push/pop and the same explicit empty/closed
  blocking behavior.
- `WaitQueue` is a bounded FIFO-by-slot waiter set. Waking removes one waiter;
  the scheduler integration will turn that token into a runnable thread at the
  blocking boundary.

The first contract is intentionally non-spinning and fixed-capacity. A full
queue never overwrites unread data, a short receiver buffer never dequeues a
message, and a revoked ring cannot be accessed again. Signal-like process
pending bits remain notifications only; they do not carry payloads or replace
these queues.

`src/ipc.rs::contract_self_check()` covers message copying, full/empty/closed
transitions, producer/consumer ownership, buffer bounds, revocation, waiter
deduplication, and blocked/wake transitions during boot on both targets.
IPC syscall exposure and capability-handle grants remain follow-up work after
the scheduler can park and wake real user threads.
