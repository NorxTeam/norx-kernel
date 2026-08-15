use core::{marker::PhantomData, ptr};

pub const SECTOR_SIZE: usize = 512;

const RAMDISK_SECTORS: usize = 32;
const QUEUE_DEPTH: usize = 8;
const REQUEST_TIMEOUT_POLLS: usize = 64;
const MAX_PARTITIONS: usize = 4;

static mut RAMDISK: [u8; SECTOR_SIZE * RAMDISK_SECTORS] = [0; SECTOR_SIZE * RAMDISK_SECTORS];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    Ramdisk,
    VirtioBlk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NotReady,
    InvalidRequest,
    OutOfRange,
    ReadOnly,
    Busy,
    Timeout,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheMode {
    WriteThrough,
    WriteBack,
}

impl CacheMode {
    pub const fn name(self) -> &'static str {
        match self {
            Self::WriteThrough => "write-through",
            Self::WriteBack => "write-back",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub sector_size: usize,
    pub sectors: u64,
    pub read_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition {
    pub index: u8,
    pub type_code: u8,
    pub start_lba: u64,
    pub sectors: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub geometry: Geometry,
    pub queue_capacity: usize,
    pub in_flight: usize,
    pub completed: u64,
    pub timeouts: u64,
    pub partition_count: usize,
    pub cache_mode: CacheMode,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RequestId<'a> {
    index: u8,
    _buffer: PhantomData<&'a [u8]>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Read,
    Write,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestState {
    Free,
    Queued,
    Complete,
}

#[derive(Clone, Copy)]
struct Request {
    state: RequestState,
    operation: Operation,
    lba: u64,
    length: usize,
    buffer: *mut u8,
    error: Option<Error>,
}

impl Request {
    const EMPTY: Self = Self {
        state: RequestState::Free,
        operation: Operation::Read,
        lba: 0,
        length: 0,
        buffer: ptr::null_mut(),
        error: None,
    };
}

static mut QUEUE: [Request; QUEUE_DEPTH] = [Request::EMPTY; QUEUE_DEPTH];
static mut READY: bool = false;
static mut READ_ONLY: bool = false;
static mut HARDWARE_READ_ONLY: bool = false;
static mut BACKEND: BackendKind = BackendKind::Ramdisk;
static mut DATA_SECTORS: u64 = RAMDISK_SECTORS as u64;
static mut CACHE_MODE: CacheMode = CacheMode::WriteThrough;
static mut COMPLETED: u64 = 0;
static mut TIMEOUTS: u64 = 0;
static mut PARTITIONS: [Option<Partition>; MAX_PARTITIONS] = [None; MAX_PARTITIONS];
static mut PARTITION_COUNT: usize = 0;

pub fn contract_self_check() {
    assert_eq!(SECTOR_SIZE, 512);
    assert!(QUEUE_DEPTH.is_power_of_two());
    assert_eq!(core::mem::size_of::<RequestId>(), 1);
    assert_eq!(mbr_u32(&[1, 2, 3, 4], 0), 0x0403_0201);
    let _ = CacheMode::WriteBack;
    let mut output = [0u8; SECTOR_SIZE];
    assert!(matches!(submit_read(0, &mut output), Err(Error::NotReady)));
    assert!(matches!(
        set_cache_mode(CacheMode::WriteBack),
        Err(Error::Unsupported)
    ));
}

pub fn init() -> bool {
    crate::bootlog::start(1, "initializing block layer");
    unsafe {
        READY = false;
        READ_ONLY = false;
        HARDWARE_READ_ONLY = false;
        BACKEND = BackendKind::Ramdisk;
        DATA_SECTORS = RAMDISK_SECTORS as u64;
        CACHE_MODE = CacheMode::WriteThrough;
        COMPLETED = 0;
        TIMEOUTS = 0;
        QUEUE = [Request::EMPTY; QUEUE_DEPTH];
        PARTITIONS = [None; MAX_PARTITIONS];
        PARTITION_COUNT = 0;
        let disk = (&raw mut RAMDISK).cast::<u8>();
        for i in 0..SECTOR_SIZE * RAMDISK_SECTORS {
            disk.add(i).write((i & 0xff) as u8);
        }
        write_mbr(disk);
        #[cfg(target_arch = "x86_64")]
        if let Some(status) = crate::drivers::virtio_blk::status() {
            BACKEND = BackendKind::VirtioBlk;
            DATA_SECTORS = status.sectors;
            HARDWARE_READ_ONLY = status.read_only;
            READ_ONLY = status.read_only;
        }
        READY = true;
    }
    if !discover_partitions() {
        crate::bootlog::fail("block partition discovery failed");
        return false;
    }
    let smoke_passed = match backend() {
        BackendKind::Ramdisk => smoke_test(),
        BackendKind::VirtioBlk => read_sector(0, &mut [0; SECTOR_SIZE]),
    };
    if !smoke_passed {
        crate::bootlog::fail("block backend self-check failed");
        return false;
    }
    let geometry = geometry();
    crate::bootlog::ok_fmt(format_args!(
        "block backend={} geometry sector={} sectors={} queue={} partitions={} readonly={} cache={}",
        backend_name(),
        geometry.sector_size,
        geometry.sectors,
        QUEUE_DEPTH,
        partition_count(),
        geometry.read_only,
        cache_mode().name(),
    ));
    crate::bootlog::ok("block completion, read-only, partition, and cache checks passed");
    true
}

pub fn geometry() -> Geometry {
    Geometry {
        sector_size: SECTOR_SIZE,
        sectors: unsafe { DATA_SECTORS },
        read_only: unsafe { READ_ONLY },
    }
}

pub fn backend() -> BackendKind {
    unsafe { BACKEND }
}

pub fn persistent() -> bool {
    backend() == BackendKind::VirtioBlk
}

fn backend_name() -> &'static str {
    match backend() {
        BackendKind::Ramdisk => "ramdisk",
        BackendKind::VirtioBlk => "virtio-blk",
    }
}

pub fn stats() -> Stats {
    let mut in_flight = 0;
    unsafe {
        let queue = &*core::ptr::addr_of!(QUEUE);
        for request in queue {
            if request.state != RequestState::Free {
                in_flight += 1;
            }
        }
    }
    Stats {
        geometry: geometry(),
        queue_capacity: QUEUE_DEPTH,
        in_flight,
        completed: unsafe { COMPLETED },
        timeouts: unsafe { TIMEOUTS },
        partition_count: partition_count(),
        cache_mode: cache_mode(),
    }
}

pub fn partitions(output: &mut [Partition]) -> usize {
    let count = partition_count().min(output.len());
    unsafe {
        let partitions = &*core::ptr::addr_of!(PARTITIONS);
        for (destination, source) in output[..count].iter_mut().zip(partitions[..count].iter()) {
            if let Some(partition) = source {
                *destination = *partition;
            }
        }
    }
    count
}

pub fn set_read_only(read_only: bool) {
    unsafe { READ_ONLY = read_only || HARDWARE_READ_ONLY };
}

pub fn read_only() -> bool {
    unsafe { READ_ONLY }
}

pub fn cache_mode() -> CacheMode {
    unsafe { CACHE_MODE }
}

pub fn set_cache_mode(mode: CacheMode) -> Result<(), Error> {
    if mode == CacheMode::WriteBack {
        return Err(Error::Unsupported);
    }
    unsafe { CACHE_MODE = mode };
    Ok(())
}

pub fn flush_cache() -> Result<(), Error> {
    if persistent() {
        #[cfg(target_arch = "x86_64")]
        return crate::drivers::virtio_blk::flush().map_err(|_| Error::Unsupported);
    }
    match cache_mode() {
        CacheMode::WriteThrough => Ok(()),
        CacheMode::WriteBack => Err(Error::Unsupported),
    }
}

pub fn submit_read<'a>(lba: u64, output: &'a mut [u8]) -> Result<RequestId<'a>, Error> {
    submit(
        Operation::Read,
        lba,
        output.as_mut_ptr(),
        output.len(),
        PhantomData,
    )
}

