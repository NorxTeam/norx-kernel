const HEAP_PAGES: usize = 16;
const PAGE_SIZE: usize = 4096;
const HEAP_BYTES: usize = HEAP_PAGES * PAGE_SIZE;

static mut HEAP: [u8; HEAP_BYTES] = [0; HEAP_BYTES];
static mut NEXT: usize = 0;

pub struct Summary {
    pub bytes: usize,
}

pub fn init() -> Option<Summary> {
    for _ in 0..HEAP_PAGES {
        crate::memory::alloc_frame()?;
    }

    unsafe {
        NEXT = 0;
    }
    Some(Summary { bytes: HEAP_BYTES })
}

pub fn alloc_bytes(size: usize, align: usize) -> Option<&'static mut [u8]> {
    if size == 0 || !align.is_power_of_two() {
        return None;
    }

    unsafe {
        let start = align_up(NEXT, align);
        let end = start.checked_add(size)?;
        if end > HEAP_BYTES {
            return None;
        }
        NEXT = end;
        Some(&mut HEAP[start..end])
    }
}

fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}
