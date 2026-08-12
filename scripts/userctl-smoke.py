#!/usr/bin/env python3
"""Run the Norx userctl administrative-session smoke through nsh and QEMU."""

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
COMMANDS = (
    ("marker", b"norx:/$ ", b"echo USERCTL:SHELL_OK\n", b"USERCTL:SHELL_OK"),
    ("marker", b"norx:/$ ", b"echo USERCTL:OK\n", b"USERCTL:OK"),
    ("exit", b"norx:/$ ", b"exit 0\n", b"nsh: external userspace ELF exited cleanly"),
)
REQUIRED_OUTPUT = (
    b"quickinit: PID 1 hand-off complete; child reaped",
    b"USERCTL:ADMIN_SESSION_OK",
)


def main() -> int:
    arch = sys.argv[1] if len(sys.argv) > 1 else "x86_64"
    if arch not in {"x86_64", "aarch64"}:
        print(f"usage: {sys.argv[0]} [x86_64|aarch64]", file=sys.stderr)
        return 2
    root = Path(__file__).resolve().parent.parent
    environment = os.environ.copy()
    environment.update(
        RUN_NSH_SMOKE="1",
        RUN_USERCTL_SMOKE="1",
        REQUIRE_USERSPACE_FIXTURE="1",
        QEMU_DISPLAY="none",
    )
    environment["NORX_USERCTL_SMOKE_EXPECT"] = "\n".join(
        marker.decode("ascii") for marker in REQUIRED_OUTPUT
    )
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
    selector = None
    if os.name == "nt":
        chunks = queue.Queue()
        threading.Thread(target=SMOKE.read_chunks, args=(process.stdout, chunks), daemon=True).start()
    else:
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
    output = bytearray()
    scan = 0
    response_start = 0
    command_index = 0
    failure = None
    deadline = time.monotonic() + float(environment.get("NORX_USERCTL_SMOKE_TIMEOUT", "180"))
    try:
        while time.monotonic() < deadline:
            if chunks is not None:
                try:
                    chunk = chunks.get(timeout=0.1)
                except queue.Empty:
                    chunk = b""
                if chunk is None and process.poll() is not None:
                    break
                if chunk:
                    output.extend(chunk)
            else:
                events = selector.select(timeout=0.1)
                if events:
                    chunk = os.read(process.stdout.fileno(), 4096)
                    if chunk:
                        output.extend(chunk)
                    elif process.poll() is not None:
                        break
            if command_index >= len(COMMANDS):
                break
            _, prompt, payload, _ = COMMANDS[command_index]
            position = output.find(prompt, scan)
            if position < 0:
                continue
            if command_index:
                previous = COMMANDS[command_index - 1]
                response = output[response_start:position]
                if previous[3] not in response:
                    failure = f"command {command_index}/{len(COMMANDS)} missing {previous[3]!r}"
                    break
            scan = position + len(prompt)
            response_start = scan
            SMOKE.send_slowly(process, payload)
            command_index += 1
    finally:
        if selector is not None:
            selector.close()
        if process.poll() is None:
            process.kill()
        process.wait()
    if failure:
        print(f"userctl smoke response check failed: {failure}", file=sys.stderr)
        sys.stdout.buffer.write(output)
        return 1
    if command_index != len(COMMANDS):
        print(f"userctl smoke stopped before command {command_index + 1}/{len(COMMANDS)}", file=sys.stderr)
        sys.stdout.buffer.write(output)
        return 1
    missing = [marker for marker in REQUIRED_OUTPUT if marker not in output]
    if missing:
        print(f"userctl smoke missing markers: {missing}", file=sys.stderr)
        sys.stdout.buffer.write(output)
        return 1
    sys.stdout.buffer.write(output)
    print(f"userctl {arch} smoke passed; serial halt reached")
    return 124


if __name__ == "__main__":
    raise SystemExit(main())
