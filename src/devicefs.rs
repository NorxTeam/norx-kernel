#![allow(
    dead_code,
    reason = "Staged ABI and hardware hooks are not all enabled in the bootstrap profile"
)]

use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::drivers::framework::{BusKind, Class, Device, DeviceId, DeviceState, DriverStage};

pub const ABI_VERSION: u16 = 1;

const NAME_MAX: usize = 31;
const MAX_COMPONENTS: usize = 16;
const MAX_RECORDS: usize = 32;
const MAX_EVENTS: usize = 64;
const MAX_TEXT: usize = 256;

const DEV_INODE: u32 = 0x4000;
const SYS_INODE: u32 = 0x4001;
const SYS_DEVICES_INODE: u32 = 0x4002;
const SYS_KERNEL_INODE: u32 = 0x4003;
const DEVICE_INODE_BASE: u32 = 0x5000;
const ATTRIBUTE_INODE_BASE: u32 = 0x6000;

static REGISTRY_LOCK: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Busy,
    Capacity,
    GenerationExhausted,
    InvalidName,
    InvalidPath,
    NameTooLong,
    NotFound,
    NotDirectory,
    IsDirectory,
    ReadOnly,
    InvalidHandle,
    StaleHandle,
    BufferTooSmall,
    EventOverflow,
    SequenceExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Path {
    NotVirtual,
    Reserved,
    Node(Node),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node {
    DevRoot,
    SysRoot,
    SysDevices,
    SysKernel,
    Device(DeviceHandle),
    Attribute(DeviceHandle, Attribute),
    KernelAttribute(KernelAttribute),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceHandle {
    slot: u8,
    generation: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attribute {
    Class,
    Bus,
    State,
    Driver,
    Stage,
    Generation,
    Uevent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernelAttribute {
    AbiVersion,
    HotplugDropped,
    EventSequence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Directory,
    Regular,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub inode: u32,
    pub kind: Kind,
    pub mode: u16,
    pub size: usize,
    pub links: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub kind: Kind,
    pub name: [u8; NAME_MAX],
    pub name_length: usize,
}

impl DirectoryEntry {
    const EMPTY: Self = Self {
        kind: Kind::Regular,
        name: [0; NAME_MAX],
        name_length: 0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Add,
    Change,
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub sequence: u64,
    pub kind: EventKind,
    pub device: DeviceHandle,
    pub id: DeviceId,
    pub class: Class,
    pub bus: BusKind,
    pub state: DeviceState,
}

pub struct EventCursor {
    next_sequence: u64,
}

#[derive(Clone, Copy)]
struct Component {
    bytes: [u8; NAME_MAX],
    length: usize,
}

impl Component {
    const EMPTY: Self = Self {
        bytes: [0; NAME_MAX],
        length: 0,
    };

    fn is(self, value: &[u8]) -> bool {
        self.length == value.len() && self.bytes[..self.length] == *value
    }
}

#[derive(Clone, Copy)]
struct Record {
    used: bool,
    id: DeviceId,
    name: [u8; NAME_MAX],
    name_length: usize,
    class: Class,
    bus: BusKind,
    stage: DriverStage,
    state: DeviceState,
    generation: u16,
}

impl Record {
    const EMPTY: Self = Self {
        used: false,
        id: 0,
        name: [0; NAME_MAX],
        name_length: 0,
        class: Class::Serial,
        bus: BusKind::Platform,
        stage: DriverStage::Runtime,
        state: DeviceState::Removed,
        generation: 0,
    };

    fn handle(self, slot: usize) -> DeviceHandle {
        DeviceHandle {
            slot: slot as u8,
            generation: self.generation,
        }
    }
}

#[derive(Clone, Copy)]
struct EventSlot {
    used: bool,
    event: Event,
}

impl EventSlot {
    const EMPTY: Self = Self {
        used: false,
        event: Event {
            sequence: 0,
            kind: EventKind::Add,
            device: DeviceHandle {
                slot: 0,
                generation: 0,
            },
            id: 0,
            class: Class::Serial,
            bus: BusKind::Platform,
            state: DeviceState::Removed,
        },
    };
}

struct Registry {
    records: [Record; MAX_RECORDS],
    events: [EventSlot; MAX_EVENTS],
    event_next: u64,
    event_len: usize,
    dropped: u64,
}

impl Registry {
    const fn new() -> Self {
        Self {
            records: [Record::EMPTY; MAX_RECORDS],
            events: [EventSlot::EMPTY; MAX_EVENTS],
            event_next: 0,
            event_len: 0,
            dropped: 0,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn find_id(&self, id: DeviceId) -> Option<usize> {
        self.records
            .iter()
            .position(|record| record.used && record.id == id)
    }

    fn find_name(&self, component: Component) -> Option<usize> {
        self.records.iter().position(|record| {
            record.used
                && record.name_length == component.length
                && record.name[..record.name_length] == component.bytes[..component.length]
        })
    }

    fn record(&self, handle: DeviceHandle) -> Result<(usize, Record), Error> {
        let slot = usize::from(handle.slot);
        let record = self.records.get(slot).ok_or(Error::InvalidHandle)?;
        if !record.used {
            return Err(Error::StaleHandle);
        }
        if record.generation != handle.generation {
            return Err(Error::StaleHandle);
        }
        Ok((slot, *record))
    }

    fn ensure_event_sequence(&self) -> Result<(), Error> {
        if self.event_next == u64::MAX {
            Err(Error::SequenceExhausted)
        } else {
            Ok(())
        }
    }

    fn emit(&mut self, kind: EventKind, record: Record, handle: DeviceHandle) -> Result<(), Error> {
        self.ensure_event_sequence()?;
        let sequence = self.event_next;
        let index = (sequence as usize) % MAX_EVENTS;
        if self.event_len == MAX_EVENTS {
            self.dropped = self.dropped.saturating_add(1);
        } else {
            self.event_len += 1;
        }
        self.events[index] = EventSlot {
            used: true,
            event: Event {
                sequence,
                kind,
                device: handle,
                id: record.id,
                class: record.class,
                bus: record.bus,
                state: record.state,
            },
        };
        self.event_next += 1;
        Ok(())
    }

    fn publish(
        &mut self,
        id: DeviceId,
        name: &str,
        class: Class,
        bus: BusKind,
        stage: DriverStage,
        state: DeviceState,
    ) -> Result<DeviceHandle, Error> {
        let (name, name_length) = checked_name(name)?;
        if let Some(slot) = self.find_id(id) {
            let old = self.records[slot];
            if old.name[..old.name_length] != name[..name_length]
                || old.name_length != name_length
                || old.class != class
                || old.bus != bus
                || old.stage != stage
            {
                return Err(Error::InvalidName);
            }
            if old.state == state {
                return Ok(old.handle(slot));
            }
            self.ensure_event_sequence()?;
            let mut updated = old;
            updated.state = state;
            self.records[slot] = updated;
            self.emit(EventKind::Change, updated, updated.handle(slot))?;
            return Ok(updated.handle(slot));
        }

        self.ensure_event_sequence()?;
        let slot = self
            .records
            .iter()
            .position(|record| !record.used)
            .ok_or(Error::Capacity)?;
        let previous_generation = self.records[slot].generation;
        let generation = previous_generation
            .checked_add(1)
            .ok_or(Error::GenerationExhausted)?;
        let record = Record {
            used: true,
            id,
            name,
            name_length,
            class,
            bus,
            stage,
            state,
            generation,
        };
        self.records[slot] = record;
        self.emit(EventKind::Add, record, record.handle(slot))?;
        Ok(record.handle(slot))
    }

    fn remove(&mut self, id: DeviceId) -> Result<(), Error> {
        let slot = self.find_id(id).ok_or(Error::NotFound)?;
        self.ensure_event_sequence()?;
        let record = self.records[slot];
        self.emit(EventKind::Remove, record, record.handle(slot))?;
        self.records[slot].used = false;
        self.records[slot].state = DeviceState::Removed;
        Ok(())
    }

    fn metadata(&self, node: Node) -> Result<Metadata, Error> {
        let (kind, mode, inode, size) = match node {
            Node::DevRoot => (Kind::Directory, 0o555, DEV_INODE, 0),
            Node::SysRoot => (Kind::Directory, 0o555, SYS_INODE, 0),
            Node::SysDevices => (Kind::Directory, 0o555, SYS_DEVICES_INODE, 0),
            Node::SysKernel => (Kind::Directory, 0o555, SYS_KERNEL_INODE, 0),
            Node::Device(handle) => {
                let (slot, record) = self.record(handle)?;
                let inode =
                    DEVICE_INODE_BASE | ((slot as u32) << 16) | u32::from(record.generation);
                (Kind::Directory, 0o555, inode, 0)
            }
            Node::Attribute(handle, attribute) => {
                let (slot, record) = self.record(handle)?;
                let mut text = [0; MAX_TEXT];
                let size = self.render_attribute(record, attribute, &mut text)?;
                let inode = ATTRIBUTE_INODE_BASE
                    | ((slot as u32) << 16)
                    | ((attribute as u32) << 8)
                    | u32::from(record.generation);
                (Kind::Regular, 0o444, inode, size)
            }
            Node::KernelAttribute(attribute) => {
                let mut text = [0; MAX_TEXT];
                let size = self.render_kernel_attribute(attribute, &mut text)?;
                let inode = ATTRIBUTE_INODE_BASE | (0xff << 16) | ((attribute as u32) << 8);
                (Kind::Regular, 0o444, inode, size)
            }
        };
        Ok(Metadata {
            inode,
            kind,
            mode,
            size,
            links: 1,
        })
    }

    fn read_node(&self, node: Node, offset: usize, output: &mut [u8]) -> Result<usize, Error> {
        let mut text = [0; MAX_TEXT];
        let length = match node {
            Node::Attribute(handle, attribute) => {
                let (_, record) = self.record(handle)?;
                self.render_attribute(record, attribute, &mut text)?
            }
            Node::KernelAttribute(attribute) => {
                self.render_kernel_attribute(attribute, &mut text)?
            }
            Node::DevRoot
            | Node::SysRoot
            | Node::SysDevices
            | Node::SysKernel
            | Node::Device(_) => return Err(Error::IsDirectory),
        };
        let start = offset.min(length);
        let count = (length - start).min(output.len());
        output[..count].copy_from_slice(&text[start..start + count]);
        Ok(count)
    }

    fn read_dir(&self, node: Node, output: &mut [DirectoryEntry]) -> Result<usize, Error> {
        if output.is_empty() {
            return Ok(0);
        }
        let mut count = 0;
        match node {
            Node::DevRoot => {}
            Node::SysRoot => {
                add_entry(output, &mut count, b"devices", Kind::Directory);
                add_entry(output, &mut count, b"kernel", Kind::Directory);
            }
            Node::SysDevices => {
                for (slot, record) in self.records.iter().enumerate() {
                    if record.used && count != output.len() {
                        output[count] = entry_from_record(record, Kind::Directory);
                        let _ = slot;
                        count += 1;
                    }
                }
            }
            Node::SysKernel => {
                add_entry(output, &mut count, b"abi_version", Kind::Regular);
                add_entry(output, &mut count, b"hotplug_dropped", Kind::Regular);
                add_entry(output, &mut count, b"event_sequence", Kind::Regular);
            }
            Node::Device(handle) => {
                let _ = self.record(handle)?;
                add_entry(output, &mut count, b"class", Kind::Regular);
                add_entry(output, &mut count, b"bus", Kind::Regular);
                add_entry(output, &mut count, b"state", Kind::Regular);
                add_entry(output, &mut count, b"driver", Kind::Regular);
                add_entry(output, &mut count, b"stage", Kind::Regular);
                add_entry(output, &mut count, b"generation", Kind::Regular);
                add_entry(output, &mut count, b"uevent", Kind::Regular);
            }
            Node::Attribute(_, _) | Node::KernelAttribute(_) => return Err(Error::NotDirectory),
        }
        Ok(count)
    }

    fn render_attribute(
        &self,
        record: Record,
        attribute: Attribute,
        output: &mut [u8; MAX_TEXT],
    ) -> Result<usize, Error> {
        let mut text = TextBuffer { output, length: 0 };
        match attribute {
            Attribute::Class => text.write_str(crate::drivers::framework::class_name(record.class)),
            Attribute::Bus => text.write_str(crate::drivers::framework::bus_name(record.bus)),
            Attribute::State => text.write_str(crate::drivers::framework::state_name(record.state)),
            Attribute::Driver => text.write_bytes(&record.name[..record.name_length]),
            Attribute::Stage => text.write_str(crate::drivers::framework::stage_name(record.stage)),
            Attribute::Generation => text.write_u64(u64::from(record.generation)),
            Attribute::Uevent => {
                text.write_str("ACTION=change\nDEVPATH=/sys/devices/")?;
                text.write_bytes(&record.name[..record.name_length])?;
                text.write_str("\nSUBSYSTEM=")?;
                text.write_str(crate::drivers::framework::class_name(record.class))?;
                text.write_str("\nSTATE=")?;
                text.write_str(crate::drivers::framework::state_name(record.state))?;
                Ok(())
            }
        }?;
        text.write_str("\n")?;
        Ok(text.length)
    }

    fn render_kernel_attribute(
        &self,
        attribute: KernelAttribute,
        output: &mut [u8; MAX_TEXT],
    ) -> Result<usize, Error> {
        let mut text = TextBuffer { output, length: 0 };
        match attribute {
            KernelAttribute::AbiVersion => text.write_u64(u64::from(ABI_VERSION)),
            KernelAttribute::HotplugDropped => text.write_u64(self.dropped),
            KernelAttribute::EventSequence => text.write_u64(self.event_next),
        }?;
        text.write_str("\n")?;
        Ok(text.length)
    }

    fn events(&self, cursor: &mut EventCursor, output: &mut [Event]) -> Result<usize, Error> {
        let oldest = self.event_next.saturating_sub(self.event_len as u64);
        if cursor.next_sequence < oldest {
            return Err(Error::EventOverflow);
        }
        let available = self.event_next.saturating_sub(cursor.next_sequence);
        let count = available.min(output.len() as u64) as usize;
        for (index, destination) in output.iter_mut().take(count).enumerate() {
            let sequence = cursor.next_sequence + index as u64;
            let slot = (sequence as usize) % MAX_EVENTS;
            let event = self.events[slot];
            if !event.used || event.event.sequence != sequence {
                return Err(Error::EventOverflow);
            }
            *destination = event.event;
        }
        cursor.next_sequence += count as u64;
        Ok(count)
    }
}

struct TextBuffer<'a> {
    output: &'a mut [u8; MAX_TEXT],
    length: usize,
}

impl From<fmt::Error> for Error {
    fn from(_: fmt::Error) -> Self {
        Self::BufferTooSmall
    }
}

impl TextBuffer<'_> {
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), fmt::Error> {
        let end = self.length.checked_add(bytes.len()).ok_or(fmt::Error)?;
        if end > self.output.len() {
            return Err(fmt::Error);
        }
        self.output[self.length..end].copy_from_slice(bytes);
        self.length = end;
        Ok(())
    }

    fn write_u64(&mut self, value: u64) -> Result<(), fmt::Error> {
        let mut digits = [0; 20];
        let mut position = digits.len();
        let mut value = value;
        if value == 0 {
            return self.write_bytes(b"0");
        }
        while value != 0 {
            position -= 1;
            digits[position] = b'0' + (value % 10) as u8;
            value /= 10;
        }
        self.write_bytes(&digits[position..])
    }
}

impl Write for TextBuffer<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.write_bytes(value.as_bytes())
    }
}

struct RegistryLock;

impl RegistryLock {
    fn acquire() -> Result<Self, Error> {
        REGISTRY_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| Self)
            .map_err(|_| Error::Busy)
    }
}

impl Drop for RegistryLock {
    fn drop(&mut self) {
        REGISTRY_LOCK.store(false, Ordering::Release);
    }
}

static mut REGISTRY: Registry = Registry::new();

fn with_registry<R>(f: impl FnOnce(&mut Registry) -> Result<R, Error>) -> Result<R, Error> {
    let _lock = RegistryLock::acquire()?;
    unsafe { f(&mut *core::ptr::addr_of_mut!(REGISTRY)) }
}

pub fn init() {
    let _ = with_registry(|registry| {
        registry.reset();
        Ok(())
    });
}

pub fn publish_driver(
    id: DeviceId,
    name: &'static str,
    class: Class,
    bus: BusKind,
    stage: DriverStage,
    state: DeviceState,
) -> Result<DeviceHandle, Error> {
    with_registry(|registry| registry.publish(id, name, class, bus, stage, state))
}

pub fn publish_device(device: &Device) -> Result<DeviceHandle, Error> {
    publish_driver(
        device.id,
        device.name,
        device.class,
        device.bus_kind,
        DriverStage::Runtime,
        device.state,
    )
}

pub fn remove_device(id: DeviceId) -> Result<(), Error> {
    with_registry(|registry| registry.remove(id))
}

pub fn classify(path: &str) -> Result<Path, Error> {
    let components = parse_components(path)?;
    with_registry(|registry| {
        if components.is_empty() {
            return Ok(Path::NotVirtual);
        }
        if components[0].is(b"dev") {
            return Ok(if components.len() == 1 {
                Path::Node(Node::DevRoot)
            } else {
                Path::Reserved
            });
        }
        if !components[0].is(b"sys") {
            return Ok(Path::NotVirtual);
        }
        if components.len() == 1 {
            return Ok(Path::Node(Node::SysRoot));
        }
        if components[1].is(b"kernel") {
            return Ok(match components.get(2..).unwrap_or_default() {
                [] => Path::Node(Node::SysKernel),
                [attribute] if attribute.is(b"abi_version") => {
                    Path::Node(Node::KernelAttribute(KernelAttribute::AbiVersion))
                }
                [attribute] if attribute.is(b"hotplug_dropped") => {
                    Path::Node(Node::KernelAttribute(KernelAttribute::HotplugDropped))
                }
                [attribute] if attribute.is(b"event_sequence") => {
                    Path::Node(Node::KernelAttribute(KernelAttribute::EventSequence))
                }
                _ => Path::Reserved,
            });
        }
        if !components[1].is(b"devices") {
            return Ok(Path::Reserved);
        }
        if components.len() == 2 {
            return Ok(Path::Node(Node::SysDevices));
        }
        let slot = registry.find_name(components[2]).ok_or(Error::NotFound)?;
        let record = registry.records[slot];
        let handle = record.handle(slot);
        if components.len() == 3 {
            return Ok(Path::Node(Node::Device(handle)));
        }
        if components.len() != 4 {
            return Ok(Path::Reserved);
        }
        let attribute = match components[3] {
            value if value.is(b"class") => Attribute::Class,
            value if value.is(b"bus") => Attribute::Bus,
            value if value.is(b"state") => Attribute::State,
            value if value.is(b"driver") => Attribute::Driver,
            value if value.is(b"stage") => Attribute::Stage,
            value if value.is(b"generation") => Attribute::Generation,
            value if value.is(b"uevent") => Attribute::Uevent,
            _ => return Ok(Path::Reserved),
        };
        Ok(Path::Node(Node::Attribute(handle, attribute)))
    })
}

pub fn metadata(node: Node) -> Result<Metadata, Error> {
    with_registry(|registry| registry.metadata(node))
}

pub fn read(node: Node, offset: usize, output: &mut [u8]) -> Result<usize, Error> {
    with_registry(|registry| registry.read_node(node, offset, output))
}

pub fn read_dir(node: Node, output: &mut [DirectoryEntry]) -> Result<usize, Error> {
    with_registry(|registry| registry.read_dir(node, output))
}

pub fn subscribe() -> Result<EventCursor, Error> {
    with_registry(|registry| {
        Ok(EventCursor {
            next_sequence: registry.event_next,
        })
    })
}

pub fn read_events(cursor: &mut EventCursor, output: &mut [Event]) -> Result<usize, Error> {
    with_registry(|registry| registry.events(cursor, output))
}

pub fn resync_events(cursor: &mut EventCursor) -> Result<(), Error> {
    with_registry(|registry| {
        cursor.next_sequence = registry.event_next;
        Ok(())
    })
}

pub fn is_reserved(path: &str) -> Result<bool, Error> {
    Ok(matches!(classify(path)?, Path::Reserved | Path::Node(_)))
}

fn checked_name(name: &str) -> Result<([u8; NAME_MAX], usize), Error> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > NAME_MAX || bytes == b"." || bytes == b".." {
        return Err(if bytes.len() > NAME_MAX {
            Error::NameTooLong
        } else {
            Error::InvalidName
        });
    }
    if bytes.iter().any(|byte| *byte == b'/' || *byte == 0) {
        return Err(Error::InvalidName);
    }
    let mut output = [0; NAME_MAX];
    output[..bytes.len()].copy_from_slice(bytes);
    Ok((output, bytes.len()))
}

