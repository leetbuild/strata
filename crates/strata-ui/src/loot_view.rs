//! Where LOOT looks at the game: the profile's merged game folder through Strata's overlay,
//! which is case-insensitive like Windows. MO2 runs lootcli inside its VFS for the same
//! reason: the masterlist's conditions name files in whatever case (`file("scripts/skse.pex")`
//! while the SKSE mod has `Scripts/SKSE.pex`), and libloot checks them with the path as
//! written.
//!
//! - The profile is deployed: the game folder already shows it; libloot reads it there.
//! - Nothing is deployed: a private mount of the same layers (as `strata deploy`'s game mount,
//!   whiteouts included) at `<instance>/strata/loot-mount`, served by this process and
//!   unmounted when the view is dropped (on success, error and cancel alike). The game
//!   folder is not touched. A mount a crash left there is removed by the next sort
//!   (`Mount::new` cleans stale Strata mounts first).
//! - Another profile is deployed: refused (the game folder shows that profile's files).
//! - No FUSE (`/dev/fuse` or `fusermount3` missing): the mods' folders go to libloot as
//!   additional data folders, as before; names are then case-sensitive (a warning says so).

use std::path::{Path, PathBuf};

use strata_core::{Instance, Profile, resolve_layers};
use strata_game::LayerKind;
use strata_vfs::{Mount, MountSpec};

use crate::{Error, Result};

/// The game as libloot should see it. Holds the private mount, if one was made.
pub struct LootView {
    /// libloot's game folder.
    pub game_path: PathBuf,
    /// Additional data folders, highest priority first (empty with the overlay).
    pub data_paths: Vec<PathBuf>,
    /// How the view was made, for the log.
    pub how: How,
    mount: Option<Mount>,
    mount_dir: Option<PathBuf>,
    _lock: Option<std::fs::File>,
}

/// How a [`LootView`] shows the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// The live deploy of the profile.
    Deployed,
    /// A private mount for the sort.
    PrivateMount,
    /// No overlay: additional data folders, case-sensitive.
    NoFuse,
}

/// Whether FUSE mounts can be made here.
pub fn fuse_available() -> bool {
    Path::new("/dev/fuse").exists()
        && std::process::Command::new("fusermount3").arg("-V").output().is_ok_and(|o| o.status.success())
}

/// The folder the private mount goes to.
pub fn mount_dir(instance: &Instance) -> PathBuf {
    instance.root.join("strata/loot-mount")
}

impl LootView {
    /// Makes the view for sorting `profile` (see the module's notes).
    pub fn open(instance: &Instance, profile: &Profile) -> Result<Self> {
        match crate::live::deployed_profile(instance)? {
            Some(p) if p == profile.name => {
                return Ok(Self {
                    game_path: instance.game_path.clone(),
                    data_paths: Vec::new(),
                    how: How::Deployed,
                    mount: None,
                    mount_dir: None,
                    _lock: None,
                });
            }
            Some(p) => {
                return Err(Error::Loot(format!(
                    "profile \"{p}\" is deployed: the game folder shows its mods. Undeploy it, or sort that profile."
                )));
            }
            None => {}
        }
        let layers = resolve_layers(instance, profile)?;
        if !fuse_available() {
            return Ok(Self {
                game_path: instance.game_path.clone(),
                data_paths: layers.iter().rev().filter(|l| l.kind != LayerKind::Game).map(|l| l.source.clone()).collect(),
                how: How::NoFuse,
                mount: None,
                mount_dir: None,
                _lock: None,
            });
        }
        let dir = mount_dir(instance);
        let io = |what: &str, e: std::io::Error| Error::Loot(format!("{what} {}: {e}", dir.display()));
        std::fs::create_dir_all(instance.root.join("strata")).map_err(|e| io("cannot create the folder of", e))?;
        // one sort of an instance at a time, across processes (the window and the CLI): a second
        // one would take the first one's mount for a stale one
        let lock = std::fs::File::create(instance.root.join("strata/loot.lock")).map_err(|e| io("cannot lock", e))?;
        lock.try_lock().map_err(|_| Error::Loot("another LOOT sort of this instance is running".to_owned()))?;
        // a mount left by a crash first, then the folder
        strata_vfs::mount::clean_stale(&dir).map_err(|e| io("cannot clean", e))?;
        std::fs::create_dir_all(&dir).map_err(|e| io("cannot create", e))?;
        let spec = MountSpec {
            id: "loot".into(),
            layers,
            mountpoint: dir.clone(),
            whiteouts: Some(instance.root.join("strata/whiteouts.txt")),
            label: Some(profile.name.clone()),
        };
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
        let mount = Mount::new(spec, threads).map_err(|e| io("cannot mount the overlay at", e))?;
        Ok(Self {
            game_path: dir.clone(),
            data_paths: Vec::new(),
            how: How::PrivateMount,
            mount: Some(mount),
            mount_dir: Some(dir),
            _lock: Some(lock),
        })
    }
}

impl Drop for LootView {
    fn drop(&mut self) {
        if let Some(mut mount) = self.mount.take() {
            // libloot has closed its files by now; lazily when something still holds one
            let _ = mount.unmount(true);
        }
        if let Some(dir) = &self.mount_dir {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

impl std::fmt::Display for How {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            How::Deployed => write!(f, "reading the deployed game folder"),
            How::PrivateMount => write!(f, "reading the profile through a private overlay mount"),
            How::NoFuse => write!(f, "FUSE is not available: reading the mods' folders directly, names are case-sensitive"),
        }
    }
}
