//! The Downloads tab: the archives in the downloads folder and their `.meta` files
//! (`DownloadManager::refreshList`, `DownloadList::data`). Read only.

use std::path::Path;
use std::time::UNIX_EPOCH;

use strata_install::meta::DownloadMeta;

/// One download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRow {
    pub file_name: String,
    /// The Name column with "meta downloads" on (`DownloadManager::getDisplayName`):
    /// `<name> (<file type>, v<version>)` when the `.meta` has a name, else the file name.
    pub display_name: String,
    /// `Installed`, `Uninstalled`, `Paused` or `Downloaded`.
    pub status: &'static str,
    pub size: u64,
    /// The archive's modification time, seconds since 1970.
    pub file_time: Option<i64>,
    pub mod_name: String,
    pub version: String,
    pub nexus_id: String,
    pub game: String,
    /// `removed=true`: hidden unless "Show Hidden" is ticked.
    pub hidden: bool,
}

/// The archive types a download can be: what MO2's archive library opens.
const EXTENSIONS: &[&str] = &["7z", "zip", "rar"];
/// A download still running or paused (`DownloadManager`'s `UNFINISHED`).
pub const UNFINISHED: &str = ".unfinished";

/// `DownloadManager::getFileTypeString`.
fn file_type(category: i64) -> &'static str {
    match category {
        1 => "Main",
        2 => "Update",
        3 => "Optional",
        4 => "Old",
        5 => "Miscellaneous",
        6 => "Deleted",
        7 => "Archived",
        _ => "Unknown",
    }
}

/// Every download in `dir`, by file name (any case). A missing folder is an empty list.
pub fn rows(dir: &Path) -> Vec<DownloadRow> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut rows: Vec<DownloadRow> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let name = name.strip_suffix(UNFINISHED).unwrap_or(&name);
            EXTENSIONS.iter().any(|ext| name.ends_with(&format!(".{ext}")))
        })
        .filter_map(|e| {
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            let file_name = e.file_name().to_string_lossy().into_owned();
            // the install area's reader of `<archive>.meta` (the name is plain text already)
            let m = DownloadMeta::read_for(&e.path()).ok().flatten().unwrap_or_default();
            // DownloadManager::refreshList, in this order.
            // an unfinished download is paused whatever its .meta says (MO2 lists it under
            // its archive name; here the file keeps its suffix, which the actions need)
            let status = if m.paused || file_name.ends_with(UNFINISHED) {
                "Paused"
            } else if m.uninstalled {
                "Uninstalled"
            } else if m.installed {
                "Installed"
            } else {
                "Downloaded"
            };
            let display_name = if m.name.is_empty() {
                file_name.clone()
            } else {
                format!("{} ({}, v{})", m.name, file_type(m.file_category), m.version)
            };
            Some(DownloadRow {
                display_name,
                status,
                size: meta.len(),
                file_time: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .and_then(|d| i64::try_from(d.as_secs()).ok()),
                mod_name: m.mod_name,
                nexus_id: if m.mod_id > 0 { m.mod_id.to_string() } else { String::new() },
                game: m.game_name,
                hidden: m.removed,
                version: m.version,
                file_name,
            })
        })
        .collect();
    rows.sort_by_cached_key(|r| r.file_name.to_lowercase());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_and_their_meta_files_are_read_as_mo2_does() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::write(d.join("B-1-2.zip"), b"12345").unwrap();
        std::fs::write(
            d.join("B-1-2.zip.meta"),
            "[General]\nname=Whiterun Expansion\nfileCategory=1\nversion=1.5.0.0\nmodName=Capital\nmodID=37982\ninstalled=true\nuninstalled=true\nremoved=false\n",
        )
        .unwrap();
        std::fs::write(d.join("a.7z"), b"").unwrap();
        std::fs::write(d.join("a.7z.meta"), "[General]\ninstalled=true\nremoved=true\n").unwrap();
        std::fs::write(d.join("notes.txt"), b"").unwrap();
        std::fs::write(d.join("c.rar"), b"").unwrap();

        let rows = rows(d);
        let names: Vec<_> = rows.iter().map(|r| r.file_name.as_str()).collect();
        assert_eq!(names, ["a.7z", "B-1-2.zip", "c.rar"]);
        assert_eq!((rows[0].status, rows[0].hidden, rows[0].display_name.as_str()), ("Installed", true, "a.7z"));
        assert_eq!(rows[1].status, "Uninstalled", "uninstalled is checked before installed");
        assert_eq!(rows[1].display_name, "Whiterun Expansion (Main, v1.5.0.0)");
        assert_eq!((rows[1].size, rows[1].mod_name.as_str(), rows[1].nexus_id.as_str()), (5, "Capital", "37982"));
        assert_eq!(rows[2].status, "Downloaded");
    }

    #[test]
    fn unfinished_downloads_are_listed_paused() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::write(d.join("Big-5-1.7z.unfinished"), b"part").unwrap();
        std::fs::write(d.join("Big-5-1.7z.unfinished.meta"), "[General]\nname=Big\nversion=1\nfileCategory=1\npaused=false\n").unwrap();
        std::fs::write(d.join("x.txt.unfinished"), b"").unwrap();
        let rows = rows(d);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].file_name.as_str(), rows[0].status), ("Big-5-1.7z.unfinished", "Paused"));
        assert_eq!(rows[0].display_name, "Big (Main, v1)");
    }
}
