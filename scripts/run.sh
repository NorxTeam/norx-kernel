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
        modules="normal configfile multiboot2 fat"
        ;;
    aarch64)
        target="aarch64-unknown-none-softfloat"
        grub_format="arm64-efi"
        boot_file="BOOTAA64.EFI"
        kernel_name="norx.img"
        qemu="qemu-system-aarch64"
        machine="virt"
        firmware="edk2-aarch64-code.fd"
        vars="edk2-arm-vars.fd"
        modules="normal configfile linux fat"
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

if [ "$arch" = aarch64 ]; then
    objcopy="${OBJCOPY:-}"
    if [ -z "$objcopy" ]; then
        for candidate in rust-objcopy llvm-objcopy objcopy; do
            if command -v "$candidate" >/dev/null 2>&1; then
                objcopy="$candidate"
                break
            fi
        done
    fi
    if [ -z "$objcopy" ]; then
        echo "an objcopy compatible with LLVM binary output is required for aarch64" >&2
        exit 1
    fi
    "$objcopy" --set-section-flags .bss=alloc,load,contents -O binary "$kernel" "$esp/boot/$kernel_name"
else
    cp "$kernel" "$esp/boot/$kernel_name"
    if command -v grub-file >/dev/null 2>&1; then
        grub-file --is-x86-multiboot2 "$kernel"
    fi
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
vars_copy="$root/$vars"
cp "$qemu_share/$vars" "$vars_copy"

exec "$qemu" \
    -M "${QEMU_MACHINE:-$machine}" \
    -m 256M \
    -display none \
    -serial stdio \
    -no-reboot \
    -no-shutdown \
    -drive "if=pflash,format=raw,readonly=on,file=$qemu_share/$firmware" \
    -drive "if=pflash,format=raw,file=$vars_copy" \
    -drive "format=raw,file=fat:rw:$esp"
