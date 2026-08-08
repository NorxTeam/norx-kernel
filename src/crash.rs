use crate::error::KernelError;

const MAGIC: u32 = 0x4e52_5843;
const VERSION: u16 = 1;
const RECORDS: usize = 4;
const RECORD_SIZE: usize = 64;
const STATE_BOOTING: u8 = 1;
const STATE_READY: u8 = 2;
const STATE_PANIC: u8 = 3;
const STATE_CHECKPOINT: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Record {
    state: u8,
    kind: u8,
    sequence: u64,
    code: u64,
    arg0: u64,
    arg1: u64,
}

static mut PANIC_ACTIVE: bool = false;
static mut EFI_DATA: [u8; crate::drivers::block::SECTOR_SIZE] =
    [0; crate::drivers::block::SECTOR_SIZE];
static mut EFI_ATTRIBUTES: u32 = 0;
static mut EFI_SIZE: usize = 0;

pub fn contract_self_check() {
    let record = Record {
        state: STATE_PANIC,
        kind: 2,
        sequence: 9,
        code: 0x2014,
        arg0: 0x4000,
        arg1: 7,
    };
    let mut bytes = [0u8; RECORD_SIZE];
    encode(record, &mut bytes);
    assert_eq!(decode(&bytes), Some(record));
    bytes[3] ^= 1;
    assert_eq!(decode(&bytes), None);

    let mut sector = [0u8; crate::drivers::block::SECTOR_SIZE];
    append_record(
        &mut sector,
        0,
        Record {
            state: STATE_BOOTING,
            kind: 0,
            sequence: 1,
            code: 0,
            arg0: 0,
            arg1: 0,
        },
    );
    append_record(
        &mut sector,
        RECORDS - 1,
        Record {
            state: STATE_PANIC,
            kind: 3,
            sequence: 2,
            code: 0x2014,
            arg0: 0x1000,
            arg1: 5,
        },
    );
    let expected = Record {
        state: STATE_PANIC,
        kind: 3,
        sequence: 2,
        code: 0x2014,
        arg0: 0x1000,
        arg1: 5,
    };
    assert_eq!(latest_record(&sector), Some((RECORDS - 1, expected)));
}

pub fn init() -> bool {
    let mut sector = [0u8; crate::drivers::block::SECTOR_SIZE];
    let (read_ok, source) = read_persistent(&mut sector);
    if !read_ok {
        crate::bootlog::warn("persistent warm-reboot backing unavailable");
        return false;
    }
    let latest = latest_record(&sector);
    let next_slot = if let Some((slot, record)) = latest {
        match record.state {
            STATE_PANIC => crate::bootlog::warn_fmt(format_args!(
                "previous boot panic sequence={} kind={} code=0x{:x} arg0=0x{:x} arg1=0x{:x}",
                record.sequence,
                kind_name(record.kind),
                record.code,
                record.arg0,
                record.arg1,
            )),
            STATE_BOOTING => crate::bootlog::warn_fmt(format_args!(
                "previous boot interrupted sequence={}",
                record.sequence
            )),
            STATE_READY => crate::bootlog::info_fmt(format_args!(
                "previous boot completed sequence={}",
                record.sequence
            )),
            STATE_CHECKPOINT => crate::bootlog::info_fmt(format_args!(
                "previous boot reached pre-architecture checkpoint sequence={}",
                record.sequence
            )),
            _ => {}
        }
        let next_slot = (slot + 1) % RECORDS;
        append_record(
            &mut sector,
            next_slot,
            Record {
                state: STATE_BOOTING,
                kind: 0,
                sequence: record.sequence.saturating_add(1),
                code: 0,
                arg0: 0,
                arg1: 0,
            },
        );
        next_slot
    } else {
        append_record(
            &mut sector,
            0,
            Record {
                state: STATE_BOOTING,
                kind: 0,
                sequence: 1,
                code: 0,
                arg0: 0,
                arg1: 0,
            },
        );
        0
    };
    let current = decode(&sector[next_slot * RECORD_SIZE..(next_slot + 1) * RECORD_SIZE])
        .expect("encoded warm-reboot record must decode");
    if !write_persistent(&sector) {
        crate::bootlog::warn("persistent warm-reboot record write failed");
        return false;
    }
    crate::bootlog::info_fmt(format_args!(
        "persistent warm-reboot log backing ready source={} records={} sequence={}",
        source, RECORDS, current.sequence,
    ));
    true
}

