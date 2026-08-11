# Terminal ABI v1

Norx exposes one controlling serial TTY on fd `0` to userspace services.
The kernel retains ownership of the terminal record; child processes inherit
the descriptor and the current foreground process group, but cannot silently
claim the terminal.

## Syscalls

| Number | Name | Arguments | Result |
| ---: | --- | --- | --- |
| 406 | `tty_get_foreground` | `fd` | foreground process group |
| 407 | `tty_set_foreground` | `fd`, `group` | `0` |
| 408 | `tty_get_info` | `fd`, `struct norx_tty_info *` | `0` |
| 409 | `tty_set_window` | `fd`, `columns`, `rows` | `0` |

`tty_get_info` returns this fixed 20-byte record:

```c
struct norx_tty_info {
    uint32_t flags;                 /* AVAILABLE=1, SERIAL=2 */
    uint32_t controlling_process;
    uint32_t foreground_group;
    uint16_t columns;
    uint16_t rows;
    uint32_t reserved;
};
```

The current terminal defaults to 80x25. Window sizes are bounded to
1..512 columns and 1..256 rows. `tty_set_window` is restricted to the
controlling process, just like foreground-group changes. `tty_get_info` is
available to a process that owns fd 0 and reports `AVAILABLE=0` when the
serial backend has failed.

Getty must validate `AVAILABLE`, the controlling process, and the foreground
group before prompting. A group mismatch is a recoverable terminal-busy
condition. Signal forwarding and shutdown remain supervisor actions; getty's
versioned `LoginHandoff` carries the target group and window metadata without
including credentials.
