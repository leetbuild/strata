//! Nexus: link parsing, the API client, the download queue.
//!
//! - [`links`]: `nxm://` and `modl://` links as pure functions (modlist area).
//! - [`api`]: the Nexus Mods API v1 client (install area).
//! - [`queue`]: downloads with pause / resume and their `.meta` (install area).
//! - [`query`]: "query info" by MD5 (install area).
//! - [`sso`]: the SSO login (an API key from the website), [`keystore`]: the key in the
//!   secret service, [`nxm`]: an nxm:// link to a queued download, [`fake`]: a local Nexus
//!   for tests (install area).

pub mod api;
/// Test support: off in the shipped library (feature `test-support`).
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
pub mod keystore;
pub mod links;
pub mod nxm;
pub mod query;
pub mod queue;
pub mod sso;

use std::path::PathBuf;

/// What can go wrong talking to Nexus or downloading.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}; check the connection and try again")]
    Network(String),
    #[error("Nexus answered {status}: {message}")]
    Api { status: u16, message: String },
    #[error("Nexus sent something unexpected: {0}")]
    Format(String),
    #[error("cannot write {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0} exists already: remove or rename it first")]
    Exists(PathBuf),
    #[error("cancelled")]
    Cancelled,
    #[error("secret service: {0}")]
    Keyring(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Writes a download's `.meta` atomically and only when it changed, keeping its
/// permissions (`strata_core::write`, as `strata-install`'s writer does).
pub(crate) fn write_meta_file(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    strata_core::write::write_if_changed(path, bytes)
        .map(|_| ())
        .map_err(|e| Error::Io(path.to_owned(), std::io::Error::other(e.to_string())))
}