pub fn mark_ready() -> bool {
    append_current(Record {
        state: STATE_READY,
        kind: 0,
        sequence: 0,
        code: 0,
        arg0: 0,
        arg1: 0,
    })
}

pub fn mark_checkpoint() -> bool {
    append_current(Record {
        state: STATE_CHECKPOINT,
        kind: 0,
        sequence: 0,
        code: 0,
        arg0: 0,
        arg1: 0,
    })
}

pub fn fatal(error: KernelError) -> ! {
    let first_panic = unsafe {
        if PANIC_ACTIVE {
            false
        } else {
            PANIC_ACTIVE = true;
            true
        }
    };
    if first_panic
        && !append_current(Record {
            state: STATE_PANIC,
            kind: kind_id(error),
            sequence: 0,
            code: error.code,
            arg0: error.arg0,
            arg1: error.arg1,
        })
    {
        crate::bootlog::warn(
            "persistent panic context write failed; serial report remains authoritative",
        );
    }
    crate::error::report(error);
    crate::arch::halt()
}

fn append_current(mut record: Record) -> bool {
    let mut sector = [0u8; crate::drivers::block::SECTOR_SIZE];
    if !read_persistent(&mut sector).0 {
        return false;
    }
    let (sequence, slot) = latest_record(&sector)
        .map(|(slot, current)| (current.sequence.saturating_add(1), (slot + 1) % RECORDS))
        .unwrap_or((1, 0));
    record.sequence = if record.sequence == 0 {
        sequence
    } else {
        record.sequence
    };
    append_record(&mut sector, slot, record);
    if !write_persistent(&sector) {
        return false;
    }
    crate::bootlog::info_fmt(format_args!(
        "persistent warm-reboot record committed state={} slot={} sequence={}",
        record.state, slot, record.sequence,
    ));
    true
}

fn read_persistent(output: &mut [u8; crate::drivers::block::SECTOR_SIZE]) -> (bool, &'static str) {
    let mut best = [0u8; crate::drivers::block::SECTOR_SIZE];
    let mut best_sequence = None;
    let mut source = "new";
    let mut candidate = [0u8; crate::drivers::block::SECTOR_SIZE];
    if cmos_read(&mut candidate) {
        if let Some(sequence) = latest_sequence(&candidate) {
            best = candidate;
            best_sequence = Some(sequence);
            source = "cmos";
        }
    }
    if efi_read(&mut candidate) {
        if let Some(sequence) = latest_sequence(&candidate) {
            if best_sequence.is_none_or(|current| sequence > current) {
                best = candidate;
                best_sequence = Some(sequence);
                source = "uefi-nvram";
            }
        }
    }
    if best_sequence.is_some() {
        output.copy_from_slice(&best);
        (true, source)
    } else {
        (true, source)
    }
}

fn write_persistent(input: &[u8; crate::drivers::block::SECTOR_SIZE]) -> bool {
    let efi = if efi_write(input) {
        let mut verify = [0u8; crate::drivers::block::SECTOR_SIZE];
        if efi_read(&mut verify) && verify == *input {
            true
        } else {
            crate::bootlog::warn("persistent EFI warm-log write read-back mismatch");
            false
        }
    } else {
        false
    };
    let cmos = cmos_write(input);
    efi || cmos
}

fn latest_record(sector: &[u8; crate::drivers::block::SECTOR_SIZE]) -> Option<(usize, Record)> {
    (0..RECORDS)
        .filter_map(|slot| {
            let offset = slot * RECORD_SIZE;
            decode(&sector[offset..offset + RECORD_SIZE]).map(|record| (slot, record))
        })
        .max_by_key(|(_, record)| record.sequence)
}

type EfiGetVariable =
    unsafe extern "efiapi" fn(*const u16, *const u8, *mut u32, *mut usize, *mut u8) -> usize;
type EfiSetVariable =
    unsafe extern "efiapi" fn(*const u16, *const u8, u32, usize, *const u8) -> usize;

