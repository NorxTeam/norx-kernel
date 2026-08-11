# Userdb ABI v1

The non-secret lookup/session ABI is published in
`userspace/include/norx/userdb.h` and staged in
`test-rootfs/usr/include/norx/userdb.h`. It is intentionally separate from
the kernel syscall table until credential transfer and owner-aware VFS checks
are stable.

The ABI exposes fixed-size `norx_userdb_user_t` and
`norx_userdb_credentials_t` records, account flags, capability bits, and
mutation operation identifiers. It never exposes password hashes, password
buffers, or authentication prompts. Consumers must treat all lengths as
bounded and must not assume NUL termination beyond the published length.

Current kernel limitations are explicit: there is no `getuid`/`setuid` syscall,
supplementary-group syscall, password syscall, credential handle transfer, or
UID/GID owner field in VFS in ABI v2. Until those exist, userdb owns parsing,
policy, lookup, authentication-provider boundaries, and atomic database
updates; `login`, `passwd`, `userctl`, `su`, `sudo`, `getty`, and quickinit may
consume the header but cannot silently bypass the kernel boundary.
