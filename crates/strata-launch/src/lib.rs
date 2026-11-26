//! Running Windows programs: Steam detection, Proton and its Steam Linux Runtime, the
//! launch command (`docs/DESIGN.md`, "Proton and launching").
//!
//! The command and environment are built here and handed back as a [`Plan`], so the caller
//! can log exactly what will run before running it.

mod plan;
pub mod setup;
pub mod steam;
mod wait;

use std::path::PathBuf;

pub use plan::{Launch, Plan, ProtonCall, Runner, SlrRunner, split_arguments};
pub use steam::{Proton, Steam};
pub use wait::{prefix_processes, wait_for_prefix};

/// What can go wrong before a program starts. Each message says what to do.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Steam was not found (looked in ~/.local/share/Steam, ~/.steam/steam and the Flatpak). Install Steam, or start it once.")]
    NoSteam,

    #[error("{path} is not a Proton: it has no `proton` script. Pick a Proton in the instance's settings.")]
    NotAProton { path: PathBuf },

    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },

    #[error(
        "{proton} needs the Steam tool with app id {app_id} ({name}), which is not installed in any Steam library. \
         Install it with: steam steam://install/{app_id}"
    )]
    RuntimeMissing { proton: String, app_id: u32, name: &'static str },

    #[error("the prefix {path} has no pfx/ folder. Set the instance's prefix to the folder that holds pfx/.")]
    NotAPrefix { path: PathBuf },

    #[error("Steam is not running, and this game needs it (Steam DRM). Start Steam and try again.")]
    SteamNotRunning,

    #[error("cannot start Steam (`steam -silent`). Is the `steam` command installed? Start Steam yourself and try again.")]
    SteamStartFailed,

    #[error("Steam did not get ready within {seconds} s (no steamwebhelper). Log in to Steam, then try again.")]
    SteamNotReady { seconds: u64 },
}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;
