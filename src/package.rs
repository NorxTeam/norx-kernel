//! Bounded package trust and generation recovery.
//!
//! The kernel consumes a small canonical `.gpk` metadata record.  The record
//! is signed by a key compiled into the image; artifact bytes are supplied by
//! the store backend and are accepted only after their signed size and
//! SHA-256 digest match.  Package installation is deliberately split into
//! prepare and commit so an incomplete write is never a boot candidate.

const MAGIC: &[u8; 4] = b"GPKM";
const FORMAT_VERSION: u16 = 1;
const HEADER_SIZE: usize = 96;
const RECORD_SIZE: usize = 120;
const SIGNATURE_SIZE: usize = 64;
const MAX_ARTIFACTS: usize = 8;
const MAX_METADATA_SIZE: usize = 16 * 1024;
const MAX_PATH: usize = 64;
const MAX_NAME: usize = 32;
const MAX_TARGET: usize = 16;
const MAX_GENERATIONS: usize = 4;
const MAX_STORE_ARTIFACTS: usize = 8;
const MAX_ARTIFACT_BYTES: usize = 8 * 1024;

const STATE_EMPTY: u8 = 0;
const STATE_PREPARED: u8 = 1;
const STATE_COMMITTED: u8 = 2;
const STATE_ROLLED_BACK: u8 = 3;

/// The key identifier is part of the signed record and prevents a future key
/// table from accidentally accepting a key for a different trust domain.
pub const ROOT_KEY_ID: [u8; 16] = *b"norx-root-v1\0\0\0\0";

/// RFC 8032 test key.  A production image must replace this compile-time root
/// through the image-signing configuration; it is intentionally public here.
pub const ROOT_PUBLIC_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

/// A small signed artifact used by the target boot self-check.  It is not a
/// service image; it proves the static bootstrap trust path before userspace
/// package installation is enabled by the storage and process gates.
pub const BOOTSTRAP_ARTIFACT: &[u8] = b"norx bootstrap artifact v1\n";

// This is the Ed25519 signature for BOOTSTRAP_METADATA_WITHOUT_SIGNATURE,
// generated from the RFC 8032 test seed during development.  No private key
// is present in the kernel image.
const BOOTSTRAP_SIGNATURE: [u8; SIGNATURE_SIZE] = [
    0xba, 0x66, 0xf, 0x92, 0xd2, 0xe9, 0x48, 0x55, 0x15, 0xef, 0x27, 0x19, 0xe8, 0x45, 0x9, 0x6b,
    0x15, 0x74, 0xab, 0x0, 0xaf, 0x7e, 0xa8, 0xb1, 0xe1, 0x92, 0xca, 0x79, 0x3e, 0xb5, 0xc, 0xa8,
    0xf6, 0xd7, 0x38, 0x35, 0x41, 0x8c, 0x96, 0x26, 0xd9, 0x56, 0x80, 0x9a, 0x3f, 0x33, 0x6e, 0xd3,
    0xa6, 0x6e, 0x4d, 0x83, 0xea, 0xa5, 0x0, 0x15, 0x15, 0x91, 0xaf, 0xd1, 0x6e, 0xaa, 0x2e, 0x5,
];

const BOOTSTRAP_METADATA_WITHOUT_SIGNATURE: [u8; HEADER_SIZE + RECORD_SIZE] =
    make_bootstrap_metadata();