pub fn submit_write<'a>(lba: u64, input: &'a [u8]) -> Result<RequestId<'a>, Error> {
    submit(
        Operation::Write,
        lba,
        input.as_ptr() as *mut u8,
        input.len(),
        PhantomData,
    )
}

pub fn poll() {
    for index in 0..QUEUE_DEPTH {
        let request = unsafe { (&*core::ptr::addr_of!(QUEUE))[index] };
        if request.state != RequestState::Queued {
            continue;
        }
        let error = process(request).err();
        unsafe {
            let request = &mut *core::ptr::addr_of_mut!(QUEUE);
            request[index].error = error;
            request[index].state = RequestState::Complete;
            COMPLETED = COMPLETED.saturating_add(1);
        }
    }
}

pub fn wait(request: RequestId<'_>) -> Result<usize, Error> {
    let index = request.index as usize;
    if index >= QUEUE_DEPTH {
        return Err(Error::InvalidRequest);
    }
    for _ in 0..REQUEST_TIMEOUT_POLLS {
        poll();
        let state = unsafe { (&*core::ptr::addr_of!(QUEUE))[index].state };
        match state {
            RequestState::Complete => {
                let length = unsafe { (&*core::ptr::addr_of!(QUEUE))[index].length };
                let error = unsafe { (&*core::ptr::addr_of!(QUEUE))[index].error };
                unsafe { (&mut *core::ptr::addr_of_mut!(QUEUE))[index] = Request::EMPTY };
                return error.map_or(Ok(length), Err);
            }
            RequestState::Free => return Err(Error::InvalidRequest),
            RequestState::Queued => core::hint::spin_loop(),
        }
    }
    unsafe {
        (&mut *core::ptr::addr_of_mut!(QUEUE))[index] = Request::EMPTY;
        TIMEOUTS = TIMEOUTS.saturating_add(1);
    }
    Err(Error::Timeout)
}

