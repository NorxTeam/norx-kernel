#!/usr/bin/env python3
"""Bounded ARM64 UEFI serial smoke without requiring GRUB."""
import argparse
import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=45)
    args = parser.parse_args()
    if not 1 <= args.timeout <= 60:
        parser.error("timeout must be between 1 and 60 seconds")
    root = Path(__file__).resolve().parent.parent
    qemu = shutil.which("qemu-system-aarch64")
    if qemu is None:
        parser.error("qemu-system-aarch64 is required")
    share = Path(os.environ.get("QEMU_SHARE", Path(qemu).resolve().parent.parent / "share/qemu"))
    code, variables = share / "edk2-aarch64-code.fd", share / "edk2-arm-vars.fd"
    if not code.is_file() or not variables.is_file():
        parser.error("ARM EDK2 firmware missing; set QEMU_SHARE")
    subprocess.run(["cargo", "build", "--locked", "--target", "aarch64-unknown-uefi"], cwd=root, check=True)
    args.log = args.log.resolve()
    args.log.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="norx-arm-efi-") as directory:
        stage = Path(directory)
        esp = stage / "esp"
        (esp / "EFI/BOOT").mkdir(parents=True)
        (esp / "boot").mkdir()
        shutil.copyfile(root / "target/aarch64-unknown-uefi/debug/norx_kernel.efi", esp / "EFI/BOOT/BOOTAA64.EFI")
        shutil.copyfile(variables, stage / "vars.fd")
        subprocess.run([qemu, "-machine", f"virt,dumpdtb={esp / 'boot/norx.dtb'}", "-cpu", "cortex-a57", "-m", "256M", "-display", "none", "-device", "ramfb"], check=True)
        command = [qemu, "-machine", "virt", "-cpu", "cortex-a57", "-m", "256M", "-display", "none", "-device", "ramfb", "-drive", f"if=pflash,format=raw,readonly=on,file={code}", "-drive", f"if=pflash,format=raw,file={stage / 'vars.fd'}", "-drive", f"format=raw,file=fat:rw:{esp}", "-serial", f"file:{args.log}", "-monitor", "none", "-no-reboot"]
        args.log.unlink(missing_ok=True)
        try:
            subprocess.run(command, timeout=args.timeout, check=True)
        except subprocess.TimeoutExpired:
            pass
    serial = args.log.read_text(errors="replace")
    markers = ["NORX_EARLY_UART_READY", "NORX_DRIVER_IRQ_CONTRACT_OK", "NORX_BLOCK_REQUEST_CONTRACT_OK", "NORX_PACKAGE_TRUST_OK", "NORX_AARCH64_GIC_TIMER_IRQ_OK", "ramfs /hello.txt readback passed", "physical allocator ownership and return checks passed", "NORX_SERIAL_DEBUGGER_READY"]
    missing = [marker for marker in markers if marker not in serial]
    if "panic: panicked" in serial or re.search(r"NORX_[A-Z_]+_FAIL", serial) or missing:
        raise SystemExit(f"ARM EFI smoke failed; missing={missing}; inspect {args.log}")
    print(f"ARM EFI serial smoke passed: {args.log}")


if __name__ == "__main__":
    main()
