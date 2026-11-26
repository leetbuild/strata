//! The overlay's journal: changes that take more than one step on disk (a folder renamed, a
//! mod's file moved to overwrite and whited out) are written down before the first step and
//! finished at the next mount when vfsd dies in the middle (`spec/03-vfs.md`, "Crash
//! safety"; the idea of Fluorine's `vfs/stagingpromotion.*`, not its code).
//!
//! Every step can be done again without harm: a move whose source is gone is taken as done,
//! a removal of something missing is fine. So a crash anywhere, even while finishing an
//! earlier journal, ends in the state the change was going to make.
//!
//! The journal lives next to the whiteouts (`<instance>/strata/overlay-journal.json`), so
//! only the game mount has one; profile mounts change single files.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One step of a change, on real paths (never through `/proc/self/fd`, which means
/// nothing to the next process).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Step {
    /// Make a folder (and its parents).
    MakeDir(PathBuf),
    /// Rename `from` to `to`, replacing `to`, making `to`'s folder first.
    Move { from: PathBuf, to: PathBuf },
    /// Delete a file.
    Remove(PathBuf),
    /// Delete a folder if it is empty.
    RemoveDir(PathBuf),
    /// Change the whiteout file: `add` and `remove` are mount paths as written in it.
    Whiteouts { add: Vec<String>, remove: Vec<String> },
}

/// The journal's file for a whiteout file.
pub(crate) fn path_for(whiteouts: &Path) -> PathBuf {
    whiteouts.with_file_name("overlay-journal.json")
}

/// Writes the journal (whole or not at all: a temp file renamed over).
pub(crate) fn write(journal: &Path, steps: &[Step]) -> io::Result<()> {
    if let Some(parent) = journal.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = journal.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec(steps).map_err(io::Error::other)?)?;
    std::fs::rename(&temp, journal)
}

/// Removes the journal once its change is done.
pub(crate) fn clear(journal: &Path) -> io::Result<()> {
    match std::fs::remove_file(journal) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Does one step. `whiteouts` is the in-memory list (folded path to path as written).
pub(crate) fn run(step: &Step, whiteouts: &mut BTreeMap<String, String>) -> io::Result<()> {
    let ignore_missing = |result: io::Result<()>| match result {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    };
    match step {
        Step::MakeDir(dir) => std::fs::create_dir_all(dir),
        Step::Move { from, to } => {
            if std::fs::symlink_metadata(from).is_err() {
                return Ok(()); // done before
            }
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(from, to)
        }
        Step::Remove(file) => ignore_missing(std::fs::remove_file(file)),
        Step::RemoveDir(dir) => {
            // Not empty (a hidden file, something new): it stays, which is fine.
            let _ = std::fs::remove_dir(dir);
            Ok(())
        }
        Step::Whiteouts { add, remove } => {
            for path in remove {
                whiteouts.remove(&super::fold(path));
            }
            for path in add {
                whiteouts.insert(super::fold(path), path.clone());
            }
            Ok(())
        }
    }
}

/// Finishes a journal left by a crash: does all its steps, saves the whiteouts, removes it.
/// Called before a mount builds its tree.
pub(crate) fn replay(whiteout_file: &Path) -> io::Result<()> {
    let journal = path_for(whiteout_file);
    let bytes = match std::fs::read(&journal) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let steps: Vec<Step> = serde_json::from_slice(&bytes).map_err(|e| {
        io::Error::other(format!("{} is damaged ({e}): move it away to mount without it", journal.display()))
    })?;
    let mut whiteouts = super::read_whiteouts(whiteout_file)?;
    let before = whiteouts.clone();
    for step in &steps {
        run(step, &mut whiteouts)?;
    }
    if whiteouts != before {
        super::write_whiteouts(whiteout_file, &whiteouts)?;
    }
    clear(&journal)
}
