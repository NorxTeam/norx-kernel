const CAPACITY: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    WouldBlock,
    BrokenPipe,
    InvalidHandle,
    InvalidEndpoint,
    NoSpace,
}

#[derive(Clone, Copy)]
pub struct Pipe {
    bytes: [u8; CAPACITY],
    read_index: usize,
    write_index: usize,
    length: usize,
    readers: usize,
    writers: usize,
}

impl Pipe {
    pub const fn new() -> Self {
        Self {
            bytes: [0; CAPACITY],
            read_index: 0,
            write_index: 0,
            length: 0,
            readers: 1,
            writers: 1,
        }
    }

    pub const fn with_endpoints(readers: usize, writers: usize) -> Self {
        Self {
            bytes: [0; CAPACITY],
            read_index: 0,
            write_index: 0,
            length: 0,
            readers,
            writers,
        }
    }

    pub const fn len(&self) -> usize {
        self.length
    }

    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub const fn is_full(&self) -> bool {
        self.length == CAPACITY
    }

    pub const fn readers(&self) -> usize {
        self.readers
    }

    pub const fn writers(&self) -> usize {
        self.writers
    }

    pub fn read(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.length == 0 {
            return if self.writers == 0 {
                Ok(0)
            } else {
                Err(Error::WouldBlock)
            };
        }
        let count = output.len().min(self.length);
        for byte in output.iter_mut().take(count) {
            *byte = self.bytes[self.read_index];
            self.read_index = (self.read_index + 1) % CAPACITY;
        }
        self.length -= count;
        Ok(count)
    }

    pub fn write(&mut self, input: &[u8]) -> Result<usize, Error> {
        if self.readers == 0 {
            return Err(Error::BrokenPipe);
        }
        if input.is_empty() {
            return Ok(0);
        }
        if self.length == CAPACITY {
            return Err(Error::WouldBlock);
        }
        let count = input.len().min(CAPACITY - self.length);
        for byte in input.iter().take(count) {
            self.bytes[self.write_index] = *byte;
            self.write_index = (self.write_index + 1) % CAPACITY;
        }
        self.length += count;
        Ok(count)
    }

    pub fn close_reader(&mut self) {
        self.readers = self.readers.saturating_sub(1);
    }

    pub fn close_writer(&mut self) {
        self.writers = self.writers.saturating_sub(1);
    }
}

const MAX_PIPES: usize = 16;
const PIPE_TAG: u32 = 1 << 30;
const PIPE_WRITE_TAG: u32 = 1 << 29;

#[derive(Clone, Copy)]
struct PipeSlot {
    used: bool,
    generation: u16,
    pipe: Pipe,
}

impl PipeSlot {
    const EMPTY: Self = Self {
        used: false,
        generation: 0,
        pipe: Pipe::with_endpoints(0, 0),
    };
}

struct PipeTable {
    slots: [PipeSlot; MAX_PIPES],
}

impl PipeTable {
    const fn new() -> Self {
        Self {
            slots: [PipeSlot::EMPTY; MAX_PIPES],
        }
    }
}

static mut PIPE_TABLE: PipeTable = PipeTable::new();

pub fn create() -> Result<(u32, u32), Error> {
    with_table(|table| {
        let slot = table
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| !slot.used)
            .ok_or(Error::NoSpace)?;
        let (index, slot) = slot;
        slot.used = true;
        slot.generation = (slot.generation.wrapping_add(1)) & 0x7fff;
        if slot.generation == 0 {
            slot.generation = 1;
        }
        slot.pipe = Pipe::new();
        Ok((
            encode(index, slot.generation, false),
            encode(index, slot.generation, true),
        ))
    })
}

pub fn is_pipe_raw(raw: u32) -> bool {
    raw & PIPE_TAG != 0
}

pub fn duplicate_raw(raw: u32) -> Result<u32, Error> {
    with_table(|table| {
        let (index, generation, write) = decode(raw)?;
        let slot = valid_slot(&table.slots, index, generation)?;
        if write {
            table.slots[slot].pipe.writers = table.slots[slot]
                .pipe
                .writers
                .checked_add(1)
                .ok_or(Error::NoSpace)?;
        } else {
            table.slots[slot].pipe.readers = table.slots[slot]
                .pipe
                .readers
                .checked_add(1)
                .ok_or(Error::NoSpace)?;
        }
        Ok(raw)
    })
}

pub fn close_raw(raw: u32) -> Result<(), Error> {
    with_table(|table| {
        let (index, generation, write) = decode(raw)?;
        let slot = valid_slot(&table.slots, index, generation)?;
        if write {
            table.slots[slot].pipe.close_writer();
        } else {
            table.slots[slot].pipe.close_reader();
        }
        if table.slots[slot].pipe.readers() == 0 && table.slots[slot].pipe.writers() == 0 {
            table.slots[slot] = PipeSlot::EMPTY;
        }
        Ok(())
    })
}

pub fn read_raw(raw: u32, output: &mut [u8]) -> Result<usize, Error> {
    with_table(|table| {
        let (index, generation, write) = decode(raw)?;
        if write {
            return Err(Error::InvalidEndpoint);
        }
        let slot = valid_slot(&table.slots, index, generation)?;
        table.slots[slot].pipe.read(output)
    })
}