fn parse_components(path: &str) -> Result<[Component; MAX_COMPONENTS], Error> {
    if !path.starts_with('/') || path.as_bytes().contains(&0) {
        return Err(Error::InvalidPath);
    }
    let mut components = [Component::EMPTY; MAX_COMPONENTS];
    let mut length: usize = 0;
    for value in path.split('/') {
        if value.is_empty() || value == "." {
            continue;
        }
        if value == ".." {
            length = length.checked_sub(1).ok_or(Error::InvalidPath)?;
            continue;
        }
        let bytes = value.as_bytes();
        if bytes.len() > NAME_MAX {
            return Err(Error::NameTooLong);
        }
        if length == MAX_COMPONENTS {
            return Err(Error::InvalidPath);
        }
        components[length].bytes[..bytes.len()].copy_from_slice(bytes);
        components[length].length = bytes.len();
        length += 1;
    }
    Ok(components)
}

fn add_entry(output: &mut [DirectoryEntry], count: &mut usize, name: &[u8], kind: Kind) {
    if *count == output.len() {
        return;
    }
    let mut entry = DirectoryEntry::EMPTY;
    entry.kind = kind;
    entry.name[..name.len()].copy_from_slice(name);
    entry.name_length = name.len();
    output[*count] = entry;
    *count += 1;
}

