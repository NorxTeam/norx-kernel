const SECTOR_SIZE: usize = 512;
const RAMDISK_SECTORS: usize = 8;

static mut RAMDISK: [u8; SECTOR_SIZE * RAMDISK_SECTORS] = [0; SECTOR_SIZE * RAMDISK_SECTORS];

#[derive(Clone, Copy)]
pub struct Device {
    pub name: &'static str,
    pub sectors: usize,
    pub sector_size: usize,
}

pub fn init() {
    unsafe {
        let disk = (&raw mut RAMDISK).cast::<u8>();
        for i in 0..SECTOR_SIZE * RAMDISK_SECTORS {
            disk.add(i).write((i & 0xff) as u8);
        }
    }
}

pub fn device() -> Device {
    Device {
        name: "norx-ram0",
        sectors: RAMDISK_SECTORS,
        sector_size: SECTOR_SIZE,
    }
}

pub fn read_sector(lba: usize, out: &mut [u8; SECTOR_SIZE]) -> bool {
    if lba >= RAMDISK_SECTORS {
        return false;
    }
    let start = lba * SECTOR_SIZE;
    unsafe {
        let disk = (&raw const RAMDISK).cast::<u8>();
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = disk.add(start + i).read();
        }
    }
    true
}

pub fn write_sector(lba: usize, input: &[u8; SECTOR_SIZE]) -> bool {
    if lba >= RAMDISK_SECTORS {
        return false;
    }
    let start = lba * SECTOR_SIZE;
    unsafe {
        let disk = (&raw mut RAMDISK).cast::<u8>();
        for (i, byte) in input.iter().enumerate() {
            disk.add(start + i).write(*byte);
        }
    }
    true
}
