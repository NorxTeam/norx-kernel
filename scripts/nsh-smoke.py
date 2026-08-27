#!/usr/bin/env python3
"""Drive the freestanding nsh smoke scenario through QEMU's serial stdin."""

from __future__ import annotations

import os
from pathlib import Path
import selectors
import subprocess
import sys
import queue
import threading
import time


COMMANDS = (
    # prompt, input, required response marker, forbidden response marker
    (b"norx:/$ ", b"pwd\n", b"/", None),
    (b"norx:/$ ", b'cd "/quoted dir"\n', None, b"nsh:"),
    (b"norx:/quoted dir$ ", b"pwd\n", b"/quoted dir", None),
    (b"norx:/quoted dir$ ", b"set 'HOME=/single value'\n", None, b"nsh:"),
    (b"norx:/quoted dir$ ", b'cd "$HOME"\n', None, b"nsh:"),
    (b"norx:/single value$ ", b"pwd\n", b"/single value", None),
    (b"norx:/single value$ ", b"partial\x03", b"^C", None),
    (b"norx:/single value$ ", b"pwd > /out\n", None, b"redirection"),
    (b"norx:/single value$ ", b"pwd < /out\n", None, b"redirection"),
    (b"norx:/single value$ ", b"rust-smoke\n", b"userspace: rust-write", None),
    (b"norx:/single value$ ", b"FOO=bar env\n", b"FOO=bar", None),
    (b"norx:/single value$ ", b"echo PIPELINE_OK | cat\n", b"PIPELINE_OK", None),
    (b"norx:/single value$ ", b"pwd &\n", b"background jobs require a process-group backend", None),
    (b"norx:/single value$ ", b"jobs\n", None, b"nsh:"),
    (b"norx:/single value$ ", b"fg %1\n", b"nsh: invalid job reference", None),
    (b"norx:/single value$ ", b'bad "quote\n', b"nsh: parse: unclosed quote", None),
    (
        b"norx:/single value$ ",
        b"missing-command\n",
        b"nsh: command not found: missing-command",
        None,
    ),
    (
        b"norx:/single value$ ",
        b'source "/etc/profile"\n',
        b"source: /etc/profile: filesystem backend is unavailable",
        None,
    ),
    (
        b"norx:/single value$ ",
        b'exec "/bin/app"\n',
        b"exec: /bin/app: process replacement is unavailable",
        None,
    ),
    (
        b"norx:/single value$ ",
        b"exit 0\n",
        b"nsh: external userspace ELF exited cleanly",
        None,
    ),
)
REQUIRED_OUTPUT = (
    b"quickinit: PID 1 hand-off complete; child reaped",
)


def send_slowly(process: subprocess.Popen[bytes], payload: bytes) -> None:
    assert process.stdin is not None
    for byte in payload:
        process.stdin.write(bytes((byte,)))
        process.stdin.flush()
        time.sleep(0.05)


def run_marker_smoke(arch: str, environment: dict[str, str]) -> int:
    """Run a non-interactive fixture and require every stable serial marker."""
    root = Path(__file__).resolve().parent.parent
    if os.name == "nt":
        try:
            command = windows_qemu_command(root, arch)
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
    assert process.stdout is not None
    chunks: queue.Queue[bytes | None] = queue.Queue()
    reader = threading.Thread(target=read_chunks, args=(process.stdout, chunks), daemon=True)
    reader.start()
    output = bytearray()
    expected = [marker.encode() for marker in environment["NORX_SMOKE_EXPECT"].splitlines() if marker]
    deadline = time.monotonic() + float(os.environ.get("NORX_SHELL_SMOKE_TIMEOUT", "90"))
    try:
        while time.monotonic() < deadline and process.poll() is None:
            try:
                chunk = chunks.get(timeout=0.1)
            except queue.Empty:
                continue
            if chunk:
                output.extend(chunk)
            if all(marker in output for marker in expected):
                break
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
    reader.join(timeout=1)
    while True:
        try:
            chunk = chunks.get_nowait()
        except queue.Empty:
            break
        if chunk:
            output.extend(chunk)
    sys.stdout.buffer.write(output)
    missing = [marker.decode() for marker in expected if marker not in output]
    if missing:
        print(f"marker smoke missing: {missing}", file=sys.stderr)
        return 1
    return 0


def read_chunks(stream, chunks: queue.Queue[bytes | None]) -> None:
    fd = stream.fileno()
    while True:
        chunk = os.read(fd, 4096)
        if not chunk:
            chunks.put(None)
            return
        chunks.put(chunk)


