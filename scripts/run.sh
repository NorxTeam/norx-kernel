#!/usr/bin/env sh
set -eu

arch="${1:-x86_64}"
profile="${PROFILE:-dev}"
mode="${MODE:-run}"
qemu_display="${QEMU_DISPLAY:-gtk}"
qemu_serial="${QEMU_SERIAL:-stdio}"

case "$arch" in
    x86_64)
        target="x86_64-unknown-none"
        grub_format="x86_64-efi"
        boot_file="BOOTX64.EFI"
        kernel_name="norx.elf"
        qemu="qemu-system-x86_64"
        machine="q35"
        cpu="max"
        firmware="edk2-x86_64-code.fd"
        vars="edk2-i386-vars.fd"
        usb_args="-device qemu-xhci,id=xhci"
        network_args="-netdev user,id=net0 -device virtio-net-pci,netdev=net0,disable-modern=on"
        modules="normal configfile multiboot2 fat part_msdos efi_gop all_video gfxterm"
        ;;
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
        echo "usage: $0 [x86_64|aarch64]" >&2
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
    if [ "$arch" = aarch64 ]; then
        kernel="target/$target/release/norx_kernel.efi"
    else
        kernel="target/$target/release/norx_kernel"
    fi
else
    cargo build --target "$target"
    if [ "$arch" = aarch64 ]; then
        kernel="target/$target/debug/norx_kernel.efi"
    else
        kernel="target/$target/debug/norx_kernel"
    fi
fi

root="build/$arch"
esp="$root/esp"
boot_dir="$esp/EFI/BOOT"
grub_cfg="config/grub/$arch.cfg"
rm -rf "$root"
mkdir -p "$boot_dir" "$esp/boot/grub"
cp "$grub_cfg" "$esp/boot/grub/grub.cfg"

cp "$kernel" "$esp/boot/$kernel_name"
if [ "$arch" = x86_64 ] && command -v grub-file >/dev/null 2>&1; then
    grub-file --is-x86-multiboot2 "$kernel"
fi
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

block_args=()
if [ -n "${QEMU_BLOCK_IMAGE:-}" ]; then
    if [ ! -f "$QEMU_BLOCK_IMAGE" ]; then
        echo "QEMU_BLOCK_IMAGE does not exist: $QEMU_BLOCK_IMAGE" >&2
        exit 1
    fi
    block_args=(
        -drive "if=none,id=norx-persist,format=raw,file=$QEMU_BLOCK_IMAGE"
        -device virtio-blk-pci,drive=norx-persist,disable-legacy=on
    )
fi

if [ "$arch" = x86_64 ]; then
    qemu_video_args="-vga none -device virtio-vga,edid=on,xres=1200,yres=800"
else
    qemu_video_args="-device ramfb"
fi

exec "$qemu" \
    -M "${QEMU_MACHINE:-$machine}" \
    -cpu "$cpu" \
    -m 256M \
    -display "$qemu_display" \
    $qmp_args \
    $qemu_video_args \
    $usb_args \
    $network_args \
    "${block_args[@]}" \
    -serial "$qemu_serial" \
    -no-reboot \
    -no-shutdown \
    -drive "if=pflash,format=raw,readonly=on,file=$qemu_share/$firmware" \
    -drive "if=pflash,format=raw,file=$vars_copy" \
    -drive "format=raw,file=fat:rw:$esp"
