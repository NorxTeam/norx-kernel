# Login/session ABI

`login` owns authentication and session construction. `getty` owns the
terminal prompt and passes a `NORX-GETTY 1` handoff; it does not verify
passwords or mutate account data.

The login port consumes `NORX-USERDB 1` through the `PasswordVerifier` trait.
Account flags are enforced before verification: disabled, locked, and expired
accounts fail closed. Passwords and encoded hashes are never put in argv,
the environment, audit records, or serial output. The target smoke uses a
deterministic verifier adapter only to exercise the boundary; a production
image must provide a real bounded Argon2id verifier.

The kernel ABI extends syscall ABI v2 with:

| Number | Name | Contract |
| ---: | --- | --- |
| 410 | `set_session` | Atomically applies credentials and bounded cwd/umask/resource attributes. |
| 411 | `get_credentials` | Returns the caller's fixed-size credentials record. |

`SessionSpec` is fixed at 72 bytes. Cwd is an absolute, NUL-free path of at
most 256 bytes; umask is limited to `0777`; file-descriptor limits are bounded
to the kernel table. A caller needs the kernel `SessionAdmin` capability to
change identity/capabilities or raise inherited limits. Unprivileged callers
may only drop capabilities and limits while retaining their identity.

`spawn2` accepts `NORX_SPAWN_INHERIT_CREDENTIALS`; the child inherits the
parent's credentials and session attributes, and its standard descriptors are
explicitly duplicated from the caller's `0/1/2`. Without that flag the
existing service-safe zero-capability bootstrap behavior remains unchanged.
