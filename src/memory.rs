#![allow(
    dead_code,
    reason = "Staged ABI and hardware hooks are not all enabled in the bootstrap profile"
)]

use crate::boot::{BootInfo, MemoryRegion};
use crate::sync::SmpLock;

const MAX_RANGES: usize = 64;
const MAX_RETURNED_FRAMES: usize = 512;
const PAGE_SIZE: u64 = 4096;
const MIN_FRAME: u64 = 0x100000;

static mut RANGES: [Range; MAX_RANGES] = [Range::empty(); MAX_RANGES];
static mut RANGE_COUNT: usize = 0;
static mut NEXT_RANGE: usize = 0;
static mut NEXT_FRAME: u64 = 0;
static mut USABLE_PAGES: u64 = 0;
static mut PHYSICAL_LIMIT: u64 = 0;
static mut ALLOCATED_FRAMES: u64 = 0;
static mut SKIPPED_RANGES: usize = 0;
static mut RETURNED_FRAMES: [u64; MAX_RETURNED_FRAMES] = [0; MAX_RETURNED_FRAMES];
static mut RETURNED_COUNT: usize = 0;
// The range table, allocation cursor, ownership marks, counters, and returned
// frame cache are one domain. IRQ masking alone is not an SMP lock.
static ALLOCATOR_LOCK: SmpLock<()> = SmpLock::new(());

#[derive(Clone, Copy)]
struct Range {
    start: u64,
    pages: u64,
    allocated_start: u64,
    allocated_end: u64,
}

impl Range {
    const fn empty() -> Self {
        Self {
            start: 0,
            pages: 0,
            allocated_start: 0,
            allocated_end: 0,
        }
    }

    fn end(self) -> u64 {
        self.start
            .saturating_add(self.pages.saturating_mul(PAGE_SIZE))
    }

    fn owns_allocated(self, frame: u64) -> bool {
        self.allocated_start != 0 && frame >= self.allocated_start && frame < self.allocated_end
    }
}

fn with_allocator<R>(f: impl FnOnce() -> R) -> R {
    ALLOCATOR_LOCK.with(|_| f())
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
    with_allocator(|| {
        unsafe {
            RANGE_COUNT = 0;
            NEXT_RANGE = 0;
            NEXT_FRAME = 0;
            USABLE_PAGES = 0;
            PHYSICAL_LIMIT = 0;
            ALLOCATED_FRAMES = 0;
            SKIPPED_RANGES = 0;
            RETURNED_COUNT = 0;
        }

        for (index, region) in info
            .memory
            .iter()
            .take(info.memory_len)
            .copied()
            .enumerate()
        {
            if index & 3 == 0 {
                crate::bootlog::pulse();
            }
            if let Some(end) = region.base.checked_add(region.length) {
                unsafe { PHYSICAL_LIMIT = PHYSICAL_LIMIT.max(end) };
            }
            add_available(region, &info);
        }

        let stats = stats_locked();
        Summary {
            descriptors: info.memory_len,
            usable_pages: stats.usable_pages,
            skipped_ranges: unsafe { SKIPPED_RANGES },
        }
    })
}

pub fn alloc_frame() -> Option<u64> {
    with_allocator(|| unsafe {
        if RETURNED_COUNT != 0 {
            RETURNED_COUNT -= 1;
            return Some(RETURNED_FRAMES[RETURNED_COUNT]);
        }
        while NEXT_RANGE < RANGE_COUNT {
            let range = RANGES[NEXT_RANGE];
            if NEXT_FRAME == 0 || NEXT_FRAME < range.start {
                NEXT_FRAME = range.start;
            }
            if NEXT_FRAME < range.end() {
                let frame = NEXT_FRAME;
                let Some(next_frame) = NEXT_FRAME.checked_add(PAGE_SIZE) else {
                    NEXT_RANGE += 1;
                    continue;
                };
                NEXT_FRAME = next_frame;
                let allocation = &mut RANGES[NEXT_RANGE];
                if allocation.allocated_start == 0 {
                    allocation.allocated_start = frame;
                }
                allocation.allocated_end = next_frame;
                ALLOCATED_FRAMES = ALLOCATED_FRAMES.saturating_add(1);
                return Some(frame);
            }
            NEXT_RANGE += 1;
        }
        None
    })
}

/// Returns a frame only if this allocator handed it out and it is not already
/// in the bounded return cache. Boot, firmware, device, and never-yet-allocated
/// frames are not valid inputs even when they are aligned and above 1 MiB.
pub fn free_frame(frame: u64) -> bool {
    if frame < MIN_FRAME || !frame.is_multiple_of(PAGE_SIZE) {
        return false;
    }
    with_allocator(|| unsafe {
        if !RANGES[..RANGE_COUNT]
            .iter()
            .copied()
            .any(|range| range.owns_allocated(frame))
        {
            return false;
        }
        if RETURNED_COUNT == MAX_RETURNED_FRAMES
            || RETURNED_FRAMES[..RETURNED_COUNT].contains(&frame)
        {
            return false;
        }
        RETURNED_FRAMES[RETURNED_COUNT] = frame;
        RETURNED_COUNT += 1;
        true
    })
}

pub fn stats() -> Stats {
    with_allocator(stats_locked)
}

fn stats_locked() -> Stats {
    unsafe {
        Stats {
            ranges: RANGE_COUNT,
            usable_pages: USABLE_PAGES,
            allocated_frames: ALLOCATED_FRAMES,
            next_frame: NEXT_FRAME,
        }
    }
}

/// Returns the first physical address above the platform memory map. The
/// direct map also needs firmware-owned page-table frames, which may be
/// reserved rather than allocator-usable.
pub fn physical_limit() -> u64 {
    with_allocator(|| unsafe { PHYSICAL_LIMIT })
}

fn first_unallocated_frame_locked() -> Option<u64> {
    unsafe {
        RANGES[..RANGE_COUNT].iter().copied().find_map(|range| {
            let candidate = if range.allocated_start == 0 {
                Some(range.start)
            } else if range.allocated_end < range.end() {
                Some(range.allocated_end)
            } else {
                None
            }?;
            (!RANGES[..RANGE_COUNT]
                .iter()
                .copied()
                .any(|owner| owner.owns_allocated(candidate)))
            .then_some(candidate)
        })
    }
}

pub fn contract_self_check() -> bool {
    assert!(!free_frame(MIN_FRAME - PAGE_SIZE));
    assert!(!free_frame(MIN_FRAME + 1));
    assert!(!free_frame(u64::MAX));

    let unallocated = with_allocator(first_unallocated_frame_locked);
    if let Some(frame) = unallocated {
        assert!(!free_frame(frame));
    }

    let Some(frame) = alloc_frame() else {
        return false;
    };
    assert!(free_frame(frame));
    assert!(!free_frame(frame));
    assert_eq!(alloc_frame(), Some(frame));
    assert!(free_frame(frame));
    true
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
            RANGES[RANGE_COUNT] = Range {
                start,
                pages,
                allocated_start: 0,
                allocated_end: 0,
            };
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
