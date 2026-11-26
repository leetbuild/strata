//! The FUSE filesystem over a [`Tree`]: what the game and its tools see.
//!
//! Lookups are case-insensitive and case-preserving. Reads go to the winning layer's file,
//! writes follow the tree's rules (`tree.rs`). Inode numbers are node ids plus one and stay
//! valid for the life of the mount.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{
    AccessFlags, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation, INodeNo, InitFlags,
    IoctlFlags, KernelConfig, LockOwner, OpenAccMode, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyIoctl, ReplyOpen, ReplyStatfs, ReplyWrite, Request, TimeOrNow,
    WriteFlags,
};

use crate::tree::{NodeId, ROOT, Tree};

/// How long the kernel may trust a name or the attributes of a file that can change behind
/// the mount (a file in overwrite, or a mod's Root file that is edited in place).
const TTL: Duration = Duration::from_secs(1);

/// The same for folders and the files of the game and the mods: they change only through
/// the mount (which tells the kernel) or a refresh (which tells it too), so the kernel may
/// keep them for the whole run. Fluorine keeps everything 24 hours (`ttlSeconds`).
const TTL_STABLE: Duration = Duration::from_secs(24 * 60 * 60);

/// How long the kernel may keep what it learnt about node `id`.
fn ttl_of(tree: &Tree, id: NodeId) -> Duration {
    match tree.node(id).map(|n| &n.kind) {
        Some(crate::tree::Kind::File(src)) if src.write_back => TTL,
        Some(crate::tree::Kind::File(_)) if tree.layer_of(id).is_some_and(|l| l.kind.is_writable()) => TTL,
        _ => TTL_STABLE,
    }
}

/// How long the kernel may trust a *name*: a folder's for the whole run, a file's not at all.
///
/// Names compare without case, so one file can be known to the kernel under several
/// spellings (`b.dds`, `B.DDS`), each its own cache entry. An unlink or a rename through one
/// spelling moves or drops only that entry; any other one would keep pointing at the file
/// (renamed away, or deleted) until told otherwise, and that can only be told after the
/// reply (the kernel holds the folder's lock until then). In that window an `open(O_CREAT)`
/// or a `stat` through the old spelling reached the wrong file: a create wrote into the
/// renamed file (the "alias race"). With no lifetime the kernel
/// asks again (`LOOKUP`) every time it walks to a file, so there is no window. Folders keep
/// a lifetime: the kernel keeps one name per folder and renames that one when another
/// spelling is looked up, so a folder never has a second, stale name. A file's attributes
/// keep their own lifetime (`ttl_of`).
fn entry_ttl(tree: &Tree, id: NodeId) -> Duration {
    match tree.node(id) {
        Some(node) if node.is_dir() => TTL_STABLE,
        _ => Duration::ZERO,
    }
}

/// `FS_IOC_GETFLAGS`: Wine asks it of a folder after an exact-case lookup failed.
const FS_IOC_GETFLAGS: u32 = 0x8008_6601;
/// `FS_CASEFOLD_FL`: "this folder is case-insensitive", so Wine skips its own slow search.
const FS_CASEFOLD_FL: u32 = 0x4000_0000;

enum Handle {
    File { file: Arc<File>, node: NodeId },
    Dir(Arc<Vec<(u64, FileType, String)>>),
}

/// The filesystem. Shared with the daemon, which answers `status` and `resolve` from it.
pub struct StrataFs {
    tree: Arc<RwLock<Tree>>,
    handles: Mutex<HashMap<u64, Handle>>,
    next_handle: AtomicU64,
    mounted_at: SystemTime,
    uid: u32,
    gid: u32,
    /// Keeps the folder under the mountpoint reachable when the mount is in place.
    _under: Option<File>,
}

/// The inode number the kernel knows a node by.
pub(crate) fn ino(id: NodeId) -> INodeNo {
    INodeNo(u64::from(id) + 1)
}

fn node_id(ino: INodeNo) -> NodeId {
    (ino.0.saturating_sub(1)) as NodeId
}

fn time(secs: i64, nanos: i64) -> SystemTime {
    if secs >= 0 {
        UNIX_EPOCH + Duration::new(secs as u64, nanos as u32)
    } else {
        UNIX_EPOCH
    }
}

