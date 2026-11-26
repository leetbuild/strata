//! Mounting a tree: in place over a folder, with stale mounts from a crash cleaned first.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, RwLock};

use fuser::{BackgroundSession, Config, MountOption};
use serde::{Deserialize, Serialize};
use strata_game::Layer;

use crate::fs::StrataFs;
use crate::tree::{Options, Tree};

/// One mount: a layer list shown at a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountSpec {
    /// A name for status and logs: `game`, `appdata`, `documents`.
    pub id: String,
    /// Lowest priority first.
    pub layers: Vec<Layer>,
    /// Where the merged tree appears. May be one of the layers' own folders (the Game Root):
    /// the mount then sits in place over it and reads it through a handle opened before.
    pub mountpoint: PathBuf,
    /// `<instance>/strata/whiteouts.txt` for the game mount; none for profile mounts.
    #[serde(default)]
    pub whiteouts: Option<PathBuf>,
    /// Free text the daemon reports back in `status` (the CLI puts the profile's name
    /// there, so a later command knows which profile is deployed).
    #[serde(default)]
    pub label: Option<String>,
}

/// A live mount. Dropping it unmounts.
pub struct Mount {
    pub id: String,
    pub mountpoint: PathBuf,
    /// [`MountSpec::label`].
    pub label: Option<String>,
    pub tree: Arc<RwLock<Tree>>,
    session: Option<BackgroundSession>,
    /// The whiteout file the tree was built with, for refreshes.
    whiteouts: Option<PathBuf>,
    /// The handle path that layers inside the mountpoint are read through.
    through: Option<PathBuf>,
}

/// What a refresh did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshReport {
    pub nodes: usize,
    /// Files the kernel was told to forget (their bytes now come from another file).
    pub changed: usize,
    /// Folder entries that appeared or went away.
    pub entries: usize,
}

/// Points layers inside `mountpoint` at `through`, the handle on the folder under it.
fn translate(layers: &mut [Layer], mountpoint: &Path, through: &Path) {
    for layer in layers {
        if let Ok(rest) = layer.source.strip_prefix(mountpoint) {
            layer.source = if rest.as_os_str().is_empty() { through.to_owned() } else { through.join(rest) };
        }
    }
}

/// The filesystem type a Strata mount shows in `/proc/self/mountinfo`.
pub const FS_SUBTYPE: &str = "strata";

/// Whether something is mounted exactly at `path`, and its filesystem type.
pub fn mounted_at(path: &Path) -> io::Result<Option<String>> {
    let wanted = path.to_string_lossy();
    let text = std::fs::read_to_string("/proc/self/mountinfo")?;
    let mut found = None;
    for line in text.lines() {
        // 36 35 98:0 /root /mount/point rw,noatime - fstype source options
        let mut fields = line.split(' ');
        let Some(mountpoint) = fields.nth(4) else { continue };
        // Spaces and a few others are octal-escaped.
        let mountpoint = mountpoint.replace("\\040", " ").replace("\\011", "\t").replace("\\012", "\n").replace("\\134", "\\");
        if mountpoint == wanted {
            let fstype = line.split(" - ").nth(1).and_then(|rest| rest.split(' ').next()).unwrap_or("");
            found = Some(fstype.to_owned());
        }
    }
    Ok(found)
}

/// Unmounts `path` with `fusermount3 -u`, lazily (`-z`) when it is busy and `lazy` allows.
pub fn unmount(path: &Path, lazy: bool) -> io::Result<()> {
    let run = |args: &[&str]| Command::new("fusermount3").args(args).arg(path).output();
    let first = run(&["-u"])?;
    if first.status.success() {
        return Ok(());
    }
    if lazy && run(&["-u", "-z"])?.status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "cannot unmount {}: {}",
        path.display(),
        String::from_utf8_lossy(&first.stderr).trim()
    )))
}

/// Removes a Strata mount a crashed daemon left at `path`. Other mounts are left alone
/// and reported as an error.
pub fn clean_stale(path: &Path) -> io::Result<bool> {
    match mounted_at(path)? {
        None => Ok(false),
        Some(fstype) if fstype == format!("fuse.{FS_SUBTYPE}") => {
            unmount(path, true)?;
            Ok(true)
        }
        Some(fstype) => Err(io::Error::other(format!(
            "{} is already a mountpoint ({fstype}), not Strata's: unmount it first",
            path.display()
        ))),
    }
}