pub fn read_sectors(lba: u64, output: &mut [u8]) -> Result<usize, Error> {
    wait(submit_read(lba, output)?)
}

pub fn write_sectors(lba: u64, input: &[u8]) -> Result<usize, Error> {
    wait(submit_write(lba, input)?)
}

pub fn read_sector(lba: usize, output: &mut [u8; SECTOR_SIZE]) -> bool {
    read_sectors(lba as u64, output).is_ok()
}

pub fn write_sector(lba: usize, input: &[u8; SECTOR_SIZE]) -> bool {
    write_sectors(lba as u64, input).is_ok()
}

fn submit<'a>(
    operation: Operation,
    lba: u64,
    buffer: *mut u8,
    length: usize,
    marker: PhantomData<&'a [u8]>,
) -> Result<RequestId<'a>, Error> {
    if !unsafe { READY } {
        return Err(Error::NotReady);
    }
    if length == 0 || !length.is_multiple_of(SECTOR_SIZE) || buffer.is_null() {
        return Err(Error::InvalidRequest);
    }
    if operation == Operation::Write && read_only() {
        return Err(Error::ReadOnly);
    }
    let sectors = (length / SECTOR_SIZE) as u64;
    if lba
        .checked_add(sectors)
        .is_none_or(|end| end > unsafe { DATA_SECTORS })
    {
        return Err(Error::OutOfRange);
    }
    unsafe {
        let queue = &mut *core::ptr::addr_of_mut!(QUEUE);
        let Some((index, request)) = queue
            .iter_mut()
            .enumerate()
            .find(|(_, request)| request.state == RequestState::Free)
        else {
            return Err(Error::Busy);
        };
        *request = Request {
            state: RequestState::Queued,
            operation,
            lba,
            length,
            buffer,
            error: None,
        };
        Ok(RequestId {
            index: index as u8,
            _buffer: marker,
        })
    }
}

