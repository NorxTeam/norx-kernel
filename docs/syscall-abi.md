# Norx syscall ABI boundary

The ABI boundary is architecture-neutral and versioned as `ABI_VERSION =
2` in `src/syscall.rs`. `Number` owns Linux-shaped syscall numbers, `Args`
always carries six 64-bit words, and `Metadata` records argument count and
restart policy. `Errno::return_value()` encodes failures as the unsigned
two's-complement form of a negative errno, so both entry wrappers expose the
same logical return model.

x86_64 enters through `syscall` and returns through `sysretq`. The wrapper saves
the user stack, `rcx`, and `r11`, switches to the TSS-provided kernel stack,
maps Linux's `r10` fourth argument into the SysV `rcx` position, and preserves
the sixth argument as the seventh SysV stack argument. It masks interrupts
through `IA32_FMASK` and refuses initialization if the TSS stack is absent.

aarch64 uses the synchronous exception vector for SVC. It recognizes the SVC
class, maps `x8` to the syscall number and `x0..x5` to the six logical
arguments, calls the same `syscall::dispatch`, restores the saved registers,
and returns with `eret`. Unsupported numbers and currently unimplemented
numbers both return `-ENOSYS` through the common dispatcher.

The first runtime slice handles `read`, `write`, `exit`, `wait`, `getpid`,
`gettid`, `yield`, `sleep`, and `close` against the bounded process table.
Every process starts with nonblocking fd 0 and writable fds 1/2. `read` polls
serial input and returns `-EAGAIN` when no byte is ready; VFS-backed descriptors
use the shared open-file offset, while pipe descriptors expose bounded
`EAGAIN`/EOF/`EPIPE` behavior. `write` copies at most 1024 bytes from the active
user address space to the kernel console or selected descriptor. Neither call
falls back to host I/O; concurrent blocking streams remain follow-up work.

## v2 process-I/O and filesystem extension boundary

ABI version 2 retains the process-I/O calls and publishes the bounded
filesystem calls needed by the first coreutils port. The numbers and layouts
are frozen so kernel, Rust, and C implementations cannot drift:

| Call | Number | Shape |
| --- | ---: | --- |
| `open` | 2 | path pointer, byte length, flags, mode |
| `pipe` | 22 | `PipeFds*`, flags |
| `dup2` | 33 | old fd, new fd |
| `wait_status` | 402 | child/group, `WaitStatus*`, options |
| `spawn2` | 401 | `SpawnSpec*` |
| `spawn_delegated` | 412 | `DelegatedSpawnSpec*` |
| `setpgid` / `getpgid` | 403 / 404 | process and group IDs |
| `killpg` | 405 | process group, signal |
| `tty_get_foreground` / `tty_set_foreground` | 406 / 407 | tty fd and process group |
| `mkdir` / `rmdir` / `unlink` | 420 / 421 / 422 | path pointer, byte length, mode only for `mkdir` |
| `rename` / `link` | 423 / 424 | two path pointer/length pairs |
| `stat` | 425 | path pointer/length, `Stat*` |
| `read_dir` | 426 | path pointer/length, bounded `DirEntry*` array, capacity |
| `fsync` | 427 | writable fd |
| `sync_path` | 428 | path pointer/length |
| `lseek` | 429 | fd, signed offset, `SEEK_SET`/`SEEK_CUR`/`SEEK_END` |
| `fstat` | 430 | fd, `Stat*` |
| `fchmod` | 431 | fd, mode |
| `fcntl` | 432 | fd, `F_GETFD`/`F_SETFD`, descriptor flags |

`PipeFds` is two 64-bit words. `WaitStatus` is four C-layout words: a kind
(`exited`, `signaled`, `stopped`, or `continued`), signed exit code, signal,
and reserved word. `SpawnSpec` is eleven 64-bit words containing copied path,
argument/environment vectors, stdio fds, process group, and flags. Open flags
are `READ`, `WRITE`, `CREATE`, `TRUNCATE`, `APPEND`, and `EXCLUSIVE`;
`EXCLUSIVE` requires `CREATE` and fails if the path already exists. `fsync` and
`sync_path` validate their descriptor/path and return `-ENOTSUP` until a
block-backed filesystem provides a real flush/publication implementation; the
volatile RAMFS never claims durability. Rename replaces an existing regular,
unopened destination atomically and refuses directories or open destinations.
Spawn flags are
`NEW_PROCESS_GROUP` and `FOREGROUND`.

The kernel implements bounded `open`, VFS-backed `read`/`write`/`close`,
`pipe`, `dup2`, synchronous `wait_status`, process-group authorization, a
single serial controlling-TTY ownership backend, a static-image `spawn2`, and a
bounded `spawn_delegated` path that copies bounded argv/environment strings,
stages the child until runtime and standard-descriptor setup succeeds, publishes
a ready child, and switches through a cooperative syscall-boundary continuation.
The current ABI inherits only the explicitly requested standard descriptors;
arbitrary parent FDs are not copied. The filesystem extension exposes regular files
and directories, fixed-size metadata, hard links, and bounded directory
records. Symlink nodes, locale state, and host I/O are not part of this ABI.
`lseek`, `fstat`, and `fchmod` operate on VFS-backed regular-file and directory
handles; pipes and serial descriptors reject filesystem-only operations.
`fcntl` currently exposes only `F_GETFD` and `F_SETFD` with the bounded
`FD_CLOEXEC` flag; unsupported commands or flags return `-EINVAL`.

When a read-only persistent mount is present, `open`, `read`, `lseek`, `stat`,
`fstat`, `read_dir`, and `fsync` dispatch to the bounded filesystem reader and
block flush boundary. An explicitly writable FAT32 mount additionally supports
bounded existing-file rewrite plus short 8.3 create/mkdir, empty-file unlink,
and same-directory rename; ext4 and btrfs remain read-only. `spawn2`
accepts `NORX_SPAWN_INHERIT_OPEN_FDS`/`SPAWN_INHERIT_OPEN_FDS` for bounded
inheritance of the parent's open descriptor table while preserving `FD_CLOEXEC`.

`Timespec` is explicitly two signed 64-bit fields; user pointers and words are
64-bit at this stage. `RestartPolicy` is metadata for the blocking boundary.
The ABI and runtime self-checks verify table size, argument width, error
encoding, structure alignment, and control syscalls during boot; the bootlog
reports the ABI version, table size, negative-error convention, and runtime
checks on both targets.

The current `usercopy` boundary validates null/canonical/overflow/size rules.
x86_64 uses an IDT assembly shim with an active-copy range and recovery RIP;
aarch64 uses the corresponding SVC-vector data-abort path and ELR recovery.
Both return an `EFAULT`-class result for an unmapped copy without entering the
fatal exception path. Pointer-accepting calls copy into or out of bounded
kernel-owned arrays; path and vector lengths are checked before every copy.
Multi-step TOCTOU pinning remains a future requirement for unbounded APIs, but
the v2 records never expose kernel pointers or host descriptors.

The public C header and ABI smoke example live under the shared
`BoaKernel/userspace` tree: `include/norx/syscall.h` and
`examples/abi_smoke.c`. They are freestanding source artifacts; linking and
user-mode execution remain deferred until the runtime prerequisites listed
above are active.
