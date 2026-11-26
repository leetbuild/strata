//! Errors. Each message says what is wrong and, where it helps, what to do.

use std::path::PathBuf;

/// What can go wrong in the core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },

    #[error("{path} is not an MO2 instance: it has no ModOrganizer.ini")]
    NotAnInstance { path: PathBuf },

    #[error("{path} has no {key} in [{section}]. Open the instance in MO2 once so it writes its settings.")]
    MissingKey { path: PathBuf, section: &'static str, key: &'static str },

    #[error("the game \"{name}\" is not supported yet (supported: Skyrim Special Edition, Skyrim)")]
    UnknownGame { name: String },

    #[error("profile \"{name}\" not found in {dir}. Profiles there: {available}")]
    NoSuchProfile { name: String, dir: PathBuf, available: String },

    #[error(
        "\"{path}\" is a Windows path on drive {drive}:, which only exists inside a Wine prefix, \
         and this instance names no prefix. Set the prefix in the instance's settings."
    )]
    NoPrefixForDrive { path: String, drive: char },

    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: std::io::Error },

    #[error("{path} already exists. Pick another name, or move it away first.")]
    Exists { path: PathBuf },

    #[error("{path} holds a tab or a line end, which Strata's journal can't record. Rename it first.")]
    UnsafePath { path: PathBuf },

    #[error("\"{name}\" is not a valid name: it is empty or reserved once the characters <>:\"/\\|?* are removed")]
    InvalidName { name: String },

    #[error("no mod named \"{name}\" in this profile. `strata list` shows the names.")]
    NoSuchMod { name: String },

    #[error("\"{name}\" is the instance's selected profile; {action}. Select another profile first.")]
    ActiveProfile { name: String, action: &'static str },

    #[error("{what}")]
    Refused { what: String },

    #[error("no plugin \"{name}\" in the profile's Data folder. `strata list plugins` shows them.")]
    NoSuchPlugin { name: String },
}

/// The core's result type.
pub type Result<T> = std::result::Result<T, Error>;
