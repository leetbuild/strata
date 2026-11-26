//! Writing MO2's files safely (`docs/DESIGN.md`, "Rules for every file").
//!
//! - [`write_if_changed`]: one file, atomically (temp file in the same folder, `fsync`,
//!   `rename`), and only when the new bytes differ from what is on disk. MO2 does the same
//!   with `SafeWriteFile::commitIfDifferent` (`Profile::doWriteModlist`).
//! - [`Transaction`]: several files (and folder renames) that must change together, such as
//!   a mod rename (the folder plus `modlist.txt` of every profile). Every new file is
//!   written in full to a temp file first; then a journal naming each step is put in place
//!   with one atomic rename; then the steps run; then the journal goes. A crash before the
//!   journal exists changes nothing; a crash after it is finished by [`recover`], which
//!   every writing operation calls first. Steps are idempotent, so running them twice is
//!   harmless.
//!
//! The journal lives in `<instance>/strata/journal` (Strata's own folder).

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

fn write_error(path: &Path, source: std::io::Error) -> Error {
    Error::Write { path: path.to_owned(), source }
}

/// The temp file next to `target` that a new version is written to.
fn temp_path(target: &Path) -> PathBuf {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    target.with_file_name(format!(".{name}.strata-{}.tmp", std::process::id()))
}

/// `fsync` on a folder, so that a rename inside it survives a power cut.
fn sync_dir(dir: &Path) -> Result<()> {
    File::open(dir).and_then(|f| f.sync_all()).map_err(|e| write_error(dir, e))
}

/// Writes `data` to a new temp file next to `target` (same permissions as `target` when it
/// exists) and returns the temp file's path. The data is on disk when this returns.
fn stage(target: &Path, data: &[u8]) -> Result<PathBuf> {
    let temp = temp_path(target);
    let result = (|| {
        let mut file = File::create(&temp)?;
        file.write_all(data)?;
        if let Ok(meta) = fs::metadata(target) {
            file.set_permissions(meta.permissions())?;
        }
        file.sync_all()
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&temp);
        return Err(write_error(&temp, e));
    }
    Ok(temp)
}

/// Whether `target` already holds exactly `data`.
fn unchanged(target: &Path, data: &[u8]) -> bool {
    fs::read(target).is_ok_and(|current| current == data)
}

/// Writes `data` to `target` atomically, unless the file already holds those bytes.
/// Returns whether the file was written.
pub fn write_if_changed(target: &Path, data: &[u8]) -> Result<bool> {
    if unchanged(target, data) {
        return Ok(false);
    }
    let temp = stage(target, data)?;
    if let Err(e) = fs::rename(&temp, target) {
        let _ = fs::remove_file(&temp);
        return Err(write_error(target, e));
    }
    if let Some(dir) = target.parent() {
        sync_dir(dir)?;
    }
    Ok(true)
}

/// One step of a [`Transaction`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// Move a complete temp file over its target.
    Replace { temp: PathBuf, target: PathBuf },
    /// Rename a file or folder. Skipped when `from` is gone and `to` exists (done before).
    Rename { from: PathBuf, to: PathBuf },
}

impl Step {
    fn run(&self) -> Result<()> {
        match self {
            Step::Replace { temp, target } => {
                if temp.exists() {
                    fs::rename(temp, target).map_err(|e| write_error(target, e))?;
                }
            }
            Step::Rename { from, to } => {
                if from.exists() || fs::symlink_metadata(from).is_ok() {
                    if to.exists() && !same_name_other_case(from, to) {
                        return Err(Error::Exists { path: to.clone() });
                    }
                    rename_any_case(from, to)?;
                }
            }
        }
        Ok(())
    }

    fn line(&self) -> String {
        let (tag, a, b) = match self {
            Step::Replace { temp, target } => ("W", temp, target),
            Step::Rename { from, to } => ("R", from, to),
        };
        format!("{tag}\t{}\t{}\n", a.display(), b.display())
    }

    fn parse(line: &str) -> Option<Self> {
        let mut parts = line.split('\t');
        let (tag, a, b) = (parts.next()?, PathBuf::from(parts.next()?), PathBuf::from(parts.next()?));
        match tag {
            "W" => Some(Step::Replace { temp: a, target: b }),
            "R" => Some(Step::Rename { from: a, to: b }),
            _ => None,
        }
    }
}

