//! Kernel-owned service authority.
//!
//! Handles in this module are process-visible values, not object addresses.
//! Every lookup checks the complete slot generation, the owning process, the
//! object kind, and the requested rights.  The table is deliberately bounded
//! until the real process/address-space ABI can provide a reclaimable handle
//! table.

use crate::drivers::framework::DeviceId;
use crate::ipc::{Channel, Error as IpcError};
use crate::process::ProcessId;
use core::ops::{BitOr, BitOrAssign};

pub const CAPABILITY_ABI_VERSION: u16 = 1;
pub const MAX_HANDLES: usize = 32;
pub const MAX_OBJECTS: usize = 32;
const MAX_PARENTS: usize = 2;
const MAX_MEMORY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_IO_BYTES: u64 = 1 << 40;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityHandle(u32);

impl CapabilityHandle {
    const SLOT_BITS: u32 = 8;
    const SLOT_MASK: u32 = (1 << Self::SLOT_BITS) - 1;
    const GENERATION_MASK: u32 = 0xffff;

    const fn new(slot: usize, generation: u16) -> Self {
        Self(((generation as u32) << Self::SLOT_BITS) | (slot as u32 + 1))
    }

    pub const fn from_raw(raw: u32) -> Option<Self> {
        let slot = raw & Self::SLOT_MASK;
        let generation = (raw >> Self::SLOT_BITS) & Self::GENERATION_MASK;
        if slot == 0 || slot as usize > MAX_HANDLES || generation == 0 || raw >> 24 != 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    const fn slot(self) -> usize {
        (self.0 & Self::SLOT_MASK) as usize - 1
    }

    const fn generation(self) -> u16 {
        (self.0 >> Self::SLOT_BITS) as u16
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionHandle(CapabilityHandle);

impl SessionHandle {
    // Staged userspace session-handle decoding boundary.
    #[allow(dead_code)]
    pub const fn from_raw(raw: u32) -> Option<Self> {
        match CapabilityHandle::from_raw(raw) {
            Some(handle) => Some(Self(handle)),
            None => None,
        }
    }

    pub const fn raw(self) -> u32 {
        self.0.raw()
    }

    pub(crate) const fn capability(self) -> CapabilityHandle {
        self.0
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rights(u32);

impl Rights {
    pub const NONE: Self = Self(0);
    pub const USE: Self = Self(1 << 0);
    pub const TRANSFER: Self = Self(1 << 1);
    pub const REVOKE: Self = Self(1 << 2);
    pub const SEND: Self = Self(1 << 3);
    pub const RECEIVE: Self = Self(1 << 4);
    pub const DEVICE_MMIO: Self = Self(1 << 5);
    pub const DEVICE_IRQ: Self = Self(1 << 6);
    pub const DEVICE_DMA: Self = Self(1 << 7);
    pub const DMA_USE: Self = Self(1 << 8);
    const ALL: Self = Self((1 << 9) - 1);

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn is_valid(self) -> bool {
        self.0 != 0 && self.0 & !Self::ALL.0 == 0
    }
}

impl BitOr for Rights {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Rights {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Session,
    Ipc,
    Device,
    Dma,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceLimits {
    pub max_cpu_ticks: u64,
    pub max_memory_bytes: u64,
    pub max_io_bytes: u64,
    pub max_handles: u16,
}

impl ResourceLimits {
    pub const DEFAULT: Self = Self {
        max_cpu_ticks: 1_000_000,
        max_memory_bytes: 16 * 1024 * 1024,
        max_io_bytes: 16 * 1024 * 1024,
        max_handles: MAX_HANDLES as u16,
    };

    pub const fn validate(self) -> Result<(), Error> {
        if self.max_cpu_ticks == 0
            || self.max_memory_bytes == 0
            || self.max_memory_bytes > MAX_MEMORY_BYTES
            || self.max_io_bytes == 0
            || self.max_io_bytes > MAX_IO_BYTES
            || self.max_handles == 0
            || self.max_handles as usize > MAX_HANDLES
        {
            Err(Error::InvalidLimits)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceUsage {
    pub cpu_ticks: u64,
    pub memory_bytes: u64,
    pub io_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityInfo {
    pub kind: ObjectKind,
    pub rights: Rights,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperationLease {
    object: ObjectId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidHandle,
    InvalidObject,
    InvalidOwner,
    InvalidKind,
    PermissionDenied,
    Revoked,
    Capacity,
    GenerationExhausted,
    InvalidRights,
    InvalidLimits,
    ResourceLimit,
    InvalidSize,
    Busy,
    InvalidOperation,
    Ipc(IpcError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObjectId {
    slot: u8,
    generation: u16,
}

impl ObjectId {
    const fn new(slot: usize, generation: u16) -> Self {
        Self {
            slot: slot as u8,
            generation,
        }
    }
}

#[derive(Clone, Copy)]
struct SessionObject {
    limits: ResourceLimits,
    usage: ResourceUsage,
}

// Fixed-capacity, allocator-free authority table keeps channel ownership inline.
#[allow(clippy::large_enum_variant)]
enum ObjectPayload {
    Session(SessionObject),
    Ipc {
        channel: Channel,
        session: ObjectId,
    },
    Device {
        id: DeviceId,
        // The parent edge owns the session; retained for the staged device ABI.
        #[allow(dead_code)]
        session: ObjectId,
    },
    Dma {
        device: ObjectId,
        session: ObjectId,
        bytes: u64,
    },
}

struct ObjectRecord {
    id: ObjectId,
    kind: ObjectKind,
    owner: ProcessId,
    active: bool,
    references: u16,
    in_flight: u16,
    dependents: u8,
    parents: [Option<ObjectId>; MAX_PARENTS],
    payload: ObjectPayload,
}

#[derive(Clone, Copy)]
struct HandleSlot {
    object: ObjectId,
    owner: ProcessId,
    rights: Rights,
}

pub struct CapabilityTable {
    handles: [Option<HandleSlot>; MAX_HANDLES],
    handle_generations: [u16; MAX_HANDLES],
    objects: [Option<ObjectRecord>; MAX_OBJECTS],
    object_generations: [u16; MAX_OBJECTS],
}

impl CapabilityTable {
    pub const fn new() -> Self {
        Self {
            handles: [None; MAX_HANDLES],
            handle_generations: [1; MAX_HANDLES],
            objects: [const { None }; MAX_OBJECTS],
            object_generations: [1; MAX_OBJECTS],
        }
    }

    pub fn create_session(
        &mut self,
        owner: ProcessId,
        limits: ResourceLimits,
    ) -> Result<SessionHandle, Error> {
        limits.validate()?;
        let object = self.create_object(
            ObjectKind::Session,
            owner,
            [None, None],
            ObjectPayload::Session(SessionObject {
                limits,
                usage: ResourceUsage {
                    cpu_ticks: 0,
                    memory_bytes: 0,
                    io_bytes: 0,
                },
            }),
        )?;
        let handle = match self.install(
            owner,
            object,
            Rights::USE | Rights::TRANSFER | Rights::REVOKE,
        ) {
            Ok(handle) => handle,
            Err(error) => {
                self.reap_unreferenced(object);
                return Err(error);
            }
        };
        Ok(SessionHandle(handle))
    }

    pub fn create_device(
        &mut self,
        owner: ProcessId,
        session: SessionHandle,
        device: DeviceId,
    ) -> Result<CapabilityHandle, Error> {
        let session = self.resolve_session(owner, session, Rights::USE)?;
        let object = self.create_object(
            ObjectKind::Device,
            owner,
            [Some(session), None],
            ObjectPayload::Device {
                id: device,
                session,
            },
        )?;
        match self.install(
            owner,
            object,
            Rights::USE
                | Rights::TRANSFER
                | Rights::REVOKE
                | Rights::DEVICE_MMIO
                | Rights::DEVICE_IRQ
                | Rights::DEVICE_DMA,
        ) {
            Ok(handle) => Ok(handle),
            Err(error) => {
                self.reap_unreferenced(object);
                Err(error)
            }
        }
    }

    pub fn create_dma(
        &mut self,
        owner: ProcessId,
        session: SessionHandle,
        device: CapabilityHandle,
        bytes: u64,
    ) -> Result<CapabilityHandle, Error> {
        if bytes == 0 || bytes > MAX_MEMORY_BYTES {
            return Err(Error::InvalidSize);
        }
        let session = self.resolve_session(owner, session, Rights::USE)?;
        let device = self.resolve(owner, device, ObjectKind::Device, Rights::DEVICE_DMA)?;
        if !matches!(self.object(device)?.payload, ObjectPayload::Device { .. }) {
            return Err(Error::InvalidKind);
        }
        self.charge_memory(session, bytes)?;
        let object = match self.create_object(
            ObjectKind::Dma,
            owner,
            [Some(device), Some(session)],
            ObjectPayload::Dma {
                device,
                session,
                bytes,
            },
        ) {
            Ok(object) => object,
            Err(error) => {
                self.uncharge_memory(session, bytes);
                return Err(error);
            }
        };
        match self.install(
            owner,
            object,
            Rights::USE | Rights::TRANSFER | Rights::DMA_USE,
        ) {
            Ok(handle) => Ok(handle),
            Err(error) => {
                self.uncharge_memory(session, bytes);
                self.reap_unreferenced(object);
                Err(error)
            }
        }
    }

    pub fn create_ipc_pair(
        &mut self,
        owner: ProcessId,
        peer: ProcessId,
        session: SessionHandle,
    ) -> Result<(CapabilityHandle, CapabilityHandle), Error> {
        if owner == peer {
            return Err(Error::InvalidOwner);
        }
        let session = self.resolve_session(owner, session, Rights::USE)?;
        let object = self.create_object(
            ObjectKind::Ipc,
            owner,
            [Some(session), None],
            ObjectPayload::Ipc {
                channel: Channel::new(),
                session,
            },
        )?;
        let sender =
            match self.install(owner, object, Rights::USE | Rights::SEND | Rights::TRANSFER) {
                Ok(handle) => handle,
                Err(error) => {
                    self.reap_unreferenced(object);
                    return Err(error);
                }
            };
        match self.install(
            peer,
            object,
            Rights::USE | Rights::RECEIVE | Rights::TRANSFER,
        ) {
            Ok(receiver) => Ok((sender, receiver)),
            Err(error) => {
                let _ = self.close(owner, sender);
                Err(error)
            }
        }
    }

    pub fn transfer(
        &mut self,
        sender: ProcessId,
        handle: CapabilityHandle,
        receiver: ProcessId,
        rights: Rights,
    ) -> Result<CapabilityHandle, Error> {
        if !rights.is_valid() {
            return Err(Error::InvalidRights);
        }
        let object = self.resolve_any(sender, handle, Rights::NONE)?;
        let source = self.handle(handle)?;
        if !source.rights.contains(Rights::TRANSFER) || !source.rights.contains(rights) {
            return Err(Error::PermissionDenied);
        }
        if !self.object(object)?.active {
            return Err(Error::Revoked);
        }
        self.install(receiver, object, rights)
    }

    pub fn close(&mut self, owner: ProcessId, handle: CapabilityHandle) -> Result<(), Error> {
        let index = self.validate_handle(owner, handle)?;
        let slot = self.handles[index].take().ok_or(Error::InvalidHandle)?;
        self.release_object(slot.object);
        self.bump_handle_generation(index)
    }

    pub fn revoke(&mut self, owner: ProcessId, handle: CapabilityHandle) -> Result<(), Error> {
        let index = self.validate_handle(owner, handle)?;
        let slot = self.handles[index].ok_or(Error::InvalidHandle)?;
        if !slot.rights.contains(Rights::REVOKE) {
            return Err(Error::PermissionDenied);
        }
        self.revoke_object(slot.object)
    }

    /// Kernel lifecycle code uses this after a service quiesces.  Userspace
    /// never gets an authority-free revoke operation.
    pub fn revoke_kernel(&mut self, handle: CapabilityHandle) -> Result<(), Error> {
        let object = self.handle(handle)?.object;
        self.revoke_object(object)
    }

    pub fn revoke_session(&mut self, session: SessionHandle) -> Result<(), Error> {
        let object = self.handle(session.capability())?.object;
        if self.object(object)?.kind != ObjectKind::Session {
            return Err(Error::InvalidKind);
        }
        self.revoke_object(object)
    }

    // Staged device-hotplug lifecycle hook, exercised when runtime removal is wired.
    #[allow(dead_code)]
    pub fn revoke_device(&mut self, device: DeviceId) -> Result<usize, Error> {
        let mut devices = [None; MAX_OBJECTS];
        let mut device_count = 0;
        for record in self.objects.iter().flatten() {
            if matches!(&record.payload, ObjectPayload::Device { id, .. } if *id == device)
                && device_count < devices.len()
            {
                devices[device_count] = Some(record.id);
                device_count += 1;
            }
        }
        let mut targets = [None; MAX_OBJECTS];
        let mut count = 0;
        for record in self.objects.iter().flatten() {
            let matches = match &record.payload {
                ObjectPayload::Device { id, .. } => *id == device,
                ObjectPayload::Dma { device, .. } => {
                    devices[..device_count].contains(&Some(*device))
                }
                ObjectPayload::Session(_) | ObjectPayload::Ipc { .. } => false,
            };
            if matches && count < targets.len() {
                targets[count] = Some(record.id);
                count += 1;
            }
        }
        for target in targets.into_iter().flatten().take(count) {
            self.revoke_object(target)?;
        }
        Ok(count)
    }

    pub fn revoke_owner(&mut self, owner: ProcessId) -> Result<(), Error> {
        let mut objects = [None; MAX_OBJECTS];
        let mut count = 0;
        for record in self.objects.iter().flatten() {
            if record.owner == owner && count < objects.len() {
                objects[count] = Some(record.id);
                count += 1;
            }
        }
        for object in objects.into_iter().flatten().take(count) {
            self.revoke_object(object)?;
        }
        let mut index = 0;
        while index < MAX_HANDLES {
            if self.handles[index].is_some_and(|slot| slot.owner == owner) {
                let generation = self.handle_generations[index];
                if let Some(handle) =
                    CapabilityHandle::from_raw(CapabilityHandle::new(index, generation).raw())
                {
                    let _ = self.close(owner, handle);
                }
            }
            index += 1;
        }
        Ok(())
    }

    pub fn info(
        &self,
        owner: ProcessId,
        handle: CapabilityHandle,
    ) -> Result<CapabilityInfo, Error> {
        let slot = self.handle_for_owner(owner, handle)?;
        let object = self.object(slot.object)?;
        Ok(CapabilityInfo {
            kind: object.kind,
            rights: slot.rights,
            active: object.active,
        })
    }

    pub fn usage(&self, owner: ProcessId, session: SessionHandle) -> Result<ResourceUsage, Error> {
        let object = self.resolve_session(owner, session, Rights::USE)?;
        match &self.object(object)?.payload {
            ObjectPayload::Session(session) => Ok(session.usage),
            _ => Err(Error::InvalidKind),
        }
    }

    pub fn charge_cpu(
        &mut self,
        owner: ProcessId,
        session: SessionHandle,
        ticks: u64,
    ) -> Result<(), Error> {
        let object = self.resolve_session(owner, session, Rights::USE)?;
        let session = self.session_mut(object)?;
        let next = session
            .usage
            .cpu_ticks
            .checked_add(ticks)
            .ok_or(Error::ResourceLimit)?;
        if next > session.limits.max_cpu_ticks {
            return Err(Error::ResourceLimit);
        }
        session.usage.cpu_ticks = next;
        Ok(())
    }

    pub fn dma_begin(
        &mut self,
        owner: ProcessId,
        handle: CapabilityHandle,
    ) -> Result<OperationLease, Error> {
        let object = self.resolve(owner, handle, ObjectKind::Dma, Rights::DMA_USE)?;
        self.begin_object(object)
    }

    pub fn operation_complete(&mut self, lease: OperationLease) -> Result<(), Error> {
        let object = self.object_mut(lease.object)?;
        if object.in_flight == 0 {
            return Err(Error::InvalidOperation);
        }
        object.in_flight -= 1;
        self.try_reap(lease.object);
        Ok(())
    }

    pub fn ipc_send(
        &mut self,
        owner: ProcessId,
        handle: CapabilityHandle,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let object = self.resolve(owner, handle, ObjectKind::Ipc, Rights::SEND)?;
        let session = self.ipc_session(object)?;
        self.charge_io(session, bytes.len() as u64)?;
        let result = self.with_ipc(object, |channel| channel.send(bytes));
        if result.is_err() {
            self.uncharge_io(session, bytes.len() as u64);
        }
        result.map_err(Error::Ipc)
    }

    pub fn ipc_receive(
        &mut self,
        owner: ProcessId,
        handle: CapabilityHandle,
        output: &mut [u8],
    ) -> Result<usize, Error> {
        let object = self.resolve(owner, handle, ObjectKind::Ipc, Rights::RECEIVE)?;
        let session = self.ipc_session(object)?;
        let length = self.ipc_next_length(object)?;
        if output.len() < length {
            return Err(Error::Ipc(IpcError::BufferTooSmall));
        }
        self.charge_io(session, length as u64)?;
        let result = self.with_ipc(object, |channel| channel.receive(output));
        if result.is_err() {
            self.uncharge_io(session, length as u64);
        }
        result.map_err(Error::Ipc)
    }

    fn create_object(
        &mut self,
        kind: ObjectKind,
        owner: ProcessId,
        parents: [Option<ObjectId>; MAX_PARENTS],
        payload: ObjectPayload,
    ) -> Result<ObjectId, Error> {
        let slot = self
            .objects
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let generation = self.object_generations[slot];
        if generation == 0 {
            return Err(Error::GenerationExhausted);
        }
        for (parent_count, parent) in parents.into_iter().flatten().enumerate() {
            if parent_count == MAX_PARENTS || !self.object(parent)?.active {
                return Err(Error::InvalidObject);
            }
        }
        let id = ObjectId::new(slot, generation);
        self.objects[slot] = Some(ObjectRecord {
            id,
            kind,
            owner,
            active: true,
            references: 0,
            in_flight: 0,
            dependents: 0,
            parents,
            payload,
        });
        for parent in parents.into_iter().flatten() {
            let dependents = self.object(parent)?.dependents;
            self.object_mut(parent)?.dependents = dependents.saturating_add(1);
        }
        Ok(id)
    }

    fn install(
        &mut self,
        owner: ProcessId,
        object: ObjectId,
        rights: Rights,
    ) -> Result<CapabilityHandle, Error> {
        if !rights.is_valid() {
            return Err(Error::InvalidRights);
        }
        if !self.object(object)?.active {
            return Err(Error::Revoked);
        }
        let slot = (0..MAX_HANDLES)
            .find(|index| self.handles[*index].is_none() && self.handle_generations[*index] != 0)
            .ok_or(Error::Capacity)?;
        let generation = self.handle_generations[slot];
        let handle = CapabilityHandle::new(slot, generation);
        self.handles[slot] = Some(HandleSlot {
            object,
            owner,
            rights,
        });
        let references = self.object(object)?.references;
        self.object_mut(object)?.references = references.saturating_add(1);
        Ok(handle)
    }

    fn resolve_session(
        &self,
        owner: ProcessId,
        handle: SessionHandle,
        rights: Rights,
    ) -> Result<ObjectId, Error> {
        self.resolve(owner, handle.capability(), ObjectKind::Session, rights)
    }

    fn resolve(
        &self,
        owner: ProcessId,
        handle: CapabilityHandle,
        kind: ObjectKind,
        rights: Rights,
    ) -> Result<ObjectId, Error> {
        let object = self.resolve_any(owner, handle, rights)?;
        if self.object(object)?.kind != kind {
            return Err(Error::InvalidKind);
        }
        Ok(object)
    }

    fn resolve_any(
        &self,
        owner: ProcessId,
        handle: CapabilityHandle,
        rights: Rights,
    ) -> Result<ObjectId, Error> {
        let slot = self.handle_for_owner(owner, handle)?;
        let object = self.object(slot.object)?;
        if !object.active {
            return Err(Error::Revoked);
        }
        if !slot.rights.contains(rights) {
            return Err(Error::PermissionDenied);
        }
        Ok(slot.object)
    }

    fn validate_handle(&self, owner: ProcessId, handle: CapabilityHandle) -> Result<usize, Error> {
        let slot = self.handle_for_owner(owner, handle)?;
        let _ = slot;
        Ok(handle.slot())
    }

    fn handle_for_owner(
        &self,
        owner: ProcessId,
        handle: CapabilityHandle,
    ) -> Result<HandleSlot, Error> {
        if handle.generation() != self.handle_generations[handle.slot()] {
            return Err(Error::InvalidHandle);
        }
        let slot = self.handles[handle.slot()].ok_or(Error::InvalidHandle)?;
        if slot.owner != owner {
            return Err(Error::InvalidOwner);
        }
        Ok(slot)
    }

    fn handle(&self, handle: CapabilityHandle) -> Result<HandleSlot, Error> {
        if handle.generation() != self.handle_generations[handle.slot()] {
            return Err(Error::InvalidHandle);
        }
        self.handles[handle.slot()].ok_or(Error::InvalidHandle)
    }

    fn object(&self, id: ObjectId) -> Result<&ObjectRecord, Error> {
        let record = self.objects[id.slot as usize]
            .as_ref()
            .ok_or(Error::InvalidObject)?;
        (record.id == id)
            .then_some(record)
            .ok_or(Error::InvalidObject)
    }

    fn object_mut(&mut self, id: ObjectId) -> Result<&mut ObjectRecord, Error> {
        let record = self.objects[id.slot as usize]
            .as_mut()
            .ok_or(Error::InvalidObject)?;
        (record.id == id)
            .then_some(record)
            .ok_or(Error::InvalidObject)
    }

    fn session_mut(&mut self, id: ObjectId) -> Result<&mut SessionObject, Error> {
        match &mut self.object_mut(id)?.payload {
            ObjectPayload::Session(session) => Ok(session),
            _ => Err(Error::InvalidKind),
        }
    }

    fn charge_memory(&mut self, session: ObjectId, bytes: u64) -> Result<(), Error> {
        let session = self.session_mut(session)?;
        let next = session
            .usage
            .memory_bytes
            .checked_add(bytes)
            .ok_or(Error::ResourceLimit)?;
        if next > session.limits.max_memory_bytes {
            return Err(Error::ResourceLimit);
        }
        session.usage.memory_bytes = next;
        Ok(())
    }

    fn uncharge_memory(&mut self, session: ObjectId, bytes: u64) {
        if let Ok(session) = self.session_mut(session) {
            session.usage.memory_bytes = session.usage.memory_bytes.saturating_sub(bytes);
        }
    }

    fn charge_io(&mut self, session: ObjectId, bytes: u64) -> Result<(), Error> {
        let session = self.session_mut(session)?;
        let next = session
            .usage
            .io_bytes
            .checked_add(bytes)
            .ok_or(Error::ResourceLimit)?;
        if next > session.limits.max_io_bytes {
            return Err(Error::ResourceLimit);
        }
        session.usage.io_bytes = next;
        Ok(())
    }

    fn uncharge_io(&mut self, session: ObjectId, bytes: u64) {
        if let Ok(session) = self.session_mut(session) {
            session.usage.io_bytes = session.usage.io_bytes.saturating_sub(bytes);
        }
    }

    fn ipc_session(&self, object: ObjectId) -> Result<ObjectId, Error> {
        match &self.object(object)?.payload {
            ObjectPayload::Ipc { session, .. } => Ok(*session),
            _ => Err(Error::InvalidKind),
        }
    }

    fn ipc_next_length(&self, object: ObjectId) -> Result<usize, Error> {
        match &self.object(object)?.payload {
            ObjectPayload::Ipc { channel, .. } => channel.next_length().ok_or_else(|| {
                Error::Ipc(if channel.is_closed() {
                    IpcError::Closed
                } else {
                    IpcError::Empty
                })
            }),
            _ => Err(Error::InvalidKind),
        }
    }

    fn with_ipc<R>(
        &mut self,
        object: ObjectId,
        f: impl FnOnce(&mut Channel) -> Result<R, IpcError>,
    ) -> Result<R, IpcError> {
        let object = self.object_mut(object).map_err(|_| IpcError::Revoked)?;
        let ObjectPayload::Ipc { channel, .. } = &mut object.payload else {
            return Err(IpcError::Closed);
        };
        f(channel)
    }

    fn begin_object(&mut self, object: ObjectId) -> Result<OperationLease, Error> {
        let record = self.object_mut(object)?;
        if !record.active {
            return Err(Error::Revoked);
        }
        record.in_flight = record.in_flight.checked_add(1).ok_or(Error::Busy)?;
        Ok(OperationLease { object })
    }

    fn revoke_object(&mut self, object: ObjectId) -> Result<(), Error> {
        let mut children = [None; MAX_OBJECTS];
        let mut count = 0;
        for record in self.objects.iter().flatten() {
            if record.parents.contains(&Some(object)) && count < children.len() {
                children[count] = Some(record.id);
                count += 1;
            }
        }
        {
            let record = self.object_mut(object)?;
            record.active = false;
            if let ObjectPayload::Ipc { channel, .. } = &mut record.payload {
                channel.revoke();
            }
        }
        for child in children.into_iter().flatten().take(count) {
            self.revoke_object(child)?;
        }
        self.try_reap(object);
        Ok(())
    }

    fn release_object(&mut self, object: ObjectId) {
        let revoke = if let Ok(record) = self.object_mut(object) {
            record.references = record.references.saturating_sub(1);
            record.references == 0 && record.active
        } else {
            false
        };
        if revoke {
            let _ = self.revoke_object(object);
        } else {
            self.try_reap(object);
        }
    }

    fn reap_unreferenced(&mut self, object: ObjectId) {
        if let Ok(record) = self.object_mut(object) {
            record.active = false;
        }
        self.try_reap(object);
    }

    fn try_reap(&mut self, object: ObjectId) {
        let Ok(record) = self.object(object) else {
            return;
        };
        if record.active
            || record.references != 0
            || record.in_flight != 0
            || record.dependents != 0
        {
            return;
        }
        let parents = record.parents;
        let slot = object.slot as usize;
        let payload = self.objects[slot].take().unwrap().payload;
        self.object_generations[slot] = self.object_generations[slot].wrapping_add(1);
        if let ObjectPayload::Dma { session, bytes, .. } = payload {
            self.uncharge_memory(session, bytes);
        }
        for parent in parents.into_iter().flatten() {
            if let Ok(record) = self.object_mut(parent) {
                record.dependents = record.dependents.saturating_sub(1);
            }
            self.try_reap(parent);
        }
    }

    fn bump_handle_generation(&mut self, slot: usize) -> Result<(), Error> {
        self.handle_generations[slot] = self.handle_generations[slot]
            .checked_add(1)
            .ok_or(Error::GenerationExhausted)?;
        Ok(())
    }
}

pub fn contract_self_check() {
    let owner = ProcessId::INIT;
    let peer = ProcessId::from_raw(2);
    let outsider = ProcessId::from_raw(3);
    assert_eq!(CapabilityHandle::from_raw(0), None);
    assert_eq!(CapabilityHandle::from_raw(u32::MAX), None);
    assert!(ResourceLimits::DEFAULT.validate().is_ok());
    assert!(ResourceLimits {
        max_memory_bytes: 0,
        ..ResourceLimits::DEFAULT
    }
    .validate()
    .is_err());

    let mut table = CapabilityTable::new();
    let limits = ResourceLimits {
        max_memory_bytes: 4096,
        max_io_bytes: 3,
        ..ResourceLimits::DEFAULT
    };
    let session = table.create_session(owner, limits).unwrap();
    let stale = session.raw();
    assert_eq!(table.usage(outsider, session), Err(Error::InvalidOwner));
    assert_eq!(
        table.charge_cpu(owner, session, limits.max_cpu_ticks + 1),
        Err(Error::ResourceLimit)
    );
    let device = table.create_device(owner, session, 7).unwrap();
    let dma = table.create_dma(owner, session, device, 4096).unwrap();
    assert_eq!(
        table.create_dma(owner, session, device, 1),
        Err(Error::ResourceLimit)
    );
    let operation = table.dma_begin(owner, dma).unwrap();
    table.revoke_kernel(device).unwrap();
    assert!(!table.info(owner, dma).unwrap().active);
    table.operation_complete(operation).unwrap();
    table.close(owner, dma).unwrap();
    table.close(owner, device).unwrap();
    table.revoke_session(session).unwrap();
    table.close(owner, session.capability()).unwrap();
    assert_eq!(
        table.info(owner, CapabilityHandle::from_raw(stale).unwrap()),
        Err(Error::InvalidHandle)
    );
    // Send and receive each consume cumulative I/O: three bytes in each direction.
    let ipc_limits = ResourceLimits {
        max_io_bytes: 6,
        ..limits
    };
    let replacement = table.create_session(owner, ipc_limits).unwrap();
    assert_ne!(replacement.raw(), stale);

    let (send, receive) = table.create_ipc_pair(owner, peer, replacement).unwrap();
    assert_eq!(table.ipc_send(owner, send, b"abc"), Ok(()));
    assert_eq!(table.usage(owner, replacement).unwrap().io_bytes, 3);
    let mut output = [0; 3];
    assert_eq!(table.ipc_receive(peer, receive, &mut output), Ok(3));
    assert_eq!(&output, b"abc");
    assert_eq!(table.usage(owner, replacement).unwrap().io_bytes, 6);
    assert_eq!(table.ipc_send(owner, send, b"x"), Err(Error::ResourceLimit));
    assert_eq!(table.usage(owner, replacement).unwrap().io_bytes, 6);
    table.revoke_kernel(send).unwrap();
    assert_eq!(table.ipc_send(owner, send, b"x"), Err(Error::Revoked));
    assert_eq!(
        table.ipc_receive(peer, receive, &mut output),
        Err(Error::Revoked)
    );
    table.close(owner, send).unwrap();
    table.close(peer, receive).unwrap();

    let transferred = table
        .transfer(owner, replacement.capability(), outsider, Rights::USE)
        .unwrap();
    assert_eq!(
        table.info(outsider, transferred).unwrap().kind,
        ObjectKind::Session
    );
    assert_eq!(table.close(owner, replacement.capability()), Ok(()));
    assert_eq!(
        table.revoke(outsider, transferred),
        Err(Error::PermissionDenied)
    );
    table.revoke_owner(outsider).unwrap();
}
