#!/usr/bin/env sh
set -eu

arch="${1:-aarch64}"
profile="${PROFILE:-dev}"
mode="${MODE:-run}"
qemu_display="${QEMU_DISPLAY:-gtk}"
qemu_serial="${QEMU_SERIAL:-stdio}"

case "$arch" in
    aarch64)
        grub_format="arm64-efi"
        boot_file="BOOTAA64.EFI"
        target="aarch64-unknown-uefi"
        kernel_name="norx.efi"
        qemu="qemu-system-aarch64"
        machine="virt"
        cpu="cortex-a57"
        firmware="edk2-aarch64-code.fd"
        vars="edk2-arm-vars.fd"
        usb_args=""
        network_args=""
        modules="normal configfile chain fat part_msdos efi_gop all_video gfxterm"
        ;;
    *)
        echo "usage: $0 [aarch64]" >&2
        exit 2
        ;;
esac

machine="${QEMU_MACHINE:-$machine}"

command -v grub-mkstandalone >/dev/null 2>&1 || {
    echo "grub-mkstandalone is required" >&2
    exit 1
}

if [ "$profile" = release ]; then
    cargo build --release --target "$target"
    kernel="target/$target/release/norx_kernel.efi"
else
    cargo build --target "$target"
    kernel="target/$target/debug/norx_kernel.efi"
fi

root="build/$arch"
esp="$root/esp"
boot_dir="$esp/EFI/BOOT"
grub_cfg="config/grub/$arch.cfg"
rm -rf "$root"
mkdir -p "$boot_dir" "$esp/boot/grub"
cp "$grub_cfg" "$esp/boot/grub/grub.cfg"

cp "$kernel" "$esp/boot/$kernel_name"
if [ "$arch" = aarch64 ] && [ -n "${NORX_DTB:-}" ]; then
    cp "$NORX_DTB" "$esp/boot/norx.dtb"
fi

grub-mkstandalone \
    -O "$grub_format" \
    -o "$boot_dir/$boot_file" \
    --modules="$modules" \
    "boot/grub/grub.cfg=$grub_cfg"

if [ "$mode" = build ]; then
    echo "$esp"
    exit 0
fi

command -v "$qemu" >/dev/null 2>&1 || {
    echo "$qemu is required to run the image" >&2
    exit 1
}

qemu_bin="$(command -v "$qemu")"
qemu_share="${QEMU_SHARE:-$(dirname "$(dirname "$(realpath "$qemu_bin")")")/share/qemu}"
if [ "$arch" = aarch64 ] && [ ! -f "$esp/boot/norx.dtb" ]; then
    "$qemu_bin" \
        -machine "$machine,dumpdtb=$esp/boot/norx.dtb" \
        -cpu "$cpu" \
        -m 256M \
        -display gtk \
        -device ramfb \
        -S \
        -no-reboot \
        -no-shutdown &
    dtb_pid=$!
    sleep 1
    kill "$dtb_pid" 2>/dev/null || true
    wait "$dtb_pid" 2>/dev/null || true
fi
vars_copy="${QEMU_VARS:-$root/$vars}"
if [ ! -f "$vars_copy" ]; then
    cp "$qemu_share/$vars" "$vars_copy"
fi

qmp_args=""
if [ -n "${QEMU_MONITOR:-}" ]; then
    qmp_args="-qmp unix:${QEMU_MONITOR},server=on,wait=off"
fi

if [ -n "${QEMU_BLOCK_IMAGE:-}" ]; then
    echo "Native ARM64 persistent block attachment is not implemented" >&2
    exit 2
fi
qemu_video_args="-device ramfb"

set -- \
    -M "${QEMU_MACHINE:-$machine}" \
    -cpu "$cpu" \
    -m 256M \
    -display "$qemu_display" \
    $qmp_args \
    $qemu_video_args \
    $usb_args \
    $network_args
set -- "$@" \
    -serial "$qemu_serial" \
    -no-reboot \
    -no-shutdown \
    -drive "if=pflash,format=raw,readonly=on,file=$qemu_share/$firmware" \
    -drive "if=pflash,format=raw,file=$vars_copy" \
    -drive "format=raw,file=fat:rw:$esp"
exec "$qemu" "$@"