fn entry_from_record(record: &Record, kind: Kind) -> DirectoryEntry {
    DirectoryEntry {
        kind,
        name: record.name,
        name_length: record.name_length,
    }
}

pub fn contract_self_check() {
    let mut registry = Registry::new();
    let handle = registry
        .publish(
            11,
            "contract-device",
            Class::Serial,
            BusKind::Platform,
            DriverStage::Runtime,
            DeviceState::Ready,
        )
        .expect("devicefs publish");
    assert_eq!(registry.event_len, 1);
    assert!(matches!(
        registry.metadata(Node::Attribute(handle, Attribute::Driver)),
        Ok(Metadata {
            kind: Kind::Regular,
            mode: 0o444,
            ..
        })
    ));
    let mut output = [0; MAX_TEXT];
    let length = registry
        .read_node(Node::Attribute(handle, Attribute::Driver), 0, &mut output)
        .expect("devicefs attribute read");
    assert_eq!(&output[..length], b"contract-device\n");
    let mut cursor = EventCursor { next_sequence: 0 };
    let mut public_events = [EventSlot::EMPTY.event; 1];
    assert_eq!(registry.events(&mut cursor, &mut public_events), Ok(1));
    assert_eq!(public_events[0].device, handle);
    registry.remove(11).expect("devicefs remove");
    assert!(matches!(
        registry.metadata(Node::Device(handle)),
        Err(Error::StaleHandle)
    ));
    let reused = registry
        .publish(
            12,
            "replacement",
            Class::Block,
            BusKind::Virtio,
            DriverStage::Runtime,
            DeviceState::Ready,
        )
        .expect("devicefs generation reuse");
    assert_ne!(handle, reused);
    let mut public_events = [EventSlot::EMPTY.event; 2];
    assert_eq!(registry.events(&mut cursor, &mut public_events), Ok(2));
}
