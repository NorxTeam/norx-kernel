# `execve`-like replacement

`src/exec.rs` implements the architecture-neutral replacement transaction used
by the native runtime contract:

1. Parse and bound-check the new ELF image.
2. Select the requested `PT_INTERP` path when present.
3. Prepare and start a new address space with copied argument/environment
   vectors and initial registers.
4. Close descriptors marked close-on-exec.
5. Destroy the old runtime only after the new runtime is ready.

Any failure before the commit returns the old runtime unchanged. The boot
self-check covers the committed path, interpreter selection, descriptor
closure, malformed-image rollback, and teardown on both supported targets.

The current contract prepares the register state and runtime boundary; entering
user mode and executing the image remain architecture-specific follow-up work.
