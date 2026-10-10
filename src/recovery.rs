#![allow(
    dead_code,
    reason = "Staged ABI and hardware hooks are not all enabled in the bootstrap profile"
)]

//! Durable recovery metadata.
//!
//! The warm-boot ring in [`crate::crash`] answers "what happened to the last
//! boot?".  This journal answers the recovery questions that a package or
//! service manager needs: whether a generation was prepared but never made
//! healthy, whether a watchdog or reboot was requested, and which rollback
//! decision was recorded.  The journal is deliberately separate from the
//! warm-boot ABI so a status record cannot be mistaken for a recovery proof.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::error::KernelError;

const MAGIC: u32 = 0x4e52_5852;
const VERSION: u16 = 1;
pub const RECORD_SIZE: usize = 192;
const RECORDS: usize = 4;
const CHECKSUM_OFFSET: usize = 164;
const MAX_CORE_DUMP_BYTES: u64 = 16 * 1024 * 1024;
const HALF_SEQUENCE: u64 = 1 << 63;

const EFI_RECOVERY_GUID: [u8; 16] = [
    0x6e, 0x6f, 0x72, 0x78, 0x2d, 0x72, 0x65, 0x63, 0x6f, 0x76, 0x65, 0x72, 0x79, 0x00, 0x01, 0x00,
];
const EFI_RECOVERY_NAMES: [[u16; 9]; RECORDS] = [
    [
        b'N' as u16,
        b'o' as u16,
        b'r' as u16,
        b'x' as u16,
        b'R' as u16,
        b'e' as u16,
        b'c' as u16,
        b'0' as u16,
        0,
    ],
    [
        b'N' as u16,
        b'o' as u16,
        b'r' as u16,
        b'x' as u16,
        b'R' as u16,
        b'e' as u16,
        b'c' as u16,
        b'1' as u16,
        0,
    ],
    [
        b'N' as u16,
        b'o' as u16,
        b'r' as u16,
        b'x' as u16,
        b'R' as u16,
        b'e' as u16,
        b'c' as u16,
        b'2' as u16,
        0,
    ],
    [
        b'N' as u16,
        b'o' as u16,
        b'r' as u16,
        b'x' as u16,
        b'R' as u16,
        b'e' as u16,
        b'c' as u16,
        b'3' as u16,
        0,
    ],
];

static READY: AtomicBool = AtomicBool::new(false);
static BOOT_ID: AtomicU64 = AtomicU64::new(0);
static WATCHDOG_ACTIVE: AtomicBool = AtomicBool::new(false);
static WATCHDOG_EXPIRED: AtomicBool = AtomicBool::new(false);
static WATCHDOG_GENERATION: AtomicU64 = AtomicU64::new(0);
static WATCHDOG_TOKEN: AtomicU64 = AtomicU64::new(0);
static WATCHDOG_DEADLINE: AtomicU64 = AtomicU64::new(0);
static WATCHDOG_TIMEOUT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RecordKind {
    BootStart = 1,
    BootReady = 2,
    Crash = 3,
    CoreDump = 4,
    Watchdog = 5,
    Reboot = 6,
    PackagePrepare = 7,
    PackageCommit = 8,
    PackageRollback = 9,
    Recovery = 10,
}