impl Mount {
    /// Builds the tree and mounts it. `threads` is the number of threads answering the kernel.
    pub fn new(spec: MountSpec, threads: usize) -> io::Result<Self> {
        clean_stale(&spec.mountpoint)?;
        if !spec.mountpoint.is_dir() {
            return Err(io::Error::other(format!("{} is not a folder", spec.mountpoint.display())));
        }

        // Layers inside the mountpoint are read through a handle on the folder underneath:
        // once mounted, the path itself shows the merged tree.
        let mut layers = spec.layers;
        let mut under = None;
        let mut through = None;
        if layers.iter().any(|l| l.source.starts_with(&spec.mountpoint)) {
            let handle = File::options().read(true).custom_flags(libc::O_DIRECTORY | libc::O_PATH).open(&spec.mountpoint)?;
            let path = PathBuf::from(format!("/proc/self/fd/{}", handle.as_raw_fd()));
            translate(&mut layers, &spec.mountpoint, &path);
            through = Some(path);
            under = Some(handle);
        }

        let options = Options { clean_temp_files: true, ..Options::default() };
        let tree = Arc::new(RwLock::new(Tree::build(layers, options, spec.whiteouts.clone())?));
        let filesystem = StrataFs::new(Arc::clone(&tree), under);

        let mut config = Config::default();
        config.mount_options = vec![
            MountOption::FSName(format!("strata-{}", spec.id)),
            MountOption::Subtype(FS_SUBTYPE.into()),
            MountOption::NoSuid,
            MountOption::NoDev,
            MountOption::NoAtime,
        ];
        config.n_threads = Some(threads.max(1));
        config.clone_fd = true;
        let session = fuser::spawn_mount(filesystem, &spec.mountpoint, &config)?;
        Ok(Self { id: spec.id, mountpoint: spec.mountpoint, label: spec.label, tree, session: Some(session), whiteouts: spec.whiteouts, through })
    }

    /// Shows a new layer list at the same mountpoint without unmounting: the tree is
    /// rebuilt and swapped in while FUSE requests wait (`Tree::rebased` keeps the node ids
    /// of unchanged paths), then the kernel is told which cached files and entries to
    /// forget. Files already open keep reading what they opened. A layer inside the
    /// mountpoint (the game folder under an in-place mount) is read through the same
    /// handle as at mount time.
    pub fn refresh(&self, mut layers: Vec<Layer>) -> io::Result<RefreshReport> {
        if let Some(through) = &self.through {
            translate(&mut layers, &self.mountpoint, through);
        } else if layers.iter().any(|l| l.source.starts_with(&self.mountpoint)) {
            return Err(io::Error::other("a new layer lies inside the mountpoint: unmount and mount again"));
        }
        let options = Options { clean_temp_files: true, ..Options::default() };
        let (refreshed, nodes) = {
            let mut tree = self.tree.write().map_err(|_| io::Error::other("the tree lock is poisoned"))?;
            let new = Tree::build(layers, options, self.whiteouts.clone())?;
            let (rebased, refreshed) = tree.rebased(new);
            *tree = rebased;
            (refreshed, tree.len())
        };
        // After the lock is released: the kernel may call back into the filesystem.
        if let Some(session) = &self.session {
            let notifier = session.notifier();
            for id in &refreshed.changed {
                let _ = notifier.inval_inode(crate::fs::ino(*id), 0, 0);
            }
            for (parent, name) in &refreshed.entries {
                let _ = notifier.inval_entry(crate::fs::ino(*parent), std::ffi::OsStr::new(name));
            }
            // and the folders' cached listings
            let mut parents: Vec<_> = refreshed.entries.iter().map(|(parent, _)| *parent).collect();
            parents.sort_unstable();
            parents.dedup();
            for parent in parents {
                let _ = notifier.inval_inode(crate::fs::ino(parent), 0, 0);
            }
        }
        Ok(RefreshReport { nodes, changed: refreshed.changed.len(), entries: refreshed.entries.len() })
    }

    /// Unmounts. With `lazy`, a busy mount is detached anyway and goes away when its last
    /// user closes; without, a busy mount is an error and stays mounted.
    pub fn unmount(&mut self, lazy: bool) -> io::Result<()> {
        if self.session.is_none() {
            return Ok(());
        }
        unmount(&self.mountpoint, lazy)?;
        if let Some(session) = self.session.take() {
            let _ = session.join();
        }
        Ok(())
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        if self.session.take().is_some() {
            let _ = unmount(&self.mountpoint, true);
        }
    }
}
