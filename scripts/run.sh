#!/usr/bin/env sh
set -eu

arch="${1:-x86_64}"
profile="${PROFILE:-dev}"
mode="${MODE:-run}"

case "$arch" in
    x86_64)
        target="x86_64-unknown-none"
        grub_format="x86_64-efi"
        boot_file="BOOTX64.EFI"
        kernel_name="norx.elf"
        qemu="qemu-system-x86_64"
        machine="q35"
        firmware="edk2-x86_64-code.fd"
        vars="edk2-i386-vars.fd"
        modules="normal configfile multiboot2 fat part_msdos efi_gop all_video gfxterm"
        ;;
    aarch64)
        grub_format="arm64-efi"
        boot_file="BOOTAA64.EFI"
        target="aarch64-unknown-uefi"
        kernel_name="norx.efi"
        qemu="qemu-system-aarch64"
        machine="virt"
        firmware="edk2-aarch64-code.fd"
        vars="edk2-arm-vars.fd"
        modules="normal configfile chain fat part_msdos efi_gop all_video gfxterm"
        ;;
    *)
        echo "usage: $0 [x86_64|aarch64]" >&2
        exit 2
        ;;
esac

command -v grub-mkstandalone >/dev/null 2>&1 || {
    echo "grub-mkstandalone is required" >&2
    exit 1
}

if [ "$profile" = release ]; then
    cargo build --release --target "$target"
    kernel="target/$target/release/norx_kernel"
else
    cargo build --target "$target"
    kernel="target/$target/debug/norx_kernel"
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
        -m 256M \
        -display none \
        -S \
        -no-reboot \
        -no-shutdown &
    dtb_pid=$!
    sleep 1
    kill "$dtb_pid" 2>/dev/null || true
    wait "$dtb_pid" 2>/dev/null || true
fi
vars_copy="$root/$vars"
cp "$qemu_share/$vars" "$vars_copy"

if [ "$arch" = x86_64 ]; then
    qemu_video_args="-vga none -device virtio-vga,edid=on,xres=1200,yres=800"
else
    qemu_video_args=""
fi

exec "$qemu" \
    -M "${QEMU_MACHINE:-$machine}" \
    -m 256M \
    -display none \
    $qemu_video_args \
    -serial stdio \
    -no-reboot \
    -no-shutdown \
    -drive "if=pflash,format=raw,readonly=on,file=$qemu_share/$firmware" \
    -drive "if=pflash,format=raw,file=$vars_copy" \
    -drive "format=raw,file=fat:rw:$esp"
