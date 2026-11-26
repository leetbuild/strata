//! Errors. Each message says what is wrong and, where it helps, what to do.

use std::path::PathBuf;

/// What can go wrong while gathering what the window shows.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] strata_core::Error),

    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },

    #[error("cannot write {path}: {source}. Check that the folder is writable and the disk not full.")]
    Write { path: PathBuf, source: std::io::Error },

    #[error("cannot build the merged Data tree: {0}")]
    Tree(std::io::Error),

    #[error("LOOT: {0}")]
    Loot(String),

    #[error("the overlay daemon: {0}")]
    Daemon(String),

    /// A request the dialog cannot carry out; the message says why.
    #[error("{0}")]
    Invalid(String),

    #[error(
        "no instance to open. Start Strata with `strata -i <instance folder>` (the folder with \
         ModOrganizer.ini), or from inside an instance folder."
    )]
    NoInstance,
}

/// This crate's result type.
pub type Result<T> = std::result::Result<T, Error>;
