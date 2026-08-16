#!/usr/bin/env python3
"""Create the bounded raw FAT32 image used by the persistence smoke."""

from __future__ import annotations

import argparse
import struct
from pathlib import Path


SECTOR_SIZE = 512
DEFAULT_SECTORS = 128 * 1024
RESERVED_SECTORS = 32
FAT_COUNT = 2
SECTORS_PER_CLUSTER = 1
PERSISTENCE_CLUSTER = 3
SEED_CLUSTER = 4
PERSISTENCE_SIZE = SECTOR_SIZE


def put_u16(buffer: bytearray, offset: int, value: int) -> None:
    struct.pack_into("<H", buffer, offset, value)


def put_u32(buffer: bytearray, offset: int, value: int) -> None:
    struct.pack_into("<I", buffer, offset, value)


def short_name(name: str) -> bytes:
    parts = name.upper().split(".")
    if len(parts) != 2 or not parts[0] or not parts[1] or len(parts[0]) > 8 or len(parts[1]) > 3:
        raise ValueError("seed name must be an 8.3 filename")
    if any(not (char.isalnum() or char in "_$~-") for char in "".join(parts)):
        raise ValueError("seed name contains unsupported characters")
    return parts[0].ljust(8).encode("ascii") + parts[1].ljust(3).encode("ascii")


def build_image(
    sectors: int,
    sequence: int,
    seed_name: str | None = None,
    seed_data: bytes = b"",
) -> bytes:
    if sectors < DEFAULT_SECTORS:
        raise ValueError("the image must contain at least 128K sectors")
    fat_size = 0
    while True:
        data_sectors = sectors - RESERVED_SECTORS - FAT_COUNT * fat_size
        cluster_count = data_sectors // SECTORS_PER_CLUSTER
        next_fat_size = (cluster_count + 2 + (SECTOR_SIZE // 4) - 1) // (
            SECTOR_SIZE // 4
        )
        if next_fat_size <= fat_size:
            break
        fat_size = next_fat_size
    data_sectors = sectors - RESERVED_SECTORS - FAT_COUNT * fat_size
    cluster_count = data_sectors // SECTORS_PER_CLUSTER
    if cluster_count < 65_525:
        raise ValueError("the geometry is not FAT32-sized")
    if PERSISTENCE_CLUSTER > cluster_count + 1:
        raise ValueError("the persistence cluster is outside the data region")
    seed_clusters = (len(seed_data) + SECTOR_SIZE - 1) // SECTOR_SIZE
    if seed_name is not None:
        if not seed_data:
            raise ValueError("the seed file must not be empty")
        short_name(seed_name)
        if SEED_CLUSTER + seed_clusters - 1 > cluster_count + 1:
            raise ValueError("the seed file is too large for the image")

    image = bytearray(sectors * SECTOR_SIZE)
    boot = memoryview(image)[:SECTOR_SIZE]
    boot[3:11] = b"NORX    "
    put_u16(boot, 11, SECTOR_SIZE)
    boot[13] = SECTORS_PER_CLUSTER
    put_u16(boot, 14, RESERVED_SECTORS)
    boot[16] = FAT_COUNT
    put_u16(boot, 17, 0)
    put_u16(boot, 19, 0)
    boot[21] = 0xF8
    put_u16(boot, 22, 0)
    put_u16(boot, 24, 63)
    put_u16(boot, 26, 255)
    put_u32(boot, 28, 0)
    put_u32(boot, 32, sectors)
    put_u32(boot, 36, fat_size)
    put_u16(boot, 40, 0)
    put_u16(boot, 42, 0)
    put_u32(boot, 44, 2)
    put_u16(boot, 48, 1)
    put_u16(boot, 50, 6)
    boot[64] = 0x80
    boot[66] = 0x29
    put_u32(boot, 67, 0x4E4F5258)
    boot[71:82] = b"NORX VOL   "
    boot[82:90] = b"FAT32   "
    boot[510:512] = b"\x55\xaa"

    fat_offset = RESERVED_SECTORS * SECTOR_SIZE
    fat_bytes = fat_size * SECTOR_SIZE
    for fat_index in range(FAT_COUNT):
        offset = fat_offset + fat_index * fat_bytes
        put_u32(image, offset + 0, 0x0FFFFFF8)
        put_u32(image, offset + 4, 0xFFFFFFFF)
        put_u32(image, offset + 2 * 4, 0x0FFFFFFF)
        put_u32(image, offset + PERSISTENCE_CLUSTER * 4, 0x0FFFFFFF)
        if seed_name is not None:
            for index in range(seed_clusters):
                cluster = SEED_CLUSTER + index
                next_cluster = (
                    cluster + 1 if index + 1 < seed_clusters else 0x0FFFFFFF
                )
                put_u32(image, offset + cluster * 4, next_cluster)

    data_offset = (RESERVED_SECTORS + FAT_COUNT * fat_size) * SECTOR_SIZE
    root_offset = data_offset
    root = memoryview(image)[root_offset : root_offset + SECTOR_SIZE]
    root[0:11] = b"NORX    PST"
    root[11] = 0x20
    root[26:28] = struct.pack("<H", PERSISTENCE_CLUSTER)
    root[28:32] = struct.pack("<I", PERSISTENCE_SIZE)

    if seed_name is not None:
        seed_entry = root[32:64]
        seed_entry[0:11] = short_name(seed_name)
        seed_entry[11] = 0x20
        seed_entry[26:28] = struct.pack("<H", SEED_CLUSTER)
        seed_entry[28:32] = struct.pack("<I", len(seed_data))

    file_offset = data_offset + (PERSISTENCE_CLUSTER - 2) * SECTOR_SIZE
    image[file_offset : file_offset + 8] = struct.pack("<Q", sequence)
    if seed_name is not None:
        seed_offset = data_offset + (SEED_CLUSTER - 2) * SECTOR_SIZE
        image[seed_offset : seed_offset + len(seed_data)] = seed_data
    return bytes(image)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    parser.add_argument("--sectors", type=int, default=DEFAULT_SECTORS)
    parser.add_argument("--sequence", type=int, default=0)
    parser.add_argument("--seed-file", type=Path)
    parser.add_argument("--seed-name", default="NORXELF.ELF")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()
    if args.output.exists() and not args.force:
        parser.error(f"refusing to overwrite {args.output}; pass --force")
    if args.sequence < 0 or args.sequence >= 1 << 64:
        parser.error("--sequence must fit in an unsigned 64-bit value")
    try:
        seed_data = args.seed_file.read_bytes() if args.seed_file else b""
        data = build_image(
            args.sectors,
            args.sequence,
            args.seed_name if args.seed_file else None,
            seed_data,
        )
    except ValueError as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(data)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
