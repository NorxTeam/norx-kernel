#!/usr/bin/env python3
"""Run every shipped coreutils command through the real nsh/QEMU rootfs."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import queue
import selectors
import subprocess
import sys
import threading
import time


def load_shell_smoke():
    path = Path(__file__).with_name("nsh-smoke.py")
    spec = importlib.util.spec_from_file_location("nsh_smoke", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot import {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


SMOKE = load_shell_smoke()

# name, prompt, input, required response marker, forbidden response marker
BASE_COMMANDS = tuple(
    (
        name,
        b"norx:/$ ",
        payload,
        required,
        forbidden,
    )
    for name, payload, required, forbidden in (
        ("pwd", b"/bin/pwd\n", b"/", None),
        ("echo", b"/bin/echo -n COREUTILS_ECHO\n", b"COREUTILS_ECHO", None),
        ("cat", b"/bin/cat /hello.txt\n", b"Welcome to Norx VFS", None),
        ("env", b"/bin/env -i FOO=bar\n", b"FOO=bar", None),
        ("ls", b"/bin/ls -al /\n", b"hello.txt", None),
        ("mkdir", b"/bin/mkdir -p /tmp/coreutils/a\n", None, b"mkdir failed"),
        ("touch", b"/bin/touch /tmp/coreutils/a/file\n", None, b"touch failed"),
        ("stat", b"/bin/stat /tmp/coreutils/a/file\n", b"Mode:", None),
        ("cp", b"/bin/cp /hello.txt /tmp/coreutils/a/copy\n", None, b"copy failed"),
        ("mv", b"/bin/mv /tmp/coreutils/a/copy /tmp/coreutils/a/moved\n", None, b"rename failed"),
        ("ln", b"/bin/ln /tmp/coreutils/a/moved /tmp/coreutils/a/link\n", None, b"link failed"),
        ("find", b"/bin/find /tmp/coreutils -name 'm*'\n", b"/tmp/coreutils/a/moved", None),
        ("grep", b"/bin/grep -n Welcome /tmp/coreutils/a/moved\n", b"1:Welcome", None),
        ("head", b"/bin/head -n 1 /tmp/coreutils/a/moved\n", b"Welcome to Norx VFS", None),
        ("tail", b"/bin/tail -n 1 /tmp/coreutils/a/moved\n", b"Welcome to Norx VFS", None),
        ("sort", b"/bin/sort /tmp/coreutils/a/moved\n", b"Welcome to Norx VFS", None),
        ("wc", b"/bin/wc -l /tmp/coreutils/a/moved\n", b"1", None),
        ("sleep", b"/bin/sleep 0\n", None, b"sleep failed"),
        ("sleep", b"/bin/sleep 1\n", None, b"sleep failed"),
        ("true", b"/bin/true\n", None, b"command is not implemented"),
        ("false", b"/bin/false\n", None, b"command is not implemented"),
        ("rm", b"/bin/rm /tmp/coreutils/a/link /tmp/coreutils/a/moved /tmp/coreutils/a/file\n", None, b"unlink failed"),
        ("rmdir", b"/bin/rmdir /tmp/coreutils/a\n", None, b"rmdir failed"),
        ("rmdir", b"/bin/rmdir /tmp/coreutils\n", None, b"rmdir failed"),
        ("exit", b"exit 0\n", b"nsh: external userspace ELF exited cleanly", None),
    )
)
COMMANDS = tuple(
    entry
    for command in BASE_COMMANDS
    for entry in (
        (command,)
        if command[0] == "exit"
        else (
            command,
            (
                f"marker:{command[0]}",
                command[1],
                b"echo COREUTILS:" + command[0].encode() + b":OK\n",
                f"COREUTILS:{command[0]}:OK".encode(),
                None,
            ),
        )
    )
)
REQUIRED_OUTPUT = (b"quickinit: PID 1 hand-off complete; child reaped",)


def main() -> int:
    arch = sys.argv[1] if len(sys.argv) > 1 else "x86_64"
    if arch not in {"x86_64", "aarch64"}:
        print(f"usage: {sys.argv[0]} [x86_64|aarch64]", file=sys.stderr)
        return 2

    root = Path(__file__).resolve().parent.parent
    environment = os.environ.copy()
    environment.update(RUN_NSH_SMOKE="1", REQUIRE_USERSPACE_FIXTURE="1", QEMU_DISPLAY="none")
    if os.name == "nt":
        try:
            command = SMOKE.windows_qemu_command(root, arch)
        except RuntimeError as error:
            print(error, file=sys.stderr)
            return 2
    else:
        command = ["./scripts/run.sh", arch]

    process = subprocess.Popen(
        command,
        cwd=root,
        env=environment,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    assert process.stdin is not None and process.stdout is not None
    chunks: queue.Queue[bytes | None] | None = None
    reader = None
    selector = None
    if os.name == "nt":
        chunks = queue.Queue()
        reader = threading.Thread(target=SMOKE.read_chunks, args=(process.stdout, chunks), daemon=True)
        reader.start()
    else:
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
    output = bytearray()
    scan = 0
    response_start = 0
    command_index = 0
    failure = None
    deadline = time.monotonic() + float(environment.get("NORX_COREUTILS_SMOKE_TIMEOUT", "150"))

    try:
        while time.monotonic() < deadline:
            if chunks is not None:
                try:
                    chunk = chunks.get(timeout=0.1)
                except queue.Empty:
                    chunk = b""
                if chunk is None:
                    if process.poll() is not None:
                        break
                elif chunk:
                    output.extend(chunk)
            else:
                events = selector.select(timeout=0.1)
                if events:
                    chunk = os.read(process.stdout.fileno(), 4096)
                    if chunk:
                        output.extend(chunk)
                    elif process.poll() is not None:
                        break

            if command_index < len(COMMANDS):
                _name, prompt, payload, required, forbidden = COMMANDS[command_index]
                position = output.find(prompt, scan)
                if position >= 0:
                    if command_index:
                        previous = COMMANDS[command_index - 1]
                        response = output[response_start:position]
                        if previous[3] and previous[3] not in response:
                            failure = f"command {command_index}/{len(COMMANDS)} missing {previous[3]!r}"
                            break
                        if previous[4] and previous[4] in response:
                            failure = f"command {command_index}/{len(COMMANDS)} returned forbidden {previous[4]!r}"
                            break
                    scan = position + len(prompt)
                    response_start = scan
                    SMOKE.send_slowly(process, payload)
                    command_index += 1
                    if command_index == len(COMMANDS):
                        response_start = len(output)
            elif COMMANDS[-1][3] and COMMANDS[-1][3] in output[response_start:]:
                break
    finally:
        if selector is not None:
            selector.close()
        if process.poll() is None:
            process.kill()
        process.wait()

    if failure:
        print(f"coreutils smoke response check failed: {failure}", file=sys.stderr)
        sys.stdout.buffer.write(output)
        return 1
    if command_index != len(COMMANDS):
        print(f"coreutils smoke stopped before command {command_index + 1}/{len(COMMANDS)}", file=sys.stderr)
        sys.stdout.buffer.write(output)
        return 1
    terminal_marker = COMMANDS[-1][3]
    if terminal_marker and terminal_marker not in output[response_start:]:
        print(f"coreutils smoke missing final response {terminal_marker!r}", file=sys.stderr)
        sys.stdout.buffer.write(output)
        return 1
    missing = [marker for marker in REQUIRED_OUTPUT if marker not in output]
    if missing:
        print(f"coreutils smoke missing markers: {missing}", file=sys.stderr)
        sys.stdout.buffer.write(output)
        return 1

    sys.stdout.buffer.write(output)
    print(f"coreutils {arch} smoke passed; serial halt reached")
    return 124


if __name__ == "__main__":
    raise SystemExit(main())