pub fn write_raw(raw: u32, input: &[u8]) -> Result<usize, Error> {
    with_table(|table| {
        let (index, generation, write) = decode(raw)?;
        if !write {
            return Err(Error::InvalidEndpoint);
        }
        let slot = valid_slot(&table.slots, index, generation)?;
        table.slots[slot].pipe.write(input)
    })
}

fn with_table<R>(f: impl FnOnce(&mut PipeTable) -> Result<R, Error>) -> Result<R, Error> {
    crate::arch::without_interrupts(|| unsafe {
        let table = &mut *core::ptr::addr_of_mut!(PIPE_TABLE);
        f(table)
    })
}

fn encode(index: usize, generation: u16, write: bool) -> u32 {
    PIPE_TAG | if write { PIPE_WRITE_TAG } else { 0 } | ((generation as u32) << 8) | index as u32
}

fn decode(raw: u32) -> Result<(usize, u16, bool), Error> {
    if raw & PIPE_TAG == 0 {
        return Err(Error::InvalidHandle);
    }
    let index = (raw & 0xff) as usize;
    let generation = ((raw >> 8) & 0x7fff) as u16;
    if generation == 0 {
        return Err(Error::InvalidHandle);
    }
    Ok((index, generation, raw & PIPE_WRITE_TAG != 0))
}

fn valid_slot(
    slots: &[PipeSlot; MAX_PIPES],
    index: usize,
    generation: u16,
) -> Result<usize, Error> {
    if index >= MAX_PIPES || !slots[index].used || slots[index].generation != generation {
        return Err(Error::InvalidHandle);
    }
    Ok(index)
}

pub fn contract_self_check() {
    let mut pipe = Pipe::new();
    assert!(pipe.is_empty());
    assert!(!pipe.is_full());
    assert_eq!(pipe.len(), 0);
    assert_eq!(pipe.readers(), 1);
    assert_eq!(pipe.writers(), 1);
    assert_eq!(pipe.read(&mut [0; 1]), Err(Error::WouldBlock));
    assert_eq!(pipe.write(b"abc"), Ok(3));
    let mut output = [0; 2];
    assert_eq!(pipe.read(&mut output), Ok(2));
    assert_eq!(&output, b"ab");
    pipe.close_writer();
    let mut tail = [0; 2];
    assert_eq!(pipe.read(&mut tail), Ok(1));
    assert_eq!(&tail[..1], b"c");
    assert_eq!(pipe.read(&mut tail), Ok(0));
    pipe.close_reader();
    assert_eq!(pipe.write(b"closed"), Err(Error::BrokenPipe));

    let mut full = Pipe::new();
    let input = [0x5a; CAPACITY + 1];
    assert_eq!(full.write(&input), Ok(CAPACITY));
    assert!(full.is_full());
    assert_eq!(full.len(), CAPACITY);
    assert_eq!(full.write(&[0x5a]), Err(Error::WouldBlock));

    let endpoints = Pipe::with_endpoints(2, 3);
    assert_eq!(endpoints.readers(), 2);
    assert_eq!(endpoints.writers(), 3);

    let (read_handle, write_handle) = create().unwrap();
    assert!(is_pipe_raw(read_handle));
    assert!(is_pipe_raw(write_handle));
    assert_eq!(write_raw(write_handle, b"ok"), Ok(2));
    let mut output = [0; 2];
    assert_eq!(read_raw(read_handle, &mut output), Ok(2));
    assert_eq!(&output, b"ok");
    assert_eq!(duplicate_raw(write_handle), Ok(write_handle));
    close_raw(write_handle).unwrap();
    close_raw(write_handle).unwrap();
    close_raw(read_handle).unwrap();
    assert_eq!(
        read_raw(read_handle, &mut output),
        Err(Error::InvalidHandle)
    );
}

#[cfg(test)]
mod tests {
    use super::{Error, Pipe, CAPACITY};

    #[test]
    fn wraps_ring_buffer_without_reordering() {
        let mut pipe = Pipe::new();
        let input = [0x11; CAPACITY];
        assert_eq!(pipe.write(&input), Ok(CAPACITY));
        let mut first = [0; 1024];
        assert_eq!(pipe.read(&mut first), Ok(first.len()));
        assert!(first.iter().all(|byte| *byte == 0x11));
        assert_eq!(pipe.write(b"next"), Ok(4));
        let mut rest = [0; CAPACITY];
        assert_eq!(pipe.read(&mut rest), Ok(CAPACITY - 1024 + 4));
        assert_eq!(&rest[CAPACITY - 1024..CAPACITY - 1024 + 4], b"next");
    }

    #[test]
    fn eof_and_broken_pipe_follow_endpoint_lifetime() {
        let mut pipe = Pipe::with_endpoints(1, 2);
        pipe.close_writer();
        assert_eq!(pipe.read(&mut [0; 1]), Err(Error::WouldBlock));
        pipe.close_writer();
        assert_eq!(pipe.read(&mut [0; 1]), Ok(0));
        pipe.close_reader();
        assert_eq!(pipe.write(b"x"), Err(Error::BrokenPipe));
    }
}