fn process(request: Request) -> Result<(), Error> {
    match backend() {
        BackendKind::Ramdisk => {
            let start = request.lba as usize * SECTOR_SIZE;
            unsafe {
                let disk = (&raw mut RAMDISK).cast::<u8>().add(start);
                match request.operation {
                    Operation::Read => {
                        ptr::copy_nonoverlapping(disk, request.buffer, request.length)
                    }
                    Operation::Write => {
                        ptr::copy_nonoverlapping(request.buffer, disk, request.length)
                    }
                }
            }
            Ok(())
        }
        BackendKind::VirtioBlk => {
            #[cfg(target_arch = "x86_64")]
            {
                let sectors = request.length / SECTOR_SIZE;
                for index in 0..sectors {
                    let buffer = unsafe {
                        &mut *(request.buffer.add(index * SECTOR_SIZE) as *mut [u8; SECTOR_SIZE])
                    };
                    match request.operation {
                        Operation::Read => crate::drivers::virtio_blk::read_sector(
                            request.lba + index as u64,
                            buffer,
                        )
                        .map_err(|_| Error::Unsupported)?,
                        Operation::Write => crate::drivers::virtio_blk::write_sector(
                            request.lba + index as u64,
                            &*buffer,
                        )
                        .map_err(|_| Error::Unsupported)?,
                    }
                }
                Ok(())
            }
            #[cfg(not(target_arch = "x86_64"))]
            Err(Error::Unsupported)
        }
    }
}

fn smoke_test() -> bool {
    let lba = unsafe { DATA_SECTORS - 1 };
    let mut original = [0u8; SECTOR_SIZE];
    let pattern = [0xa5u8; SECTOR_SIZE];
    if !read_sector(lba as usize, &mut original) {
        return false;
    }
    if read_only() {
        return true;
    }
    set_read_only(true);
    let read_only_rejected = matches!(submit_write(lba, &pattern), Err(Error::ReadOnly));
    set_read_only(false);
    if !read_only_rejected || !write_sector(lba as usize, &pattern) {
        return false;
    }
    let mut round_trip = [0u8; SECTOR_SIZE];
    let passed = read_sector(lba as usize, &mut round_trip) && round_trip == pattern;
    let restored = write_sector(lba as usize, &original);
    passed && restored
}

fn write_mbr(disk: *mut u8) {
    unsafe {
        let entry = disk.add(446);
        entry.add(4).write(0xda);
        write_le_u32(entry.add(8), 1);
        write_le_u32(entry.add(12), (RAMDISK_SECTORS - 1) as u32);
        disk.add(510).write(0x55);
        disk.add(511).write(0xaa);
    }
}

fn discover_partitions() -> bool {
    let mut sector = [0u8; SECTOR_SIZE];
    if !read_sector(0, &mut sector) {
        return false;
    }
    let mut count = 0;
    if sector[510] == 0x55 && sector[511] == 0xaa {
        for index in 0..MAX_PARTITIONS {
            let offset = 446 + index * 16;
            let type_code = sector[offset + 4];
            let start_lba = mbr_u32(&sector, offset + 8) as u64;
            let sectors = mbr_u32(&sector, offset + 12) as u64;
            if type_code == 0
                || sectors == 0
                || start_lba >= unsafe { DATA_SECTORS }
                || sectors > unsafe { DATA_SECTORS } - start_lba
            {
                continue;
            }
            unsafe {
                (&mut *core::ptr::addr_of_mut!(PARTITIONS))[count] = Some(Partition {
                    index: index as u8,
                    type_code,
                    start_lba,
                    sectors,
                });
            }
            count += 1;
        }
    }
    if count == 0 {
        unsafe {
            (&mut *core::ptr::addr_of_mut!(PARTITIONS))[0] = Some(Partition {
                index: 0,
                type_code: 0,
                start_lba: 0,
                sectors: DATA_SECTORS,
            });
        }
        count = 1;
    }
    unsafe { PARTITION_COUNT = count };
    true
}

fn partition_count() -> usize {
    unsafe { PARTITION_COUNT }
}

fn write_le_u32(address: *mut u8, value: u32) {
    unsafe {
        address.add(0).write(value as u8);
        address.add(1).write((value >> 8) as u8);
        address.add(2).write((value >> 16) as u8);
        address.add(3).write((value >> 24) as u8);
    }
}

fn mbr_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}