/// Whether `a` and `b` are the same path apart from the case of the last part.
fn same_name_other_case(a: &Path, b: &Path) -> bool {
    a.parent() == b.parent()
        && a.file_name().map(|n| n.to_string_lossy().to_lowercase())
            == b.file_name().map(|n| n.to_string_lossy().to_lowercase())
}

/// Renames `from` to `to`, also when only the case changes (through a temp name, as
/// `ModInfoRegular::setName` does for case-insensitive file systems).
fn rename_any_case(from: &Path, to: &Path) -> Result<()> {
    if same_name_other_case(from, to) && from != to {
        let between = temp_path(to);
        fs::rename(from, &between).map_err(|e| write_error(from, e))?;
        return fs::rename(&between, to).map_err(|e| write_error(to, e));
    }
    fs::rename(from, to).map_err(|e| write_error(to, e))
}

/// Several changes that happen together or not at all (see the module doc).
#[derive(Debug)]
pub struct Transaction {
    journal: PathBuf,
    steps: Vec<Step>,
}

impl Transaction {
    /// A transaction journaled in `strata_dir` (`<instance>/strata`). Finishes any earlier
    /// transaction a crash interrupted.
    pub fn new(strata_dir: &Path) -> Result<Self> {
        recover(strata_dir)?;
        Ok(Self { journal: strata_dir.join("journal"), steps: Vec::new() })
    }

    /// Adds a file write. Nothing is added when the file already holds `data`. The data is
    /// written to a temp file now; the file itself changes at [`Transaction::commit`].
    pub fn write(&mut self, target: &Path, data: &[u8]) -> Result<()> {
        check_path(target)?;
        if unchanged(target, data) {
            return Ok(());
        }
        let temp = stage(target, data)?;
        self.steps.push(Step::Replace { temp, target: target.to_owned() });
        Ok(())
    }

    /// Adds a rename of a file or folder. Fails at commit when `to` exists (unless it is
    /// `from` in another case).
    pub fn rename(&mut self, from: &Path, to: &Path) -> Result<()> {
        check_path(from)?;
        check_path(to)?;
        if from != to {
            self.steps.push(Step::Rename { from: from.to_owned(), to: to.to_owned() });
        }
        Ok(())
    }

    /// Whether nothing would change.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Runs every step. A single step needs no journal: it is atomic by itself.
    pub fn commit(mut self) -> Result<()> {
        let steps = std::mem::take(&mut self.steps);
        // Refuse before anything changes, so a journal never holds a step that must fail.
        let blocked = steps.iter().find_map(|step| match step {
            Step::Rename { from, to } if to.exists() && !same_name_other_case(from, to) => Some(to.clone()),
            _ => None,
        });
        if let Some(path) = blocked {
            self.steps = steps;
            return Err(Error::Exists { path });
        }
        if steps.len() > 1 {
            let dir = self.journal.parent().unwrap_or(Path::new("."));
            fs::create_dir_all(dir).map_err(|e| write_error(dir, e))?;
            let text: String = steps.iter().map(Step::line).collect();
            write_if_changed(&self.journal, text.as_bytes())?;
        }
        for step in &steps {
            step.run()?;
        }
        for dir in steps.iter().filter_map(|s| match s {
            Step::Replace { target, .. } | Step::Rename { to: target, .. } => target.parent(),
        }) {
            sync_dir(dir)?;
        }
        if steps.len() > 1 {
            fs::remove_file(&self.journal).map_err(|e| write_error(&self.journal, e))?;
        }
        Ok(())
    }
}

impl Drop for Transaction {
    /// A transaction dropped without commit leaves nothing behind.
    fn drop(&mut self) {
        for step in &self.steps {
            if let Step::Replace { temp, .. } = step {
                let _ = fs::remove_file(temp);
            }
        }
    }
}

/// Journal lines are tab-separated: paths with tabs or line ends can't be journaled.
fn check_path(path: &Path) -> Result<()> {
    let text = path.to_string_lossy();
    if text.contains(['\t', '\n', '\r']) {
        return Err(Error::UnsafePath { path: path.to_owned() });
    }
    Ok(())
}

