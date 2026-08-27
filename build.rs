use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=linker/x86_64.ld");
    println!("cargo:rerun-if-changed=linker/aarch64.ld");
    println!("cargo:rerun-if-env-changed=ROOTFS_PATH");
    println!("cargo:rerun-if-env-changed=REQUIRE_USERSPACE_FIXTURE");
    println!("cargo:rerun-if-env-changed=RUN_NSH_SMOKE");
    println!("cargo:rerun-if-env-changed=RUN_LOGIN_SMOKE");
    println!("cargo:rerun-if-env-changed=RUN_PASSWD_SMOKE");
    println!("cargo:rerun-if-env-changed=RUN_USERCTL_SMOKE");
    println!("cargo:rerun-if-env-changed=RUN_SUDO_SMOKE");
    println!("cargo:rerun-if-env-changed=REQUIRE_COREUTILS_FIXTURE");

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let (triple, machine, code) = match target_arch.as_str() {
        "x86_64" => (
            "x86_64-unknown-norx",
            62u16,
            vec![
                0xb8, 0x3c, 0x00, 0x00, 0x00, 0xbf, 0x2a, 0x00, 0x00, 0x00, 0x0f, 0x05,
            ],
        ),
        "aarch64" => (
            "aarch64-unknown-norx",
            183u16,
            vec![
                0x88, 0x07, 0x80, 0xd2, 0x40, 0x05, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4,
            ],
        ),
        other => panic!("unsupported userspace fixture architecture: {other}"),
    };
    let rootfs = env::var_os("ROOTFS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("..").join("test-rootfs"));
    let require_fixture = env::var_os("REQUIRE_USERSPACE_FIXTURE").is_some();

    let fixtures = [
        (
            "quickinit",
            rootfs
                .join("tests")
                .join("quickinit")
                .join(triple)
                .join("quickinit.elf"),
            "quickinit.elf",
        ),
        (
            "rust",
            rootfs
                .join("tests")
                .join("rust")
                .join(triple)
                .join("userspace-smoke.elf"),
            "userspace-smoke.elf",
        ),
        (
            "c",
            rootfs
                .join("tests")
                .join("runtime")
                .join(triple)
                .join("runtime-c.elf"),
            "userspace-c.elf",
        ),
        (
            "cxx",
            rootfs
                .join("tests")
                .join("runtime")
                .join(triple)
                .join("runtime-cxx.elf"),
            "userspace-cxx.elf",
        ),
        (
            "nsh",
            rootfs
                .join("tests")
                .join("nsh")
                .join(triple)
                .join("nsh.elf"),
            "nsh.elf",
        ),
        (
            "coreutils",
            rootfs
                .join("tests")
                .join("coreutils")
                .join(triple)
                .join("coreutils.elf"),
            "coreutils.elf",
        ),
        (
            "userdb",
            rootfs
                .join("tests")
                .join("userdb")
                .join(triple)
                .join("userdb-smoke.elf"),
            "userdb-smoke.elf",
        ),
        (
            "getty",
            rootfs
                .join("tests")
                .join("getty")
                .join(triple)
                .join("getty-smoke.elf"),
            "getty-smoke.elf",
        ),
        (
            "login",
            rootfs
                .join("tests")
                .join("login")
                .join(triple)
                .join("login-smoke.elf"),
            "login-smoke.elf",
        ),
        (
            "passwd",
            rootfs
                .join("tests")
                .join("passwd")
                .join(triple)
                .join("passwd-smoke.elf"),
            "passwd-smoke.elf",
        ),
        (
            "userctl",
            rootfs
                .join("tests")
                .join("userctl")
                .join(triple)
                .join("userctl-smoke.elf"),
            "userctl-smoke.elf",
        ),
        (
            "sudo",
            rootfs
                .join("tests")
                .join("sudo")
                .join(triple)
                .join("sudo-smoke.elf"),
            "sudo-smoke.elf",
        ),
    ];

    for (name, source, destination_name) in fixtures {
        let destination = output.join(destination_name);
        let variable = format!("{}_FIXTURE", name.to_uppercase());
        println!("cargo:rerun-if-changed={}", source.display());
        if source.is_file() {
            fs::copy(&source, &destination).unwrap_or_else(|error| {
                panic!("copying userspace fixture {}: {error}", source.display())
            });
            println!("cargo:rustc-env={variable}=external");
        } else if require_fixture {
            panic!(
                "required userspace fixture is missing: {}",
                source.display()
            );
        } else {
            fs::write(&destination, fallback_image(machine, &code)).unwrap();
            println!("cargo:rustc-env={variable}=fallback");
            println!(
                "cargo:warning={name} userspace fixture missing; embedding the bounded fallback exit image"
            );
        }
    }

    if env::var_os("RUN_NSH_SMOKE").is_some() {
        println!("cargo:rustc-env=NSH_SMOKE=enabled");
    } else {
        println!("cargo:rustc-env=NSH_SMOKE=disabled");
    }
    if env::var_os("RUN_LOGIN_SMOKE").is_some() {
        println!("cargo:rustc-env=LOGIN_SMOKE=enabled");
    } else {
        println!("cargo:rustc-env=LOGIN_SMOKE=disabled");
    }
    if env::var_os("RUN_PASSWD_SMOKE").is_some() {
        println!("cargo:rustc-env=PASSWD_SMOKE=enabled");
    } else {
        println!("cargo:rustc-env=PASSWD_SMOKE=disabled");
    }
    if env::var_os("RUN_USERCTL_SMOKE").is_some() {
        println!("cargo:rustc-env=USERCTL_SMOKE=enabled");
    } else {
        println!("cargo:rustc-env=USERCTL_SMOKE=disabled");
    }
    if env::var_os("RUN_SUDO_SMOKE").is_some() {
        println!("cargo:rustc-env=SUDO_SMOKE=enabled");
    } else {
        println!("cargo:rustc-env=SUDO_SMOKE=disabled");
    }
}

fn fallback_image(machine: u16, code: &[u8]) -> Vec<u8> {
    const PAGE_SIZE: usize = 4096;
    const HEADER_SIZE: usize = 64;
    const PROGRAM_HEADER_SIZE: usize = 56;
    let mut image = vec![0u8; PAGE_SIZE + code.len()];
    image[0..4].copy_from_slice(b"\x7fELF");
    image[4] = 2;
    image[5] = 1;
    image[6] = 1;
    write_u16(&mut image, 16, 2);
    write_u16(&mut image, 18, machine);
    write_u32(&mut image, 20, 1);
    write_u64(&mut image, 24, 0x400000);
    write_u64(&mut image, 32, HEADER_SIZE as u64);
    write_u16(&mut image, 52, HEADER_SIZE as u16);
    write_u16(&mut image, 54, PROGRAM_HEADER_SIZE as u16);
    write_u16(&mut image, 56, 1);
    write_u32(&mut image, HEADER_SIZE, 1);
    write_u32(&mut image, HEADER_SIZE + 4, 5);
    write_u64(&mut image, HEADER_SIZE + 8, PAGE_SIZE as u64);
    write_u64(&mut image, HEADER_SIZE + 16, 0x400000);
    write_u64(&mut image, HEADER_SIZE + 32, code.len() as u64);
    write_u64(&mut image, HEADER_SIZE + 40, PAGE_SIZE as u64);
    write_u64(&mut image, HEADER_SIZE + 48, PAGE_SIZE as u64);
    image[PAGE_SIZE..].copy_from_slice(code);
    image
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
