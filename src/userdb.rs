//! Kernel-owned account-database publication boundary.
//!
//! User processes never provide a database pathname to this module.  The
//! fixed names, whole-transaction lock, owner tuple, generation, nonce, and
//! MAC are all checked in the kernel before bytes are returned or promoted.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::crypto::HmacSha256;
use crate::process::{Capability, Credentials};
use crate::vfs::{self, OpenOptions};

pub const COMMITTED_PATH: &str = "/cfg/userdb/users.db";
pub const TEMP_PATH: &str = "/cfg/userdb/users.db.tmp";
pub const PARENT_PATH: &str = "/cfg/userdb";
pub const MAX_ENVELOPE_BYTES: usize = 4096;
pub const HEADER_BYTES: usize = 80;
pub const MAX_PAYLOAD_BYTES: usize = MAX_ENVELOPE_BYTES - HEADER_BYTES;
pub const LOAD_CURRENT: u32 = 0;
pub const LOAD_RECOVERED_TEMP: u32 = 1;

const MAGIC: &[u8; 8] = b"NORXDB\0\x01";
const VERSION: u16 = 1;
const KEY: [u8; 33] = [
    0x4e, 0x6f, 0x72, 0x78, 0x20, 0x55, 0x73, 0x65, 0x72, 0x44, 0x42, 0x20, 0x73, 0x74, 0x6f, 0x72,
    0x61, 0x67, 0x65, 0x20, 0x6b, 0x65, 0x79, 0x20, 0x76, 0x31, 0x20, 0x70, 0x72, 0x69, 0x76, 0x61,
    0x74,
];
const DOMAIN: &[u8] = b"Norx userdb storage envelope v1";
const NONCE_DOMAIN: &[u8] = b"Norx userdb storage nonce v1";

const OWNER_UID_OFFSET: usize = 12;
const OWNER_GID_OFFSET: usize = 16;
const GENERATION_OFFSET: usize = 20;
const NONCE_OFFSET: usize = 28;
const PAYLOAD_LENGTH_OFFSET: usize = 44;
const MAC_OFFSET: usize = 48;

static BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    PermissionDenied,
    Missing,
    Corrupt,
    Busy,
    TooLarge,
    NoSpace,
    ReadOnly,
    Backend,
    Committed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadMetadata {
    pub state: u32,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub generation: u64,
    pub payload_length: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EnvelopeMetadata {
    owner_uid: u32,
    owner_gid: u32,
    generation: u64,
    payload_length: usize,
}

struct BusyGuard;

impl BusyGuard {
    fn acquire() -> Result<Self, Error> {
        BUSY.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| Self)
            .map_err(|_| Error::Busy)
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

pub fn load(output: &mut [u8], actor: Credentials) -> Result<LoadMetadata, Error> {
    authorize(actor)?;
    if output.len() < MAX_PAYLOAD_BYTES {
        return Err(Error::TooLarge);
    }
    let _busy = BusyGuard::acquire()?;
    let mut committed_bytes = [0u8; MAX_ENVELOPE_BYTES];
    let committed = match read_file(COMMITTED_PATH, &mut committed_bytes)? {
        Some(length) => match parse_envelope(&committed_bytes[..length], actor) {
            Ok(metadata) => {
                discard_stale_temp();
                copy_payload(&committed_bytes[..length], metadata, output);
                return Ok(LoadMetadata {
                    state: LOAD_CURRENT,
                    owner_uid: metadata.owner_uid,
                    owner_gid: metadata.owner_gid,
                    generation: metadata.generation,
                    payload_length: metadata.payload_length,
                });
            }
            Err(Error::PermissionDenied) => return Err(Error::PermissionDenied),
            Err(_) => Some(length),
        },
        None => None,
    };

    let mut temporary_bytes = [0u8; MAX_ENVELOPE_BYTES];
    let Some(temporary_length) = read_file(TEMP_PATH, &mut temporary_bytes)? else {
        return Err(Error::Corrupt);
    };
    let temporary = parse_envelope(&temporary_bytes[..temporary_length], actor)?;
    if let Some(committed_length) = committed {
        let Some((owner_uid, owner_gid, generation)) =
            envelope_header(&committed_bytes[..committed_length])
        else {
            return Err(Error::Corrupt);
        };
        if owner_uid != actor.effective_uid || owner_gid != actor.effective_gid {
            return Err(Error::PermissionDenied);
        }
        if temporary.generation <= generation {
            return Err(Error::Corrupt);
        }
    }
    publish_temp()?;
    copy_payload(&temporary_bytes[..temporary_length], temporary, output);
    Ok(LoadMetadata {
        state: LOAD_RECOVERED_TEMP,
        owner_uid: temporary.owner_uid,
        owner_gid: temporary.owner_gid,
        generation: temporary.generation,
        payload_length: temporary.payload_length,
    })
}

pub fn commit(payload: &[u8], actor: Credentials) -> Result<u64, Error> {
    authorize(actor)?;
    if payload.is_empty() || payload.len() > MAX_PAYLOAD_BYTES {
        return Err(Error::TooLarge);
    }
    let _busy = BusyGuard::acquire()?;
    let mut committed_bytes = [0u8; MAX_ENVELOPE_BYTES];
    let generation = match read_file(COMMITTED_PATH, &mut committed_bytes)? {
        Some(length) => {
            let metadata = parse_envelope(&committed_bytes[..length], actor)?;
            metadata.generation.checked_add(1).ok_or(Error::NoSpace)?
        }
        None => 1,
    };
    if read_file_size(TEMP_PATH)?.is_some() {
        return Err(Error::Busy);
    }

    let mut envelope = [0u8; MAX_ENVELOPE_BYTES];
    let envelope_length = make_envelope(payload, actor, generation, &mut envelope);
    let handle = match vfs::open(
        TEMP_PATH,
        OpenOptions {
            read: false,
            write: true,
            create: true,
            truncate: false,
            append: false,
            exclusive: true,
            mode: 0o600,
        },
    ) {
        Ok(handle) => handle,
        Err(vfs::Error::AlreadyExists) => return Err(Error::Busy),
        Err(error) => return Err(map_vfs_error(error)),
    };
    let write_result = (|| {
        let written =
            vfs::write_handle(handle, &envelope[..envelope_length]).map_err(map_vfs_error)?;
        if written != envelope_length {
            return Err(Error::Backend);
        }
        vfs::sync_handle_for_storage(handle).map_err(map_vfs_error)
    })();
    let close_result = vfs::close(handle).map_err(map_vfs_error);
    if let Err(error) = write_result {
        let _ = vfs::unlink(TEMP_PATH);
        let _ = close_result;
        return Err(error);
    }
    close_result?;
    if let Err(error) = vfs::rename(TEMP_PATH, COMMITTED_PATH).map_err(map_vfs_error) {
        let _ = vfs::unlink(TEMP_PATH);
        return Err(error);
    }
    if vfs::sync_parent_for_storage(PARENT_PATH).is_err() {
        return Err(Error::Committed);
    }
    Ok(generation)
}

fn authorize(actor: Credentials) -> Result<(), Error> {
    actor
        .authorize(Capability::AccountAdmin)
        .map_err(|_| Error::PermissionDenied)
}

fn read_file(path: &str, output: &mut [u8; MAX_ENVELOPE_BYTES]) -> Result<Option<usize>, Error> {
    let Some(size) = read_file_size(path)? else {
        return Ok(None);
    };
    if size > MAX_ENVELOPE_BYTES {
        return Err(Error::TooLarge);
    }
    let handle = vfs::open(path, OpenOptions::read()).map_err(map_vfs_error)?;
    let result = vfs::read_handle(handle, &mut output[..size]).map_err(map_vfs_error);
    let close = vfs::close(handle).map_err(map_vfs_error);
    let length = result?;
    close?;
    if length != size {
        return Err(Error::Backend);
    }
    Ok(Some(length))
}

fn read_file_size(path: &str) -> Result<Option<usize>, Error> {
    match vfs::stat(path) {
        Ok(stat) if stat.kind == vfs::NodeType::Regular => Ok(Some(stat.size)),
        Ok(_) => Err(Error::Corrupt),
        Err(vfs::Error::NotFound) => Ok(None),
        Err(error) => Err(map_vfs_error(error)),
    }
}

fn parse_envelope(bytes: &[u8], actor: Credentials) -> Result<EnvelopeMetadata, Error> {
    if bytes.len() < HEADER_BYTES
        || bytes.get(..8) != Some(MAGIC)
        || read_u16(bytes, 8) != Some(VERSION)
        || read_u16(bytes, 10) != Some(0)
    {
        return Err(Error::Corrupt);
    }
    let owner_uid = read_u32(bytes, OWNER_UID_OFFSET).ok_or(Error::Corrupt)?;
    let owner_gid = read_u32(bytes, OWNER_GID_OFFSET).ok_or(Error::Corrupt)?;
    if owner_uid != actor.effective_uid || owner_gid != actor.effective_gid {
        return Err(Error::PermissionDenied);
    }
    let generation = read_u64(bytes, GENERATION_OFFSET).ok_or(Error::Corrupt)?;
    let payload_length =
        usize::try_from(read_u32(bytes, PAYLOAD_LENGTH_OFFSET).ok_or(Error::Corrupt)?)
            .map_err(|_| Error::Corrupt)?;
    let total = HEADER_BYTES
        .checked_add(payload_length)
        .ok_or(Error::Corrupt)?;
    if generation == 0 || payload_length > MAX_PAYLOAD_BYTES || total != bytes.len() {
        return Err(Error::Corrupt);
    }
    let payload = &bytes[HEADER_BYTES..];
    let mut expected_nonce = [0u8; 32];
    nonce(actor, generation, payload, &mut expected_nonce);
    if !constant_time_equal(
        &bytes[NONCE_OFFSET..PAYLOAD_LENGTH_OFFSET],
        &expected_nonce[..16],
    ) {
        return Err(Error::Corrupt);
    }
    let mut mac = HmacSha256::new(&KEY);
    mac.update(DOMAIN);
    mac.update(&bytes[..MAC_OFFSET]);
    mac.update(payload);
    let expected_mac = mac.finalize();
    if !constant_time_equal(&bytes[MAC_OFFSET..HEADER_BYTES], &expected_mac) {
        return Err(Error::Corrupt);
    }
    Ok(EnvelopeMetadata {
        owner_uid,
        owner_gid,
        generation,
        payload_length,
    })
}

fn envelope_header(bytes: &[u8]) -> Option<(u32, u32, u64)> {
    if bytes.len() < HEADER_BYTES
        || bytes.get(..8) != Some(MAGIC)
        || read_u16(bytes, 8) != Some(VERSION)
    {
        return None;
    }
    Some((
        read_u32(bytes, OWNER_UID_OFFSET)?,
        read_u32(bytes, OWNER_GID_OFFSET)?,
        read_u64(bytes, GENERATION_OFFSET)?,
    ))
}

fn make_envelope(
    payload: &[u8],
    actor: Credentials,
    generation: u64,
    output: &mut [u8; MAX_ENVELOPE_BYTES],
) -> usize {
    output[..8].copy_from_slice(MAGIC);
    output[8..10].copy_from_slice(&VERSION.to_le_bytes());
    output[10..12].fill(0);
    output[OWNER_UID_OFFSET..OWNER_UID_OFFSET + 4]
        .copy_from_slice(&actor.effective_uid.to_le_bytes());
    output[OWNER_GID_OFFSET..OWNER_GID_OFFSET + 4]
        .copy_from_slice(&actor.effective_gid.to_le_bytes());
    output[GENERATION_OFFSET..GENERATION_OFFSET + 8].copy_from_slice(&generation.to_le_bytes());
    let mut nonce_bytes = [0u8; 32];
    nonce(actor, generation, payload, &mut nonce_bytes);
    output[NONCE_OFFSET..PAYLOAD_LENGTH_OFFSET].copy_from_slice(&nonce_bytes[..16]);
    output[PAYLOAD_LENGTH_OFFSET..MAC_OFFSET]
        .copy_from_slice(&(payload.len() as u32).to_le_bytes());
    output[MAC_OFFSET..HEADER_BYTES].fill(0);
    output[HEADER_BYTES..HEADER_BYTES + payload.len()].copy_from_slice(payload);
    let mut mac = HmacSha256::new(&KEY);
    mac.update(DOMAIN);
    mac.update(&output[..MAC_OFFSET]);
    mac.update(payload);
    output[MAC_OFFSET..HEADER_BYTES].copy_from_slice(&mac.finalize());
    HEADER_BYTES + payload.len()
}

fn nonce(actor: Credentials, generation: u64, payload: &[u8], output: &mut [u8; 32]) {
    let mut mac = HmacSha256::new(&KEY);
    mac.update(NONCE_DOMAIN);
    mac.update(&actor.effective_uid.to_le_bytes());
    mac.update(&actor.effective_gid.to_le_bytes());
    mac.update(&generation.to_le_bytes());
    mac.update(payload);
    *output = mac.finalize();
}

fn copy_payload(bytes: &[u8], metadata: EnvelopeMetadata, output: &mut [u8]) {
    output[..metadata.payload_length]
        .copy_from_slice(&bytes[HEADER_BYTES..HEADER_BYTES + metadata.payload_length]);
}

fn publish_temp() -> Result<(), Error> {
    vfs::rename(TEMP_PATH, COMMITTED_PATH).map_err(map_vfs_error)?;
    vfs::sync_parent_for_storage(PARENT_PATH).map_err(|_| Error::Committed)
}

fn discard_stale_temp() {
    let _ = vfs::unlink(TEMP_PATH);
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in left.iter().zip(right.iter()) {
        difference |= left ^ right;
    }
    difference == 0
}

fn map_vfs_error(error: vfs::Error) -> Error {
    match error {
        vfs::Error::NotFound => Error::Missing,
        vfs::Error::AlreadyExists | vfs::Error::Busy | vfs::Error::MountPointBusy => Error::Busy,
        vfs::Error::NoSpace => Error::NoSpace,
        vfs::Error::ReadOnly => Error::ReadOnly,
        vfs::Error::PermissionDenied => Error::PermissionDenied,
        vfs::Error::InvalidPath | vfs::Error::NotDirectory | vfs::Error::IsDirectory => {
            Error::Corrupt
        }
        _ => Error::Backend,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_rejects_payload_and_owner_tampering() {
        let actor = Credentials {
            effective_uid: 0,
            effective_gid: 0,
            ..Credentials::BOOTSTRAP
        };
        let mut bytes = [0u8; MAX_ENVELOPE_BYTES];
        let length = make_envelope(b"database", actor, 1, &mut bytes);
        assert_eq!(
            parse_envelope(&bytes[..length], actor).unwrap().generation,
            1
        );
        bytes[HEADER_BYTES] ^= 1;
        assert_eq!(parse_envelope(&bytes[..length], actor), Err(Error::Corrupt));
        let other = Credentials {
            effective_uid: 1000,
            effective_gid: 1000,
            ..actor
        };
        assert_eq!(
            parse_envelope(&bytes[..length], other),
            Err(Error::PermissionDenied)
        );
    }

    #[test]
    fn envelope_generations_change_the_nonce() {
        let actor = Credentials::BOOTSTRAP;
        let mut first = [0u8; MAX_ENVELOPE_BYTES];
        let mut second = [0u8; MAX_ENVELOPE_BYTES];
        make_envelope(b"database", actor, 1, &mut first);
        make_envelope(b"database", actor, 2, &mut second);
        assert_ne!(
            &first[NONCE_OFFSET..PAYLOAD_LENGTH_OFFSET],
            &second[NONCE_OFFSET..PAYLOAD_LENGTH_OFFSET]
        );
    }
}
