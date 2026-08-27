"""Run the target login/session smoke fixture under QEMU."""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[2]


def main() -> int:
    target = sys.argv[1] if len(sys.argv) > 1 else "x86_64"
    if target not in {"x86_64", "aarch64"}:
        raise SystemExit(f"unsupported target: {target}")
    env = os.environ.copy()
    env["RUN_LOGIN_SMOKE"] = "1"
    env["NORX_SMOKE_COMMAND"] = "/bin/login-smoke"
    env["NORX_SMOKE_EXPECT"] = (
        "LOGIN:BAD_CREDENTIALS\n"
        "LOGIN:LOCKED\n"
        "LOGIN:EXPIRED\n"
        "LOGIN:AUTH_SUCCESS\n"
        "LOGIN:ENV_OK\n"
        "LOGIN:CWD_OK\n"
        "LOGIN:UMASK_OK\n"
        "LOGIN:CREDENTIALS_OK\n"
        "LOGIN:SHELL_EXIT\n"
        "LOGIN:SESSION_RECLAIMED\n"
    )
    command = [sys.executable, str(ROOT / "norx-kernel" / "scripts" / "nsh-smoke.py"), target]
    result = subprocess.run(command, cwd=ROOT, env=env, check=False)
    if result.returncode not in {0, 124}:
        return result.returncode
    print(f"login {target} smoke passed; serial halt reached")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
