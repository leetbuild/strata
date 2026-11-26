//! Backups of profile files, as the profile row's backup buttons make them
//! (`MainWindow::createBackup`, `on_saveModsButton_clicked`, `on_saveButton_clicked`,
//! `queryRestore`, `on_restore*Button_clicked`).
//!
//! A backup is a copy named `<file>.yyyy_MM_dd_hh_mm_ss` next to the file; the 10 newest
//! (by name) are kept, older ones are deleted for good (`removeOldFiles`, no recycling).

use std::path::{Path, PathBuf};

use crate::time::LocalTime;
use crate::{Error, Result, write::write_if_changed};

/// How many backups of one file MO2 keeps.
pub const BACKUPS_KEPT: usize = 10;

/// The files the "backup load order" button copies, in its order.
pub const PLUGIN_FILES: [&str; 3] = ["plugins.txt", "loadorder.txt", "lockedorder.txt"];

/// One backup of a file, as the restore dialog lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    /// What follows `<file>.`: a date (`2026_09_16_21_35_38`) or any other text
    /// (`before-strata-probe`).
    pub suffix: String,
    /// Whether the suffix is MO2's date form.
    pub dated: bool,
}

/// `PATTERN_BACKUP_GLOB` (`.????_??_??_??_??_??`) and `PATTERN_BACKUP_REGEX` (digits).
fn is_date_suffix(suffix: &str, digits_only: bool) -> bool {
    let b = suffix.as_bytes();
    b.len() == 19
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 | 10 | 13 | 16 => *c == b'_',
            _ => !digits_only || c.is_ascii_digit(),
        })
}

/// `MainWindow::createBackup`: copies `file` to `<file>.<suffix>` and keeps the newest
/// [`BACKUPS_KEPT`]. Returns the copy's path.
pub fn create_backup(file: &Path, suffix: &str) -> Result<PathBuf> {
    let data = std::fs::read(file).map_err(|source| Error::Read { path: file.to_owned(), source })?;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let copy = file.with_file_name(format!("{name}.{suffix}"));
    write_if_changed(&copy, &data)?;

    let dir = file.parent().unwrap_or(Path::new("."));
    let mut old: Vec<String> = std::fs::read_dir(dir)
        .map_err(|source| Error::Read { path: dir.to_owned(), source })?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.strip_prefix(&name).and_then(|r| r.strip_prefix('.')).is_some_and(|s| is_date_suffix(s, false)))
        .collect();
    old.sort();
    let extra = old.len().saturating_sub(BACKUPS_KEPT);
    for gone in &old[..extra] {
        let path = dir.join(gone);
        std::fs::remove_file(&path).map_err(|source| Error::Write { path, source })?;
    }
    Ok(copy)
}

/// A suffix for backups made now.
pub fn now_suffix() -> String {
    LocalTime::now().backup_suffix()
}

/// `queryRestore`: every `<file>.*` next to the file, newest name first.
pub fn list_backups(file: &Path) -> Result<Vec<Backup>> {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = file.parent().unwrap_or(Path::new("."));
    let mut found: Vec<Backup> = std::fs::read_dir(dir)
        .map_err(|source| Error::Read { path: dir.to_owned(), source })?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let file_name = e.file_name().to_string_lossy().into_owned();
            let suffix = file_name.strip_prefix(&name)?.strip_prefix('.')?.to_owned();
            (!suffix.is_empty()).then(|| Backup { dated: is_date_suffix(&suffix, true), suffix })
        })
        .collect();
    found.sort_by(|a, b| b.suffix.cmp(&a.suffix));
    Ok(found)
}

/// Copies `<file>.<suffix>` over `file` (atomically).
pub fn restore_backup(file: &Path, suffix: &str) -> Result<()> {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let copy = file.with_file_name(format!("{name}.{suffix}"));
    let data = std::fs::read(&copy).map_err(|source| Error::Read { path: copy.clone(), source })?;
    write_if_changed(file, &data).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backups_are_named_listed_trimmed_and_restored() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("modlist.txt");
        std::fs::write(&file, "v1").unwrap();
        std::fs::write(tmp.path().join("modlist.txt.before-strata-probe"), "x").unwrap();
        for i in 0..12 {
            create_backup(&file, &format!("2026_09_16_21_35_{i:02}")).unwrap();
        }
        let list = list_backups(&file).unwrap();
        let dated: Vec<_> = list.iter().filter(|b| b.dated).map(|b| b.suffix.as_str()).collect();
        assert_eq!(dated.len(), BACKUPS_KEPT);
        assert_eq!(dated[0], "2026_09_16_21_35_11", "newest first");
        assert_eq!(*dated.last().unwrap(), "2026_09_16_21_35_02", "the two oldest went");
        assert!(list.iter().any(|b| b.suffix == "before-strata-probe" && !b.dated), "other backups are listed too, never removed");

        std::fs::write(&file, "v2").unwrap();
        restore_backup(&file, "2026_09_16_21_35_05").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"v1");
        assert!(now_suffix().len() == 19 && is_date_suffix(&now_suffix(), true));
    }
}
