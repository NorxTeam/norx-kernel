use crate::uefi::{MemoryDescriptor, MemoryMapInfo};

const MAP_BYTES: usize = 64 * 1024;
const MAX_RANGES: usize = 64;
const PAGE_SIZE: u64 = 4096;
const MIN_FRAME: u64 = 0x100000;

static mut MAP_BUFFER: [u8; MAP_BYTES] = [0; MAP_BYTES];
static mut RANGES: [Range; MAX_RANGES] = [Range::empty(); MAX_RANGES];
static mut RANGE_COUNT: usize = 0;
static mut NEXT_RANGE: usize = 0;
static mut NEXT_FRAME: u64 = 0;
static mut USABLE_PAGES: u64 = 0;
static mut ALLOCATED_FRAMES: u64 = 0;

#[derive(Clone, Copy)]
struct Range {
    start: u64,
    pages: u64,
}

impl Range {
    const fn empty() -> Self {
        Self { start: 0, pages: 0 }
    }

    fn end(self) -> u64 {
        self.start + self.pages * PAGE_SIZE
    }
}

pub struct Summary {
    pub descriptors: usize,
    pub usable_pages: u64,
    pub skipped_ranges: usize,
    pub descriptor_size: usize,
    pub descriptor_version: u32,
}

#[derive(Clone, Copy)]
pub struct Stats {
    pub ranges: usize,
    pub usable_pages: u64,
    pub allocated_frames: u64,
    pub next_frame: u64,
}

pub fn exit_boot_services(
    image: crate::uefi::Handle,
    system_table: *mut crate::uefi::SystemTable,
) -> Option<Summary> {
    let buffer = (&raw mut MAP_BUFFER).cast::<MemoryDescriptor>();
    let info = unsafe { crate::uefi::memory_map(system_table, buffer, MAP_BYTES)? };

    if unsafe {
        crate::uefi::is_error(crate::uefi::exit_boot_services(
            image,
            system_table,
            info.map_key,
        ))
    } {
        let retry = unsafe { crate::uefi::memory_map(system_table, buffer, MAP_BYTES)? };
        let status = unsafe { crate::uefi::exit_boot_services(image, system_table, retry.map_key) };
        if crate::uefi::is_error(status) {
            return None;
        }
        return Some(load_ranges(buffer, retry));
    }

    Some(load_ranges(buffer, info))
}

pub fn alloc_frame() -> Option<u64> {
    unsafe {
        while NEXT_RANGE < RANGE_COUNT {
            let range = RANGES[NEXT_RANGE];
            if NEXT_FRAME == 0 || NEXT_FRAME < range.start {
                NEXT_FRAME = range.start;
            }
            if NEXT_FRAME < range.end() {
                let frame = NEXT_FRAME;
                NEXT_FRAME += PAGE_SIZE;
                ALLOCATED_FRAMES = ALLOCATED_FRAMES.saturating_add(1);
                return Some(frame);
            }
            NEXT_RANGE += 1;
        }
    }
    None
}

pub fn stats() -> Stats {
    unsafe {
        Stats {
            ranges: RANGE_COUNT,
            usable_pages: USABLE_PAGES,
            allocated_frames: ALLOCATED_FRAMES,
            next_frame: NEXT_FRAME,
        }
    }
}

fn load_ranges(buffer: *const MemoryDescriptor, info: MemoryMapInfo) -> Summary {
    unsafe {
        RANGE_COUNT = 0;
        NEXT_RANGE = 0;
        NEXT_FRAME = 0;
        USABLE_PAGES = 0;
        ALLOCATED_FRAMES = 0;
    }

    let count = info.map_size / info.descriptor_size;
    let mut usable_pages = 0u64;
    let mut skipped_ranges = 0;

    for i in 0..count {
        let desc = unsafe {
            ((buffer as *const u8).add(i * info.descriptor_size) as *const MemoryDescriptor)
                .read_unaligned()
        };
        if desc.ty != crate::uefi::MEMORY_CONVENTIONAL || desc.number_of_pages == 0 {
            continue;
        }
        let Some(bytes) = desc.number_of_pages.checked_mul(PAGE_SIZE) else {
            skipped_ranges += 1;
            continue;
        };
        if desc.physical_start.checked_add(bytes).is_none() {
            skipped_ranges += 1;
            continue;
        }
        let mut start = desc.physical_start;
        let mut pages = desc.number_of_pages;
        if start < MIN_FRAME {
            let drop_bytes = MIN_FRAME - start;
            let drop_pages = drop_bytes.div_ceil(PAGE_SIZE).min(pages);
            start += drop_pages * PAGE_SIZE;
            pages -= drop_pages;
        }
        if pages == 0 {
            continue;
        }

        unsafe {
            let merged = RANGE_COUNT != 0 && RANGES[RANGE_COUNT - 1].end() == start;
            if merged {
                RANGES[RANGE_COUNT - 1].pages = RANGES[RANGE_COUNT - 1].pages.saturating_add(pages);
                usable_pages = usable_pages.saturating_add(pages);
            } else if RANGE_COUNT < MAX_RANGES {
                RANGES[RANGE_COUNT] = Range { start, pages };
                RANGE_COUNT += 1;
                usable_pages = usable_pages.saturating_add(pages);
            } else {
                skipped_ranges += 1;
            }
        }
    }

    unsafe {
        USABLE_PAGES = usable_pages;
    }

    Summary {
        descriptors: count,
        usable_pages,
        skipped_ranges,
        descriptor_size: info.descriptor_size,
        descriptor_version: info.descriptor_version,
    }
}
