#!/usr/bin/env sh
set -eu

arch="${1:-x86_64}"
profile="${PROFILE:-dev}"
mode="${MODE:-debug}"

case "$arch" in
    x86_64)
        target="x86_64-unknown-uefi"
        qemu="qemu-system-x86_64"
        qemu_args="-M q35 -m 256M -serial stdio -no-reboot -no-shutdown"
        firmware="edk2-x86_64-code.fd"
        vars="edk2-i386-vars.fd"
        boot="BOOTX64.EFI"
        ;;
    aarch64)
        target="aarch64-unknown-uefi"
        qemu="qemu-system-aarch64"
        qemu_args="-M virt -cpu cortex-a72 -m 256M -serial stdio -device ramfb -no-reboot -no-shutdown"
        firmware="edk2-aarch64-code.fd"
        vars="edk2-arm-vars.fd"
        boot="BOOTAA64.EFI"
        ;;
    *)
        echo "usage: $0 [x86_64|aarch64]" >&2
        exit 2
        ;;
esac

qemu_bin="$(command -v "$qemu")"
qemu_share="$(dirname "$(dirname "$(realpath "$qemu_bin")")")/share/qemu"
vars_copy="build/$arch/$vars"

if [ "$profile" = release ]; then
    cargo build --release --target "$target"
    kernel="target/$target/release/norx_kernel.efi"
else
    cargo build --target "$target"
    kernel="target/$target/debug/norx_kernel.efi"
fi

esp="build/$arch/esp"
boot_dir="$esp/EFI/BOOT"
rm -rf "$esp"
mkdir -p "$boot_dir"
cp "$kernel" "$boot_dir/$boot"
cp "$qemu_share/$vars" "$vars_copy"

if [ "$mode" = build ]; then
    echo "$esp"
    exit 0
fi

exec "$qemu" $qemu_args \
    -drive "if=pflash,format=raw,readonly=on,file=$qemu_share/$firmware" \
    -drive "if=pflash,format=raw,file=$vars_copy" \
    -drive "format=raw,file=fat:rw:$esp"
