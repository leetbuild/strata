//! Managing Overwrite (`ModListViewActions::clearOverwrite`, `createModFromOverwrite`,
//! `moveOverwriteContentToExistingMod`, `moveOverwriteContentsTo`).
//!
//! Skyrim SE has the single default mod mapping, so the whole of Overwrite is cleared or
//! moved. The caller makes sure nothing is mounted while this runs.

use std::path::Path;

use crate::events::ModListEvent;
use crate::trash::{self, Trash};
use crate::{Error, Instance, Result};

/// Overwrite's top-level entries.
fn entries(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut list: Vec<_> = std::fs::read_dir(dir)
        .map_err(|source| Error::Read { path: dir.to_owned(), source })?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    list.sort();
    Ok(list)
}

impl Instance {
    /// Whether Overwrite holds anything (MO2 shows its actions only then).
    pub fn overwrite_is_empty(&self) -> bool {
        entries(&self.paths.overwrite).map_or(true, |e| e.is_empty())
    }

    /// `clearOverwrite`: every entry of Overwrite to the trash (or for good with `None`),
    /// and Strata's whiteouts (`strata/whiteouts.txt`): deletions are run-time changes too.
    pub fn clear_overwrite(&self, trash: Option<&Trash>) -> Result<Vec<ModListEvent>> {
        for entry in entries(&self.paths.overwrite)? {
            trash::remove(&entry, trash)?;
        }
        let whiteouts = self.strata_dir().join("whiteouts.txt");
        if whiteouts.exists() {
            trash::remove(&whiteouts, None)?;
        }
        Ok(vec![ModListEvent::OverwriteChanged])
    }

    /// `moveOverwriteContentsTo`: moves Overwrite's content into the mod folder `target`,
    /// merging folders. A file that exists in both is replaced by Overwrite's (it was the
    /// one the game saw). Whiteouts stay: a mod can't record a deletion.
    pub fn move_overwrite_to(&self, target: &Path) -> Result<Vec<ModListEvent>> {
        if !target.is_dir() {
            return Err(Error::Refused { what: format!("{} is not a mod folder", target.display()) });
        }
        for entry in entries(&self.paths.overwrite)? {
            let name = entry.file_name().unwrap_or_default();
            merge_move(&entry, &target.join(name))?;
        }
        Ok(vec![ModListEvent::OverwriteChanged])
    }
}

/// Moves `from` to `to`; folders merge into existing folders, files replace files.
fn merge_move(from: &Path, to: &Path) -> Result<()> {
    let from_dir = std::fs::symlink_metadata(from).is_ok_and(|m| m.is_dir());
    let to_meta = std::fs::symlink_metadata(to).ok();
    match (from_dir, to_meta) {
        (true, Some(m)) if m.is_dir() => {
            for entry in entries(from)? {
                let name = entry.file_name().unwrap_or_default();
                merge_move(&entry, &to.join(name))?;
            }
            std::fs::remove_dir(from).map_err(|source| Error::Write { path: from.to_owned(), source })
        }
        (false, Some(m)) if m.is_dir() => Err(Error::Refused {
            what: format!("{} is a file in Overwrite but a folder in the mod; move it by hand", from.display()),
        }),
        (true, Some(_)) => Err(Error::Refused {
            what: format!("{} is a folder in Overwrite but a file in the mod; move it by hand", from.display()),
        }),
        _ => std::fs::rename(from, to).map_err(|source| Error::Write { path: to.to_owned(), source }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_merges_folders_and_replaces_files() {
        let tmp = tempfile::tempdir().unwrap();
        let (ow, m) = (tmp.path().join("overwrite"), tmp.path().join("mod"));
        for (path, text) in [("overwrite/SKSE/Plugins/a.ini", "new"), ("overwrite/top.txt", "t"), ("mod/SKSE/Plugins/a.ini", "old"), ("mod/SKSE/Plugins/b.ini", "b")] {
            let p = tmp.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        for entry in entries(&ow).unwrap() {
            merge_move(&entry, &m.join(entry.file_name().unwrap())).unwrap();
        }
        assert!(entries(&ow).unwrap().is_empty());
        assert_eq!(std::fs::read(m.join("SKSE/Plugins/a.ini")).unwrap(), b"new");
        assert_eq!(std::fs::read(m.join("SKSE/Plugins/b.ini")).unwrap(), b"b");
        assert_eq!(std::fs::read(m.join("top.txt")).unwrap(), b"t");
    }
}