impl RecordKind {
    fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::BootStart,
            2 => Self::BootReady,
            3 => Self::Crash,
            4 => Self::CoreDump,
            5 => Self::Watchdog,
            6 => Self::Reboot,
            7 => Self::PackagePrepare,
            8 => Self::PackageCommit,
            9 => Self::PackageRollback,
            10 => Self::Recovery,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum RecordState {
    Pending = 1,
    Committed = 2,
    Required = 3,
    Requested = 4,
    Complete = 5,
}

impl RecordState {
    fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Pending,
            2 => Self::Committed,
            3 => Self::Required,
            4 => Self::Requested,
            5 => Self::Complete,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CoreDumpState {
    None = 0,
    MetadataOnly = 1,
    Complete = 2,
}

impl CoreDumpState {
    fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::None,
            1 => Self::MetadataOnly,
            2 => Self::Complete,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebootReason {
    User,
    Watchdog,
    Recovery,
    PackageUpdate,
}

impl RebootReason {
    const fn code(self) -> u64 {
        match self {
            Self::User => 1,
            Self::Watchdog => 2,
            Self::Recovery => 3,
            Self::PackageUpdate => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoreDumpMetadata {
    pub process: u32,
    pub thread: u32,
    pub format: u8,
    pub payload_offset: u64,
    pub payload_length: u64,
    pub payload_checksum: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchdogHandle {
    generation: u64,
    token: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackageTransaction {
    candidate: u64,
    base: u64,
    sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unavailable,
    WriteFailed,
    InvalidState,
    InvalidGeneration,
    Busy,
    StaleHandle,
    NoRollback,
    Overflow,
    InvalidCoreDump,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub persistent: bool,
    pub last_sequence: u64,
    pub last_kind: Option<RecordKind>,
    pub active_generation: u64,
    pub pending_generation: Option<u64>,
    pub rollback_required: bool,
    pub watchdog_armed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Record {
    kind: RecordKind,
    state: RecordState,
    sequence: u64,
    boot_id: u64,
    ticks: u64,
    reason: u64,
    arg0: u64,
    arg1: u64,
    process: u32,
    thread: u32,
    core_state: CoreDumpState,
    core_format: u8,
    core_offset: u64,
    core_length: u64,
    core_checksum: u64,
    active_generation: u64,
    candidate_generation: u64,
    previous_generation: u64,
    digest: [u8; 32],
    watchdog_token: u64,
    watchdog_deadline: u64,
}

impl Record {
    const EMPTY: Self = Self {
        kind: RecordKind::BootStart,
        state: RecordState::Pending,
        sequence: 0,
        boot_id: 0,
        ticks: 0,
        reason: 0,
        arg0: 0,
        arg1: 0,
        process: 0,
        thread: 0,
        core_state: CoreDumpState::None,
        core_format: 0,
        core_offset: 0,
        core_length: 0,
        core_checksum: 0,
        active_generation: 0,
        candidate_generation: 0,
        previous_generation: 0,
        digest: [0; 32],
        watchdog_token: 0,
        watchdog_deadline: 0,
    };
}

#[derive(Clone, Copy)]
struct PackageState {
    active: u64,
    previous: u64,
    digest: [u8; 32],
    pending: Option<(u64, u64, u64, [u8; 32])>,
}

impl PackageState {
    const EMPTY: Self = Self {
        active: 0,
        previous: 0,
        digest: [0; 32],
        pending: None,
    };
}

pub fn contract_self_check() {
    assert_eq!(RECORD_SIZE, 192);
    assert!(RECORD_SIZE <= crate::crash::record_capacity());

    let record = Record {
        kind: RecordKind::CoreDump,
        state: RecordState::Committed,
        sequence: 9,
        boot_id: 8,
        ticks: 7,
        reason: 6,
        arg0: 5,
        arg1: 4,
        process: 3,
        thread: 2,
        core_state: CoreDumpState::MetadataOnly,
        core_format: 1,
        core_offset: 4096,
        core_length: 8192,
        core_checksum: 0xaabb_ccdd,
        active_generation: 11,
        candidate_generation: 12,
        previous_generation: 10,
        digest: [0x5a; 32],
        watchdog_token: 13,
        watchdog_deadline: 14,
    };
    let mut bytes = [0u8; RECORD_SIZE];
    assert!(encode(record, &mut bytes));
    assert_eq!(decode(&bytes), Some(record));
    bytes[0] ^= 1;
    assert_eq!(decode(&bytes), None);
    assert_eq!(decode(&bytes[..RECORD_SIZE - 1]), None);
    assert!(validate_core_dump(CoreDumpMetadata {
        process: 1,
        thread: 2,
        format: 3,
        payload_offset: 0x1000,
        payload_length: 64,
        payload_checksum: 4,
    })
    .is_ok());
    assert_eq!(
        validate_core_dump(CoreDumpMetadata {
            process: 0,
            thread: 0,
            format: 0,
            payload_offset: u64::MAX,
            payload_length: 2,
            payload_checksum: 0,
        }),
        Err(Error::InvalidCoreDump)
    );
    assert_eq!(next_deadline(u64::MAX - 1, 2), None);
    assert!(deadline_due(11, 10));
    assert!(!deadline_due(9, 10));

    let prepare = Record {
        kind: RecordKind::PackagePrepare,
        state: RecordState::Pending,
        sequence: 1,
        active_generation: 4,
        candidate_generation: 5,
        previous_generation: 2,
        digest: [7; 32],
        ..Record::EMPTY
    };
    let mut state = PackageState::EMPTY;
    apply_package_record(&mut state, prepare);
    assert_eq!(state.pending, Some((5, 4, 2, [7; 32])));
    let commit = Record {
        kind: RecordKind::PackageCommit,
        state: RecordState::Committed,
        sequence: 2,
        active_generation: 5,
        candidate_generation: 5,
        previous_generation: 4,
        digest: [7; 32],
        ..Record::EMPTY
    };
    apply_package_record(&mut state, commit);
    assert_eq!(state.active, 5);
    assert_eq!(state.previous, 4);
    assert!(state.pending.is_none());
    let rollback = Record {
        kind: RecordKind::PackageRollback,
        state: RecordState::Committed,
        sequence: 3,
        active_generation: 4,
        candidate_generation: 5,
        previous_generation: 0,
        ..Record::EMPTY
    };
    apply_package_record(&mut state, rollback);
    assert_eq!(state.active, 4);
    assert_eq!(state.previous, 0);
}

pub fn init() -> bool {
    if !crate::crash::efi_runtime_available() {
        crate::bootlog::warn("durable recovery journal unavailable; recovery claims disabled");
        READY.store(false, Ordering::Release);
        return false;
    }

    let mut records = [None; RECORDS];
    read_records(&mut records);
    let previous = latest(&records);
    if let Some(record) = previous.map(|(_, record)| record) {
        log_previous(record);
    }
    let package = package_state(&records);
    if let Some((candidate, target, reason)) = recovery_target(&records, package) {
        let _ = append(Record {
            kind: RecordKind::Recovery,
            state: RecordState::Required,
            reason,
            active_generation: target,
            candidate_generation: candidate,
            ..Record::EMPTY
        });
        crate::bootlog::warn_fmt(format_args!(
            "package recovery required candidate={} target={}",
            candidate, target
        ));
    }

    let sequence = previous
        .map(|(_, record)| record.sequence)
        .unwrap_or(0)
        .saturating_add(1);
    let boot_id = crate::time::ticks()
        .wrapping_add(sequence.rotate_left(17))
        .max(1);
    BOOT_ID.store(boot_id, Ordering::Release);
    READY.store(true, Ordering::Release);
    if append(Record {
        kind: RecordKind::BootStart,
        state: RecordState::Pending,
        boot_id,
        ticks: crate::time::ticks(),
        ..Record::EMPTY
    })
    .is_err()
    {
        READY.store(false, Ordering::Release);
        crate::bootlog::warn("durable recovery boot-start record failed");
        return false;
    }
    crate::bootlog::ok_fmt(format_args!(
        "durable recovery journal ready records={} sequence={}",
        RECORDS, sequence
    ));
    true
}

pub fn mark_ready() -> bool {
    append(Record {
        kind: RecordKind::BootReady,
        state: RecordState::Committed,
        boot_id: BOOT_ID.load(Ordering::Acquire),
        ticks: crate::time::ticks(),
        ..Record::EMPTY
    })
    .is_ok()
}

pub fn record_crash(error: KernelError) -> bool {
    append(Record {
        kind: RecordKind::Crash,
        state: RecordState::Committed,
        boot_id: BOOT_ID.load(Ordering::Acquire),
        ticks: crate::time::ticks(),
        reason: error.code,
        arg0: error.arg0,
        arg1: error.arg1,
        ..Record::EMPTY
    })
    .is_ok()
}

pub fn record_core_dump_metadata(metadata: CoreDumpMetadata) -> Result<(), Error> {
    validate_core_dump(metadata)?;
    append(Record {
        kind: RecordKind::CoreDump,
        state: RecordState::Committed,
        boot_id: BOOT_ID.load(Ordering::Acquire),
        ticks: crate::time::ticks(),
        process: metadata.process,
        thread: metadata.thread,
        core_state: CoreDumpState::MetadataOnly,
        core_format: metadata.format,
        core_offset: metadata.payload_offset,
        core_length: metadata.payload_length,
        core_checksum: metadata.payload_checksum,
        ..Record::EMPTY
    })
    .map(|_| ())
}

pub fn arm_watchdog(timeout_ticks: u64) -> Result<WatchdogHandle, Error> {
    let now = crate::time::ticks();
    let deadline = next_deadline(now, timeout_ticks).ok_or(Error::Overflow)?;
    if WATCHDOG_ACTIVE.load(Ordering::Acquire) {
        return Err(Error::Busy);
    }
    let generation = WATCHDOG_GENERATION
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1)
        .max(1);
    let token = now
        .rotate_left(23)
        .wrapping_add(generation.rotate_left(41))
        .max(1);
    WATCHDOG_TOKEN.store(token, Ordering::Release);
    WATCHDOG_DEADLINE.store(deadline, Ordering::Release);
    WATCHDOG_TIMEOUT.store(timeout_ticks, Ordering::Release);
    WATCHDOG_EXPIRED.store(false, Ordering::Release);
    if WATCHDOG_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(Error::Busy);
    }
    Ok(WatchdogHandle { generation, token })
}

pub fn heartbeat_watchdog(handle: WatchdogHandle) -> Result<(), Error> {
    if !watchdog_matches(handle) {
        return Err(Error::StaleHandle);
    }
    let deadline = next_deadline(
        crate::time::ticks(),
        WATCHDOG_TIMEOUT.load(Ordering::Acquire),
    )
    .ok_or(Error::Overflow)?;
    WATCHDOG_DEADLINE.store(deadline, Ordering::Release);
    Ok(())
}

pub fn disarm_watchdog(handle: WatchdogHandle) -> Result<(), Error> {
    if !watchdog_matches(handle) {
        return Err(Error::StaleHandle);
    }
    WATCHDOG_ACTIVE.store(false, Ordering::Release);
    Ok(())
}

pub fn poll_watchdog() -> bool {
    if !WATCHDOG_ACTIVE.load(Ordering::Acquire)
        || WATCHDOG_EXPIRED.load(Ordering::Acquire)
        || !deadline_due(
            crate::time::ticks(),
            WATCHDOG_DEADLINE.load(Ordering::Acquire),
        )
    {
        return false;
    }
    if WATCHDOG_EXPIRED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    WATCHDOG_ACTIVE.store(false, Ordering::Release);
    let token = WATCHDOG_TOKEN.load(Ordering::Acquire);
    let deadline = WATCHDOG_DEADLINE.load(Ordering::Acquire);
    let _ = append(Record {
        kind: RecordKind::Watchdog,
        state: RecordState::Committed,
        boot_id: BOOT_ID.load(Ordering::Acquire),
        ticks: crate::time::ticks(),
        arg0: token,
        watchdog_token: token,
        watchdog_deadline: deadline,
        ..Record::EMPTY
    });
    true
}

pub fn reboot(reason: RebootReason) -> ! {
    let _ = append(Record {
        kind: RecordKind::Reboot,
        state: RecordState::Committed,
        boot_id: BOOT_ID.load(Ordering::Acquire),
        ticks: crate::time::ticks(),
        reason: reason.code(),
        ..Record::EMPTY
    });
    crate::arch::reboot()
}

pub fn prepare_package_generation(
    candidate: u64,
    digest: [u8; 32],
) -> Result<PackageTransaction, Error> {
    if candidate == 0 {
        return Err(Error::InvalidGeneration);
    }
    let records = load_records()?;
    let state = package_state(&records);
    if state.pending.is_some() {
        return Err(Error::Busy);
    }
    if candidate <= state.active {
        return Err(Error::InvalidGeneration);
    }
    let sequence = append(Record {
        kind: RecordKind::PackagePrepare,
        state: RecordState::Pending,
        boot_id: BOOT_ID.load(Ordering::Acquire),
        ticks: crate::time::ticks(),
        active_generation: state.active,
        candidate_generation: candidate,
        previous_generation: state.previous,
        digest,
        ..Record::EMPTY
    })?;
    Ok(PackageTransaction {
        candidate,
        base: state.active,
        sequence,
    })
}

pub fn commit_package_generation(transaction: PackageTransaction) -> Result<(), Error> {
    let records = load_records()?;
    let state = package_state(&records);
    let Some((candidate, base, _previous, digest)) = state.pending else {
        return Err(Error::StaleHandle);
    };
    let Some((_, latest_prepare)) = latest_kind(&records, RecordKind::PackagePrepare) else {
        return Err(Error::StaleHandle);
    };
    if candidate != transaction.candidate
        || base != transaction.base
        || latest_prepare.sequence != transaction.sequence
    {
        return Err(Error::StaleHandle);
    }
    append(Record {
        kind: RecordKind::PackageCommit,
        state: RecordState::Committed,
        boot_id: BOOT_ID.load(Ordering::Acquire),
        ticks: crate::time::ticks(),
        active_generation: candidate,
        candidate_generation: candidate,
        previous_generation: base,
        digest,
        ..Record::EMPTY
    })?;
    Ok(())
}

pub fn rollback_package_generation() -> Result<u64, Error> {
    let records = load_records()?;
    let state = package_state(&records);
    let (target, failed, previous, digest) =
        if let Some((candidate, base, old_previous, digest)) = state.pending {
            (base, candidate, old_previous, digest)
        } else if state.active != 0 && state.previous != 0 {
            (state.previous, state.active, 0, state.digest)
        } else {
            return Err(Error::NoRollback);
        };
    append(Record {
        kind: RecordKind::PackageRollback,
        state: RecordState::Committed,
        boot_id: BOOT_ID.load(Ordering::Acquire),
        ticks: crate::time::ticks(),
        active_generation: target,
        candidate_generation: failed,
        previous_generation: previous,
        digest,
        ..Record::EMPTY
    })?;
    if latest_recovery_state(&records) == Some(RecordState::Required)
        || latest_recovery_state(&records) == Some(RecordState::Requested)
    {
        append(Record {
            kind: RecordKind::Recovery,
            state: RecordState::Complete,
            boot_id: BOOT_ID.load(Ordering::Acquire),
            ticks: crate::time::ticks(),
            active_generation: target,
            candidate_generation: failed,
            ..Record::EMPTY
        })?;
    }
    Ok(target)
}

pub fn request_recovery_rollback() -> Result<(u64, u64), Error> {
    let records = load_records()?;
    let package = package_state(&records);
    let Some((candidate, target, _)) = recovery_target(&records, package) else {
        return Err(Error::NoRollback);
    };
    append(Record {
        kind: RecordKind::Recovery,
        state: RecordState::Requested,
        boot_id: BOOT_ID.load(Ordering::Acquire),
        ticks: crate::time::ticks(),
        active_generation: target,
        candidate_generation: candidate,
        ..Record::EMPTY
    })?;
    Ok((candidate, target))
}

pub fn status() -> Status {
    let Ok(records) = load_records() else {
        return Status {
            persistent: false,
            last_sequence: 0,
            last_kind: None,
            active_generation: 0,
            pending_generation: None,
            rollback_required: false,
            watchdog_armed: WATCHDOG_ACTIVE.load(Ordering::Acquire),
        };
    };
    let package = package_state(&records);
    let last = latest(&records).map(|(_, record)| record);
    let recovery = latest_recovery(&records);
    Status {
        persistent: true,
        last_sequence: last.map(|record| record.sequence).unwrap_or(0),
        last_kind: last.map(|record| record.kind),
        active_generation: package.active,
        pending_generation: package.pending.map(|(candidate, _, _, _)| candidate),
        rollback_required: matches!(
            recovery.map(|(_, record)| record.state),
            Some(RecordState::Required | RecordState::Requested)
        ),
        watchdog_armed: WATCHDOG_ACTIVE.load(Ordering::Acquire),
    }
}

fn load_records() -> Result<[Option<Record>; RECORDS], Error> {
    if !crate::crash::efi_runtime_available() {
        return Err(Error::Unavailable);
    }
    let mut records = [None; RECORDS];
    read_records(&mut records);
    Ok(records)
}

fn read_records(records: &mut [Option<Record>; RECORDS]) {
    records.fill(None);
    for (slot, name) in EFI_RECOVERY_NAMES.iter().enumerate() {
        let mut bytes = [0u8; RECORD_SIZE];
        if crate::crash::efi_read_variable(name, &EFI_RECOVERY_GUID, &mut bytes) {
            records[slot] = decode(&bytes);
        }
    }
}

fn append(mut record: Record) -> Result<u64, Error> {
    if !crate::crash::efi_runtime_available() {
        return Err(Error::Unavailable);
    }
    let mut records = [None; RECORDS];
    read_records(&mut records);
    let (slot, sequence) = latest(&records)
        .map(|(slot, previous)| {
            (
                (slot + 1) % RECORDS,
                previous.sequence.checked_add(1).ok_or(Error::Overflow),
            )
        })
        .map_or(Ok((0, 1)), |(slot, sequence)| {
            sequence.map(|sequence| (slot, sequence))
        })?;
    record.sequence = sequence;
    let mut bytes = [0u8; RECORD_SIZE];
    if !encode(record, &mut bytes)
        || !crate::crash::efi_write_variable(&EFI_RECOVERY_NAMES[slot], &EFI_RECOVERY_GUID, &bytes)
    {
        return Err(Error::WriteFailed);
    }
    let mut verify = [0u8; RECORD_SIZE];
    if !crate::crash::efi_read_variable(&EFI_RECOVERY_NAMES[slot], &EFI_RECOVERY_GUID, &mut verify)
        || decode(&verify) != Some(record)
    {
        return Err(Error::WriteFailed);
    }
    Ok(sequence)
}

fn latest(records: &[Option<Record>; RECORDS]) -> Option<(usize, Record)> {
    records
        .iter()
        .enumerate()
        .filter_map(|(slot, record)| record.map(|record| (slot, record)))
        .max_by(|(_, left), (_, right)| sequence_cmp(left.sequence, right.sequence))
}

fn latest_kind(records: &[Option<Record>; RECORDS], kind: RecordKind) -> Option<(usize, Record)> {
    records
        .iter()
        .enumerate()
        .filter_map(|(slot, record)| {
            record
                .filter(|record| record.kind == kind)
                .map(|record| (slot, record))
        })
        .max_by(|(_, left), (_, right)| sequence_cmp(left.sequence, right.sequence))
}

fn latest_recovery(records: &[Option<Record>; RECORDS]) -> Option<(usize, Record)> {
    latest_kind(records, RecordKind::Recovery)
}

fn latest_recovery_state(records: &[Option<Record>; RECORDS]) -> Option<RecordState> {
    latest_recovery(records).map(|(_, record)| record.state)
}

fn ordered(records: &[Option<Record>; RECORDS]) -> ([Record; RECORDS], usize) {
    let mut ordered = [Record::EMPTY; RECORDS];
    let mut length = 0;
    for record in records.iter().flatten().copied() {
        let mut index = length;
        while index != 0 && sequence_cmp(record.sequence, ordered[index - 1].sequence).is_lt() {
            ordered[index] = ordered[index - 1];
            index -= 1;
        }
        ordered[index] = record;
        length += 1;
    }
    (ordered, length)
}

fn package_state(records: &[Option<Record>; RECORDS]) -> PackageState {
    let (ordered, length) = ordered(records);
    let mut state = PackageState::EMPTY;
    for record in ordered[..length].iter().copied() {
        apply_package_record(&mut state, record);
    }
    state
}

fn apply_package_record(state: &mut PackageState, record: Record) {
    match record.kind {
        RecordKind::PackagePrepare if record.state == RecordState::Pending => {
            state.pending = Some((
                record.candidate_generation,
                record.active_generation,
                record.previous_generation,
                record.digest,
            ));
        }
        RecordKind::PackageCommit if record.state == RecordState::Committed => {
            state.active = record.active_generation;
            state.previous = record.previous_generation;
            state.digest = record.digest;
            state.pending = None;
        }
        RecordKind::PackageRollback if record.state == RecordState::Committed => {
            state.active = record.active_generation;
            state.previous = record.previous_generation;
            state.digest = record.digest;
            state.pending = None;
        }
        _ => {}
    }
}

fn recovery_target(
    records: &[Option<Record>; RECORDS],
    package: PackageState,
) -> Option<(u64, u64, u64)> {
    let latest = latest(records)?.1;
    if matches!(latest.kind, RecordKind::Reboot | RecordKind::BootReady) {
        return None;
    }
    if let Some((candidate, base, _, _)) = package.pending {
        return Some((candidate, base, latest.reason));
    }
    if latest.kind == RecordKind::PackageCommit
        && latest.active_generation != 0
        && latest.previous_generation != 0
    {
        return Some((
            latest.active_generation,
            latest.previous_generation,
            latest.reason,
        ));
    }
    None
}

fn log_previous(record: Record) {
    match record.kind {
        RecordKind::BootReady => crate::bootlog::ok_fmt(format_args!(
            "previous recovery boot completed sequence={} generation={}",
            record.sequence, record.active_generation
        )),
        RecordKind::Crash => crate::bootlog::warn_fmt(format_args!(
            "previous recovery crash sequence={} code=0x{:x}",
            record.sequence, record.reason
        )),
        RecordKind::Watchdog => crate::bootlog::warn_fmt(format_args!(
            "previous recovery watchdog expiry sequence={} token=0x{:x}",
            record.sequence, record.watchdog_token
        )),
        RecordKind::Reboot => crate::bootlog::ok_fmt(format_args!(
            "previous recovery reboot intent sequence={} reason={}",
            record.sequence, record.reason
        )),
        RecordKind::PackagePrepare | RecordKind::PackageCommit => {
            crate::bootlog::warn_fmt(format_args!(
                "previous package generation incomplete sequence={} candidate={} active={}",
                record.sequence, record.candidate_generation, record.active_generation
            ))
        }
        _ => {}
    }
}

fn watchdog_matches(handle: WatchdogHandle) -> bool {
    WATCHDOG_ACTIVE.load(Ordering::Acquire)
        && !WATCHDOG_EXPIRED.load(Ordering::Acquire)
        && WATCHDOG_GENERATION.load(Ordering::Acquire) == handle.generation
        && WATCHDOG_TOKEN.load(Ordering::Acquire) == handle.token
}

fn validate_core_dump(metadata: CoreDumpMetadata) -> Result<(), Error> {
    if metadata.format == 0
        || metadata.payload_length == 0
        || metadata.payload_length > MAX_CORE_DUMP_BYTES
        || metadata
            .payload_offset
            .checked_add(metadata.payload_length)
            .is_none()
    {
        return Err(Error::InvalidCoreDump);
    }
    Ok(())
}

fn next_deadline(now: u64, timeout: u64) -> Option<u64> {
    (timeout != 0 && timeout < HALF_SEQUENCE)
        .then(|| now.checked_add(timeout))
        .flatten()
}

fn deadline_due(now: u64, deadline: u64) -> bool {
    now.wrapping_sub(deadline) < HALF_SEQUENCE
}

fn sequence_cmp(left: u64, right: u64) -> core::cmp::Ordering {
    if left == right {
        core::cmp::Ordering::Equal
    } else if left.wrapping_sub(right) < HALF_SEQUENCE {
        core::cmp::Ordering::Greater
    } else {
        core::cmp::Ordering::Less
    }
}

fn encode(record: Record, output: &mut [u8]) -> bool {
    if output.len() < RECORD_SIZE || record.sequence == 0 {
        return false;
    }
    output[..RECORD_SIZE].fill(0);
    put_u32(output, 0, MAGIC);
    put_u16(output, 4, VERSION);
    output[6] = record.kind as u8;
    output[7] = record.state as u8;
    put_u64(output, 8, record.sequence);
    put_u64(output, 16, record.boot_id);
    put_u64(output, 24, record.ticks);
    put_u64(output, 32, record.reason);
    put_u64(output, 40, record.arg0);
    put_u64(output, 48, record.arg1);
    put_u32(output, 56, record.process);
    put_u32(output, 60, record.thread);
    output[64] = record.core_state as u8;
    output[65] = record.core_format;
    put_u64(output, 68, record.core_offset);
    put_u64(output, 76, record.core_length);
    put_u64(output, 84, record.core_checksum);
    put_u64(output, 92, record.active_generation);
    put_u64(output, 100, record.candidate_generation);
    put_u64(output, 108, record.previous_generation);
    output[116..148].copy_from_slice(&record.digest);
    put_u64(output, 148, record.watchdog_token);
    put_u64(output, 156, record.watchdog_deadline);
    put_u32(output, CHECKSUM_OFFSET, checksum(&output[..RECORD_SIZE]));
    true
}

fn decode(input: &[u8]) -> Option<Record> {
    if input.len() < RECORD_SIZE
        || u32::from_le_bytes(input[0..4].try_into().ok()?) != MAGIC
        || u16::from_le_bytes(input[4..6].try_into().ok()?) != VERSION
        || input[66] != 0
        || input[67] != 0
        || input[168..RECORD_SIZE].iter().any(|byte| *byte != 0)
    {
        return None;
    }
    let kind = RecordKind::from_u8(input[6])?;
    let state = RecordState::from_u8(input[7])?;
    let sequence = u64::from_le_bytes(input[8..16].try_into().ok()?);
    if sequence == 0 {
        return None;
    }
    let stored = u32::from_le_bytes(
        input[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 4]
            .try_into()
            .ok()?,
    );
    let mut checked = [0u8; RECORD_SIZE];
    checked.copy_from_slice(&input[..RECORD_SIZE]);
    checked[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 4].fill(0);
    if stored != checksum(&checked) {
        return None;
    }
    Some(Record {
        kind,
        state,
        sequence,
        boot_id: u64::from_le_bytes(input[16..24].try_into().ok()?),
        ticks: u64::from_le_bytes(input[24..32].try_into().ok()?),
        reason: u64::from_le_bytes(input[32..40].try_into().ok()?),
        arg0: u64::from_le_bytes(input[40..48].try_into().ok()?),
        arg1: u64::from_le_bytes(input[48..56].try_into().ok()?),
        process: u32::from_le_bytes(input[56..60].try_into().ok()?),
        thread: u32::from_le_bytes(input[60..64].try_into().ok()?),
        core_state: CoreDumpState::from_u8(input[64])?,
        core_format: input[65],
        core_offset: u64::from_le_bytes(input[68..76].try_into().ok()?),
        core_length: u64::from_le_bytes(input[76..84].try_into().ok()?),
        core_checksum: u64::from_le_bytes(input[84..92].try_into().ok()?),
        active_generation: u64::from_le_bytes(input[92..100].try_into().ok()?),
        candidate_generation: u64::from_le_bytes(input[100..108].try_into().ok()?),
        previous_generation: u64::from_le_bytes(input[108..116].try_into().ok()?),
        digest: input[116..148].try_into().ok()?,
        watchdog_token: u64::from_le_bytes(input[148..156].try_into().ok()?),
        watchdog_deadline: u64::from_le_bytes(input[156..164].try_into().ok()?),
    })
}

fn checksum(bytes: &[u8]) -> u32 {
    let mut value = 0x9e37_79b9u32;
    for (index, byte) in bytes.iter().enumerate() {
        if (CHECKSUM_OFFSET..CHECKSUM_OFFSET + 4).contains(&index) {
            continue;
        }
        value = value.rotate_left(5) ^ u32::from(*byte);
        value = value.wrapping_mul(0x0100_019f);
    }
    value
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
