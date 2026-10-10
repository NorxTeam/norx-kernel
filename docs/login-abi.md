# Login/session ABI

`login` owns authentication and session construction. `getty` owns the
terminal prompt and passes a `NORX-GETTY 2` handoff; it does not verify
passwords or mutate account data.

The login port consumes `NORX-USERDB 1` through the `PasswordVerifier` trait.
Every eligible authentication attempt invokes the verifier exactly once before
the account-policy result is applied. Disabled, locked, expired, and
passwordless accounts still fail closed; unknown or passwordless accounts use
the fixed-policy `DUMMY_PASSWORD_HASH`. Login returns one generic
bad-credentials result for all account authentication failures while retaining
policy-specific events only in the protected audit record. Passwords and
encoded hashes are never put in argv, the environment, audit records, or
serial output. The target smoke uses a deterministic verifier adapter only to
exercise the boundary; a production image must provide a real bounded Argon2id
verifier.

The kernel ABI extends syscall ABI v2 with:

| Number | Name | Contract |
| ---: | --- | --- |
| 410 | `set_session` | Atomically applies credentials and bounded cwd/umask/resource attributes. |
| 411 | `get_credentials` | Returns the caller's fixed-size credentials record. |

`SessionSpec` is 80 bytes and begins with the common 8-byte syscall-record
header: the current ABI version, the exact declared size, and a zero reserved
word. The kernel rejects any other version/size and any non-zero reserved word
with `EINVAL` before applying the session. Cwd is an absolute, NUL-free path
of at most 256 bytes; umask is limited to `0777`; file-descriptor limits are
bounded to the kernel table. A caller needs the kernel `SessionAdmin`
capability to change identity/capabilities or raise inherited limits.
Unprivileged callers may only drop capabilities and limits while retaining
their identity.

`spawn2` accepts `NORX_SPAWN_INHERIT_CREDENTIALS`; the child inherits the
parent's credentials and session attributes, and its standard descriptors are
explicitly duplicated from the caller's `0/1/2`. Without that flag the
existing service-safe zero-capability bootstrap behavior remains unchanged.

The version-2 getty handoff is a fixed 80-byte record containing non-zero
`terminal_nonce` and `session_nonce` generation values. Login must echo both
values in its `LoginResult`; session close and session-scoped control events
must also carry the matching pair. Terminal-scoped events carry the terminal
nonce. Getty rotates the terminal generation on restart and returns the
previous session identity in `RestartPrompt` so the supervisor can terminate
and revoke the old session before accepting the new prompt. These values are
correlation data, not authorization capabilities; kernel checks remain
mandatory.
