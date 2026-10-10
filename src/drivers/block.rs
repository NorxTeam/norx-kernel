#![allow(
    dead_code,
    reason = "Staged ABI and hardware hooks are not all enabled in the bootstrap profile"
)]

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
    #[cfg(target_arch = "x86_64")]
    Device,
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
    generation: u64,
    _buffer: PhantomData<&'a [u8]>,
}

impl RequestId<'_> {
    fn completion_id(&self) -> CompletionId {
        CompletionId {
            index: self.index,
            generation: self.generation,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompletionId {
    index: u8,
    generation: u64,
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
    Quarantined,
}

#[derive(Clone, Copy)]
struct Request {
    state: RequestState,
    generation: u64,
    operation: Operation,
    lba: u64,
    length: usize,
    buffer: [u8; SECTOR_SIZE],
    user_buffer: *mut u8,
    error: Option<Error>,
}

impl Request {
    const EMPTY: Self = Self {
        state: RequestState::Free,
        generation: 0,
        operation: Operation::Read,
        lba: 0,
        length: 0,
        buffer: [0; SECTOR_SIZE],
        user_buffer: ptr::null_mut(),
        error: None,
    };

    fn release(&mut self) {
        let generation = self.generation;
        *self = Self::EMPTY;
        self.generation = generation;
    }

    fn completion_id(&self, index: usize) -> CompletionId {
        CompletionId {
            index: index as u8,
            generation: self.generation,
        }
    }
}

enum ProcessResult {
    Complete(Result<(), Error>),
    Pending,
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
static mut ACTIVE_PARTITION: usize = 0;

pub fn contract_self_check() {
    assert_eq!(SECTOR_SIZE, 512);
    assert!(QUEUE_DEPTH.is_power_of_two());
    assert_eq!(mbr_u32(&[1, 2, 3, 4], 0), 0x0403_0201);
    let _ = CacheMode::WriteBack;
    let mut output = [0u8; SECTOR_SIZE];
    assert!(matches!(submit_read(0, &mut output), Err(Error::NotReady)));
    assert!(matches!(
        set_cache_mode(CacheMode::WriteBack),
        Err(Error::Unsupported)
    ));

    let mut delayed_queue = [Request::EMPTY; QUEUE_DEPTH];
    let delayed = reserve_request(
        &mut delayed_queue,
        Operation::Read,
        0,
        ptr::null_mut(),
        None,
    )
    .expect("delayed request slot");
    assert!(quarantine_request_in(
        &mut delayed_queue,
        delayed,
        Error::Timeout
    ));
    let mut occupied = [CompletionId {
        index: 0,
        generation: 0,
    }; QUEUE_DEPTH - 1];
    for slot in &mut occupied {
        *slot = reserve_request(
            &mut delayed_queue,
            Operation::Read,
            0,
            ptr::null_mut(),
            None,
        )
        .expect("non-quarantined request slot");
    }
    assert_eq!(
        reserve_request(
            &mut delayed_queue,
            Operation::Read,
            0,
            ptr::null_mut(),
            None,
        ),
        Err(Error::Busy)
    );
    assert!(!complete_request_in(
        &mut delayed_queue,
        CompletionId {
            generation: delayed.generation.wrapping_add(1),
            ..delayed
        },
        None,
    ));
    assert!(complete_request_in(&mut delayed_queue, delayed, None));
    let reused = reserve_request(
        &mut delayed_queue,
        Operation::Read,
        0,
        ptr::null_mut(),
        None,
    )
    .expect("released request slot");
    assert_eq!(reused.index, delayed.index);
    assert_ne!(reused.generation, delayed.generation);
    assert!(!complete_request_in(&mut delayed_queue, delayed, None));
    assert!(complete_request_in(&mut delayed_queue, reused, None));

    let mut cancellation_queue = [Request::EMPTY; QUEUE_DEPTH];
    let cancelled = reserve_request(
        &mut cancellation_queue,
        Operation::Read,
        0,
        ptr::null_mut(),
        None,
    )
    .expect("cancellable request slot");
    assert!(quarantine_request_in(
        &mut cancellation_queue,
        cancelled,
        Error::Timeout
    ));
    assert!(cancellation_confirmed_in(
        &mut cancellation_queue,
        cancelled
    ));
    assert!(!cancellation_confirmed_in(
        &mut cancellation_queue,
        cancelled
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
        ACTIVE_PARTITION = 0;
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

pub fn partition(index: usize) -> Option<Partition> {
    if index >= partition_count() || index >= MAX_PARTITIONS {
        return None;
    }
    unsafe { (&*core::ptr::addr_of!(PARTITIONS))[index] }
}

/// Return the bounded partition selected for filesystem probes.
///
/// An MBR bootable entry wins; otherwise the first valid entry is used. A
/// disk without a valid partition table is represented by the whole-disk
/// fallback entry created by `discover_partitions`.
pub fn active_partition() -> Option<Partition> {
    partition(unsafe { ACTIVE_PARTITION })
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
        return crate::drivers::virtio_blk::flush().map_err(map_virtio_error);
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
        None,
        PhantomData,
    )
}

pub fn submit_write<'a>(lba: u64, input: &'a [u8]) -> Result<RequestId<'a>, Error> {
    submit(
        Operation::Write,
        lba,
        input.as_ptr() as *mut u8,
        input.len(),
        Some(input),
        PhantomData,
    )
}

pub fn poll() {
    for index in 0..QUEUE_DEPTH {
        let (completion, result) = unsafe {
            let queue = &mut *core::ptr::addr_of_mut!(QUEUE);
            let request = &mut queue[index];
            if request.state != RequestState::Queued {
                continue;
            }
            (request.completion_id(index), process(request))
        };
        match result {
            ProcessResult::Pending => {}
            ProcessResult::Complete(Ok(())) => {
                let _ = complete_request(completion, None);
            }
            ProcessResult::Complete(Err(error)) => {
                let _ = quarantine_request(completion, error);
            }
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
        let state = unsafe { (&*core::ptr::addr_of!(QUEUE))[index] };
        if state.generation != request.generation {
            return Err(Error::InvalidRequest);
        }
        match state.state {
            RequestState::Complete => return take_completed(request),
            RequestState::Quarantined => return Err(state.error.unwrap_or(Error::Timeout)),
            RequestState::Free => return Err(Error::InvalidRequest),
            RequestState::Queued => core::hint::spin_loop(),
        }
    }
    let _ = quarantine_request(request.completion_id(), Error::Timeout);
    Err(Error::Timeout)
}

pub fn read_sectors(lba: u64, output: &mut [u8]) -> Result<usize, Error> {
    if output.is_empty() || !output.len().is_multiple_of(SECTOR_SIZE) {
        return Err(Error::InvalidRequest);
    }
    for (index, sector) in output.chunks_exact_mut(SECTOR_SIZE).enumerate() {
        let sector_lba = lba.checked_add(index as u64).ok_or(Error::OutOfRange)?;
        wait(submit_read(sector_lba, sector)?)?;
    }
    Ok(output.len())
}

pub fn write_sectors(lba: u64, input: &[u8]) -> Result<usize, Error> {
    if input.is_empty() || !input.len().is_multiple_of(SECTOR_SIZE) {
        return Err(Error::InvalidRequest);
    }
    for (index, sector) in input.chunks_exact(SECTOR_SIZE).enumerate() {
        let sector_lba = lba.checked_add(index as u64).ok_or(Error::OutOfRange)?;
        wait(submit_write(sector_lba, sector)?)?;
    }
    Ok(input.len())
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
    input: Option<&[u8]>,
    marker: PhantomData<&'a [u8]>,
) -> Result<RequestId<'a>, Error> {
    if !unsafe { READY } {
        return Err(Error::NotReady);
    }
    if length != SECTOR_SIZE || buffer.is_null() {
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
        let completion = reserve_request(queue, operation, lba, buffer, input)?;
        Ok(RequestId {
            index: completion.index,
            generation: completion.generation,
            _buffer: marker,
        })
    }
}

fn reserve_request(
    queue: &mut [Request; QUEUE_DEPTH],
    operation: Operation,
    lba: u64,
    user_buffer: *mut u8,
    input: Option<&[u8]>,
) -> Result<CompletionId, Error> {
    let Some((index, request)) = queue
        .iter_mut()
        .enumerate()
        .find(|(_, request)| request.state == RequestState::Free)
    else {
        return Err(Error::Busy);
    };
    let generation = request.generation.wrapping_add(1).max(1);
    *request = Request {
        state: RequestState::Queued,
        generation,
        operation,
        lba,
        length: SECTOR_SIZE,
        buffer: [0; SECTOR_SIZE],
        user_buffer,
        error: None,
    };
    if let Some(input) = input {
        request.buffer.copy_from_slice(input);
    }
    Ok(request.completion_id(index))
}

fn process(request: &mut Request) -> ProcessResult {
    match backend() {
        BackendKind::Ramdisk => {
            let start = request.lba as usize * SECTOR_SIZE;
            unsafe {
                let disk = (&raw mut RAMDISK).cast::<u8>().add(start);
                match request.operation {
                    Operation::Read => {
                        ptr::copy_nonoverlapping(disk, request.buffer.as_mut_ptr(), request.length)
                    }
                    Operation::Write => {
                        ptr::copy_nonoverlapping(request.buffer.as_ptr(), disk, request.length)
                    }
                }
            }
            ProcessResult::Complete(Ok(()))
        }
        BackendKind::VirtioBlk => {
            #[cfg(target_arch = "x86_64")]
            {
                let result = match request.operation {
                    Operation::Read => {
                        crate::drivers::virtio_blk::read_sector(request.lba, &mut request.buffer)
                    }
                    Operation::Write => {
                        crate::drivers::virtio_blk::write_sector(request.lba, &request.buffer)
                    }
                }
                .map_err(map_virtio_error);
                ProcessResult::Complete(result)
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                ProcessResult::Complete(Err(Error::Unsupported))
            }
        }
    }
}

fn complete_request_in(
    queue: &mut [Request; QUEUE_DEPTH],
    completion: CompletionId,
    error: Option<Error>,
) -> bool {
    let Some(request) = queue.get_mut(completion.index as usize) else {
        return false;
    };
    if request.generation != completion.generation {
        return false;
    }
    match request.state {
        RequestState::Queued => {
            request.error = error;
            request.state = RequestState::Complete;
            true
        }
        RequestState::Quarantined => {
            request.release();
            true
        }
        RequestState::Free | RequestState::Complete => false,
    }
}

fn quarantine_request_in(
    queue: &mut [Request; QUEUE_DEPTH],
    completion: CompletionId,
    error: Error,
) -> bool {
    let Some(request) = queue.get_mut(completion.index as usize) else {
        return false;
    };
    if request.generation != completion.generation || request.state != RequestState::Queued {
        return false;
    }
    request.error = Some(error);
    request.state = RequestState::Quarantined;
    true
}

fn cancellation_confirmed_in(queue: &mut [Request; QUEUE_DEPTH], completion: CompletionId) -> bool {
    let Some(request) = queue.get_mut(completion.index as usize) else {
        return false;
    };
    if request.generation != completion.generation {
        return false;
    }
    match request.state {
        RequestState::Queued => {
            request.error = Some(Error::Timeout);
            request.state = RequestState::Complete;
            true
        }
        RequestState::Quarantined => {
            request.release();
            true
        }
        RequestState::Free | RequestState::Complete => false,
    }
}

fn complete_request(completion: CompletionId, error: Option<Error>) -> bool {
    let completed =
        unsafe { complete_request_in(&mut *core::ptr::addr_of_mut!(QUEUE), completion, error) };
    if completed {
        unsafe { COMPLETED = COMPLETED.saturating_add(1) };
    }
    completed
}

fn quarantine_request(completion: CompletionId, error: Error) -> bool {
    let quarantined =
        unsafe { quarantine_request_in(&mut *core::ptr::addr_of_mut!(QUEUE), completion, error) };
    if quarantined && error == Error::Timeout {
        unsafe { TIMEOUTS = TIMEOUTS.saturating_add(1) };
    }
    quarantined
}

/// Notify the block layer that a lower driver completed a request.
///
/// A completion for a quarantined request only releases the request-owned
/// bounce buffer; it never copies data into the caller's buffer. The lower
/// driver must pass the generation it received when it took ownership.
pub(crate) fn complete(completion: CompletionId, error: Option<Error>) -> bool {
    complete_request(completion, error)
}

/// Notify the block layer that a lower driver cancelled and quiesced a request.
///
/// This is an ownership release acknowledgement, not a cancellation request.
/// The caller must have stopped the device and detached its descriptor before
/// invoking it.
pub(crate) fn cancellation_confirmed(completion: CompletionId) -> bool {
    let cancelled =
        unsafe { cancellation_confirmed_in(&mut *core::ptr::addr_of_mut!(QUEUE), completion) };
    if cancelled {
        unsafe { COMPLETED = COMPLETED.saturating_add(1) };
    }
    cancelled
}

fn take_completed(request: RequestId<'_>) -> Result<usize, Error> {
    let index = request.index as usize;
    unsafe {
        let queue = &mut *core::ptr::addr_of_mut!(QUEUE);
        let queued_request = queue.get_mut(index).ok_or(Error::InvalidRequest)?;
        if queued_request.generation != request.generation
            || queued_request.state != RequestState::Complete
        {
            return Err(Error::InvalidRequest);
        }
        let length = queued_request.length;
        let error = queued_request.error;
        if error.is_none() && queued_request.operation == Operation::Read {
            ptr::copy_nonoverlapping(
                queued_request.buffer.as_ptr(),
                queued_request.user_buffer,
                length,
            );
        }
        queued_request.release();
        error.map_or(Ok(length), Err)
    }
}

#[cfg(target_arch = "x86_64")]
fn map_virtio_error(error: crate::drivers::virtio_blk::Error) -> Error {
    match error {
        crate::drivers::virtio_blk::Error::Timeout => Error::Timeout,
        crate::drivers::virtio_blk::Error::OutOfRange => Error::OutOfRange,
        crate::drivers::virtio_blk::Error::Busy => Error::Busy,
        crate::drivers::virtio_blk::Error::NotReady => Error::NotReady,
        crate::drivers::virtio_blk::Error::FlushUnsupported
        | crate::drivers::virtio_blk::Error::UnsupportedFeatures => Error::Unsupported,
        _ => Error::Device,
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
        for offset in 446..510 {
            disk.add(offset).write(0);
        }
        let entry = disk.add(446);
        entry.write(0x80);
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
    let mut active = None;
    let has_mbr = sector[510] == 0x55 && sector[511] == 0xaa;
    if has_mbr {
        for index in 0..MAX_PARTITIONS {
            let offset = 446 + index * 16;
            let status = sector[offset];
            let type_code = sector[offset + 4];
            let start_lba = mbr_u32(&sector, offset + 8) as u64;
            let sectors = mbr_u32(&sector, offset + 12) as u64;
            if !matches!(status, 0x00 | 0x80)
                || type_code == 0
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
            if status == 0x80 && active.is_none() {
                active = Some(count);
            }
            count += 1;
        }
    }
    if count == 0 && !has_mbr {
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
    unsafe {
        ACTIVE_PARTITION = active.unwrap_or(0);
        PARTITION_COUNT = count;
    }
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