const EFI_VARIABLE_NON_VOLATILE: u32 = 0x0000_0001;
const EFI_VARIABLE_BOOTSERVICE_ACCESS: u32 = 0x0000_0002;
const EFI_VARIABLE_RUNTIME_ACCESS: u32 = 0x0000_0004;
const EFI_WARM_LOG_GUID: [u8; 16] = [
    0x6e, 0x6f, 0x72, 0x78, 0x2d, 0x77, 0x61, 0x72, 0x6d, 0x2d, 0x6c, 0x6f, 0x67, 0x00, 0x01, 0x00,
];
const EFI_WARM_LOG_NAMES: [[u16; 10]; RECORDS] = [
    [
        b'N' as u16,
        b'o' as u16,
        b'r' as u16,
        b'x' as u16,
        b'W' as u16,
        b'a' as u16,
        b'r' as u16,
        b'm' as u16,
        b'0' as u16,
        0,
    ],
    [
        b'N' as u16,
        b'o' as u16,
        b'r' as u16,
        b'x' as u16,
        b'W' as u16,
        b'a' as u16,
        b'r' as u16,
        b'm' as u16,
        b'1' as u16,
        0,
    ],
    [
        b'N' as u16,
        b'o' as u16,
        b'r' as u16,
        b'x' as u16,
        b'W' as u16,
        b'a' as u16,
        b'r' as u16,
        b'm' as u16,
        b'2' as u16,
        0,
    ],
    [
        b'N' as u16,
        b'o' as u16,
        b'r' as u16,
        b'x' as u16,
        b'W' as u16,
        b'a' as u16,
        b'r' as u16,
        b'm' as u16,
        b'3' as u16,
        0,
    ],
];

fn efi_read(output: &mut [u8; crate::drivers::block::SECTOR_SIZE]) -> bool {
    let system_table = crate::boot::efi_system_table();
    if system_table == 0 {
        return false;
    }
    crate::arch::without_interrupts(|| unsafe {
        let runtime = match read_u64(system_table as usize + 88) {
            Some(runtime) => runtime,
            None => return false,
        };
        let function = match read_u64(runtime as usize + 72) {
            Some(function) if function != 0 => function,
            _ => return false,
        };
        let get_variable: EfiGetVariable = core::mem::transmute(function);
        let attributes = core::ptr::addr_of_mut!(EFI_ATTRIBUTES);
        let size = core::ptr::addr_of_mut!(EFI_SIZE);
        let data = core::ptr::addr_of_mut!(EFI_DATA).cast::<u8>();
        if !crate::arch::prepare_firmware_runtime() {
            return false;
        }
        output.fill(0);
        for (slot, name) in EFI_WARM_LOG_NAMES.iter().enumerate() {
            (*attributes) = 0;
            (*size) = RECORD_SIZE;
            let status = get_variable(
                name.as_ptr(),
                EFI_WARM_LOG_GUID.as_ptr(),
                attributes,
                size,
                data,
            );
            if status == 0 && (*size) == RECORD_SIZE {
                let offset = slot * RECORD_SIZE;
                core::ptr::copy_nonoverlapping(data, output.as_mut_ptr().add(offset), RECORD_SIZE);
            }
        }
        crate::arch::finish_firmware_runtime();
        true
    })
}

fn efi_write(input: &[u8; crate::drivers::block::SECTOR_SIZE]) -> bool {
    let system_table = crate::boot::efi_system_table();
    if system_table == 0 {
        return false;
    }
    crate::arch::without_interrupts(|| unsafe {
        let runtime = match read_u64(system_table as usize + 88) {
            Some(runtime) => runtime,
            None => return false,
        };
        let function = match read_u64(runtime as usize + 88) {
            Some(function) if function != 0 => function,
            _ => return false,
        };
        let set_variable: EfiSetVariable = core::mem::transmute(function);
        let data = core::ptr::addr_of_mut!(EFI_DATA).cast::<u8>();
        if !crate::arch::prepare_firmware_runtime() {
            return false;
        }
        let mut success = true;
        for (slot, name) in EFI_WARM_LOG_NAMES.iter().enumerate() {
            let offset = slot * RECORD_SIZE;
            core::ptr::copy_nonoverlapping(input.as_ptr().add(offset), data, RECORD_SIZE);
            let status = set_variable(
                name.as_ptr(),
                EFI_WARM_LOG_GUID.as_ptr(),
                EFI_VARIABLE_NON_VOLATILE
                    | EFI_VARIABLE_BOOTSERVICE_ACCESS
                    | EFI_VARIABLE_RUNTIME_ACCESS,
                RECORD_SIZE,
                data,
            );
            if status != 0 {
                success = false;
                break;
            }
        }
        crate::arch::finish_firmware_runtime();
        success
    })
}

unsafe fn read_u64(address: usize) -> Option<u64> {
    address
        .checked_add(8)
        .map(|_| core::ptr::read_unaligned(address as *const u64))
}

fn latest_sequence(sector: &[u8; crate::drivers::block::SECTOR_SIZE]) -> Option<u64> {
    latest_record(sector).map(|(_, record)| record.sequence)
}