/// Finishes a transaction that a crash interrupted after its journal was written. Does
/// nothing (and writes nothing) when there is no journal.
pub fn recover(strata_dir: &Path) -> Result<()> {
    let journal = strata_dir.join("journal");
    let Ok(text) = fs::read_to_string(&journal) else { return Ok(()) };
    for step in text.lines().filter_map(Step::parse) {
        step.run()?;
    }
    fs::remove_file(&journal).map_err(|e| write_error(&journal, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_only_when_the_content_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("modlist.txt");
        assert!(write_if_changed(&file, b"a").unwrap());
        let before = fs::metadata(&file).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!write_if_changed(&file, b"a").unwrap());
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), before);
        assert!(write_if_changed(&file, b"b").unwrap());
        assert_eq!(fs::read(&file).unwrap(), b"b");
        // no temp file is left behind
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_transaction_changes_everything_and_leaves_no_journal() {
        let tmp = tempfile::tempdir().unwrap();
        let strata = tmp.path().join("strata");
        fs::create_dir(tmp.path().join("Old")).unwrap();
        fs::write(tmp.path().join("a.txt"), "old").unwrap();
        let mut t = Transaction::new(&strata).unwrap();
        t.write(&tmp.path().join("a.txt"), b"new").unwrap();
        t.write(&tmp.path().join("b.txt"), b"b").unwrap();
        t.rename(&tmp.path().join("Old"), &tmp.path().join("New")).unwrap();
        t.commit().unwrap();
        assert_eq!(fs::read(tmp.path().join("a.txt")).unwrap(), b"new");
        assert_eq!(fs::read(tmp.path().join("b.txt")).unwrap(), b"b");
        assert!(tmp.path().join("New").is_dir() && !tmp.path().join("Old").exists());
        assert!(!strata.join("journal").exists());
    }

    #[test]
    fn a_dropped_transaction_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), "old").unwrap();
        {
            let mut t = Transaction::new(&tmp.path().join("strata")).unwrap();
            t.write(&tmp.path().join("a.txt"), b"new").unwrap();
        }
        assert_eq!(fs::read(tmp.path().join("a.txt")).unwrap(), b"old");
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    /// A crash after the journal was written: the next transaction finishes the work.
    #[test]
    fn an_interrupted_transaction_is_finished_by_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let strata = tmp.path().join("strata");
        fs::create_dir_all(&strata).unwrap();
        fs::create_dir(tmp.path().join("Mod")).unwrap();
        fs::write(tmp.path().join("modlist.txt"), "+Mod").unwrap();
        let temp = temp_path(&tmp.path().join("modlist.txt"));
        fs::write(&temp, "+Renamed").unwrap();
        let steps = [
            Step::Rename { from: tmp.path().join("Mod"), to: tmp.path().join("Renamed") },
            Step::Replace { temp, target: tmp.path().join("modlist.txt") },
        ];
        // the folder rename ran, then the "crash"
        steps[0].run().unwrap();
        fs::write(strata.join("journal"), steps.iter().map(Step::line).collect::<String>()).unwrap();

        recover(&strata).unwrap();
        assert_eq!(fs::read(tmp.path().join("modlist.txt")).unwrap(), b"+Renamed");
        assert!(tmp.path().join("Renamed").is_dir() && !tmp.path().join("Mod").exists());
        assert!(!strata.join("journal").exists());
        // and recovering again is harmless
        recover(&strata).unwrap();
    }

    #[test]
    fn a_rename_onto_an_existing_folder_fails_but_a_case_change_works() {
        let tmp = tempfile::tempdir().unwrap();
        let strata = tmp.path().join("strata");
        for name in ["A", "B"] {
            fs::create_dir(tmp.path().join(name)).unwrap();
        }
        let mut t = Transaction::new(&strata).unwrap();
        t.rename(&tmp.path().join("A"), &tmp.path().join("B")).unwrap();
        assert!(matches!(t.commit(), Err(Error::Exists { .. })));

        let mut t = Transaction::new(&strata).unwrap();
        t.rename(&tmp.path().join("A"), &tmp.path().join("a")).unwrap();
        t.commit().unwrap();
        assert!(tmp.path().join("a").is_dir());
    }
}
