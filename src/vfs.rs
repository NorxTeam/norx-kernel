const HELLO_LBA: usize = 2;
const BIN_HELLO_LBA: usize = 3;
const BIN_ARGS_LBA: usize = 4;
const FILE_MAX: usize = 511;

#[derive(Clone, Copy)]
pub struct Stats {
    pub mounts: usize,
    pub files: usize,
    pub bytes: usize,
}

pub fn init() {
    let mut sector = [0u8; 512];
    let text = b"Welcome to Norr VFS\n";
    sector[0] = text.len() as u8;
    sector[1..1 + text.len()].copy_from_slice(text);
    let _ = crate::drivers::block::write_sector(HELLO_LBA, &sector);
    write_object(
        BIN_HELLO_LBA,
        1,
        crate::capability::Set::CLOCK.union(crate::capability::Set::LOG_WRITE),
    );
    write_object(BIN_ARGS_LBA, 2, crate::capability::Set::LOG_WRITE);
}

pub fn stats() -> Stats {
    Stats {
        mounts: 1,
        files: 3,
        bytes: read_len(HELLO_LBA) + read_len(BIN_HELLO_LBA) + read_len(BIN_ARGS_LBA),
    }
}

pub fn list(mut f: impl FnMut(&'static str, usize)) {
    f("/hello.txt", read_len(HELLO_LBA));
    f("/bin/hello", read_len(BIN_HELLO_LBA));
    f("/bin/args", read_len(BIN_ARGS_LBA));
}

pub fn read(path: &str, out: &mut [u8; FILE_MAX]) -> Option<usize> {
    let lba = file_lba(path)?;
    let mut sector = [0u8; 512];
    if !crate::drivers::block::read_sector(lba, &mut sector) {
        return None;
    }
    let len = (sector[0] as usize).min(FILE_MAX);
    out[..len].copy_from_slice(&sector[1..1 + len]);
    Some(len)
}

pub fn exists(path: &str) -> bool {
    file_lba(path).is_some()
}

pub fn write(path: &str, input: &[u8]) -> bool {
    if path != "/hello.txt" || input.len() > FILE_MAX {
        return false;
    }
    let mut sector = [0u8; 512];
    sector[0] = input.len() as u8;
    sector[1..1 + input.len()].copy_from_slice(input);
    crate::drivers::block::write_sector(HELLO_LBA, &sector)
}

fn file_lba(path: &str) -> Option<usize> {
    match path {
        "/hello.txt" => Some(HELLO_LBA),
        "/bin/hello" => Some(BIN_HELLO_LBA),
        "/bin/args" => Some(BIN_ARGS_LBA),
        _ => None,
    }
}

fn read_len(lba: usize) -> usize {
    let mut sector = [0u8; 512];
    if !crate::drivers::block::read_sector(lba, &mut sector) {
        return 0;
    }
    (sector[0] as usize).min(FILE_MAX)
}

fn write_object(lba: usize, entry: u16, caps: crate::capability::Set) {
    let mut sector = [0u8; 512];
    let code = object_code();
    let len = 20 + code.len();
    sector[0] = len as u8;
    sector[1..5].copy_from_slice(&crate::process::NORR_EXEC_MAGIC.to_le_bytes());
    sector[5..7].copy_from_slice(&crate::process::NORR_EXEC_ABI.to_le_bytes());
    sector[7..9].copy_from_slice(&entry.to_le_bytes());
    sector[9..13].copy_from_slice(&0u32.to_le_bytes());
    sector[13..17].copy_from_slice(&(caps.bits() as u32).to_le_bytes());
    sector[17..19].copy_from_slice(&(code.len() as u16).to_le_bytes());
    sector[21..21 + code.len()].copy_from_slice(code);
    let _ = crate::drivers::block::write_sector(lba, &sector);
}

fn object_code() -> &'static [u8] {
    #[cfg(target_arch = "x86_64")]
    {
        &[
            0x48, 0xbf, 0x42, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00,
            0x00, // mov rdi, USER_CODE_BASE + string
            0xbe, 0x03, 0x00, 0x00, 0x00, // mov esi, 3
            0xb8, 0x03, 0x00, 0x00, 0x00, // mov eax, Write
            0xcd, 0x80, // int 0x80
            0x48, 0xbb, 0x00, 0xff, 0x0f, 0x00, 0x00, 0x08, 0x00, 0x00, // mov rbx, ARGV_BASE
            0x80, 0x3b, 0x00, // cmp byte [rbx], 0
            0x74, 0x0f, // je exit
            0x48, 0x8b, 0x7b, 0x08, // mov rdi, [rbx + 8]
            0x48, 0x8b, 0x73, 0x10, // mov rsi, [rbx + 16]
            0xb8, 0x03, 0x00, 0x00, 0x00, // mov eax, Write
            0xcd, 0x80, // int 0x80
            0xbf, 0x2a, 0x00, 0x00, 0x00, // mov edi, 42
            0xb8, 0x01, 0x00, 0x00, 0x00, // mov eax, Exit
            0xcd, 0x80, // int 0x80
            0x0f, 0x0b, // ud2
            b'H', b'i', b'\n',
        ]
    }
    #[cfg(target_arch = "aarch64")]
    {
        &[
            0x80, 0x0a, 0x80, 0xd2, // mov x0, #0x54
            0x00, 0x00, 0xa0, 0xf2, // movk x0, #0, lsl #16
            0x00, 0x00, 0xc1, 0xf2, // movk x0, #0x800, lsl #32
            0x00, 0x00, 0xe0, 0xf2, // movk x0, #0, lsl #48
            0x61, 0x00, 0x80, 0xd2, // mov x1, #3
            0x68, 0x00, 0x80, 0xd2, // mov x8, #Write
            0x01, 0x00, 0x00, 0xd4, // svc #0
            0x02, 0xe0, 0x9f, 0xd2, // mov x2, #0xff00
            0xe2, 0x01, 0xa0, 0xf2, // movk x2, #0xf, lsl #16
            0x02, 0x00, 0xc1, 0xf2, // movk x2, #0x800, lsl #32
            0x02, 0x00, 0xe0, 0xf2, // movk x2, #0, lsl #48
            0x43, 0x00, 0x40, 0x39, // ldrb w3, [x2]
            0xa3, 0x00, 0x00, 0x34, // cbz w3, exit
            0x40, 0x04, 0x40, 0xf9, // ldr x0, [x2, #8]
            0x41, 0x08, 0x40, 0xf9, // ldr x1, [x2, #16]
            0x68, 0x00, 0x80, 0xd2, // mov x8, #Write
            0x01, 0x00, 0x00, 0xd4, // svc #0
            0x40, 0x05, 0x80, 0xd2, // mov x0, #42
            0x28, 0x00, 0x80, 0xd2, // mov x8, #Exit
            0x01, 0x00, 0x00, 0xd4, // svc #0
            0x00, 0x00, 0x20, 0xd4, // brk #0
            b'H', b'i', b'\n',
        ]
    }
}