#[cfg(target_arch = "x86_64")]
fn cmos_read(output: &mut [u8; crate::drivers::block::SECTOR_SIZE]) -> bool {
    let ports = crate::io::PioRegion::new(0x70, 2).unwrap();
    let mut record = [0u8; RECORD_SIZE];
    for (offset, byte) in record.iter_mut().enumerate() {
        if !ports.write_u8(0, 0x80 | (0x40 + offset as u8)) {
            return false;
        }
        *byte = match ports.read_u8(1) {
            Some(byte) => byte,
            None => return false,
        };
    }
    if decode(&record).is_none() {
        return false;
    }
    output.fill(0);
    output[..RECORD_SIZE].copy_from_slice(&record);
    true
}

#[cfg(not(target_arch = "x86_64"))]
fn cmos_read(_output: &mut [u8; crate::drivers::block::SECTOR_SIZE]) -> bool {
    false
}

#[cfg(target_arch = "x86_64")]
fn cmos_write(input: &[u8; crate::drivers::block::SECTOR_SIZE]) -> bool {
    let Some((slot, _)) = (0..RECORDS)
        .filter_map(|slot| {
            let offset = slot * RECORD_SIZE;
            decode(&input[offset..offset + RECORD_SIZE]).map(|record| (slot, record))
        })
        .max_by_key(|(_, record)| record.sequence)
    else {
        return false;
    };
    let offset = slot * RECORD_SIZE;
    let ports = crate::io::PioRegion::new(0x70, 2).unwrap();
    for (index, byte) in input[offset..offset + RECORD_SIZE].iter().enumerate() {
        if !ports.write_u8(0, 0x80 | (0x40 + index as u8)) || !ports.write_u8(1, *byte) {
            return false;
        }
    }
    true
}

#[cfg(not(target_arch = "x86_64"))]
fn cmos_write(_input: &[u8; crate::drivers::block::SECTOR_SIZE]) -> bool {
    false
}

fn append_record(
    sector: &mut [u8; crate::drivers::block::SECTOR_SIZE],
    slot: usize,
    record: Record,
) {
    let offset = slot * RECORD_SIZE;
    encode(record, &mut sector[offset..offset + RECORD_SIZE]);
}

fn encode(record: Record, output: &mut [u8]) {
    output.fill(0);
    put_u32(output, 0, MAGIC);
    put_u16(output, 4, VERSION);
    output[6] = record.state;
    output[7] = record.kind;
    put_u64(output, 8, record.sequence);
    put_u64(output, 16, record.code);
    put_u64(output, 24, record.arg0);
    put_u64(output, 32, record.arg1);
    put_u32(output, 40, checksum(output));
}

fn decode(input: &[u8]) -> Option<Record> {
    if input.len() < RECORD_SIZE
        || u32::from_le_bytes(input[0..4].try_into().ok()?) != MAGIC
        || u16::from_le_bytes(input[4..6].try_into().ok()?) != VERSION
    {
        return None;
    }
    let stored = u32::from_le_bytes(input[40..44].try_into().ok()?);
    let mut checked = [0u8; RECORD_SIZE];
    checked.copy_from_slice(&input[..RECORD_SIZE]);
    checked[40..44].fill(0);
    (stored == checksum(&checked)).then_some(Record {
        state: input[6],
        kind: input[7],
        sequence: u64::from_le_bytes(input[8..16].try_into().ok()?),
        code: u64::from_le_bytes(input[16..24].try_into().ok()?),
        arg0: u64::from_le_bytes(input[24..32].try_into().ok()?),
        arg1: u64::from_le_bytes(input[32..40].try_into().ok()?),
    })
}

fn checksum(bytes: &[u8]) -> u32 {
    let mut value = 0x9e37_79b9u32;
    for (index, byte) in bytes.iter().enumerate() {
        if (40..44).contains(&index) {
            continue;
        }
        value = value.rotate_left(5) ^ u32::from(*byte);
        value = value.wrapping_mul(0x0100_0193);
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

fn kind_id(error: KernelError) -> u8 {
    match error.kind {
        crate::error::ErrorKind::Panic => 1,
        crate::error::ErrorKind::CpuException => 2,
        crate::error::ErrorKind::PageFault => 3,
    }
}

fn kind_name(kind: u8) -> &'static str {
    match kind {
        1 => "panic",
        2 => "cpu-exception",
        3 => "page-fault",
        _ => "unknown",
    }
}
