use core::ptr;

use crate::{btrfs, ext4, fat32};

const MAX_INODES: usize = 32;
const MAX_CHILDREN: usize = 16;
const MAX_OPEN_HANDLES: usize = 32;
const MAX_MOUNTS: usize = 8;
const MAX_NAMESPACES: usize = 4;
const MAX_DENTRIES: usize = 16;
const MAX_COMPONENTS: usize = 16;
const MAX_PATH: usize = 256;
const NAME_MAX: usize = 31;
const FILE_MAX: usize = 4096;
const MAX_PERSISTENT_ENTRIES: usize = 16;
const HELLO_TEXT: &[u8] = b"Welcome to Norx VFS\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NotMounted,
    AlreadyMounted,
    Busy,
    InvalidPath,
    NameTooLong,
    NotFound,
    AlreadyExists,
    NotDirectory,
    IsDirectory,
    NotEmpty,
    NoSpace,
    PermissionDenied,
    ReadOnly,
    InvalidHandle,
    OffsetOutOfRange,
    MountNotFound,
    MountPointBusy,
    NamespaceNotFound,
    InvalidMountTarget,
    PropagationDenied,
    BackendError,
    BackendUnsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeType {
    Directory,
    Regular,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub inode: u32,
    pub kind: NodeType,
    pub mode: u16,
    pub size: usize,
    pub links: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub kind: NodeType,
    pub mode: u16,
    pub size: usize,
    pub links: u32,
    pub name: [u8; NAME_MAX],
    pub name_length: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenOptions {
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub truncate: bool,
    pub append: bool,
    pub exclusive: bool,
    pub mode: u16,
}

impl OpenOptions {
    pub const fn read() -> Self {
        Self {
            read: true,
            write: false,
            create: false,
            truncate: false,
            append: false,
            exclusive: false,
            mode: 0o644,
        }
    }

    pub const fn write_create() -> Self {
        Self {
            read: false,
            write: true,
            create: true,
            truncate: false,
            append: false,
            exclusive: false,
            mode: 0o644,
        }
    }

    pub const fn read_write_create() -> Self {
        Self {
            read: true,
            write: true,
            create: true,
            truncate: false,
            append: false,
            exclusive: false,
            mode: 0o644,
        }
    }

    pub const fn write_truncate() -> Self {
        Self {
            read: false,
            write: true,
            create: false,
            truncate: true,
            append: false,
            exclusive: false,
            mode: 0o644,
        }
    }

    pub const fn write_append() -> Self {
        Self {
            read: false,
            write: true,
            create: false,
            truncate: false,
            append: true,
            exclusive: false,
            mode: 0o644,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountId(u8);

impl MountId {
    pub const ROOT: Self = Self(0);

    pub const fn from_raw(value: u8) -> Option<Self> {
        if (value as usize) < MAX_MOUNTS {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn raw(self) -> u8 {
        self.0
    }

    const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamespaceId(u8);

impl NamespaceId {
    pub const ROOT: Self = Self(0);

    pub const fn from_raw(value: u8) -> Option<Self> {
        if (value as usize) < MAX_NAMESPACES {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn raw(self) -> u8 {
        self.0
    }

    const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountSource {
    Ramfs,
    Fat32,
    Ext4,
    Btrfs,
}

impl MountSource {
    pub const fn persistent_kind(self) -> Option<PersistentBackendKind> {
        match self {
            Self::Ramfs => None,
            Self::Fat32 => Some(PersistentBackendKind::Fat32),
            Self::Ext4 => Some(PersistentBackendKind::Ext4),
            Self::Btrfs => Some(PersistentBackendKind::Btrfs),
        }
    }

    pub const fn is_persistent(self) -> bool {
        self.persistent_kind().is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistentBackendKind {
    Fat32,
    Ext4,
    Btrfs,
}

impl PersistentBackendKind {
    pub const fn source(self) -> MountSource {
        match self {
            Self::Fat32 => MountSource::Fat32,
            Self::Ext4 => MountSource::Ext4,
            Self::Btrfs => MountSource::Btrfs,
        }
    }

    pub const fn supports_stat(self) -> bool {
        true
    }

    pub const fn read_only(self) -> bool {
        matches!(self, Self::Ext4 | Self::Btrfs)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PersistentPath {
    bytes: [u8; MAX_PATH],
    length: u16,
}

impl PersistentPath {
    const ROOT: Self = {
        let mut bytes = [0; MAX_PATH];
        bytes[0] = b'/';
        Self { bytes, length: 1 }
    };

    fn as_str(&self) -> Result<&str, Error> {
        // The bytes are copied only from `&str` path components, so this conversion
        // is a bounded reconstruction of already-valid UTF-8.
        let bytes = &self.bytes[..self.length as usize];
        core::str::from_utf8(bytes).map_err(|_| Error::InvalidPath)
    }

    fn push(&mut self, component: Name) -> Result<(), Error> {
        if component.is_special() {
            return Err(Error::InvalidPath);
        }
        let length = self.length as usize;
        let component_length = component.len as usize;
        let separator_length = usize::from(length != 1);
        let end = length
            .checked_add(separator_length)
            .and_then(|value| value.checked_add(component_length))
            .ok_or(Error::InvalidPath)?;
        if end > MAX_PATH {
            return Err(Error::InvalidPath);
        }
        if separator_length != 0 {
            self.bytes[length] = b'/';
        }
        self.bytes[length + separator_length..end]
            .copy_from_slice(&component.bytes[..component_length]);
        self.length = end as u16;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Propagation {
    Private,
    Shared,
    Slave,
    Unbindable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountFlags {
    pub read_only: bool,
    pub no_exec: bool,
    pub no_suid: bool,
    pub no_dev: bool,
}

impl MountFlags {
    pub const fn defaults() -> Self {
        Self {
            read_only: false,
            no_exec: false,
            no_suid: false,
            no_dev: false,
        }
    }

    pub const fn read_only() -> Self {
        Self {
            read_only: true,
            ..Self::defaults()
        }
    }

    pub const fn library() -> Self {
        Self {
            read_only: true,
            no_exec: false,
            no_suid: true,
            no_dev: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountInfo {
    pub id: MountId,
    pub namespace: NamespaceId,
    pub parent: Option<MountId>,
    pub mountpoint_inode: u16,
    pub source: MountSource,
    pub flags: MountFlags,
    pub propagation: Propagation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dentry {
    pub mount: MountId,
    pub inode: u16,
    persistent: bool,
    path: [u8; MAX_PATH],
    path_length: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct DentryHandle {
    dentry: Dentry,
    slot: u8,
    generation: u16,
}

impl DentryHandle {
    pub const fn dentry(&self) -> Dentry {
        self.dentry
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileHandle {
    slot: u8,
    generation: u16,
}

impl FileHandle {
    const TAG: u32 = 1 << 31;

    pub const fn raw(self) -> u32 {
        Self::TAG | ((self.generation as u32) << 8) | self.slot as u32
    }

    pub const fn from_raw(raw: u32) -> Option<Self> {
        if raw & Self::TAG == 0 {
            return None;
        }
        Some(Self {
            slot: (raw & 0xff) as u8,
            generation: ((raw >> 8) & 0x7fff_ffff) as u16,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Name {
    bytes: [u8; NAME_MAX],
    len: u8,
}

impl Name {
    const EMPTY: Self = Self {
        bytes: [0; NAME_MAX],
        len: 0,
    };

    const fn dot() -> Self {
        let mut bytes = [0; NAME_MAX];
        bytes[0] = b'.';
        Self { bytes, len: 1 }
    }

    const fn dotdot() -> Self {
        let mut bytes = [0; NAME_MAX];
        bytes[0] = b'.';
        bytes[1] = b'.';
        Self { bytes, len: 2 }
    }

    fn is_dot(self) -> bool {
        self == Self::dot()
    }

    fn is_dotdot(self) -> bool {
        self == Self::dotdot()
    }

    fn is_special(self) -> bool {
        self.is_dot() || self.is_dotdot()
    }
}

#[derive(Clone, Copy)]
struct Child {
    used: bool,
    inode: u16,
    name: Name,
}

impl Child {
    const EMPTY: Self = Self {
        used: false,
        inode: 0,
        name: Name::EMPTY,
    };
}

#[derive(Clone, Copy)]
struct Inode {
    used: bool,
    kind: NodeType,
    parent: u16,
    mode: u16,
    size: usize,
    data: [u8; FILE_MAX],
    children: [Child; MAX_CHILDREN],
}

impl Inode {
    const EMPTY: Self = Self {
        used: false,
        kind: NodeType::Regular,
        parent: 0,
        mode: 0,
        size: 0,
        data: [0; FILE_MAX],
        children: [Child::EMPTY; MAX_CHILDREN],
    };

    const fn new(kind: NodeType, parent: u16, mode: u16) -> Self {
        Self {
            used: true,
            kind,
            parent,
            mode,
            size: 0,
            data: [0; FILE_MAX],
            children: [Child::EMPTY; MAX_CHILDREN],
        }
    }
}

#[derive(Clone, Copy)]
struct HandleSlot {
    used: bool,
    mount: MountId,
    inode: u16,
    persistent: bool,
    path: [u8; MAX_PATH],
    path_length: u16,
    generation: u16,
    references: u16,
    offset: usize,
    readable: bool,
    writable: bool,
    append: bool,
}

impl HandleSlot {
    const EMPTY: Self = Self {
        used: false,
        mount: MountId::ROOT,
        inode: 0,
        persistent: false,
        path: [0; MAX_PATH],
        path_length: 0,
        generation: 0,
        references: 0,
        offset: 0,
        readable: false,
        writable: false,
        append: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PersistentFileInfo {
    pub kind: NodeType,
    pub mode: u32,
    pub size: u64,
}

#[derive(Clone, Copy)]
struct PersistentDirectoryEntry {
    kind: NodeType,
    mode: u32,
    size: u64,
    name: [u8; NAME_MAX],
    name_length: usize,
}

impl PersistentDirectoryEntry {
    const EMPTY: Self = Self {
        kind: NodeType::Regular,
        mode: 0,
        size: 0,
        name: [0; NAME_MAX],
        name_length: 0,
    };
}

#[derive(Clone, Copy)]
enum PersistentBackend {
    Fat32(fat32::Mount),
    Ext4(ext4::Mount),
    Btrfs(btrfs::Mount),
}

impl PersistentBackend {
    fn probe(source: PersistentBackendKind) -> Result<Self, Error> {
        match source {
            PersistentBackendKind::Fat32 => {
                let mount =
                    if crate::drivers::block::persistent() && !crate::drivers::block::read_only() {
                        fat32::probe_block_rw()
                    } else {
                        fat32::probe_block()
                    };
                mount.map(Self::Fat32).map_err(map_fat32_error)
            }
            PersistentBackendKind::Ext4 => {
                ext4::probe_block().map(Self::Ext4).map_err(map_ext4_error)
            }
            PersistentBackendKind::Btrfs => btrfs::probe_block()
                .map(Self::Btrfs)
                .map_err(map_btrfs_error),
        }
    }

    const fn kind(self) -> PersistentBackendKind {
        match self {
            Self::Fat32(_) => PersistentBackendKind::Fat32,
            Self::Ext4(_) => PersistentBackendKind::Ext4,
            Self::Btrfs(_) => PersistentBackendKind::Btrfs,
        }
    }

    fn read_only(self) -> bool {
        match self {
            Self::Fat32(mount) => mount.geometry().read_only,
            Self::Ext4(mount) => mount.geometry().read_only,
            Self::Btrfs(mount) => mount.geometry().read_only,
        }
    }

    fn write_file(self, path: &str, input: &[u8]) -> Result<usize, Error> {
        match self {
            Self::Fat32(mount) => mount.write_file(path, input).map_err(map_fat32_error),
            Self::Ext4(mount) => mount.write_file(path, input).map_err(map_ext4_error),
            Self::Btrfs(mount) => mount.write_file(path, input).map_err(map_btrfs_error),
        }
    }

    fn create_file(self, path: &str) -> Result<(), Error> {
        match self {
            Self::Fat32(mount) => mount.create_file(path).map_err(map_fat32_error),
            Self::Ext4(_) | Self::Btrfs(_) => Err(Error::ReadOnly),
        }
    }

    fn mkdir(self, path: &str) -> Result<(), Error> {
        match self {
            Self::Fat32(mount) => mount.mkdir(path).map_err(map_fat32_error),
            Self::Ext4(_) | Self::Btrfs(_) => Err(Error::ReadOnly),
        }
    }

    fn unlink(self, path: &str) -> Result<(), Error> {
        match self {
            Self::Fat32(mount) => mount.unlink(path).map_err(map_fat32_error),
            Self::Ext4(_) | Self::Btrfs(_) => Err(Error::ReadOnly),
        }
    }

    fn rename(self, old_path: &str, new_path: &str) -> Result<(), Error> {
        match self {
            Self::Fat32(mount) => mount.rename(old_path, new_path).map_err(map_fat32_error),
            Self::Ext4(_) | Self::Btrfs(_) => Err(Error::ReadOnly),
        }
    }

    fn read_file(self, path: &str, output: &mut [u8]) -> Result<usize, Error> {
        match self {
            Self::Fat32(mount) => mount.read_file(path, output).map_err(map_fat32_error),
            Self::Ext4(mount) => mount.read_file(path, output).map_err(map_ext4_error),
            Self::Btrfs(mount) => mount.read_file(path, output).map_err(map_btrfs_error),
        }
    }

    fn stat(self, path: &str) -> Result<PersistentFileInfo, Error> {
        match self {
            Self::Fat32(mount) => {
                let info = mount.stat(path).map_err(map_fat32_error)?;
                Ok(PersistentFileInfo {
                    kind: if info.directory {
                        NodeType::Directory
                    } else {
                        NodeType::Regular
                    },
                    mode: if info.directory { 0o755 } else { 0o644 },
                    size: info.size,
                })
            }
            Self::Ext4(mount) => {
                let info = mount.stat(path).map_err(map_ext4_error)?;
                Ok(PersistentFileInfo {
                    kind: if info.directory {
                        NodeType::Directory
                    } else {
                        NodeType::Regular
                    },
                    mode: info.mode as u32,
                    size: info.size,
                })
            }
            Self::Btrfs(mount) => {
                let info = mount.stat(path).map_err(map_btrfs_error)?;
                Ok(PersistentFileInfo {
                    kind: if info.directory {
                        NodeType::Directory
                    } else {
                        NodeType::Regular
                    },
                    mode: info.mode,
                    size: info.size,
                })
            }
        }
    }

    fn read_dir(self, path: &str, output: &mut [PersistentDirectoryEntry]) -> Result<usize, Error> {
        if output.len() > MAX_PERSISTENT_ENTRIES {
            return Err(Error::NoSpace);
        }
        match self {
            Self::Fat32(mount) => {
                let mut entries = [fat32::DirectoryEntry::EMPTY; MAX_PERSISTENT_ENTRIES];
                let count = mount
                    .read_dir(path, &mut entries[..output.len()])
                    .map_err(map_fat32_error)?;
                for (destination, source) in output.iter_mut().zip(entries.iter()).take(count) {
                    let name_length = usize::from(source.name_len);
                    if name_length > NAME_MAX {
                        return Err(Error::NameTooLong);
                    }
                    destination.kind = if source.directory {
                        NodeType::Directory
                    } else {
                        NodeType::Regular
                    };
                    destination.mode = if source.directory { 0o755 } else { 0o644 };
                    destination.size = source.size;
                    destination.name[..name_length].copy_from_slice(&source.name[..name_length]);
                    destination.name_length = name_length;
                }
                Ok(count)
            }
            Self::Ext4(mount) => {
                let mut entries = [ext4::DirectoryEntry::EMPTY; MAX_PERSISTENT_ENTRIES];
                let count = mount
                    .read_dir(path, &mut entries[..output.len()])
                    .map_err(map_ext4_error)?;
                for (destination, source) in output.iter_mut().zip(entries.iter()).take(count) {
                    if source.name_length > NAME_MAX {
                        return Err(Error::NameTooLong);
                    }
                    destination.kind = if source.directory {
                        NodeType::Directory
                    } else {
                        NodeType::Regular
                    };
                    destination.mode = source.mode as u32;
                    destination.size = source.size;
                    destination.name[..source.name_length]
                        .copy_from_slice(&source.name[..source.name_length]);
                    destination.name_length = source.name_length;
                }
                Ok(count)
            }
            Self::Btrfs(mount) => {
                let mut entries = [btrfs::DirectoryEntry::EMPTY; MAX_PERSISTENT_ENTRIES];
                let count = mount
                    .read_dir(path, &mut entries[..output.len()])
                    .map_err(map_btrfs_error)?;
                for (destination, source) in output.iter_mut().zip(entries.iter()).take(count) {
                    if source.name_length > NAME_MAX {
                        return Err(Error::NameTooLong);
                    }
                    destination.kind = if source.directory {
                        NodeType::Directory
                    } else {
                        NodeType::Regular
                    };
                    destination.mode = source.mode as u32;
                    destination.size = source.size;
                    destination.name[..source.name_length]
                        .copy_from_slice(&source.name[..source.name_length]);
                    destination.name_length = source.name_length;
                }
                Ok(count)
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct PersistentMount {
    backend: PersistentBackend,
    writable: bool,
}

impl PersistentMount {
    fn open_with_access(source: PersistentBackendKind, writable: bool) -> Result<Self, Error> {
        Ok(Self {
            backend: PersistentBackend::probe(source)?,
            writable,
        })
    }

    pub const fn kind(self) -> PersistentBackendKind {
        self.backend.kind()
    }

    pub const fn source(self) -> MountSource {
        self.kind().source()
    }

    pub fn read_only(self) -> bool {
        !self.writable || self.backend.read_only()
    }

    pub fn read_file(self, path: &str, output: &mut [u8]) -> Result<usize, Error> {
        validate_persistent_path(path)?;
        self.backend.read_file(path, output)
    }

    pub fn stat(self, path: &str) -> Result<PersistentFileInfo, Error> {
        validate_persistent_path(path)?;
        self.backend.stat(path)
    }

    fn read_dir(self, path: &str, output: &mut [PersistentDirectoryEntry]) -> Result<usize, Error> {
        validate_persistent_path(path)?;
        self.backend.read_dir(path, output)
    }

    pub fn write_file(self, path: &str, input: &[u8]) -> Result<usize, Error> {
        validate_persistent_path(path)?;
        if self.read_only() {
            return Err(Error::ReadOnly);
        }
        self.backend.write_file(path, input)
    }

    fn create_file(self, path: &str) -> Result<(), Error> {
        validate_persistent_path(path)?;
        if self.read_only() {
            return Err(Error::ReadOnly);
        }
        self.backend.create_file(path)
    }

    fn mkdir(self, path: &str) -> Result<(), Error> {
        validate_persistent_path(path)?;
        if self.read_only() {
            return Err(Error::ReadOnly);
        }
        self.backend.mkdir(path)
    }

    fn unlink(self, path: &str) -> Result<(), Error> {
        validate_persistent_path(path)?;
        if self.read_only() {
            return Err(Error::ReadOnly);
        }
        self.backend.unlink(path)
    }

    fn rename(self, old_path: &str, new_path: &str) -> Result<(), Error> {
        validate_persistent_path(old_path)?;
        validate_persistent_path(new_path)?;
        if self.read_only() {
            return Err(Error::ReadOnly);
        }
        self.backend.rename(old_path, new_path)
    }
}

#[derive(Clone, Copy)]
enum MountedBackend {
    Ramfs,
    Persistent(PersistentMount),
}

impl MountedBackend {
    const fn persistent(self) -> Option<PersistentMount> {
        match self {
            Self::Ramfs => None,
            Self::Persistent(backend) => Some(backend),
        }
    }
}

#[derive(Clone, Copy)]
struct MountNode {
    used: bool,
    namespace: NamespaceId,
    parent: Option<MountId>,
    mountpoint_inode: u16,
    root_inode: u16,
    source: MountSource,
    flags: MountFlags,
    propagation: Propagation,
    dentry_references: usize,
    backend: MountedBackend,
}

impl MountNode {
    const EMPTY: Self = Self {
        used: false,
        namespace: NamespaceId::ROOT,
        parent: None,
        mountpoint_inode: 0,
        root_inode: 0,
        source: MountSource::Ramfs,
        flags: MountFlags::defaults(),
        propagation: Propagation::Private,
        dentry_references: 0,
        backend: MountedBackend::Ramfs,
    };

    const fn root(namespace: NamespaceId) -> Self {
        Self {
            used: true,
            namespace,
            parent: None,
            mountpoint_inode: 0,
            root_inode: 0,
            source: MountSource::Ramfs,
            flags: MountFlags::defaults(),
            propagation: Propagation::Private,
            dentry_references: 0,
            backend: MountedBackend::Ramfs,
        }
    }
}

#[derive(Clone, Copy)]
struct NamespaceSlot {
    used: bool,
    root_mount: MountId,
}

impl NamespaceSlot {
    const EMPTY: Self = Self {
        used: false,
        root_mount: MountId::ROOT,
    };
}

#[derive(Clone, Copy)]
struct DentrySlot {
    used: bool,
    dentry: Dentry,
    generation: u16,
}

impl DentrySlot {
    const EMPTY: Self = Self {
        used: false,
        dentry: Dentry {
            mount: MountId::ROOT,
            inode: 0,
            persistent: false,
            path: [0; MAX_PATH],
            path_length: 0,
        },
        generation: 0,
    };
}

struct FileSystem {
    inodes: [Inode; MAX_INODES],
    handles: [HandleSlot; MAX_OPEN_HANDLES],
    open_count: usize,
}

impl FileSystem {
    const fn new() -> Self {
        let mut inodes = [Inode::EMPTY; MAX_INODES];
        inodes[0] = Inode::new(NodeType::Directory, 0, 0o755);
        Self {
            inodes,
            handles: [HandleSlot::EMPTY; MAX_OPEN_HANDLES],
            open_count: 0,
        }
    }

    fn reset(&mut self) {
        self.inodes.fill(Inode::EMPTY);
        self.inodes[0] = Inode::new(NodeType::Directory, 0, 0o755);
        self.handles.fill(HandleSlot::EMPTY);
        self.open_count = 0;
    }
}

#[derive(Clone, Copy)]
pub struct Stats {
    pub mounts: usize,
    pub files: usize,
    pub directories: usize,
    pub bytes: usize,
    pub open_handles: usize,
}

static mut FILE_SYSTEM: FileSystem = FileSystem::new();
static mut MOUNTED: bool = false;
static mut MOUNTS: [MountNode; MAX_MOUNTS] = [MountNode::EMPTY; MAX_MOUNTS];
static mut NAMESPACES: [NamespaceSlot; MAX_NAMESPACES] = [NamespaceSlot::EMPTY; MAX_NAMESPACES];
static mut DENTRIES: [DentrySlot; MAX_DENTRIES] = [DentrySlot::EMPTY; MAX_DENTRIES];

pub fn contract_self_check() {
    assert!(MAX_CHILDREN.is_power_of_two());
    assert!(MountId::from_raw((MAX_MOUNTS - 1) as u8).is_some());
    assert!(MountId::from_raw(MAX_MOUNTS as u8).is_none());
    assert!(NamespaceId::from_raw((MAX_NAMESPACES - 1) as u8).is_some());
    assert!(NamespaceId::from_raw(MAX_NAMESPACES as u8).is_none());
    let options = OpenOptions::read();
    assert!(options.read);
    assert!(!options.write);
    assert!(!options.create);
    let (_, count) = parse_path("/a/../b").expect("valid ramfs path");
    assert_eq!(count, 3);
    assert!(parse_path("relative").is_err());
    assert!(parse_path("/a\0/b").is_err());
    assert!(!MountSource::Ramfs.is_persistent());
    assert_eq!(
        MountSource::Fat32.persistent_kind(),
        Some(PersistentBackendKind::Fat32)
    );
    assert_eq!(
        MountSource::Ext4.persistent_kind(),
        Some(PersistentBackendKind::Ext4)
    );
    assert_eq!(
        MountSource::Btrfs.persistent_kind(),
        Some(PersistentBackendKind::Btrfs)
    );
    assert_eq!(PersistentBackendKind::Ext4.source(), MountSource::Ext4);
    assert!(PersistentBackendKind::Fat32.supports_stat());
    assert!(PersistentBackendKind::Ext4.supports_stat());
    assert!(PersistentBackendKind::Btrfs.supports_stat());
    assert!(validate_persistent_path("/bounded/path").is_ok());
    assert!(validate_persistent_path("relative").is_err());
    assert_eq!(validate_persistent_path("/a/../b"), Err(Error::InvalidPath));
    assert!(!PersistentBackendKind::Fat32.read_only());
    assert!(PersistentBackendKind::Ext4.read_only());
    assert!(PersistentBackendKind::Btrfs.read_only());
    let mut persistent_path = PersistentPath::ROOT;
    assert_eq!(persistent_path.as_str(), Ok("/"));
    let mut component = Name::EMPTY;
    component.bytes[..4].copy_from_slice(b"file");
    component.len = 4;
    persistent_path.push(component).unwrap();
    assert_eq!(persistent_path.as_str(), Ok("/file"));
}

pub fn init() -> bool {
    crate::bootlog::start(1, "mounting ramfs");
    if mount_ramfs().is_err() || mount_tests().is_err() {
        let _ = unmount_ramfs();
        crate::bootlog::fail("ramfs mount/self-check failed");
        return false;
    }
    if !write("/hello.txt", HELLO_TEXT) {
        let _ = unmount_ramfs();
        crate::bootlog::fail("ramfs initial file write failed");
        return false;
    }
    if !readback("/hello.txt", HELLO_TEXT) {
        let _ = unmount_ramfs();
        crate::bootlog::fail("ramfs /hello.txt readback failed");
        return false;
    }
    if mkdir("/tmp").is_err() {
        let _ = unmount_ramfs();
        crate::bootlog::fail("ramfs temporary directory creation failed");
        return false;
    }
    if mount_tree_self_check().is_err() {
        let _ = unmount_ramfs();
        crate::bootlog::fail("VFS mount tree/self-check failed");
        return false;
    }
    if let Err(error) = mount_library_roots() {
        let _ = unmount_ramfs();
        crate::bootlog::fail_fmt(format_args!("VFS library roots failed: {:?}", error));
        return false;
    }
    crate::bootlog::ok_fmt(format_args!(
        "ramfs mounted root directories={} files={} bytes={}",
        stats().directories,
        stats().files,
        stats().bytes,
    ));
    crate::bootlog::ok("ramfs /hello.txt readback passed");
    crate::bootlog::ok(
        "ramfs mount, offsets, permissions, rename, unlink, and unmount checks passed",
    );
    crate::bootlog::ok(
        "VFS mount tree, namespace, dentry lifetime, flags, propagation, and unmount checks passed",
    );
    crate::bootlog::ok("VFS library root mounted read-only: /lib");
    true
}

fn mount_ramfs() -> Result<(), Error> {
    unsafe {
        if MOUNTED {
            return Err(Error::AlreadyMounted);
        }
        crate::bootlog::ok("ramfs resetting static inode table");
        (&mut *core::ptr::addr_of_mut!(FILE_SYSTEM)).reset();
        crate::bootlog::ok("ramfs inode table reset");
        MOUNTED = true;
        let mounts = &mut *core::ptr::addr_of_mut!(MOUNTS);
        mounts.fill(MountNode::EMPTY);
        mounts[MountId::ROOT.index()] = MountNode::root(NamespaceId::ROOT);
        let namespaces = &mut *core::ptr::addr_of_mut!(NAMESPACES);
        namespaces.fill(NamespaceSlot::EMPTY);
        namespaces[NamespaceId::ROOT.index()] = NamespaceSlot {
            used: true,
            root_mount: MountId::ROOT,
        };
        (&mut *core::ptr::addr_of_mut!(DENTRIES)).fill(DentrySlot::EMPTY);
    }
    Ok(())
}

fn unmount_ramfs() -> Result<(), Error> {
    unsafe {
        if !MOUNTED {
            return Err(Error::NotMounted);
        }
        let fs = &*core::ptr::addr_of!(FILE_SYSTEM);
        let mounts = &*core::ptr::addr_of!(MOUNTS);
        let dentries = &*core::ptr::addr_of!(DENTRIES);
        if fs.open_count != 0
            || mounts.iter().skip(1).any(|mount| mount.used)
            || dentries.iter().any(|dentry| dentry.used)
        {
            return Err(Error::Busy);
        }
        MOUNTED = false;
    }
    Ok(())
}

pub fn mount(source: MountSource, target: &str, flags: MountFlags) -> Result<MountId, Error> {
    mount_with_propagation(
        NamespaceId::ROOT,
        source,
        target,
        flags,
        Propagation::Private,
    )
}

/// Mount a persistent reader and return its backend-scoped read-only handle.
///
/// The mount is exposed through the bounded namespace resolver without copying
/// persistent file contents into the RAMFS inode table.
pub fn mount_persistent(source: MountSource, target: &str) -> Result<PersistentMount, Error> {
    if !source.is_persistent() {
        return Err(Error::InvalidMountTarget);
    }
    let id = mount(source, target, MountFlags::read_only())?;
    match persistent_backend(id) {
        Ok(backend) => Ok(backend),
        Err(error) => {
            let _ = unmount_mount(id);
            Err(error)
        }
    }
}

pub fn mount_with_propagation(
    namespace: NamespaceId,
    source: MountSource,
    target: &str,
    flags: MountFlags,
    propagation: Propagation,
) -> Result<MountId, Error> {
    mount_in_namespace(namespace, source, target, flags, propagation)
}

pub fn mount_in_namespace(
    namespace: NamespaceId,
    source: MountSource,
    target: &str,
    flags: MountFlags,
    propagation: Propagation,
) -> Result<MountId, Error> {
    with_fs(|fs| {
        let persistent_backend = if source.is_persistent() {
            Some(PersistentMount::open_with_access(
                source.persistent_kind().ok_or(Error::InvalidMountTarget)?,
                !flags.read_only,
            )?)
        } else {
            None
        };
        if !flags.read_only && persistent_backend.is_some_and(|backend| backend.read_only()) {
            return Err(Error::ReadOnly);
        }
        let (parent, parent_inode, name) = match parent_and_name_mount(fs, target, namespace) {
            Err(Error::InvalidPath) if target == "/" => return Err(Error::InvalidMountTarget),
            result => result?,
        };
        let mountpoint = find_child(fs, parent_inode, name).ok_or(Error::NotFound)?;
        if mountpoint == 0 || fs.inodes[mountpoint as usize].kind != NodeType::Directory {
            return Err(Error::InvalidMountTarget);
        }
        let parent_node = mount_node(parent)?;
        if parent_node.propagation == Propagation::Unbindable
            || (propagation == Propagation::Slave
                && parent_node.propagation == Propagation::Private)
        {
            return Err(Error::PropagationDenied);
        }
        if find_mount_child(namespace, parent, mountpoint).is_some() {
            return Err(Error::MountPointBusy);
        }
        let mounts = unsafe { &mut *core::ptr::addr_of_mut!(MOUNTS) };
        let Some((index, node)) = mounts.iter_mut().enumerate().find(|(_, node)| !node.used) else {
            return Err(Error::NoSpace);
        };
        let backend = match (source, persistent_backend) {
            (MountSource::Ramfs, None) => MountedBackend::Ramfs,
            (_, Some(backend)) => MountedBackend::Persistent(backend),
            _ => return Err(Error::InvalidMountTarget),
        };
        *node = MountNode {
            used: true,
            namespace,
            parent: Some(parent),
            mountpoint_inode: mountpoint,
            root_inode: mountpoint,
            source,
            flags,
            propagation,
            dentry_references: 0,
            backend,
        };
        Ok(MountId(index as u8))
    })
}

pub fn unmount_mount(id: MountId) -> Result<(), Error> {
    with_fs(|fs| {
        let node = mount_node(id)?;
        if namespace_root(node.namespace)? == id {
            return Err(Error::Busy);
        }
        let mounts = unsafe { &*core::ptr::addr_of!(MOUNTS) };
        if node.dentry_references != 0
            || fs
                .handles
                .iter()
                .any(|handle| handle.used && handle.mount == id)
            || mounts
                .iter()
                .any(|child| child.used && child.parent == Some(id))
        {
            return Err(Error::Busy);
        }
        unsafe {
            (&mut *core::ptr::addr_of_mut!(MOUNTS))[id.index()] = MountNode::EMPTY;
        }
        Ok(())
    })
}

pub fn namespace_root(namespace: NamespaceId) -> Result<MountId, Error> {
    unsafe {
        let namespaces = &*core::ptr::addr_of!(NAMESPACES);
        let slot = namespaces
            .get(namespace.index())
            .filter(|slot| slot.used)
            .ok_or(Error::NamespaceNotFound)?;
        Ok(slot.root_mount)
    }
}

pub fn create_namespace() -> Result<NamespaceId, Error> {
    unsafe {
        if !MOUNTED {
            return Err(Error::NotMounted);
        }
        let namespaces = &mut *core::ptr::addr_of_mut!(NAMESPACES);
        let Some(namespace_index) = (1..MAX_NAMESPACES).find(|index| !namespaces[*index].used)
        else {
            return Err(Error::NoSpace);
        };
        let mounts = &mut *core::ptr::addr_of_mut!(MOUNTS);
        let Some(mount_index) = (1..MAX_MOUNTS).find(|index| !mounts[*index].used) else {
            return Err(Error::NoSpace);
        };
        let namespace = NamespaceId(namespace_index as u8);
        let mount = MountId(mount_index as u8);
        mounts[mount_index] = MountNode::root(namespace);
        namespaces[namespace_index] = NamespaceSlot {
            used: true,
            root_mount: mount,
        };
        Ok(namespace)
    }
}

pub fn destroy_namespace(namespace: NamespaceId) -> Result<(), Error> {
    if namespace == NamespaceId::ROOT {
        return Err(Error::Busy);
    }
    with_fs(|fs| {
        let root = namespace_root(namespace)?;
        let mounts = unsafe { &*core::ptr::addr_of!(MOUNTS) };
        if mounts.iter().any(|node| {
            node.used
                && node.namespace == namespace
                && (node.parent.is_some() || node.dentry_references != 0)
        }) || fs.handles.iter().any(|handle| {
            handle.used
                && mounts
                    .get(handle.mount.index())
                    .is_some_and(|node| node.used && node.namespace == namespace)
        }) {
            return Err(Error::Busy);
        }
        unsafe {
            (&mut *core::ptr::addr_of_mut!(MOUNTS))[root.index()] = MountNode::EMPTY;
            (&mut *core::ptr::addr_of_mut!(NAMESPACES))[namespace.index()] = NamespaceSlot::EMPTY;
        }
        Ok(())
    })
}

pub fn mount_info(id: MountId) -> Result<MountInfo, Error> {
    let node = mount_node(id)?;
    Ok(MountInfo {
        id,
        namespace: node.namespace,
        parent: node.parent,
        mountpoint_inode: node.mountpoint_inode,
        source: node.source,
        flags: node.flags,
        propagation: node.propagation,
    })
}

pub fn list_mounts(mut f: impl FnMut(MountInfo)) {
    unsafe {
        if !MOUNTED {
            return;
        }
        let mounts = &*core::ptr::addr_of!(MOUNTS);
        for (index, node) in mounts.iter().enumerate() {
            if node.used {
                let id = MountId(index as u8);
                f(MountInfo {
                    id,
                    namespace: node.namespace,
                    parent: node.parent,
                    mountpoint_inode: node.mountpoint_inode,
                    source: node.source,
                    flags: node.flags,
                    propagation: node.propagation,
                });
            }
        }
    }
}

pub fn lookup(path: &str) -> Result<DentryHandle, Error> {
    lookup_in_namespace(NamespaceId::ROOT, path)
}

pub fn lookup_in_namespace(namespace: NamespaceId, path: &str) -> Result<DentryHandle, Error> {
    with_fs(|fs| {
        let (mount, inode, persistent_path) =
            if let Some((mount, persistent_path)) = locate_persistent_path(fs, path, namespace)? {
                let path_str = persistent_path.as_str()?;
                let _ = persistent_backend(mount)?.stat(path_str)?;
                (mount, 0, Some(persistent_path))
            } else {
                let (mount, inode) = resolve_mount(fs, path, namespace)?;
                (mount, inode, None)
            };
        let dentries = unsafe { &mut *core::ptr::addr_of_mut!(DENTRIES) };
        let Some((slot, entry)) = dentries
            .iter_mut()
            .enumerate()
            .find(|(_, entry)| !entry.used)
        else {
            return Err(Error::NoSpace);
        };
        entry.used = true;
        let (persistent, path, path_length) = match persistent_path {
            Some(path) => (true, path.bytes, path.length),
            None => (false, [0; MAX_PATH], 0),
        };
        entry.dentry = Dentry {
            mount,
            inode,
            persistent,
            path,
            path_length,
        };
        entry.generation = entry.generation.wrapping_add(1);
        if entry.generation == 0 {
            entry.generation = 1;
        }
        unsafe {
            (&mut *core::ptr::addr_of_mut!(MOUNTS))[mount.index()].dentry_references += 1;
        }
        Ok(DentryHandle {
            dentry: entry.dentry,
            slot: slot as u8,
            generation: entry.generation,
        })
    })
}

pub fn release_dentry(handle: DentryHandle) -> Result<(), Error> {
    with_fs(|_| {
        let dentries = unsafe { &mut *core::ptr::addr_of_mut!(DENTRIES) };
        let slot = handle.slot as usize;
        if slot >= MAX_DENTRIES
            || !dentries[slot].used
            || dentries[slot].dentry != handle.dentry
            || dentries[slot].generation != handle.generation
        {
            return Err(Error::InvalidHandle);
        }
        let mount = handle.dentry.mount;
        let generation = dentries[slot].generation;
        dentries[slot] = DentrySlot {
            used: false,
            generation,
            ..DentrySlot::EMPTY
        };
        unsafe {
            let mounts = &mut *core::ptr::addr_of_mut!(MOUNTS);
            mounts[mount.index()].dentry_references =
                mounts[mount.index()].dentry_references.saturating_sub(1);
        }
        Ok(())
    })
}

pub fn open(path: &str, options: OpenOptions) -> Result<FileHandle, Error> {
    with_fs(|fs| {
        if options.truncate && !options.write {
            return Err(Error::PermissionDenied);
        }
        if options.append && !options.write {
            return Err(Error::PermissionDenied);
        }
        if let Some((mount, persistent_path)) = locate_persistent_path(fs, path, NamespaceId::ROOT)?
        {
            let path_str = persistent_path.as_str()?;
            let backend = persistent_backend(mount)?;
            let writable = !mount_flags(mount)?.read_only && !backend.read_only();
            if options.write && !writable {
                return Err(Error::ReadOnly);
            }
            if options.create && !writable && options.write {
                return Err(Error::ReadOnly);
            }
            if !options.read && !options.write {
                return Err(Error::PermissionDenied);
            }
            let info = match backend.stat(path_str) {
                Ok(info) => {
                    if options.create && options.exclusive {
                        return Err(Error::AlreadyExists);
                    }
                    info
                }
                Err(Error::NotFound) if options.create && writable => {
                    backend.create_file(path_str)?;
                    backend.stat(path_str)?
                }
                Err(Error::NotFound) if options.create => return Err(Error::ReadOnly),
                Err(error) => return Err(error),
            };
            if info.kind == NodeType::Directory {
                return Err(Error::IsDirectory);
            }
            if options.read && info.mode & 0o444 == 0 {
                return Err(Error::PermissionDenied);
            }
            if options.write && info.mode & 0o222 == 0 {
                return Err(Error::PermissionDenied);
            }
            if options.truncate {
                backend.write_file(path_str, &[])?;
            }
            let slot = fs
                .handles
                .iter()
                .position(|handle| !handle.used)
                .ok_or(Error::NoSpace)?;
            let handle = &mut fs.handles[slot];
            handle.used = true;
            handle.mount = mount;
            handle.inode = 0;
            handle.persistent = true;
            handle.path = persistent_path.bytes;
            handle.path_length = persistent_path.length;
            handle.generation = (handle.generation.wrapping_add(1)) & 0x7fff;
            if handle.generation == 0 {
                handle.generation = 1;
            }
            handle.references = 1;
            handle.offset = 0;
            handle.readable = options.read;
            handle.writable = options.write;
            handle.append = options.append;
            fs.open_count += 1;
            return Ok(FileHandle {
                slot: slot as u8,
                generation: handle.generation,
            });
        }
        let mut reserved_slot = None;
        let existed = resolve_mount(fs, path, NamespaceId::ROOT).is_ok();
        let (mount, inode) = match resolve_mount(fs, path, NamespaceId::ROOT) {
            Ok(result) => result,
            Err(Error::NotFound) if options.create => {
                let (mount, parent, name) = parent_and_name_mount(fs, path, NamespaceId::ROOT)?;
                if mount_flags(mount)?.read_only {
                    return Err(Error::ReadOnly);
                }
                reserved_slot = Some(
                    fs.handles
                        .iter()
                        .position(|handle| !handle.used)
                        .ok_or(Error::NoSpace)?,
                );
                (
                    mount,
                    create_node_at(fs, parent, name, NodeType::Regular, options.mode & 0o777)?,
                )
            }
            Err(error) => return Err(error),
        };
        if options.create && options.exclusive && existed {
            return Err(Error::AlreadyExists);
        }
        if (options.write || options.create) && mount_flags(mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        let node = fs.inodes[inode as usize];
        if node.kind == NodeType::Directory {
            return Err(Error::IsDirectory);
        }
        if options.read && node.mode & 0o444 == 0 {
            return Err(Error::PermissionDenied);
        }
        if options.write && node.mode & 0o222 == 0 {
            return Err(Error::PermissionDenied);
        }
        let slot = reserved_slot
            .or_else(|| fs.handles.iter().position(|handle| !handle.used))
            .ok_or(Error::NoSpace)?;
        if options.truncate {
            fs.inodes[inode as usize].size = 0;
        }
        let handle = &mut fs.handles[slot];
        handle.used = true;
        handle.mount = mount;
        handle.inode = inode;
        handle.persistent = false;
        handle.path = [0; MAX_PATH];
        handle.path_length = 0;
        handle.generation = (handle.generation.wrapping_add(1)) & 0x7fff;
        if handle.generation == 0 {
            handle.generation = 1;
        }
        handle.references = 1;
        handle.offset = 0;
        handle.readable = options.read;
        handle.writable = options.write;
        handle.append = options.append;
        fs.open_count += 1;
        Ok(FileHandle {
            slot: slot as u8,
            generation: handle.generation,
        })
    })
}

pub fn close(handle: FileHandle) -> Result<(), Error> {
    with_fs(|fs| {
        let slot = validate_handle(fs, &handle)?;
        let description = &mut fs.handles[slot];
        description.references = description.references.saturating_sub(1);
        if description.references == 0 {
            let generation = description.generation;
            fs.handles[slot] = HandleSlot {
                generation,
                ..HandleSlot::EMPTY
            };
            fs.open_count = fs.open_count.saturating_sub(1);
        }
        Ok(())
    })
}

pub fn sync_path(path: &str) -> Result<(), Error> {
    with_fs(|fs| {
        if let Some((mount, persistent_path)) = locate_persistent_path(fs, path, NamespaceId::ROOT)?
        {
            let path_str = persistent_path.as_str()?;
            let _ = persistent_backend(mount)?.stat(path_str)?;
            return crate::drivers::block::flush_cache().map_err(map_block_error);
        }
        let _ = resolve_mount(fs, path, NamespaceId::ROOT)?;
        Err(Error::BackendUnsupported)
    })
}

pub fn sync_handle(handle: FileHandle) -> Result<(), Error> {
    with_fs(|fs| {
        let slot = validate_handle(fs, &handle)?;
        if !fs.handles[slot].persistent {
            return Err(Error::BackendUnsupported);
        }
        let _ = persistent_backend(fs.handles[slot].mount)?;
        crate::drivers::block::flush_cache().map_err(map_block_error)
    })
}

pub fn duplicate(handle: FileHandle) -> Result<FileHandle, Error> {
    with_fs(|fs| {
        let slot = validate_handle(fs, &handle)?;
        fs.handles[slot].references = fs.handles[slot]
            .references
            .checked_add(1)
            .ok_or(Error::NoSpace)?;
        Ok(handle)
    })
}

pub fn read_handle(handle: FileHandle, output: &mut [u8]) -> Result<usize, Error> {
    with_fs(|fs| {
        let slot = validate_handle(fs, &handle)?;
        if !fs.handles[slot].readable {
            return Err(Error::PermissionDenied);
        }
        if fs.handles[slot].persistent {
            let description = fs.handles[slot];
            let path = PersistentPath {
                bytes: description.path,
                length: description.path_length,
            };
            let path_str = path.as_str()?;
            let backend = persistent_backend(description.mount)?;
            let info = backend.stat(path_str)?;
            if info.kind == NodeType::Directory {
                return Err(Error::IsDirectory);
            }
            if output.is_empty() {
                return Ok(0);
            }
            let mut data = [0u8; FILE_MAX];
            let length = backend.read_file(path_str, &mut data)?;
            let available = length.saturating_sub(description.offset);
            let read_length = available.min(output.len());
            if read_length != 0 {
                output[..read_length]
                    .copy_from_slice(&data[description.offset..description.offset + read_length]);
            }
            fs.handles[slot].offset = description.offset + read_length;
            return Ok(read_length);
        }
        let inode_id = fs.handles[slot].inode;
        let offset = fs.handles[slot].offset;
        let inode = &fs.inodes[inode_id as usize];
        let available = inode.size.saturating_sub(offset);
        let length = available.min(output.len());
        unsafe {
            ptr::copy_nonoverlapping(inode.data.as_ptr().add(offset), output.as_mut_ptr(), length);
        }
        fs.handles[slot].offset += length;
        Ok(length)
    })
}

pub fn write_handle(handle: FileHandle, input: &[u8]) -> Result<usize, Error> {
    with_fs(|fs| {
        let slot = validate_handle(fs, &handle)?;
        let mount = fs.handles[slot].mount;
        if mount_flags(mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        if !fs.handles[slot].writable {
            return Err(Error::PermissionDenied);
        }
        if fs.handles[slot].persistent {
            let description = fs.handles[slot];
            let path = PersistentPath {
                bytes: description.path,
                length: description.path_length,
            };
            let path_str = path.as_str()?;
            let backend = persistent_backend(mount)?;
            if backend.read_only() {
                return Err(Error::ReadOnly);
            }
            let info = backend.stat(path_str)?;
            if info.kind == NodeType::Directory {
                return Err(Error::IsDirectory);
            }
            let mut data = [0u8; FILE_MAX];
            let length = backend.read_file(path_str, &mut data)?;
            let offset = if description.append {
                length
            } else {
                description.offset
            };
            let end = offset.checked_add(input.len()).ok_or(Error::NoSpace)?;
            if end > FILE_MAX {
                return Err(Error::NoSpace);
            }
            if offset > length {
                data[length..offset].fill(0);
            }
            data[offset..end].copy_from_slice(input);
            let new_length = length.max(end);
            backend.write_file(path_str, &data[..new_length])?;
            fs.handles[slot].offset = end;
            return Ok(input.len());
        }
        let inode_id = fs.handles[slot].inode;
        let append = fs.handles[slot].append;
        let current_offset = fs.handles[slot].offset;
        let inode = &mut fs.inodes[inode_id as usize];
        let offset = if append { inode.size } else { current_offset };
        let end = offset.checked_add(input.len()).ok_or(Error::NoSpace)?;
        if end > FILE_MAX {
            return Err(Error::NoSpace);
        }
        unsafe {
            ptr::copy_nonoverlapping(
                input.as_ptr(),
                inode.data.as_mut_ptr().add(offset),
                input.len(),
            );
        }
        fs.handles[slot].offset = end;
        inode.size = inode.size.max(end);
        Ok(input.len())
    })
}

pub fn seek(handle: FileHandle, offset: isize) -> Result<usize, Error> {
    seek_from(handle, offset as i64, 1)
}

pub fn seek_from(handle: FileHandle, offset: i64, whence: u32) -> Result<usize, Error> {
    with_fs(|fs| {
        let slot = validate_handle(fs, &handle)?;
        let persistent = fs.handles[slot].persistent;
        let base = if persistent {
            let description = fs.handles[slot];
            let path = PersistentPath {
                bytes: description.path,
                length: description.path_length,
            };
            let info = persistent_backend(description.mount)?.stat(path.as_str()?)?;
            if info.kind == NodeType::Directory {
                return Err(Error::IsDirectory);
            }
            match whence {
                0 => 0,
                1 => description.offset,
                2 => usize::try_from(info.size).map_err(|_| Error::OffsetOutOfRange)?,
                _ => return Err(Error::InvalidPath),
            }
        } else {
            match whence {
                0 => 0,
                1 => fs.handles[slot].offset,
                2 => fs.inodes[fs.handles[slot].inode as usize].size,
                _ => return Err(Error::InvalidPath),
            }
        };
        let next = if offset.is_negative() {
            let distance =
                usize::try_from(offset.unsigned_abs()).map_err(|_| Error::OffsetOutOfRange)?;
            base.checked_sub(distance).ok_or(Error::OffsetOutOfRange)?
        } else {
            let distance = usize::try_from(offset).map_err(|_| Error::OffsetOutOfRange)?;
            base.checked_add(distance).ok_or(Error::OffsetOutOfRange)?
        };
        if next > FILE_MAX {
            return Err(Error::OffsetOutOfRange);
        }
        fs.handles[slot].offset = next;
        Ok(next)
    })
}

pub fn close_raw(raw: u32) -> Result<(), Error> {
    close(FileHandle::from_raw(raw).ok_or(Error::InvalidHandle)?)
}

pub fn duplicate_raw(raw: u32) -> Result<u32, Error> {
    Ok(duplicate(FileHandle::from_raw(raw).ok_or(Error::InvalidHandle)?)?.raw())
}

pub fn read_raw(raw: u32, output: &mut [u8]) -> Result<usize, Error> {
    read_handle(
        FileHandle::from_raw(raw).ok_or(Error::InvalidHandle)?,
        output,
    )
}

pub fn write_raw(raw: u32, input: &[u8]) -> Result<usize, Error> {
    write_handle(
        FileHandle::from_raw(raw).ok_or(Error::InvalidHandle)?,
        input,
    )
}

pub fn stat_handle(handle: FileHandle) -> Result<FileStat, Error> {
    with_fs(|fs| {
        let slot = validate_handle(fs, &handle)?;
        if fs.handles[slot].persistent {
            let description = fs.handles[slot];
            let path = PersistentPath {
                bytes: description.path,
                length: description.path_length,
            };
            let info = persistent_backend(description.mount)?.stat(path.as_str()?)?;
            return Ok(FileStat {
                inode: persistent_inode(path.as_str()?),
                kind: info.kind,
                mode: info.mode as u16,
                size: usize::try_from(info.size).map_err(|_| Error::OffsetOutOfRange)?,
                links: 1,
            });
        }
        let inode = fs.handles[slot].inode;
        let node = fs.inodes[inode as usize];
        Ok(FileStat {
            inode: inode as u32,
            kind: node.kind,
            mode: node.mode,
            size: node.size,
            links: link_count(fs, inode),
        })
    })
}

pub fn chmod(path: &str, mode: u16) -> Result<(), Error> {
    with_fs(|fs| {
        if locate_persistent_path(fs, path, NamespaceId::ROOT)?.is_some() {
            return Err(Error::ReadOnly);
        }
        let (mount, inode) = resolve_mount(fs, path, NamespaceId::ROOT)?;
        if mount_flags(mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        fs.inodes[inode as usize].mode = mode & 0o777;
        Ok(())
    })
}

pub fn fchmod(handle: FileHandle, mode: u16) -> Result<(), Error> {
    with_fs(|fs| {
        let slot = validate_handle(fs, &handle)?;
        if fs.handles[slot].persistent {
            return Err(Error::ReadOnly);
        }
        let mount = fs.handles[slot].mount;
        if mount_flags(mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        fs.inodes[fs.handles[slot].inode as usize].mode = mode & 0o777;
        Ok(())
    })
}

pub fn mkdir(path: &str) -> Result<(), Error> {
    mkdir_with_mode(path, 0o755)
}

pub fn mkdir_with_mode(path: &str, mode: u16) -> Result<(), Error> {
    with_fs(|fs| {
        if let Some((mount, persistent_path)) = locate_persistent_path(fs, path, NamespaceId::ROOT)?
        {
            let backend = persistent_backend(mount)?;
            if mount_flags(mount)?.read_only || backend.read_only() {
                return Err(Error::ReadOnly);
            }
            let _ = mode;
            return backend.mkdir(persistent_path.as_str()?);
        }
        let (mount, parent, name) = parent_and_name_mount(fs, path, NamespaceId::ROOT)?;
        if mount_flags(mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        create_node_at(fs, parent, name, NodeType::Directory, mode & 0o777).map(|_| ())
    })
}

pub fn unlink(path: &str) -> Result<(), Error> {
    with_fs(|fs| {
        if let Some((mount, persistent_path)) = locate_persistent_path(fs, path, NamespaceId::ROOT)?
        {
            let backend = persistent_backend(mount)?;
            if mount_flags(mount)?.read_only || backend.read_only() {
                return Err(Error::ReadOnly);
            }
            return backend.unlink(persistent_path.as_str()?);
        }
        let (mount, parent, name) = parent_and_name_mount(fs, path, NamespaceId::ROOT)?;
        if mount_flags(mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        let child = find_child(fs, parent, name).ok_or(Error::NotFound)?;
        if fs.inodes[child as usize].kind == NodeType::Directory {
            return Err(Error::IsDirectory);
        }
        if find_mount_child(NamespaceId::ROOT, mount, child).is_some() {
            return Err(Error::Busy);
        }
        if has_open_handle(fs, child) {
            return Err(Error::Busy);
        }
        remove_child(fs, parent, name)?;
        if link_count(fs, child) == 0 {
            fs.inodes[child as usize] = Inode::EMPTY;
        }
        Ok(())
    })
}

pub fn remove_dir(path: &str) -> Result<(), Error> {
    with_fs(|fs| {
        if let Some((mount, _persistent_path)) =
            locate_persistent_path(fs, path, NamespaceId::ROOT)?
        {
            let backend = persistent_backend(mount)?;
            if mount_flags(mount)?.read_only || backend.read_only() {
                return Err(Error::ReadOnly);
            }
            return Err(Error::BackendUnsupported);
        }
        let (mount, parent, name) = parent_and_name_mount(fs, path, NamespaceId::ROOT)?;
        if mount_flags(mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        let child = find_child(fs, parent, name).ok_or(Error::NotFound)?;
        let inode = &fs.inodes[child as usize];
        if inode.kind != NodeType::Directory {
            return Err(Error::NotDirectory);
        }
        if find_mount_child(NamespaceId::ROOT, mount, child).is_some() {
            return Err(Error::Busy);
        }
        if inode.children.iter().any(|entry| entry.used) {
            return Err(Error::NotEmpty);
        }
        remove_child(fs, parent, name)?;
        fs.inodes[child as usize] = Inode::EMPTY;
        Ok(())
    })
}

pub fn rename(old_path: &str, new_path: &str) -> Result<(), Error> {
    with_fs(|fs| {
        let old_persistent = locate_persistent_path(fs, old_path, NamespaceId::ROOT)?;
        let new_persistent = locate_persistent_path(fs, new_path, NamespaceId::ROOT)?;
        if old_persistent.is_some() || new_persistent.is_some() {
            let Some((old_mount, old_path)) = old_persistent else {
                return Err(Error::InvalidPath);
            };
            let Some((new_mount, new_path)) = new_persistent else {
                return Err(Error::InvalidPath);
            };
            if old_mount != new_mount {
                return Err(Error::InvalidPath);
            }
            let backend = persistent_backend(old_mount)?;
            if mount_flags(old_mount)?.read_only || backend.read_only() {
                return Err(Error::ReadOnly);
            }
            return backend.rename(old_path.as_str()?, new_path.as_str()?);
        }
        let (old_mount, old_parent, old_name) =
            parent_and_name_mount(fs, old_path, NamespaceId::ROOT)?;
        let inode = find_child(fs, old_parent, old_name).ok_or(Error::NotFound)?;
        if find_mount_child(NamespaceId::ROOT, old_mount, inode).is_some() {
            return Err(Error::Busy);
        }
        let (new_mount, new_parent, new_name) =
            parent_and_name_mount(fs, new_path, NamespaceId::ROOT)?;
        if old_mount != new_mount {
            return Err(Error::InvalidPath);
        }
        if mount_flags(old_mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        if old_parent == new_parent && old_name == new_name {
            return Ok(());
        }
        if fs.inodes[inode as usize].kind == NodeType::Directory
            && is_descendant(fs, new_parent, inode)
        {
            return Err(Error::InvalidPath);
        }
        let old_slot = find_child_slot(fs, old_parent, old_name).ok_or(Error::NotFound)?;
        let new_slot = find_child_slot(fs, new_parent, new_name);
        let replaced = new_slot.map(|slot| fs.inodes[new_parent as usize].children[slot]);
        if let Some(child) = replaced {
            if find_mount_child(NamespaceId::ROOT, new_mount, child.inode).is_some() {
                return Err(Error::Busy);
            }
            if fs.inodes[child.inode as usize].kind == NodeType::Directory {
                return Err(Error::IsDirectory);
            }
            if has_open_handle(fs, child.inode) {
                return Err(Error::Busy);
            }
        }
        if old_parent == new_parent {
            fs.inodes[old_parent as usize].children[old_slot].name = new_name;
            if let Some(slot) = new_slot {
                let replaced = fs.inodes[new_parent as usize].children[slot];
                fs.inodes[new_parent as usize].children[slot] = Child::EMPTY;
                if replaced.inode != inode && link_count(fs, replaced.inode) == 0 {
                    fs.inodes[replaced.inode as usize] = Inode::EMPTY;
                }
            }
            return Ok(());
        }
        if new_slot.is_none() && !has_free_child(fs, new_parent) {
            return Err(Error::NoSpace);
        }
        let old_child = fs.inodes[old_parent as usize].children[old_slot];
        fs.inodes[old_parent as usize].children[old_slot] = Child::EMPTY;
        if let Some(slot) = new_slot {
            fs.inodes[new_parent as usize].children[slot] = Child::EMPTY;
        }
        let destination_slot = new_slot.or_else(|| {
            fs.inodes[new_parent as usize]
                .children
                .iter()
                .position(|child| !child.used)
        });
        let Some(destination_slot) = destination_slot else {
            fs.inodes[old_parent as usize].children[old_slot] = old_child;
            if let (Some(slot), Some(child)) = (new_slot, replaced) {
                fs.inodes[new_parent as usize].children[slot] = child;
            }
            return Err(Error::NoSpace);
        };
        fs.inodes[new_parent as usize].children[destination_slot] = Child {
            used: true,
            inode,
            name: new_name,
        };
        if let Some(child) = replaced {
            if child.inode != inode && link_count(fs, child.inode) == 0 {
                fs.inodes[child.inode as usize] = Inode::EMPTY;
            }
        }
        fs.inodes[inode as usize].parent = new_parent;
        Ok(())
    })
}

pub fn link(old_path: &str, new_path: &str) -> Result<(), Error> {
    with_fs(|fs| {
        if locate_persistent_path(fs, old_path, NamespaceId::ROOT)?.is_some()
            || locate_persistent_path(fs, new_path, NamespaceId::ROOT)?.is_some()
        {
            return Err(Error::ReadOnly);
        }
        let (old_mount, old_inode) = resolve_mount(fs, old_path, NamespaceId::ROOT)?;
        if fs.inodes[old_inode as usize].kind == NodeType::Directory {
            return Err(Error::IsDirectory);
        }
        let (new_mount, new_parent, new_name) =
            parent_and_name_mount(fs, new_path, NamespaceId::ROOT)?;
        if old_mount != new_mount {
            return Err(Error::InvalidPath);
        }
        if mount_flags(new_mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        if find_child(fs, new_parent, new_name).is_some() {
            return Err(Error::AlreadyExists);
        }
        add_child(fs, new_parent, new_name, old_inode)
    })
}

pub fn stat(path: &str) -> Result<FileStat, Error> {
    with_fs(|fs| {
        if let Some((mount, persistent_path)) = locate_persistent_path(fs, path, NamespaceId::ROOT)?
        {
            let path_str = persistent_path.as_str()?;
            let info = persistent_backend(mount)?.stat(path_str)?;
            return Ok(FileStat {
                inode: persistent_inode(path_str),
                kind: info.kind,
                mode: info.mode as u16,
                size: usize::try_from(info.size).map_err(|_| Error::OffsetOutOfRange)?,
                links: 1,
            });
        }
        let (_mount, inode) = resolve_mount(fs, path, NamespaceId::ROOT)?;
        let node = fs.inodes[inode as usize];
        Ok(FileStat {
            inode: inode as u32,
            kind: node.kind,
            mode: node.mode,
            size: node.size,
            links: link_count(fs, inode),
        })
    })
}

pub fn read_dir(path: &str, output: &mut [DirectoryEntry]) -> Result<usize, Error> {
    with_fs(|fs| {
        if let Some((mount, persistent_path)) = locate_persistent_path(fs, path, NamespaceId::ROOT)?
        {
            if output.len() > MAX_PERSISTENT_ENTRIES {
                return Err(Error::NoSpace);
            }
            let path_str = persistent_path.as_str()?;
            let backend = persistent_backend(mount)?;
            let info = backend.stat(path_str)?;
            if info.kind != NodeType::Directory {
                return Err(Error::NotDirectory);
            }
            let capacity = output.len().saturating_add(2).min(MAX_PERSISTENT_ENTRIES);
            let mut entries = [PersistentDirectoryEntry::EMPTY; MAX_PERSISTENT_ENTRIES];
            let backend_count = backend.read_dir(path_str, &mut entries[..capacity])?;
            let mut count = 0;
            for entry in entries.iter().take(backend_count) {
                if entry.name_length == 1 && entry.name[0] == b'.'
                    || entry.name_length == 2 && entry.name[0] == b'.' && entry.name[1] == b'.'
                {
                    continue;
                }
                if count == output.len() {
                    break;
                }
                output[count] = DirectoryEntry {
                    kind: entry.kind,
                    mode: entry.mode as u16,
                    size: usize::try_from(entry.size).map_err(|_| Error::OffsetOutOfRange)?,
                    links: 1,
                    name: entry.name,
                    name_length: entry.name_length,
                };
                count += 1;
            }
            return Ok(count);
        }
        let (_mount, inode) = resolve_mount(fs, path, NamespaceId::ROOT)?;
        if fs.inodes[inode as usize].kind != NodeType::Directory {
            return Err(Error::NotDirectory);
        }
        let mut count = 0;
        for child in fs.inodes[inode as usize]
            .children
            .iter()
            .filter(|child| child.used)
        {
            if count == output.len() {
                break;
            }
            let child_inode = fs.inodes[child.inode as usize];
            let mut name = [0; NAME_MAX];
            let name_length = child.name.len as usize;
            name[..name_length].copy_from_slice(&child.name.bytes[..name_length]);
            output[count] = DirectoryEntry {
                kind: child_inode.kind,
                mode: child_inode.mode,
                size: child_inode.size,
                links: link_count(fs, child.inode),
                name,
                name_length,
            };
            count += 1;
        }
        Ok(count)
    })
}

pub fn stats() -> Stats {
    let Ok(stats) = with_fs(|fs| {
        let mut files = 0;
        let mut directories = 0;
        let mut bytes = 0;
        for inode in fs.inodes.iter().filter(|inode| inode.used) {
            match inode.kind {
                NodeType::Directory => directories += 1,
                NodeType::Regular => {
                    files += 1;
                    bytes += inode.size;
                }
            }
        }
        Ok(Stats {
            mounts: mount_count(NamespaceId::ROOT),
            files,
            directories,
            bytes,
            open_handles: fs.open_count,
        })
    }) else {
        return Stats {
            mounts: 0,
            files: 0,
            directories: 0,
            bytes: 0,
            open_handles: 0,
        };
    };
    stats
}

pub fn list(mut f: impl FnMut(&str, usize)) {
    let _ = with_fs(|fs| {
        let mut path = [0u8; MAX_PATH];
        path[0] = b'/';
        list_dir(fs, 0, &mut path, 1, 0, &mut f);
        Ok(())
    });
}

fn link_count(fs: &FileSystem, inode: u16) -> u32 {
    fs.inodes
        .iter()
        .flat_map(|parent| parent.children.iter())
        .filter(|child| child.used && child.inode == inode)
        .count()
        .max(1) as u32
}

pub fn read(path: &str, output: &mut [u8; 255]) -> Option<usize> {
    let handle = open(path, OpenOptions::read()).ok()?;
    let result = read_handle(handle, output).ok();
    let _ = close(handle);
    result
}

fn readback(path: &str, expected: &[u8]) -> bool {
    let mut output = [0; 255];
    let Some(length) = read(path, &mut output) else {
        return false;
    };
    length == expected.len() && output[..length] == *expected
}

pub fn write(path: &str, input: &[u8]) -> bool {
    let Ok(handle) = open(path, OpenOptions::write_create()) else {
        return false;
    };
    let result = write_handle(handle, input).is_ok();
    let _ = close(handle);
    result
}

fn mount_tree_self_check() -> Result<(), Error> {
    mkdir("/mnt")?;
    if !write("/mnt/hello.txt", HELLO_TEXT) {
        return Err(Error::NoSpace);
    }
    let shared_mount = mount_with_propagation(
        NamespaceId::ROOT,
        MountSource::Ramfs,
        "/mnt",
        MountFlags::defaults(),
        Propagation::Shared,
    )?;
    let info = mount_info(shared_mount)?;
    if info.parent != Some(MountId::ROOT) || info.propagation != Propagation::Shared {
        return Err(Error::InvalidMountTarget);
    }
    if mount(MountSource::Ramfs, "/mnt", MountFlags::defaults()) != Err(Error::MountPointBusy) {
        return Err(Error::MountPointBusy);
    }
    let dentry = lookup("/mnt/hello.txt")?;
    if dentry.dentry().mount != shared_mount {
        return Err(Error::InvalidMountTarget);
    }
    if unmount_mount(shared_mount) != Err(Error::Busy) {
        return Err(Error::Busy);
    }
    release_dentry(dentry)?;
    let handle = open("/mnt/hello.txt", OpenOptions::read())?;
    if unmount_mount(shared_mount) != Err(Error::Busy) {
        return Err(Error::Busy);
    }
    close(handle)?;
    unmount_mount(shared_mount)?;

    let read_only = mount(MountSource::Ramfs, "/mnt", MountFlags::read_only())?;
    if remove_dir("/mnt") != Err(Error::Busy) || rename("/mnt", "/moved") != Err(Error::Busy) {
        let _ = unmount_mount(read_only);
        return Err(Error::Busy);
    }
    if !matches!(
        open("/mnt/hello.txt", OpenOptions::write_create()),
        Err(Error::ReadOnly)
    ) {
        let _ = unmount_mount(read_only);
        return Err(Error::ReadOnly);
    }
    unmount_mount(read_only)?;

    let namespace = create_namespace()?;
    let namespace_mount = mount_in_namespace(
        namespace,
        MountSource::Ramfs,
        "/mnt",
        MountFlags::read_only(),
        Propagation::Private,
    )?;
    let namespace_dentry = lookup_in_namespace(namespace, "/mnt/hello.txt")?;
    if namespace_dentry.dentry().mount != namespace_mount {
        let _ = release_dentry(namespace_dentry);
        let _ = unmount_mount(namespace_mount);
        let _ = destroy_namespace(namespace);
        return Err(Error::InvalidMountTarget);
    }
    let root_dentry = lookup("/mnt/hello.txt")?;
    if root_dentry.dentry().mount != MountId::ROOT {
        let _ = release_dentry(root_dentry);
        let _ = release_dentry(namespace_dentry);
        let _ = unmount_mount(namespace_mount);
        let _ = destroy_namespace(namespace);
        return Err(Error::InvalidMountTarget);
    }
    release_dentry(root_dentry)?;
    if destroy_namespace(namespace) != Err(Error::Busy) {
        let _ = release_dentry(namespace_dentry);
        let _ = unmount_mount(namespace_mount);
        let _ = destroy_namespace(namespace);
        return Err(Error::Busy);
    }
    release_dentry(namespace_dentry)?;
    if destroy_namespace(namespace) != Err(Error::Busy) {
        let _ = unmount_mount(namespace_mount);
        let _ = destroy_namespace(namespace);
        return Err(Error::Busy);
    }
    unmount_mount(namespace_mount)?;
    let dentry = lookup_in_namespace(namespace, "/hello.txt")?;
    release_dentry(dentry)?;
    destroy_namespace(namespace)?;
    unlink("/mnt/hello.txt")?;
    remove_dir("/mnt")?;
    Ok(())
}

fn mount_library_roots() -> Result<(), Error> {
    mkdir("/lib")?;
    if !write("/lib/libdep.so", b"Norx staged shared object\n") {
        return Err(Error::NoSpace);
    }
    let lib = mount(MountSource::Ramfs, "/lib", MountFlags::library())?;
    if mount_info(lib)?.flags != MountFlags::library() {
        return Err(Error::InvalidMountTarget);
    }
    let dentry = lookup("/lib")?;
    if dentry.dentry().mount != lib {
        let _ = release_dentry(dentry);
        return Err(Error::InvalidMountTarget);
    }
    release_dentry(dentry)?;
    Ok(())
}

fn mount_tests() -> Result<(), Error> {
    if mount_ramfs().is_ok() {
        return Err(Error::AlreadyMounted);
    }
    mkdir("/self-test")?;
    let handle = open("/self-test/file", OpenOptions::read_write_create())?;
    write_handle(handle, b"abcdef")?;
    if stat_handle(handle)?.size != 6 {
        return Err(Error::OffsetOutOfRange);
    }
    fchmod(handle, 0o600)?;
    if stat_handle(handle)?.mode != 0o600 {
        return Err(Error::PermissionDenied);
    }
    seek_from(handle, 0, 0)?;
    seek_from(handle, 0, 2)?;
    if !matches!(lookup("/self-test/file/."), Err(Error::NotDirectory))
        || !matches!(lookup("/self-test/file/.."), Err(Error::NotDirectory))
    {
        return Err(Error::NotDirectory);
    }
    let normalized = lookup("/self-test/../self-test/file")?;
    release_dentry(normalized)?;
    if !matches!(
        open("/self-test/missing", OpenOptions::read()),
        Err(Error::NotFound)
    ) || !matches!(
        open("/self-test", OpenOptions::read()),
        Err(Error::IsDirectory)
    ) {
        return Err(Error::NotFound);
    }
    let mut entries = [DirectoryEntry {
        kind: NodeType::Regular,
        mode: 0,
        size: 0,
        links: 0,
        name: [0; NAME_MAX],
        name_length: 0,
    }; MAX_CHILDREN];
    let entry_count = read_dir("/self-test", &mut entries)?;
    if !entries[..entry_count]
        .iter()
        .any(|entry| entry.name_length == 4 && entry.name[..4] == *b"file")
    {
        return Err(Error::NotFound);
    }
    if remove_dir("/self-test") != Err(Error::NotEmpty) {
        return Err(Error::NotEmpty);
    }
    seek(handle, -4)?;
    let mut output = [0u8; 3];
    if read_handle(handle, &mut output)? != 3 || output != *b"cde" {
        return Err(Error::OffsetOutOfRange);
    }
    let duplicate = duplicate(handle)?;
    let mut tail = [0u8; 1];
    if read_handle(duplicate, &mut tail)? != 1 || tail != *b"f" {
        return Err(Error::OffsetOutOfRange);
    }
    let mut eof = [0u8; 1];
    if read_handle(handle, &mut eof)? != 0 {
        return Err(Error::OffsetOutOfRange);
    }
    close(duplicate)?;
    let stale = handle.raw();
    close(handle)?;
    let reopened = open("/self-test/file", OpenOptions::read())?;
    if close_raw(stale).is_ok() {
        let _ = close(reopened);
        return Err(Error::InvalidHandle);
    }
    close(reopened)?;
    let truncated = open("/self-test/file", OpenOptions::write_truncate())?;
    write_handle(truncated, b"xy")?;
    close(truncated)?;
    let appended = open("/self-test/file", OpenOptions::write_append())?;
    write_handle(appended, b"z")?;
    close(appended)?;
    let combined = open("/self-test/file", OpenOptions::read())?;
    let mut combined_output = [0u8; 3];
    if read_handle(combined, &mut combined_output)? != 3 || combined_output != *b"xyz" {
        return Err(Error::OffsetOutOfRange);
    }
    close(combined)?;
    chmod("/self-test/file", 0o400)?;
    if !matches!(
        open("/self-test/file", OpenOptions::write_create()),
        Err(Error::PermissionDenied)
    ) {
        return Err(Error::PermissionDenied);
    }
    let invalid_truncate = OpenOptions {
        truncate: true,
        ..OpenOptions::read()
    };
    if open("/self-test/file", invalid_truncate) != Err(Error::PermissionDenied) {
        return Err(Error::PermissionDenied);
    }
    chmod("/self-test/file", 0o644)?;
    link("/self-test/file", "/self-test/alias")?;
    if stat("/self-test/file")?.links != 2 || stat("/self-test/alias")?.links != 2 {
        return Err(Error::InvalidPath);
    }
    unlink("/self-test/file")?;
    if stat("/self-test/alias")?.links != 1 || !readback("/self-test/alias", b"xyz") {
        return Err(Error::InvalidPath);
    }
    rename("/self-test/alias", "/self-test/renamed")?;
    let replacement = open("/self-test/replacement", OpenOptions::read_write_create())?;
    write_handle(replacement, b"old")?;
    close(replacement)?;
    let incoming = open("/self-test/incoming", OpenOptions::read_write_create())?;
    write_handle(incoming, b"new")?;
    close(incoming)?;
    rename("/self-test/incoming", "/self-test/replacement")?;
    let replaced = open("/self-test/replacement", OpenOptions::read())?;
    let mut replacement_output = [0; 3];
    if read_handle(replaced, &mut replacement_output)? != 3 || replacement_output != *b"new" {
        return Err(Error::OffsetOutOfRange);
    }
    close(replaced)?;
    if open("/self-test/incoming", OpenOptions::read()).is_ok() {
        return Err(Error::AlreadyExists);
    }
    let open_handle = open("/self-test/renamed", OpenOptions::read())?;
    if unmount_ramfs() != Err(Error::Busy) {
        return Err(Error::Busy);
    }
    close(open_handle)?;
    let stale_dentry = lookup("/self-test")?;
    release_dentry(stale_dentry)?;
    let live_dentry = lookup("/self-test")?;
    if release_dentry(stale_dentry).is_ok() {
        let _ = release_dentry(live_dentry);
        return Err(Error::InvalidHandle);
    }
    release_dentry(live_dentry)?;
    unlink("/self-test/renamed")?;
    unlink("/self-test/replacement")?;
    remove_dir("/self-test")?;
    unmount_ramfs()?;
    mount_ramfs()
}

fn with_fs<R>(f: impl FnOnce(&mut FileSystem) -> Result<R, Error>) -> Result<R, Error> {
    unsafe {
        if !MOUNTED {
            return Err(Error::NotMounted);
        }
        let fs = &mut *core::ptr::addr_of_mut!(FILE_SYSTEM);
        f(fs)
    }
}

fn persistent_backend(id: MountId) -> Result<PersistentMount, Error> {
    mount_node(id)?
        .backend
        .persistent()
        .ok_or(Error::BackendUnsupported)
}

fn validate_persistent_path(path: &str) -> Result<(), Error> {
    let (components, count) = parse_path(path)?;
    if components
        .iter()
        .take(count)
        .any(|component| component.is_special())
    {
        return Err(Error::InvalidPath);
    }
    Ok(())
}

fn persistent_inode(path: &str) -> u32 {
    let mut hash = 2_166_136_261u32;
    for byte in path.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

fn map_block_error(error: crate::drivers::block::Error) -> Error {
    match error {
        crate::drivers::block::Error::ReadOnly => Error::ReadOnly,
        crate::drivers::block::Error::NotReady
        | crate::drivers::block::Error::InvalidRequest
        | crate::drivers::block::Error::OutOfRange
        | crate::drivers::block::Error::Busy
        | crate::drivers::block::Error::Unsupported => Error::BackendUnsupported,
        crate::drivers::block::Error::Timeout => Error::BackendError,
        #[cfg(target_arch = "x86_64")]
        crate::drivers::block::Error::Device => Error::BackendError,
    }
}

fn map_fat32_error(error: crate::fat32::Error) -> Error {
    match error {
        crate::fat32::Error::InvalidPath => Error::InvalidPath,
        crate::fat32::Error::NotFound => Error::NotFound,
        crate::fat32::Error::AlreadyExists => Error::AlreadyExists,
        crate::fat32::Error::NotDirectory => Error::NotDirectory,
        crate::fat32::Error::IsDirectory => Error::IsDirectory,
        crate::fat32::Error::BufferTooSmall => Error::BackendError,
        crate::fat32::Error::ReadOnly => Error::ReadOnly,
        crate::fat32::Error::Io
        | crate::fat32::Error::InvalidBpb
        | crate::fat32::Error::BadClusterChain
        | crate::fat32::Error::InvalidPersistenceRecord => Error::BackendError,
        crate::fat32::Error::Unsupported => Error::BackendUnsupported,
        crate::fat32::Error::NoSpace => Error::NoSpace,
        _ => Error::BackendError,
    }
}

fn map_ext4_error(error: crate::ext4::Error) -> Error {
    match error {
        crate::ext4::Error::InvalidPath => Error::InvalidPath,
        crate::ext4::Error::NotFound => Error::NotFound,
        crate::ext4::Error::NotDirectory => Error::NotDirectory,
        crate::ext4::Error::IsDirectory => Error::IsDirectory,
        crate::ext4::Error::BufferTooSmall => Error::BackendError,
        crate::ext4::Error::PermissionDenied => Error::PermissionDenied,
        crate::ext4::Error::ReadOnly => Error::ReadOnly,
        crate::ext4::Error::UnsupportedFeature => Error::BackendUnsupported,
        crate::ext4::Error::Io
        | crate::ext4::Error::InvalidSuperblock
        | crate::ext4::Error::JournalRecoveryRequired
        | crate::ext4::Error::BadExtent
        | crate::ext4::Error::BadDirectory => Error::BackendError,
    }
}

fn map_btrfs_error(error: crate::btrfs::Error) -> Error {
    match error {
        crate::btrfs::Error::InvalidPath => Error::InvalidPath,
        crate::btrfs::Error::NotFound => Error::NotFound,
        crate::btrfs::Error::NotDirectory => Error::NotDirectory,
        crate::btrfs::Error::IsDirectory => Error::IsDirectory,
        crate::btrfs::Error::BufferTooSmall => Error::BackendError,
        crate::btrfs::Error::PermissionDenied => Error::PermissionDenied,
        crate::btrfs::Error::ReadOnly => Error::ReadOnly,
        crate::btrfs::Error::UnsupportedChecksum | crate::btrfs::Error::UnsupportedFeature => {
            Error::BackendUnsupported
        }
        crate::btrfs::Error::Io
        | crate::btrfs::Error::InvalidSuperblock
        | crate::btrfs::Error::ChecksumMismatch
        | crate::btrfs::Error::UnmappedLogical
        | crate::btrfs::Error::TreeCorrupt => Error::BackendError,
    }
}

fn mount_node(id: MountId) -> Result<MountNode, Error> {
    unsafe {
        (&*core::ptr::addr_of!(MOUNTS))
            .get(id.index())
            .copied()
            .filter(|node| node.used)
            .ok_or(Error::MountNotFound)
    }
}

fn mount_flags(id: MountId) -> Result<MountFlags, Error> {
    Ok(mount_node(id)?.flags)
}

fn mount_count(namespace: NamespaceId) -> usize {
    unsafe {
        (&*core::ptr::addr_of!(MOUNTS))
            .iter()
            .filter(|node| node.used && node.namespace == namespace)
            .count()
    }
}

fn find_mount_child(namespace: NamespaceId, parent: MountId, mountpoint: u16) -> Option<MountId> {
    unsafe {
        (&*core::ptr::addr_of!(MOUNTS))
            .iter()
            .enumerate()
            .find(|(_, node)| {
                node.used
                    && node.namespace == namespace
                    && node.parent == Some(parent)
                    && node.mountpoint_inode == mountpoint
            })
            .map(|(index, _)| MountId(index as u8))
    }
}

fn resolve_mount(
    fs: &FileSystem,
    path: &str,
    namespace: NamespaceId,
) -> Result<(MountId, u16), Error> {
    let (components, count) = parse_path(path)?;
    let mut mount = namespace_root(namespace)?;
    let mut inode = 0;
    for component in components.iter().take(count) {
        walk_component(fs, namespace, &mut mount, &mut inode, *component)?;
    }
    Ok((mount, inode))
}

fn locate_persistent_path(
    fs: &FileSystem,
    path: &str,
    namespace: NamespaceId,
) -> Result<Option<(MountId, PersistentPath)>, Error> {
    let (components, count) = parse_path(path)?;
    let mut mount = namespace_root(namespace)?;
    let mut inode = 0;
    for (index, component) in components.iter().take(count).enumerate() {
        if component.is_special() {
            walk_component(fs, namespace, &mut mount, &mut inode, *component)?;
            continue;
        }
        if fs.inodes[inode as usize].kind != NodeType::Directory {
            return Err(Error::NotDirectory);
        }
        let Some(next_inode) = find_child(fs, inode, *component) else {
            // A path that never reaches a persistent mount belongs to the
            // ordinary resolver, which may still create its final component.
            return Ok(None);
        };
        inode = next_inode;
        if let Some(child_mount) = find_mount_child(namespace, mount, inode) {
            let child = mount_node(child_mount)?;
            if child.backend.persistent().is_some() {
                if components.iter().take(count).any(|part| part.is_special()) {
                    return Err(Error::InvalidPath);
                }
                let mut relative = PersistentPath::ROOT;
                for part in components.iter().skip(index + 1).take(count - index - 1) {
                    relative.push(*part)?;
                }
                return Ok(Some((child_mount, relative)));
            }
            mount = child_mount;
            inode = child.root_inode;
        }
    }
    Ok(None)
}

fn parent_and_name_mount(
    fs: &FileSystem,
    path: &str,
    namespace: NamespaceId,
) -> Result<(MountId, u16, Name), Error> {
    let (components, count) = parse_path(path)?;
    if count == 0 {
        return Err(Error::InvalidPath);
    }
    let mut mount = namespace_root(namespace)?;
    let mut parent = 0;
    for component in components.iter().take(count - 1) {
        walk_component(fs, namespace, &mut mount, &mut parent, *component)?;
    }
    if fs.inodes[parent as usize].kind != NodeType::Directory {
        return Err(Error::NotDirectory);
    }
    let name = components[count - 1];
    if name.is_special() {
        return Err(Error::InvalidPath);
    }
    Ok((mount, parent, name))
}

fn walk_component(
    fs: &FileSystem,
    namespace: NamespaceId,
    mount: &mut MountId,
    inode: &mut u16,
    component: Name,
) -> Result<(), Error> {
    if fs.inodes[*inode as usize].kind != NodeType::Directory {
        return Err(Error::NotDirectory);
    }
    if component.is_dot() {
        return Ok(());
    }
    if component.is_dotdot() {
        let node = mount_node(*mount)?;
        if let Some(parent_mount) = node.parent {
            if *inode == node.root_inode {
                *mount = parent_mount;
                *inode = fs.inodes[node.mountpoint_inode as usize].parent;
                return Ok(());
            }
        }
        *inode = fs.inodes[*inode as usize].parent;
        return Ok(());
    }
    *inode = find_child(fs, *inode, component).ok_or(Error::NotFound)?;
    if let Some(child_mount) = find_mount_child(namespace, *mount, *inode) {
        // Do not reinterpret a persistent mount as RAMFS: dispatch must be added before
        // generic namespace operations can traverse its paths.
        if mount_node(child_mount)?.backend.persistent().is_some() {
            return Err(Error::BackendUnsupported);
        }
        *mount = child_mount;
        *inode = mount_node(child_mount)?.root_inode;
    }
    Ok(())
}

fn create_node_at(
    fs: &mut FileSystem,
    parent: u16,
    name: Name,
    kind: NodeType,
    mode: u16,
) -> Result<u16, Error> {
    if find_child(fs, parent, name).is_some() {
        return Err(Error::AlreadyExists);
    }
    let parent_inode = &fs.inodes[parent as usize];
    if parent_inode.kind != NodeType::Directory {
        return Err(Error::NotDirectory);
    }
    if !has_free_child(fs, parent) {
        return Err(Error::NoSpace);
    }
    let inode = fs
        .inodes
        .iter()
        .position(|inode| !inode.used)
        .ok_or(Error::NoSpace)? as u16;
    fs.inodes[inode as usize] = Inode::new(kind, parent, mode);
    add_child(fs, parent, name, inode)?;
    Ok(inode)
}

fn parse_path(path: &str) -> Result<([Name; MAX_COMPONENTS], usize), Error> {
    let bytes = path.as_bytes();
    if bytes.first().copied() != Some(b'/') || bytes.len() > MAX_PATH {
        return Err(Error::InvalidPath);
    }
    let mut components = [Name::EMPTY; MAX_COMPONENTS];
    let mut count = 0usize;
    let mut position = 1;
    while position < bytes.len() {
        while position < bytes.len() && bytes[position] == b'/' {
            position += 1;
        }
        if position == bytes.len() {
            break;
        }
        let start = position;
        while position < bytes.len() && bytes[position] != b'/' {
            if bytes[position] == 0 {
                return Err(Error::InvalidPath);
            }
            position += 1;
        }
        let part = &bytes[start..position];
        if part.len() > NAME_MAX {
            return Err(Error::NameTooLong);
        }
        if count == MAX_COMPONENTS {
            return Err(Error::InvalidPath);
        }
        let mut name = Name::EMPTY;
        name.bytes[..part.len()].copy_from_slice(part);
        name.len = part.len() as u8;
        components[count] = name;
        count += 1;
    }
    Ok((components, count))
}

fn find_child(fs: &FileSystem, parent: u16, name: Name) -> Option<u16> {
    fs.inodes[parent as usize]
        .children
        .iter()
        .find(|child| child.used && child.name == name)
        .map(|child| child.inode)
}

fn find_child_slot(fs: &FileSystem, parent: u16, name: Name) -> Option<usize> {
    fs.inodes[parent as usize]
        .children
        .iter()
        .position(|child| child.used && child.name == name)
}

fn has_free_child(fs: &FileSystem, parent: u16) -> bool {
    fs.inodes[parent as usize]
        .children
        .iter()
        .any(|child| !child.used)
}

fn add_child(fs: &mut FileSystem, parent: u16, name: Name, inode: u16) -> Result<(), Error> {
    let child = fs.inodes[parent as usize]
        .children
        .iter_mut()
        .find(|child| !child.used)
        .ok_or(Error::NoSpace)?;
    *child = Child {
        used: true,
        inode,
        name,
    };
    Ok(())
}

fn remove_child(fs: &mut FileSystem, parent: u16, name: Name) -> Result<u16, Error> {
    let child = fs.inodes[parent as usize]
        .children
        .iter_mut()
        .find(|child| child.used && child.name == name)
        .ok_or(Error::NotFound)?;
    let inode = child.inode;
    *child = Child::EMPTY;
    Ok(inode)
}

fn has_open_handle(fs: &FileSystem, inode: u16) -> bool {
    fs.handles
        .iter()
        .any(|handle| handle.used && !handle.persistent && handle.inode == inode)
}

fn validate_handle(fs: &FileSystem, handle: &FileHandle) -> Result<usize, Error> {
    let slot = handle.slot as usize;
    if slot >= MAX_OPEN_HANDLES || !fs.handles[slot].used {
        return Err(Error::InvalidHandle);
    }
    let description = fs.handles[slot];
    if description.generation != handle.generation
        || (!description.persistent && !fs.inodes[description.inode as usize].used)
        || mount_node(description.mount).is_err()
    {
        return Err(Error::InvalidHandle);
    }
    Ok(slot)
}

fn is_descendant(fs: &FileSystem, candidate: u16, ancestor: u16) -> bool {
    let mut current = candidate;
    for _ in 0..MAX_INODES {
        if current == ancestor {
            return true;
        }
        if current == 0 {
            return false;
        }
        current = fs.inodes[current as usize].parent;
    }
    true
}

fn list_dir(
    fs: &FileSystem,
    inode: u16,
    path: &mut [u8; MAX_PATH],
    path_len: usize,
    depth: usize,
    f: &mut impl FnMut(&str, usize),
) {
    if depth == MAX_COMPONENTS {
        return;
    }
    let children = fs.inodes[inode as usize].children;
    for child in children.iter().filter(|child| child.used) {
        let mut end = path_len;
        if end > 1 {
            if end >= MAX_PATH {
                continue;
            }
            path[end] = b'/';
            end += 1;
        }
        let name_len = child.name.len as usize;
        if end + name_len >= MAX_PATH {
            continue;
        }
        path[end..end + name_len].copy_from_slice(&child.name.bytes[..name_len]);
        end += name_len;
        let path_str = core::str::from_utf8(&path[..end]).unwrap_or("/");
        let node = fs.inodes[child.inode as usize];
        f(path_str, node.size);
        if node.kind == NodeType::Directory {
            list_dir(fs, child.inode, path, end, depth + 1, f);
        }
    }
}