const fn make_bootstrap_metadata() -> [u8; HEADER_SIZE + RECORD_SIZE] {
    let mut bytes = [0u8; HEADER_SIZE + RECORD_SIZE];
    bytes[0] = MAGIC[0];
    bytes[1] = MAGIC[1];
    bytes[2] = MAGIC[2];
    bytes[3] = MAGIC[3];
    put_u16(&mut bytes, 4, FORMAT_VERSION);
    put_u16(&mut bytes, 6, HEADER_SIZE as u16);
    put_u64(&mut bytes, 8, 1);
    put_u64(&mut bytes, 16, 0);
    bytes[24] = 1;
    bytes[25] = 0;
    put_u32(&mut bytes, 28, 1);
    copy_fixed(&mut bytes, 32, b"quickinit-bootstrap");
    copy_fixed(&mut bytes, 64, b"bootstrap");
    copy_fixed(&mut bytes, 80, &ROOT_KEY_ID);
    copy_fixed(&mut bytes, HEADER_SIZE, b"bin/bootstrap");
    put_u64(
        &mut bytes,
        HEADER_SIZE + 64,
        BOOTSTRAP_ARTIFACT.len() as u64,
    );
    put_u32(&mut bytes, HEADER_SIZE + 72, 0o755);
    let digest = [
        0x0, 0x89, 0x1b, 0x33, 0x3d, 0x88, 0x3b, 0xc2, 0xbc, 0xdd, 0xa9, 0x96, 0xff, 0x2a, 0xcd,
        0xb5, 0x3a, 0xab, 0xd7, 0x71, 0xf2, 0x2b, 0xeb, 0x19, 0x6c, 0xa3, 0x31, 0xee, 0xb2, 0xe7,
        0xd1, 0xbf,
    ];
    copy_fixed(&mut bytes, HEADER_SIZE + 80, &digest);
    bytes
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    TooShort,
    TooLarge,
    BadMagic,
    BadVersion,
    // Reserved diagnostic for the staged external package importer.
    #[allow(dead_code)]
    BadHeader,
    BadLength,
    BadCount,
    BadFlags,
    BadGeneration,
    BadKey,
    BadName,
    // Target selection is staged until package installation becomes available.
    #[allow(dead_code)]
    BadTarget,
    BadPath,
    DuplicatePath,
    BadRecord,
    BadSignature,
    BadArtifactSize,
    BadArtifactDigest,
    NoSpace,
    InvalidToken,
    InvalidState,
    NoGeneration,
    GenerationConflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactInfo {
    pub path: [u8; MAX_PATH],
    pub path_length: u8,
    pub size: u64,
    pub mode: u32,
    pub digest: [u8; 32],
}

#[derive(Clone, Copy)]
struct ArtifactSlot {
    used: bool,
    generation: u64,
    info: ArtifactInfo,
    length: usize,
    bytes: [u8; MAX_ARTIFACT_BYTES],
}

impl ArtifactSlot {
    const EMPTY: Self = Self {
        used: false,
        generation: 0,
        info: ArtifactInfo {
            path: [0; MAX_PATH],
            path_length: 0,
            size: 0,
            mode: 0,
            digest: [0; 32],
        },
        length: 0,
        bytes: [0; MAX_ARTIFACT_BYTES],
    };
}

#[derive(Clone, Copy)]
struct GenerationSlot {
    state: u8,
    generation: u64,
    metadata_length: usize,
    metadata: [u8; MAX_METADATA_SIZE],
    artifact_slots: [u8; MAX_ARTIFACTS],
    artifact_count: u8,
}

impl GenerationSlot {
    const EMPTY: Self = Self {
        state: STATE_EMPTY,
        generation: 0,
        metadata_length: 0,
        metadata: [0; MAX_METADATA_SIZE],
        artifact_slots: [0; MAX_ARTIFACTS],
        artifact_count: 0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationToken {
    slot: u8,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recovery {
    pub generation: u64,
    pub rolled_back: bool,
}

pub struct Store {
    generations: [GenerationSlot; MAX_GENERATIONS],
    artifacts: [ArtifactSlot; MAX_STORE_ARTIFACTS],
    active: Option<usize>,
}

impl Store {
    pub const fn new() -> Self {
        Self {
            generations: [GenerationSlot::EMPTY; MAX_GENERATIONS],
            artifacts: [ArtifactSlot::EMPTY; MAX_STORE_ARTIFACTS],
            active: None,
        }
    }

    pub fn stage(
        &mut self,
        metadata_bytes: &[u8],
        artifact_bytes: &[&[u8]],
    ) -> Result<GenerationToken, Error> {
        let metadata = verify_metadata(metadata_bytes)?;
        if metadata.artifact_count() != artifact_bytes.len() {
            return Err(Error::BadCount);
        }
        let predecessor = self.active_generation();
        if metadata.previous_generation() != predecessor {
            return Err(Error::GenerationConflict);
        }

        let generation_slot = self
            .generations
            .iter()
            .position(|slot| slot.state == STATE_EMPTY)
            .ok_or(Error::NoSpace)?;
        let mut staged = [u8::MAX; MAX_ARTIFACTS];
        let mut staged_count = 0;
        for (index, bytes) in artifact_bytes.iter().enumerate() {
            let info = metadata.artifact(index)?;
            verify_artifact_info(&info, bytes)?;
            if bytes.len() > MAX_ARTIFACT_BYTES {
                return Err(Error::TooLarge);
            }
            let slot = match self.artifacts.iter().position(|slot| !slot.used) {
                Some(slot) => slot,
                None => {
                    self.release_artifacts(&staged, staged_count);
                    return Err(Error::NoSpace);
                }
            };
            self.artifacts[slot].used = true;
            self.artifacts[slot].generation = metadata.generation();
            self.artifacts[slot].info = info;
            self.artifacts[slot].length = bytes.len();
            self.artifacts[slot].bytes[..bytes.len()].copy_from_slice(bytes);
            staged[staged_count] = slot as u8;
            staged_count += 1;
        }

        let generation = &mut self.generations[generation_slot];
        generation.state = STATE_PREPARED;
        generation.generation = metadata.generation();
        generation.metadata_length = metadata_bytes.len();
        generation.metadata[..metadata_bytes.len()].copy_from_slice(metadata_bytes);
        generation.artifact_slots = [0; MAX_ARTIFACTS];
        generation.artifact_slots[..staged_count].copy_from_slice(&staged[..staged_count]);
        generation.artifact_count = staged_count as u8;
        Ok(GenerationToken {
            slot: generation_slot as u8,
            generation: metadata.generation(),
        })
    }

    pub fn commit(&mut self, token: GenerationToken) -> Result<Recovery, Error> {
        let slot = self.token_slot(token)?;
        if self.generations[slot].state != STATE_PREPARED {
            return Err(Error::InvalidState);
        }
        self.generations[slot].state = STATE_COMMITTED;
        self.recover()
    }

    pub fn recover(&mut self) -> Result<Recovery, Error> {
        let previous = self.active_generation();
        let mut best: Option<usize> = None;
        for index in 0..MAX_GENERATIONS {
            let generation = self.generations[index];
            if generation.state != STATE_COMMITTED {
                continue;
            }
            if self.valid_generation(index).is_ok()
                && best.is_none_or(|current| {
                    generation.generation > self.generations[current].generation
                })
            {
                best = Some(index);
            }
        }
        self.active = best;
        let index = best.ok_or(Error::NoGeneration)?;
        Ok(Recovery {
            generation: self.generations[index].generation,
            rolled_back: previous != 0 && self.generations[index].generation < previous,
        })
    }

    pub fn rollback(&mut self) -> Result<Recovery, Error> {
        let current = self.active.ok_or(Error::NoGeneration)?;
        let current_generation = self.generations[current].generation;
        let mut older: Option<usize> = None;
        for index in 0..MAX_GENERATIONS {
            let generation = self.generations[index].generation;
            if index != current
                && self.generations[index].state == STATE_COMMITTED
                && generation < current_generation
                && self.valid_generation(index).is_ok()
                && older.is_none_or(|old| generation > self.generations[old].generation)
            {
                older = Some(index);
            }
        }
        let older = older.ok_or(Error::NoGeneration)?;
        for index in 0..MAX_GENERATIONS {
            if self.generations[index].state == STATE_COMMITTED
                && self.generations[index].generation > self.generations[older].generation
            {
                self.generations[index].state = STATE_ROLLED_BACK;
            }
        }
        self.active = Some(older);
        Ok(Recovery {
            generation: self.generations[older].generation,
            rolled_back: true,
        })
    }

    pub fn active_generation(&self) -> u64 {
        self.active
            .map(|slot| self.generations[slot].generation)
            .unwrap_or(0)
    }

    pub fn active_artifact(&self, path: &[u8]) -> Result<&[u8], Error> {
        let generation = self.active.ok_or(Error::NoGeneration)?;
        let record = self.valid_generation(generation)?;
        for index in 0..record.artifact_count() {
            let info = record.artifact(index)?;
            if info.path_length as usize == path.len() && info.path[..path.len()] == *path {
                let slot = self.generations[generation].artifact_slots[index] as usize;
                return Ok(&self.artifacts[slot].bytes[..self.artifacts[slot].length]);
            }
        }
        Err(Error::BadPath)
    }

    fn token_slot(&self, token: GenerationToken) -> Result<usize, Error> {
        let slot = token.slot as usize;
        if slot >= MAX_GENERATIONS
            || self.generations[slot].generation != token.generation
            || self.generations[slot].state != STATE_PREPARED
        {
            return Err(Error::InvalidToken);
        }
        Ok(slot)
    }

    fn valid_generation(&self, slot: usize) -> Result<Metadata<'_>, Error> {
        let generation = &self.generations[slot];
        if generation.state != STATE_COMMITTED {
            return Err(Error::InvalidState);
        }
        let metadata = verify_metadata(&generation.metadata[..generation.metadata_length])?;
        if metadata.generation() != generation.generation
            || metadata.artifact_count() != generation.artifact_count as usize
        {
            return Err(Error::BadRecord);
        }
        for index in 0..generation.artifact_count as usize {
            let artifact_slot = generation.artifact_slots[index] as usize;
            if artifact_slot >= MAX_STORE_ARTIFACTS {
                return Err(Error::BadRecord);
            }
            let artifact = &self.artifacts[artifact_slot];
            if !artifact.used || artifact.generation != generation.generation {
                return Err(Error::BadRecord);
            }
            let info = metadata.artifact(index)?;
            if artifact.info != info || sha256(&artifact.bytes[..artifact.length]) != info.digest {
                return Err(Error::BadArtifactDigest);
            }
        }
        Ok(metadata)
    }

    fn release_artifacts(&mut self, staged: &[u8; MAX_ARTIFACTS], count: usize) {
        for index in staged[..count].iter().copied() {
            if index != u8::MAX {
                self.artifacts[index as usize] = ArtifactSlot::EMPTY;
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct Metadata<'a> {
    bytes: &'a [u8],
    artifact_count: usize,
}

impl<'a> Metadata<'a> {
    pub fn generation(&self) -> u64 {
        read_u64(self.bytes, 8)
    }

    pub fn previous_generation(&self) -> u64 {
        read_u64(self.bytes, 16)
    }

    pub fn artifact_count(&self) -> usize {
        self.artifact_count
    }

    pub fn package_name(&self) -> &'a [u8] {
        fixed_slice(self.bytes, 32, MAX_NAME)
    }

    pub fn target(&self) -> &'a [u8] {
        fixed_slice(self.bytes, 64, MAX_TARGET)
    }

    pub fn artifact(&self, index: usize) -> Result<ArtifactInfo, Error> {
        if index >= self.artifact_count {
            return Err(Error::BadCount);
        }
        let offset = HEADER_SIZE
            .checked_add(index.checked_mul(RECORD_SIZE).ok_or(Error::BadLength)?)
            .ok_or(Error::BadLength)?;
        let mut path = [0u8; MAX_PATH];
        path.copy_from_slice(&self.bytes[offset..offset + MAX_PATH]);
        Ok(ArtifactInfo {
            path,
            path_length: u8::try_from(path_length(&path).ok_or(Error::BadPath)?)
                .map_err(|_| Error::BadPath)?,
            size: read_u64(self.bytes, offset + 64),
            mode: read_u32(self.bytes, offset + 72),
            digest: self.bytes[offset + 80..offset + 112]
                .try_into()
                .map_err(|_| Error::BadRecord)?,
        })
    }
}

// Staged package-bootstrap importer currently verifies the fixture in self-checks.
#[allow(dead_code)]
pub fn bootstrap_metadata() -> &'static [u8] {
    // The signature is filled by the checked-in fixture below.  Keeping the
    // message and signature separate makes accidental signing of a different
    // wire representation impossible in the self-check.
    &BOOTSTRAP_METADATA_WITHOUT_SIGNATURE
}

pub fn verify_metadata(input: &[u8]) -> Result<Metadata<'_>, Error> {
    if input.len() > MAX_METADATA_SIZE {
        return Err(Error::TooLarge);
    }
    if input.len() < HEADER_SIZE + SIGNATURE_SIZE {
        return Err(Error::TooShort);
    }
    if &input[..4] != MAGIC {
        return Err(Error::BadMagic);
    }
    if read_u16(input, 4) != FORMAT_VERSION || read_u16(input, 6) as usize != HEADER_SIZE {
        return Err(Error::BadVersion);
    }
    let count = input[24] as usize;
    if count == 0 || count > MAX_ARTIFACTS {
        return Err(Error::BadCount);
    }
    let expected = HEADER_SIZE
        .checked_add(count.checked_mul(RECORD_SIZE).ok_or(Error::BadLength)?)
        .and_then(|value| value.checked_add(SIGNATURE_SIZE))
        .ok_or(Error::BadLength)?;
    if input.len() != expected {
        return Err(Error::BadLength);
    }
    if read_u16(input, 26) != 0 || read_u16(input, 26) != 0 {
        return Err(Error::BadFlags);
    }
    let generation = read_u64(input, 8);
    let previous = read_u64(input, 16);
    if generation == 0 || previous >= generation {
        return Err(Error::BadGeneration);
    }
    if input[80..96] != ROOT_KEY_ID {
        return Err(Error::BadKey);
    }
    validate_fixed_name(&input[32..64], MAX_NAME)?;
    validate_fixed_name(&input[64..80], MAX_TARGET)?;
    for index in 0..count {
        let offset = HEADER_SIZE + index * RECORD_SIZE;
        let path = &input[offset..offset + MAX_PATH];
        validate_path(path)?;
        if read_u32(input, offset + 76) != 0
            || input[offset + 112..offset + RECORD_SIZE]
                .iter()
                .any(|byte| *byte != 0)
        {
            return Err(Error::BadRecord);
        }
        for previous_path in 0..index {
            let previous_offset = HEADER_SIZE + previous_path * RECORD_SIZE;
            if input[previous_offset..previous_offset + MAX_PATH] == *path {
                return Err(Error::DuplicatePath);
            }
        }
    }
    if !ed25519_verify(
        &input[input.len() - SIGNATURE_SIZE..],
        &ROOT_PUBLIC_KEY,
        &input[..input.len() - SIGNATURE_SIZE],
    ) {
        return Err(Error::BadSignature);
    }
    Ok(Metadata {
        bytes: input,
        artifact_count: count,
    })
}

pub fn verify_artifact(metadata: Metadata<'_>, index: usize, bytes: &[u8]) -> Result<(), Error> {
    let info = metadata.artifact(index)?;
    verify_artifact_info(&info, bytes)
}

fn verify_artifact_info(info: &ArtifactInfo, bytes: &[u8]) -> Result<(), Error> {
    if info.size != bytes.len() as u64 {
        return Err(Error::BadArtifactSize);
    }
    if sha256(bytes) != info.digest {
        return Err(Error::BadArtifactDigest);
    }
    Ok(())
}

pub fn contract_self_check() {
    assert_eq!(
        sha256(b"abc"),
        [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ]
    );
    let mut signature = [0u8; SIGNATURE_SIZE];
    signature.copy_from_slice(&BOOTSTRAP_SIGNATURE);
    assert!(ed25519_verify(
        &signature,
        &ROOT_PUBLIC_KEY,
        &BOOTSTRAP_METADATA_WITHOUT_SIGNATURE
    ));

    let mut metadata = [0u8; HEADER_SIZE + RECORD_SIZE + SIGNATURE_SIZE];
    metadata[..BOOTSTRAP_METADATA_WITHOUT_SIGNATURE.len()]
        .copy_from_slice(&BOOTSTRAP_METADATA_WITHOUT_SIGNATURE);
    metadata[BOOTSTRAP_METADATA_WITHOUT_SIGNATURE.len()..].copy_from_slice(&signature);
    let parsed = verify_metadata(&metadata).expect("signed bootstrap metadata");
    assert_eq!(parsed.generation(), 1);
    assert_eq!(parsed.artifact_count(), 1);
    assert_eq!(parsed.package_name(), b"quickinit-bootstrap");
    assert_eq!(parsed.target(), b"bootstrap");
    assert_eq!(verify_artifact(parsed, 0, BOOTSTRAP_ARTIFACT), Ok(()));

    let mut bad = metadata;
    bad[HEADER_SIZE + 64] ^= 1;
    assert!(matches!(verify_metadata(&bad), Err(Error::BadSignature)));
    assert_eq!(
        verify_artifact(parsed, 0, b"tampered"),
        Err(Error::BadArtifactSize)
    );

    let mut store = Store::new();
    let token = store
        .stage(&metadata, &[BOOTSTRAP_ARTIFACT])
        .expect("stage");
    assert_eq!(store.active_generation(), 0);
    assert_eq!(store.commit(token).expect("commit").generation, 1);
    assert_eq!(
        store.active_artifact(b"bin/bootstrap"),
        Ok(BOOTSTRAP_ARTIFACT)
    );
    assert_eq!(store.commit(token), Err(Error::InvalidToken));
    assert_eq!(store.rollback(), Err(Error::NoGeneration));

    let mut second = metadata;
    put_u64(&mut second, 8, 2);
    put_u64(&mut second, 16, 1);
    // The second record is intentionally not accepted until it is signed;
    // this checks that generation numbers cannot be used as authority.
    assert!(matches!(verify_metadata(&second), Err(Error::BadSignature)));
    assert_eq!(
        store.stage(&metadata, &[BOOTSTRAP_ARTIFACT]),
        Err(Error::GenerationConflict)
    );

    let mut prepared = Store::new();
    let token = prepared
        .stage(&metadata, &[BOOTSTRAP_ARTIFACT])
        .expect("stage");
    assert_eq!(prepared.recover(), Err(Error::NoGeneration));
    assert!(prepared.token_slot(token).is_ok());
}

fn validate_fixed_name(bytes: &[u8], max: usize) -> Result<(), Error> {
    let length = path_length(bytes).ok_or(Error::BadName)?;
    if length == 0 || length > max || bytes[length..].iter().any(|byte| *byte != 0) {
        return Err(Error::BadName);
    }
    if bytes[..length]
        .iter()
        .any(|byte| !matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.'))
    {
        return Err(Error::BadName);
    }
    Ok(())
}

fn validate_path(bytes: &[u8]) -> Result<(), Error> {
    let length = path_length(bytes).ok_or(Error::BadPath)?;
    if length == 0 || bytes[length..].iter().any(|byte| *byte != 0) {
        return Err(Error::BadPath);
    }
    let path = &bytes[..length];
    if path[0] == b'/' || path.windows(2).any(|window| window == b"//") {
        return Err(Error::BadPath);
    }
    for component in path.split(|byte| *byte == b'/') {
        if component.is_empty() || component == b"." || component == b".." {
            return Err(Error::BadPath);
        }
    }
    if path.iter().any(
        |byte| !matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' | b'-' | b'/'),
    ) {
        return Err(Error::BadPath);
    }
    Ok(())
}

fn path_length(bytes: &[u8]) -> Option<usize> {
    let length = bytes.iter().position(|byte| *byte == 0)?;
    Some(length)
}

fn fixed_slice(bytes: &[u8], offset: usize, length: usize) -> &[u8] {
    let bytes = &bytes[offset..offset + length];
    &bytes[..path_length(bytes).unwrap_or(0)]
}

const fn copy_fixed(output: &mut [u8], offset: usize, input: &[u8]) {
    let mut index = 0;
    while index < input.len() {
        output[offset + index] = input[index];
        index += 1;
    }
}

const fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    let bytes = value.to_le_bytes();
    output[offset] = bytes[0];
    output[offset + 1] = bytes[1];
}

const fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    let bytes = value.to_le_bytes();
    copy_fixed(output, offset, &bytes);
}

const fn put_u64(output: &mut [u8], offset: usize, value: u64) {
    let bytes = value.to_le_bytes();
    copy_fixed(output, offset, &bytes);
}

fn read_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([input[offset], input[offset + 1]])
}

fn read_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(input[offset..offset + 4].try_into().unwrap())
}

fn read_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(input[offset..offset + 8].try_into().unwrap())
}

fn sha256(input: &[u8]) -> [u8; 32] {
    crate::crypto::sha256(input)
}

fn ed25519_verify(signature: &[u8], public_key: &[u8; 32], message: &[u8]) -> bool {
    let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    let Ok(signature) = ed25519_dalek::Signature::from_slice(signature) else {
        return false;
    };
    key.verify_strict(message, &signature).is_ok()
}
