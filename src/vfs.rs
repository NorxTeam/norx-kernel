use core::ptr;

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

    const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountSource {
    Ramfs,
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
}

#[derive(Debug)]
pub struct DentryHandle {
    dentry: Dentry,
    slot: u8,
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
        generation: 0,
        references: 0,
        offset: 0,
        readable: false,
        writable: false,
        append: false,
    };
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
}

impl DentrySlot {
    const EMPTY: Self = Self {
        used: false,
        dentry: Dentry {
            mount: MountId::ROOT,
            inode: 0,
        },
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
    let options = OpenOptions::read();
    assert!(options.read);
    assert!(!options.write);
    assert!(!options.create);
    let (_, count) = parse_path("/a/../b").expect("valid ramfs path");
    assert_eq!(count, 1);
}

pub fn init() -> bool {
    crate::bootlog::start(1, "mounting ramfs");
    unsafe { MOUNTED = false };
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
        if fs.open_count != 0 {
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
        if source != MountSource::Ramfs {
            return Err(Error::InvalidMountTarget);
        }
        let (parent, mountpoint) = resolve_mount(fs, target, namespace)?;
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
        if mounts
            .iter()
            .any(|node| node.used && node.namespace == namespace && node.dentry_references != 0)
            || fs.handles.iter().any(|handle| {
                handle.used
                    && mounts
                        .get(handle.mount.index())
                        .is_some_and(|node| node.used && node.namespace == namespace)
            })
        {
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
        let (mount, inode) = resolve_mount(fs, path, namespace)?;
        let dentries = unsafe { &mut *core::ptr::addr_of_mut!(DENTRIES) };
        let Some((slot, entry)) = dentries
            .iter_mut()
            .enumerate()
            .find(|(_, entry)| !entry.used)
        else {
            return Err(Error::NoSpace);
        };
        entry.used = true;
        entry.dentry = Dentry { mount, inode };
        unsafe {
            (&mut *core::ptr::addr_of_mut!(MOUNTS))[mount.index()].dentry_references += 1;
        }
        Ok(DentryHandle {
            dentry: entry.dentry,
            slot: slot as u8,
        })
    })
}

pub fn release_dentry(handle: DentryHandle) -> Result<(), Error> {
    with_fs(|_| {
        let dentries = unsafe { &mut *core::ptr::addr_of_mut!(DENTRIES) };
        let slot = handle.slot as usize;
        if slot >= MAX_DENTRIES || !dentries[slot].used || dentries[slot].dentry != handle.dentry {
            return Err(Error::InvalidHandle);
        }
        let mount = handle.dentry.mount;
        dentries[slot] = DentrySlot::EMPTY;
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
        let (mount, inode) = match resolve_mount(fs, path, NamespaceId::ROOT) {
            Ok(result) => result,
            Err(Error::NotFound) if options.create => {
                let (mount, parent, name) = parent_and_name_mount(fs, path, NamespaceId::ROOT)?;
                if mount_flags(mount)?.read_only {
                    return Err(Error::ReadOnly);
                }
                (
                    mount,
                    create_node_at(fs, parent, name, NodeType::Regular, options.mode & 0o777)?,
                )
            }
            Err(error) => return Err(error),
        };
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
        if options.truncate {
            fs.inodes[inode as usize].size = 0;
        }
        let Some((slot, handle)) = fs
            .handles
            .iter_mut()
            .enumerate()
            .find(|(_, handle)| !handle.used)
        else {
            return Err(Error::NoSpace);
        };
        handle.used = true;
        handle.mount = mount;
        handle.inode = inode;
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
            fs.handles[slot] = HandleSlot::EMPTY;
            fs.open_count = fs.open_count.saturating_sub(1);
        }
        Ok(())
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
    with_fs(|fs| {
        let slot = validate_handle(fs, &handle)?;
        let current_offset = fs.handles[slot].offset;
        let next = if offset.is_negative() {
            current_offset
                .checked_sub(offset.unsigned_abs())
                .ok_or(Error::OffsetOutOfRange)?
        } else {
            current_offset
                .checked_add(offset as usize)
                .ok_or(Error::OffsetOutOfRange)?
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

pub fn chmod(path: &str, mode: u16) -> Result<(), Error> {
    with_fs(|fs| {
        let (mount, inode) = resolve_mount(fs, path, NamespaceId::ROOT)?;
        if mount_flags(mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        fs.inodes[inode as usize].mode = mode & 0o777;
        Ok(())
    })
}

pub fn mkdir(path: &str) -> Result<(), Error> {
    mkdir_with_mode(path, 0o755)
}

pub fn mkdir_with_mode(path: &str, mode: u16) -> Result<(), Error> {
    with_fs(|fs| {
        let (mount, parent, name) = parent_and_name_mount(fs, path, NamespaceId::ROOT)?;
        if mount_flags(mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        create_node_at(fs, parent, name, NodeType::Directory, mode & 0o777).map(|_| ())
    })
}

pub fn unlink(path: &str) -> Result<(), Error> {
    with_fs(|fs| {
        let (mount, parent, name) = parent_and_name_mount(fs, path, NamespaceId::ROOT)?;
        if mount_flags(mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        let child = find_child(fs, parent, name).ok_or(Error::NotFound)?;
        if fs.inodes[child as usize].kind == NodeType::Directory {
            return Err(Error::IsDirectory);
        }
        if has_open_handle(fs, child) {
            return Err(Error::Busy);
        }
        remove_child(fs, parent, name)?;
        fs.inodes[child as usize] = Inode::EMPTY;
        Ok(())
    })
}

pub fn remove_dir(path: &str) -> Result<(), Error> {
    with_fs(|fs| {
        let (mount, parent, name) = parent_and_name_mount(fs, path, NamespaceId::ROOT)?;
        if mount_flags(mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        let child = find_child(fs, parent, name).ok_or(Error::NotFound)?;
        let inode = &fs.inodes[child as usize];
        if inode.kind != NodeType::Directory {
            return Err(Error::NotDirectory);
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
        let (old_mount, old_parent, old_name) =
            parent_and_name_mount(fs, old_path, NamespaceId::ROOT)?;
        let inode = find_child(fs, old_parent, old_name).ok_or(Error::NotFound)?;
        let (new_mount, new_parent, new_name) =
            parent_and_name_mount(fs, new_path, NamespaceId::ROOT)?;
        if old_mount != new_mount {
            return Err(Error::InvalidPath);
        }
        if mount_flags(old_mount)?.read_only {
            return Err(Error::ReadOnly);
        }
        if find_child(fs, new_parent, new_name).is_some() {
            return Err(Error::AlreadyExists);
        }
        if fs.inodes[inode as usize].kind == NodeType::Directory
            && is_descendant(fs, new_parent, inode)
        {
            return Err(Error::InvalidPath);
        }
        if old_parent == new_parent {
            let slot = find_child_slot(fs, old_parent, old_name).ok_or(Error::NotFound)?;
            fs.inodes[old_parent as usize].children[slot].name = new_name;
            return Ok(());
        }
        if !has_free_child(fs, new_parent) {
            return Err(Error::NoSpace);
        }
        remove_child(fs, old_parent, old_name)?;
        if add_child(fs, new_parent, new_name, inode).is_err() {
            let _ = add_child(fs, old_parent, old_name, inode);
            return Err(Error::NoSpace);
        }
        fs.inodes[inode as usize].parent = new_parent;
        Ok(())
    })
}

pub fn link(old_path: &str, new_path: &str) -> Result<(), Error> {
    with_fs(|fs| {
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
    if !matches!(
        open("/mnt/hello.txt", OpenOptions::write_create()),
        Err(Error::ReadOnly)
    ) {
        let _ = unmount_mount(read_only);
        return Err(Error::ReadOnly);
    }
    unmount_mount(read_only)?;

    let namespace = create_namespace()?;
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
    close(handle)?;
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
    chmod("/self-test/file", 0o644)?;
    rename("/self-test/file", "/self-test/renamed")?;
    let open_handle = open("/self-test/renamed", OpenOptions::read())?;
    if unmount_ramfs() != Err(Error::Busy) {
        return Err(Error::Busy);
    }
    close(open_handle)?;
    unlink("/self-test/renamed")?;
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
        if fs.inodes[inode as usize].kind != NodeType::Directory {
            return Err(Error::NotDirectory);
        }
        inode = find_child(fs, inode, *component).ok_or(Error::NotFound)?;
        if let Some(child_mount) = find_mount_child(namespace, mount, inode) {
            mount = child_mount;
            inode = mount_node(child_mount)?.root_inode;
        }
    }
    Ok((mount, inode))
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
        if fs.inodes[parent as usize].kind != NodeType::Directory {
            return Err(Error::NotDirectory);
        }
        parent = find_child(fs, parent, *component).ok_or(Error::NotFound)?;
        if let Some(child_mount) = find_mount_child(namespace, mount, parent) {
            mount = child_mount;
            parent = mount_node(child_mount)?.root_inode;
        }
    }
    if fs.inodes[parent as usize].kind != NodeType::Directory {
        return Err(Error::NotDirectory);
    }
    Ok((mount, parent, components[count - 1]))
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
        if part == b"." {
            continue;
        }
        if part == b".." {
            count = count.saturating_sub(1);
            continue;
        }
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
        .any(|handle| handle.used && handle.inode == inode)
}

fn validate_handle(fs: &FileSystem, handle: &FileHandle) -> Result<usize, Error> {
    let slot = handle.slot as usize;
    if slot >= MAX_OPEN_HANDLES || !fs.handles[slot].used {
        return Err(Error::InvalidHandle);
    }
    let description = fs.handles[slot];
    if description.generation != handle.generation
        || !fs.inodes[description.inode as usize].used
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
