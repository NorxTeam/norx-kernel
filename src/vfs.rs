const HELLO_LBA: usize = 2;
const FILE_MAX: usize = 255;

#[derive(Clone, Copy)]
pub struct Stats {
    pub mounts: usize,
    pub files: usize,
    pub bytes: usize,
}

pub fn init() {
    let mut sector = [0u8; 512];
    let text = b"Welcome to Norx VFS\n";
    sector[0] = text.len() as u8;
    sector[1..1 + text.len()].copy_from_slice(text);
    let _ = crate::drivers::block::write_sector(HELLO_LBA, &sector);
}

pub fn stats() -> Stats {
    Stats {
        mounts: 1,
        files: 1,
        bytes: read_len(HELLO_LBA),
    }
}

pub fn list(mut f: impl FnMut(&'static str, usize)) {
    f("/hello.txt", read_len(HELLO_LBA));
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
