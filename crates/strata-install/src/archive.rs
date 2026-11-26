//! Reading archives (7z, zip, rar): the small trait the pipeline needs, over the `conflicts`
//! area's `strata_archive::Archive` (which runs the `7z` program).
//!
//! The trait keeps the installers independent of the backend and lets tests replace it.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::{Error, Result};

/// One entry of an archive listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// `/`-separated path inside the archive.
    pub path: String,
    pub is_dir: bool,
}

/// What the installers need from an archive backend.
pub trait Archives: Send + Sync {
    /// Every entry of the archive. Each call takes the archive's password, if one was given:
    /// a protected archive fails with [`Error::PasswordNeeded`] / [`Error::WrongPassword`].
    fn list(&self, archive: &Path, password: Option<&str>) -> Result<Vec<ArchiveEntry>>;

    /// Extracts the whole archive into `dest`. `progress` gets percentages; when `cancel`
    /// becomes true the extraction stops with [`Error::Cancelled`].
    fn extract(
        &self,
        archive: &Path,
        password: Option<&str>,
        dest: &Path,
        progress: &dyn Fn(u8),
        cancel: &AtomicBool,
    ) -> Result<()>;

    /// Extracts the listed files (archive paths) into `dest`, keeping their paths.
    fn extract_files(&self, archive: &Path, password: Option<&str>, files: &[String], dest: &Path) -> Result<()>;
}

/// `strata_archive` (7-Zip).
#[derive(Debug, Clone, Default)]
pub struct SevenZip;

fn convert(archive: &Path, e: strata_archive::Error) -> Error {
    match e {
        strata_archive::Error::Cancelled => Error::Cancelled,
        strata_archive::Error::PasswordNeeded(_) => Error::PasswordNeeded(archive.to_owned()),
        strata_archive::Error::WrongPassword(_) => Error::WrongPassword(archive.to_owned()),
        other => Error::Archive { path: archive.to_owned(), reason: other.to_string() },
    }
}

fn open(archive: &Path, password: Option<&str>) -> Result<strata_archive::Archive> {
    strata_archive::Archive::open(archive, password).map_err(|e| convert(archive, e))
}

impl Archives for SevenZip {
    fn list(&self, archive: &Path, password: Option<&str>) -> Result<Vec<ArchiveEntry>> {
        Ok(open(archive, password)?.entries().iter().map(|e| ArchiveEntry { path: e.path.clone(), is_dir: e.is_dir }).collect())
    }

    fn extract(
        &self,
        archive: &Path,
        password: Option<&str>,
        dest: &Path,
        progress: &dyn Fn(u8),
        cancel: &AtomicBool,
    ) -> Result<()> {
        open(archive, password)?.extract_all(dest, &mut |p| progress(p.percent), cancel).map_err(|e| convert(archive, e))
    }

    fn extract_files(&self, archive: &Path, password: Option<&str>, files: &[String], dest: &Path) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        let a = open(archive, password)?;
        let plan: Vec<(usize, String)> = a
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, e)| files.contains(&e.path))
            .map(|(i, e)| (i, e.path.clone()))
            .collect();
        a.extract(dest, &plan, &mut |_| {}, &AtomicBool::new(false)).map_err(|e| convert(archive, e))
    }
}