def windows_qemu_command(root: Path, arch: str) -> list[str]:
    qemu_name = f"qemu-system-{arch}.exe"
    qemu_value = os.environ.get("QEMU_BIN")
    qemu = Path(qemu_value) if qemu_value else Path()
    if not qemu_value:
        qemu = Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "qemu" / qemu_name
    if not qemu.is_file():
        raise RuntimeError(f"QEMU executable not found: {qemu}")
    share_value = os.environ.get("QEMU_SHARE")
    qemu_share = Path(share_value) if share_value else Path()
    if not share_value:
        qemu_share = qemu.parent / "share"
    firmware = qemu_share / ("edk2-x86_64-code.fd" if arch == "x86_64" else "edk2-aarch64-code.fd")
    if arch == "x86_64":
        esp = root / "build" / "x86_64" / "esp"
        vars_file = root / "build" / "x86_64" / "nsh-smoke-vars.fd"
        if not vars_file.is_file():
            vars_file = root / "build" / "x86_64" / "edk2-i386-vars.fd"
    else:
        esp = root / "build" / "aarch64-current" / "esp"
        vars_file = root / "build" / "aarch64-current" / "vars.fd"
        if not vars_file.is_file():
            vars_file = root / "build" / "lifecycle-aarch64" / "final-vars.fd"
    for path in (firmware, esp, vars_file):
        if not path.exists():
            raise RuntimeError(f"QEMU smoke input not found: {path}")
    machine = "q35" if arch == "x86_64" else "virt"
    cpu = "max" if arch == "x86_64" else "cortex-a57"
    command = [
        str(qemu),
        "-M", machine,
        "-cpu", cpu,
        "-m", os.environ.get("QEMU_MEMORY", "256M"),
        "-display", os.environ.get("QEMU_DISPLAY", "none"),
        "-no-reboot",
        "-no-shutdown",
    ]
    accel = os.environ.get("QEMU_ACCEL")
    if accel:
        command += ["-accel", accel]
    minimal_devices = os.environ.get("QEMU_MINIMAL_DEVICES") == "1"
    if arch == "x86_64" and not minimal_devices:
        command += [
            "-vga", "none",
            "-device", "virtio-vga,edid=on,xres=1200,yres=800",
            "-device", "qemu-xhci,id=xhci",
            "-netdev", "user,id=net0",
            "-device", "virtio-net-pci,netdev=net0,disable-modern=on",
        ]
    elif arch != "x86_64" and not minimal_devices:
        command += ["-device", "ramfb"]
    command += [
        "-serial", "stdio",
        "-drive", f"if=pflash,format=raw,readonly=on,file={firmware}",
        "-drive", f"if=pflash,format=raw,file={vars_file}",
        "-drive", f"format=raw,file=fat:rw:{esp}",
    ]
    return command


def main() -> int:
    arch = sys.argv[1] if len(sys.argv) > 1 else "x86_64"
    if arch not in {"x86_64", "aarch64"}:
        print(f"usage: {sys.argv[0]} [x86_64|aarch64]", file=sys.stderr)
        return 2

    root = Path(__file__).resolve().parent.parent
    environment = os.environ.copy()
    environment.update(
        RUN_NSH_SMOKE="1",
        REQUIRE_USERSPACE_FIXTURE="1",
        QEMU_DISPLAY="none",
    )
    if environment.get("NORX_SMOKE_COMMAND") and environment.get("NORX_SMOKE_EXPECT"):
        return run_marker_smoke(arch, environment)
    if os.name == "nt":
        try:
            command = windows_qemu_command(root, arch)
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
    assert process.stdout is not None
    selector = None
    chunks: queue.Queue[bytes | None] | None = None
    reader = None
    if os.name == "nt":
        chunks = queue.Queue()
        reader = threading.Thread(target=read_chunks, args=(process.stdout, chunks), daemon=True)
        reader.start()
    else:
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
    output = bytearray()
    scan = 0
    command_index = 0
    response_start = 0
    response_failure = None
    killed_for_timeout = False
    deadline = time.monotonic() + float(os.environ.get("NORX_SHELL_SMOKE_TIMEOUT", "90"))

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
                prompt, payload, required, forbidden = COMMANDS[command_index]
                position = output.find(prompt, scan)
                if position >= 0:
                    if command_index != 0:
                        previous = COMMANDS[command_index - 1]
                        response = output[response_start:position]
                        if previous[2] and previous[2] not in response:
                            response_failure = (
                                f"command {command_index}/{len(COMMANDS)} missing response "
                                f"{previous[2]!r}"
                            )
                            break
                        if previous[3] and previous[3] in response:
                            response_failure = (
                                f"command {command_index}/{len(COMMANDS)} returned forbidden "
                                f"response {previous[3]!r}"
                            )
                            break
                    scan = position + len(prompt)
                    response_start = scan
                    send_slowly(process, payload)
                    command_index += 1
                    if command_index == len(COMMANDS):
                        response_start = len(output)
            elif COMMANDS[-1][2] and COMMANDS[-1][2] in output[response_start:]:
                break

            if process.poll() is not None:
                break
    finally:
        if selector is not None:
            selector.close()
        if process.poll() is None:
            killed_for_timeout = True
            process.kill()
        process.wait()

    if response_failure:
        print(f"nsh smoke response check failed: {response_failure}", file=sys.stderr)
        sys.stdout.buffer.write(output)
        return 1
    if command_index != len(COMMANDS):
        print(f"nsh smoke stopped before command {command_index + 1}/{len(COMMANDS)}", file=sys.stderr)
        sys.stdout.buffer.write(output)
        return 1
    terminal_marker = COMMANDS[-1][2]
    if terminal_marker and terminal_marker not in output[response_start:]:
        print(f"nsh smoke missing final response {terminal_marker!r}", file=sys.stderr)
        sys.stdout.buffer.write(output)
        return 1
    missing = [marker for marker in REQUIRED_OUTPUT if marker not in output]
    if missing:
        print(f"nsh smoke missing markers: {missing}", file=sys.stderr)
        sys.stdout.buffer.write(output)
        return 1
    if not killed_for_timeout:
        print(f"nsh smoke expected QEMU to remain halted, got exit={process.returncode}", file=sys.stderr)
        sys.stdout.buffer.write(output)
        return 1

    sys.stdout.buffer.write(output)
    print(f"nsh {arch} smoke passed; serial halt reached")
    return 124


if __name__ == "__main__":
    raise SystemExit(main())