impl StrataFs {
    /// `under` is the handle on the folder below an in-place mount, kept open for its life.
    pub fn new(tree: Arc<RwLock<Tree>>, under: Option<File>) -> Self {
        // SAFETY: getuid and getgid cannot fail and touch no memory.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        Self {
            tree,
            handles: Mutex::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
            mounted_at: SystemTime::now(),
            uid,
            gid,
            _under: under,
        }
    }

    /// The node, or `ESTALE` when the kernel holds an id that went away (a refresh, or a
    /// name it cached that now means another node): the kernel then looks the path up again.
    fn live(tree: &Tree, id: NodeId) -> Result<&crate::tree::Node, Errno> {
        tree.node(id).ok_or(Errno::ESTALE)
    }

    fn dir_attr(&self, id: NodeId) -> FileAttr {
        FileAttr {
            ino: ino(id),
            size: 0,
            blocks: 0,
            atime: self.mounted_at,
            mtime: self.mounted_at,
            ctime: self.mounted_at,
            crtime: self.mounted_at,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    fn file_attr(&self, id: NodeId, meta: &std::fs::Metadata) -> FileAttr {
        FileAttr {
            ino: ino(id),
            size: meta.len(),
            blocks: meta.blocks(),
            atime: time(meta.atime(), meta.atime_nsec()),
            mtime: time(meta.mtime(), meta.mtime_nsec()),
            ctime: time(meta.ctime(), meta.ctime_nsec()),
            crtime: time(meta.mtime(), meta.mtime_nsec()),
            kind: FileType::RegularFile,
            perm: (meta.mode() & 0o7777) as u16,
            nlink: 1,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    /// The attributes of a node, from the real file for files.
    fn attr(&self, tree: &Tree, id: NodeId) -> io::Result<FileAttr> {
        let node = tree.node(id).ok_or_else(|| io::Error::from_raw_os_error(libc::ESTALE))?;
        if node.is_dir() {
            return Ok(self.dir_attr(id));
        }
        let real = tree.real_path(id).ok_or_else(|| io::Error::from_raw_os_error(libc::ENOENT))?;
        Ok(self.file_attr(id, &std::fs::metadata(real)?))
    }

    fn add_handle(&self, handle: Handle) -> FileHandle {
        let fh = self.next_handle.fetch_add(1, Ordering::Relaxed);
        self.handles.lock().expect("handle table").insert(fh, handle);
        FileHandle(fh)
    }

    /// Some file held open on node `id`.
    fn open_file_of(&self, id: NodeId) -> Option<Arc<File>> {
        self.handles.lock().expect("handle table").values().find_map(|h| match h {
            Handle::File { file, node } if *node == id => Some(Arc::clone(file)),
            _ => None,
        })
    }

    fn file(&self, fh: FileHandle) -> Option<(Arc<File>, NodeId)> {
        match self.handles.lock().expect("handle table").get(&fh.0) {
            Some(Handle::File { file, node }) => Some((Arc::clone(file), *node)),
            _ => None,
        }
    }

    /// Opens the real file behind `id` for the access `flags` ask for, copying it up first
    /// when they ask for writing.
    fn open_node(&self, id: NodeId, flags: i32) -> io::Result<File> {
        let writing = flags & libc::O_ACCMODE != libc::O_RDONLY || flags & libc::O_TRUNC != 0;
        if self.tree.read().expect("tree").node(id).is_none() {
            return Err(io::Error::from_raw_os_error(libc::ESTALE));
        }
        let real = if writing {
            self.tree.write().expect("tree").make_writable(id)?
        } else {
            let tree = self.tree.read().expect("tree");
            if tree.node(id).is_some_and(|n| n.is_dir()) {
                return Err(io::Error::from_raw_os_error(libc::EISDIR));
            }
            tree.real_path(id).ok_or_else(|| io::Error::from_raw_os_error(libc::ENOENT))?
        };
        let mut options = File::options();
        match flags & libc::O_ACCMODE {
            libc::O_WRONLY => options.write(true),
            libc::O_RDWR => options.read(true).write(true),
            _ => options.read(true),
        };
        if flags & libc::O_TRUNC != 0 {
            options.write(true).truncate(true);
        }
        options.custom_flags(libc::O_CLOEXEC).open(real)
    }

    fn name(name: &OsStr) -> Result<&str, Errno> {
        // Names that are not UTF-8 cannot exist in the tree.
        name.to_str().ok_or(Errno::ENOENT)
    }
}

impl Filesystem for StrataFs {
    fn init(&mut self, _req: &Request, config: &mut KernelConfig) -> io::Result<()> {
        // Optional speed-ups: the kernel may not offer them, and that is fine.
        let _ = config.add_capabilities(InitFlags::FUSE_PARALLEL_DIROPS);
        let _ = config.add_capabilities(InitFlags::FUSE_ASYNC_READ);
        // Names with attributes when the kernel sees them used (`readdirplus`).
        let _ = config.add_capabilities(InitFlags::FUSE_DO_READDIRPLUS | InitFlags::FUSE_READDIRPLUS_AUTO);
        let _ = config.set_max_readahead(1 << 20);
        let _ = config.set_max_write(1 << 20);
        Ok(())
    }

    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let name = match Self::name(name) {
            Ok(name) => name,
            Err(e) => return reply.error(e),
        };
        let tree = self.tree.read().expect("tree");
        if let Err(e) = Self::live(&tree, node_id(parent)) {
            return reply.error(e);
        }
        match tree.lookup(node_id(parent), name) {
            Some(id) => match self.attr(&tree, id) {
                Ok(attr) => reply.entry_with_ttls(&ttl_of(&tree, id), &entry_ttl(&tree, id), &attr, Generation(0)),
                Err(e) => reply.error(e.into()),
            },
            None => reply.error(Errno::ENOENT),
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, fh: Option<FileHandle>, reply: ReplyAttr) {
        let id = node_id(ino);
        if let Some((file, node)) = fh.and_then(|fh| self.file(fh))
            && let Ok(meta) = file.metadata()
        {
            return reply.attr(&TTL, &self.file_attr(node, &meta));
        }
        let tree = self.tree.read().expect("tree");
        match self.attr(&tree, id) {
            Ok(attr) => reply.attr(&ttl_of(&tree, id), &attr),
            Err(e) => match self.open_file_of(id).and_then(|file| file.metadata().ok()) {
                // `fstat` of a file held open whose id went (deleted, or gone in a
                // refresh): the kernel sends no handle with it
                Some(meta) => reply.attr(&TTL, &self.file_attr(id, &meta)),
                None => reply.error(e.into()),
            },
        }
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<fuser::BsdFileFlags>,
        _kill_suid_gid: bool,
        reply: ReplyAttr,
    ) {
        let id = node_id(ino);
        let apply = |file: &File| -> io::Result<()> {
            if let Some(size) = size {
                file.set_len(size)?;
            }
            if let Some(mode) = mode {
                use std::os::unix::fs::PermissionsExt;
                // Keep the owner's access: a copy the game cannot write again is no use.
                file.set_permissions(std::fs::Permissions::from_mode((mode & 0o7777) | 0o600))?;
            }
            if atime.is_some() || mtime.is_some() {
                let resolve = |t: TimeOrNow| match t {
                    TimeOrNow::SpecificTime(t) => t,
                    TimeOrNow::Now => SystemTime::now(),
                };
                let mut times = std::fs::FileTimes::new();
                if let Some(t) = atime {
                    times = times.set_accessed(resolve(t));
                }
                if let Some(t) = mtime {
                    times = times.set_modified(resolve(t));
                }
                file.set_times(times)?;
            }
            Ok(())
        };
        let result = (|| -> io::Result<FileAttr> {
            let held = fh.and_then(|fh| self.file(fh)).map(|(file, _)| file);
            let node = self.tree.read().expect("tree").node(id).map(|n| n.is_dir());
            let is_dir = match (node, &held) {
                (Some(is_dir), _) => is_dir,
                // the id went (deleted, or gone in a refresh) but the file is held open:
                // `ftruncate` and `futimens` work on it, as on any file system
                (None, Some(file)) => {
                    apply(file)?;
                    return Ok(self.file_attr(id, &file.metadata()?));
                }
                (None, None) => return Err(io::Error::from_raw_os_error(libc::ESTALE)),
            };
            if is_dir {
                // Folders are synthetic: there is nothing to change. Say yes, as Wine expects.
                return Ok(self.dir_attr(id));
            }
            if size.is_some() || mode.is_some() || atime.is_some() || mtime.is_some() {
                let file = match held {
                    // A handle opened for reading cannot truncate: open the file again.
                    Some(file) if size.is_none() => file,
                    _ => {
                        let real = self.tree.write().expect("tree").make_writable(id)?;
                        Arc::new(File::options().write(true).open(real)?)
                    }
                };
                apply(&file)?;
            }
            let tree = self.tree.read().expect("tree");
            self.attr(&tree, id)
        })();
        match result {
            Ok(attr) => reply.attr(&TTL, &attr),
            Err(e) => reply.error(e.into()),
        }
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _owner: fuser::Owner,
        reply: ReplyEntry,
    ) {
        let name = match Self::name(name) {
            Ok(name) => name,
            Err(_) => return reply.error(Errno::EINVAL),
        };
        let mut tree = self.tree.write().expect("tree");
        if let Err(e) = Self::live(&tree, node_id(parent)) {
            return reply.error(e);
        }
        match tree.create_dir(node_id(parent), name) {
            Ok(id) => reply.entry(&TTL_STABLE, &self.dir_attr(id), Generation(0)),
            Err(e) => reply.error(e.into()),
        }
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = match Self::name(name) {
            Ok(name) => name,
            Err(e) => return reply.error(e),
        };
        let mut tree = self.tree.write().expect("tree");
        if let Err(e) = Self::live(&tree, node_id(parent)) {
            return reply.error(e);
        }
        match tree.unlink(node_id(parent), name) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e.into()),
        }
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = match Self::name(name) {
            Ok(name) => name,
            Err(e) => return reply.error(e),
        };
        let mut tree = self.tree.write().expect("tree");
        if let Err(e) = Self::live(&tree, node_id(parent)) {
            return reply.error(e);
        }
        match tree.rmdir(node_id(parent), name) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e.into()),
        }
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: RenameFlags,
        _owner: Option<fuser::Owner>,
        reply: ReplyEmpty,
    ) {
        let (Ok(name), Ok(newname)) = (Self::name(name), Self::name(newname)) else {
            return reply.error(Errno::EINVAL);
        };
        let mut tree = self.tree.write().expect("tree");
        let bits = flags.bits();
        if bits & libc::RENAME_EXCHANGE != 0 {
            return reply.error(Errno::EINVAL);
        }
        if bits & libc::RENAME_NOREPLACE != 0 {
            let source = tree.lookup(node_id(parent), name);
            let target = tree.lookup(node_id(newparent), newname);
            if target.is_some() && target != source {
                return reply.error(Errno::EEXIST);
            }
        }
        for id in [parent, newparent] {
            if let Err(e) = Self::live(&tree, node_id(id)) {
                return reply.error(e);
            }
        }
        match tree.rename(node_id(parent), name, node_id(newparent), newname) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e.into()),
        }
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, _kill_suid_gid: bool, reply: ReplyOpen) {
        let id = node_id(ino);
        match self.open_node(id, flags.0) {
            Ok(file) => {
                let read_only = flags.acc_mode() == OpenAccMode::O_RDONLY && flags.0 & libc::O_TRUNC == 0;
                let fh = self.add_handle(Handle::File { file: Arc::new(file), node: id });
                // Keep the page cache between opens of a file that is only read: big
                // archives are opened again and again.
                reply.opened(fh, if read_only { FopenFlags::FOPEN_KEEP_CACHE } else { FopenFlags::empty() });
            }
            Err(e) => reply.error(e.into()),
        }
    }

    fn read(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let Some((file, _)) = self.file(fh) else { return reply.error(Errno::EBADF) };
        // One buffer per FUSE thread, kept: no allocation and no zeroing per request.
        thread_local! {
            static BUFFER: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        BUFFER.with_borrow_mut(|buffer| {
            let size = size as usize;
            if buffer.len() < size {
                buffer.resize(size, 0);
            }
            let mut filled = 0;
            while filled < size {
                match file.read_at(&mut buffer[filled..size], offset + filled as u64) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return reply.error(e.into()),
                }
            }
            reply.data(&buffer[..filled]);
        });
    }

    fn write(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let Some((file, _)) = self.file(fh) else { return reply.error(Errno::EBADF) };
        match file.write_all_at(data, offset) {
            Ok(()) => reply.written(data.len() as u32),
            Err(e) => reply.error(e.into()),
        }
    }

    fn flush(&self, _req: &Request, _ino: INodeNo, _fh: FileHandle, _lock_owner: LockOwner, reply: ReplyEmpty) {
        reply.ok();
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.handles.lock().expect("handle table").remove(&fh.0);
        reply.ok();
    }

    fn fsync(&self, _req: &Request, _ino: INodeNo, fh: FileHandle, datasync: bool, reply: ReplyEmpty) {
        let Some((file, _)) = self.file(fh) else { return reply.error(Errno::EBADF) };
        match if datasync { file.sync_data() } else { file.sync_all() } {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e.into()),
        }
    }

    fn opendir(&self, _req: &Request, ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        let id = node_id(ino);
        let tree = self.tree.read().expect("tree");
        let node = match Self::live(&tree, id) {
            Ok(node) => node,
            Err(e) => return reply.error(e),
        };
        if !node.is_dir() {
            return reply.error(Errno::ENOTDIR);
        }
        let mut entries = vec![
            (ino.0, FileType::Directory, ".".to_owned()),
            (u64::from(node.parent) + 1, FileType::Directory, "..".to_owned()),
        ];
        entries.extend(tree.children(id).map(|(child, node)| {
            let kind = if node.is_dir() { FileType::Directory } else { FileType::RegularFile };
            (u64::from(child) + 1, kind, node.name.to_string())
        }));
        drop(tree);
        // The kernel may keep the listing across opens: changes through the mount reset it
        // (the kernel bumps the folder's version), a refresh invalidates the folders it
        // changed (`Mount::refresh`).
        reply.opened(
            self.add_handle(Handle::Dir(Arc::new(entries))),
            FopenFlags::FOPEN_CACHE_DIR | FopenFlags::FOPEN_KEEP_CACHE,
        );
    }

    fn readdir(&self, _req: &Request, _ino: INodeNo, fh: FileHandle, offset: u64, mut reply: ReplyDirectory) {
        let entries = match self.handles.lock().expect("handle table").get(&fh.0) {
            Some(Handle::Dir(entries)) => Arc::clone(entries),
            _ => return reply.error(Errno::EBADF),
        };
        for (index, (ino, kind, name)) in entries.iter().enumerate().skip(offset as usize) {
            if reply.add(INodeNo(*ino), index as u64 + 1, *kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn readdirplus(&self, _req: &Request, _ino: INodeNo, fh: FileHandle, offset: u64, mut reply: fuser::ReplyDirectoryPlus) {
        let entries = match self.handles.lock().expect("handle table").get(&fh.0) {
            Some(Handle::Dir(entries)) => Arc::clone(entries),
            _ => return reply.error(Errno::EBADF),
        };
        // The names with their attributes: one request where `ls -l` or Wine's directory
        // listing would send a lookup per name.
        let tree = self.tree.read().expect("tree");
        for (index, (ino, _, name)) in entries.iter().enumerate().skip(offset as usize) {
            let id = node_id(INodeNo(*ino));
            let Ok(attr) = self.attr(&tree, id) else { continue };
            // one lifetime for name and attributes here: a file's name gets none (`entry_ttl`)
            if reply.add(INodeNo(*ino), index as u64 + 1, name, &entry_ttl(&tree, id), &attr, Generation(0)) {
                break;
            }
        }
        reply.ok();
    }

    fn releasedir(&self, _req: &Request, _ino: INodeNo, fh: FileHandle, _flags: OpenFlags, reply: ReplyEmpty) {
        self.handles.lock().expect("handle table").remove(&fh.0);
        reply.ok();
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        // The space that matters is where writes land: the highest layer.
        let source = self.tree.read().expect("tree").layers().last().map(|l| l.source.clone());
        let stat = source.and_then(|path| statvfs(&path).ok());
        match stat {
            Some(s) => reply.statfs(s.f_blocks, s.f_bfree, s.f_bavail, s.f_files, s.f_ffree, s.f_bsize as u32, 255, s.f_frsize as u32),
            None => reply.statfs(0, 0, 0, 0, 0, 4096, 255, 4096),
        }
    }

    fn access(&self, _req: &Request, ino: INodeNo, _mask: AccessFlags, reply: ReplyEmpty) {
        match self.tree.read().expect("tree").node(node_id(ino)) {
            Some(_) => reply.ok(),
            None => reply.error(Errno::ESTALE),
        }
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        flags: i32,
        _kill_suid_gid: bool,
        _owner: fuser::Owner,
        reply: ReplyCreate,
    ) {
        let name = match Self::name(name) {
            Ok(name) => name,
            Err(_) => return reply.error(Errno::EINVAL),
        };
        let result = (|| -> io::Result<(NodeId, File, Duration)> {
            let mut tree = self.tree.write().expect("tree");
            if tree.node(node_id(parent)).is_none() {
                return Err(io::Error::from_raw_os_error(libc::ESTALE));
            }
            if let Some(existing) = tree.lookup(node_id(parent), name) {
                let ttl = entry_ttl(&tree, existing);
                // Lost a race with another create, or the name exists in another case.
                if flags & libc::O_EXCL != 0 {
                    return Err(io::Error::from_raw_os_error(libc::EEXIST));
                }
                drop(tree);
                return Ok((existing, self.open_node(existing, flags)?, ttl));
            }
            let (id, real) = tree.create_file(node_id(parent), name, (mode & 0o777) | 0o600)?;
            let file = File::options().read(true).write(true).custom_flags(libc::O_CLOEXEC).open(&real);
            match file {
                Ok(file) => Ok((id, file, entry_ttl(&tree, id))),
                Err(e) => {
                    // Undo the node: nothing exists on disk for it.
                    let _ = tree.unlink(node_id(parent), name);
                    Err(e)
                }
            }
        })();
        match result {
            Ok((id, file, ttl)) => {
                let attr = file.metadata().map(|meta| self.file_attr(id, &meta));
                match attr {
                    Ok(attr) => {
                        let fh = self.add_handle(Handle::File { file: Arc::new(file), node: id });
                        reply.created(&ttl, &attr, Generation(0), fh, FopenFlags::empty());
                    }
                    Err(e) => reply.error(e.into()),
                }
            }
            Err(e) => reply.error(e.into()),
        }
    }

    fn fallocate(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        length: u64,
        mode: i32,
        reply: ReplyEmpty,
    ) {
        use std::os::fd::AsRawFd;
        let Some((file, _)) = self.file(fh) else { return reply.error(Errno::EBADF) };
        // SAFETY: the descriptor is open for the life of `file`.
        let result = unsafe { libc::fallocate(file.as_raw_fd(), mode, offset as libc::off_t, length as libc::off_t) };
        if result == 0 {
            reply.ok();
        } else {
            reply.error(io::Error::last_os_error().into());
        }
    }

    fn ioctl(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _flags: IoctlFlags,
        cmd: u32,
        _in_data: &[u8],
        out_size: u32,
        reply: ReplyIoctl,
    ) {
        // Only folders say they fold case; for anything else the answer stays "no such
        // ioctl", as on a filesystem without inode flags (Fluorine's mo2_ioctl does the same).
        let is_dir = self.tree.read().expect("tree").node(node_id(ino)).is_some_and(|n| n.is_dir());
        if cmd == FS_IOC_GETFLAGS && out_size >= 4 && is_dir {
            reply.ioctl(0, &FS_CASEFOLD_FL.to_ne_bytes());
        } else {
            reply.error(Errno::ENOTTY);
        }
    }
}

fn statvfs(path: &Path) -> io::Result<libc::statvfs> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: `c_path` is a valid C string and `stat` is a properly sized, writable struct.
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 { Ok(stat) } else { Err(io::Error::last_os_error()) }
    }
}

/// Node ids are also what `status` reports; keep the root's inode at FUSE's fixed value.
const _: () = assert!(ROOT == 0);
