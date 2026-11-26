//! Installers (quick, manual model, BAIN, FOMOD, bundle) and the Root sorter: MO2's
//! `InstallationManager` and its installer plugins, without dialogs.
//!
//! The pipeline is [`install::install`]; every question it asks goes through
//! [`install::InstallUi`], which the shell answers with dialogs and the CLI with options.
//! See `README.md` and `docs/ARCHITECTURE.md`.

pub mod archive;
pub mod bsa;
pub mod checker;
pub mod downloads;
pub mod fomod;
pub mod install;
pub mod instance;
pub mod installers;
pub mod meta;
pub mod name;
pub mod profile;
pub mod rootsorter;
pub mod settings;
pub mod tree;
pub mod version;

use std::path::{Path, PathBuf};

/// What can go wrong in an install. Each message says what to do.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read or write {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("cannot read the archive {path}: {reason}")]
    Archive { path: PathBuf, reason: String },
    #[error("the archive {0} is password protected: give its password (--password in the CLI)")]
    PasswordNeeded(PathBuf),
    #[error("the password for {0} is wrong")]
    WrongPassword(PathBuf),
    #[error("FOMOD installer: {0}")]
    Fomod(String),
    #[error("the install was cancelled; nothing was changed")]
    Cancelled,
    #[error("files of type \".{0}\" cannot be installed: give a .zip, .rar, .7z, .fomod or .001 archive")]
    Unsupported(String),
    #[error("installing mods for \"{0}\" is not supported yet (only Skyrim Special Edition)")]
    UnsupportedGame(String),
    #[error("no installer can handle this archive; it may be damaged, or need the manual installer of the window")]
    NoInstaller,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Core(#[from] strata_core::Error),
}

impl Error {
    pub(crate) fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Error::Io { path: path.as_ref().to_owned(), source }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
