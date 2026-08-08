# Norx address-space contract

`address_space::AddressSpace` is the first bounded process address-space
implementation. Each instance owns one root page-table frame and every mapped
user page frame; `destroy()` releases all of them through the physical-frame
allocator. A stale or destroyed object cannot map, unmap, or grow a stack.

Only page-aligned addresses in the architecture's user range are accepted.
Mappings are explicitly user-readable/writable/executable and W+X is rejected.
Kernel mappings are not exposed through this API, so a process cannot request a
kernel page in its user table. Anonymous pages and stack pages are tracked
separately, while a guard page remains unmapped below the stack.

The stack starts with two pages and grows only when the fault page equals the
current guard page, up to eight pages. `AslrHook` provides a deterministic,
seeded base-selection hook for a later entropy source; it never accepts a range
outside the user limit.

The contract is exercised during boot on x86_64 and aarch64. Actual CR3/TTBR0
installation and user-mode context switching are intentionally not performed
yet; those belong to the thread/context-switch work that follows this model.
