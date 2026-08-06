use crate::boot::{BootInfo, MemoryRegion};

const MAX_RANGES: usize = 64;
const PAGE_SIZE: u64 = 4096;
const MIN_FRAME: u64 = 0x100000;

static mut RANGES: [Range; MAX_RANGES] = [Range::empty(); MAX_RANGES];
static mut RANGE_COUNT: usize = 0;
static mut NEXT_RANGE: usize = 0;
static mut NEXT_FRAME: u64 = 0;
static mut USABLE_PAGES: u64 = 0;
static mut ALLOCATED_FRAMES: u64 = 0;
static mut SKIPPED_RANGES: usize = 0;

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
        self.start
            .saturating_add(self.pages.saturating_mul(PAGE_SIZE))
    }
}

pub struct Summary {
    pub descriptors: usize,
    pub usable_pages: u64,
    pub skipped_ranges: usize,
}

#[derive(Clone, Copy)]
pub struct Stats {
    pub ranges: usize,
    pub usable_pages: u64,
    pub allocated_frames: u64,
    pub next_frame: u64,
}

pub fn init(info: BootInfo) -> Summary {
    unsafe {
        RANGE_COUNT = 0;
        NEXT_RANGE = 0;
        NEXT_FRAME = 0;
        USABLE_PAGES = 0;
        ALLOCATED_FRAMES = 0;
        SKIPPED_RANGES = 0;
    }

    for region in info.memory.iter().take(info.memory_len).copied() {
        add_available(region, &info);
    }

    let stats = stats();
    Summary {
        descriptors: info.memory_len,
        usable_pages: stats.usable_pages,
        skipped_ranges: unsafe { SKIPPED_RANGES },
    }
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

fn add_available(region: MemoryRegion, info: &BootInfo) {
    let Some(end) = region.base.checked_add(region.length) else {
        unsafe { SKIPPED_RANGES = SKIPPED_RANGES.saturating_add(1) };
        return;
    };
    let start = region.base.max(MIN_FRAME);
    if start >= end {
        return;
    }

    let mut cursor = align_up(start);
    let end = end & !(PAGE_SIZE - 1);
    if cursor >= end {
        return;
    }

    while cursor < end {
        let mut next = end;
        let mut covered_end = cursor;
        for reserved in info.reserved.iter().take(info.reserved_len).copied() {
            let Some(reserved_end) = reserved.base.checked_add(reserved.length) else {
                continue;
            };
            if reserved_end <= cursor || reserved.base >= end {
                continue;
            }
            if reserved.base <= cursor {
                covered_end = covered_end.max(reserved_end.min(end));
            } else {
                next = next.min(reserved.base);
            }
        }

        if covered_end > cursor {
            cursor = align_up(covered_end);
            continue;
        }
        let available_end = next.min(end) & !(PAGE_SIZE - 1);
        if available_end > cursor {
            push_range(cursor, (available_end - cursor) / PAGE_SIZE);
        }
        cursor = align_up(available_end.max(cursor.saturating_add(PAGE_SIZE)));
    }
}

fn push_range(start: u64, pages: u64) {
    if pages == 0 {
        return;
    }
    unsafe {
        let merged = RANGE_COUNT != 0 && RANGES[RANGE_COUNT - 1].end() == start;
        if merged {
            RANGES[RANGE_COUNT - 1].pages = RANGES[RANGE_COUNT - 1].pages.saturating_add(pages);
            USABLE_PAGES = USABLE_PAGES.saturating_add(pages);
        } else if RANGE_COUNT < MAX_RANGES {
            RANGES[RANGE_COUNT] = Range { start, pages };
            RANGE_COUNT += 1;
            USABLE_PAGES = USABLE_PAGES.saturating_add(pages);
        } else {
            SKIPPED_RANGES = SKIPPED_RANGES.saturating_add(1);
        }
    }
}

fn align_up(value: u64) -> u64 {
    value
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value & !(PAGE_SIZE - 1))
        .unwrap_or(!(PAGE_SIZE - 1))
}
