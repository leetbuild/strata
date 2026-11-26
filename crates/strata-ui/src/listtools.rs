//! Three entries of the mod list's options menu and Overwrite's context menu:
//!
//! - "Export to csv..." (`ModListViewActions::exportModListCSV` with MO2's `CSVBuilder`),
//! - "Auto assign categories" (`ModListViewActions::assignCategories`),
//! - "Sync to Mods..." (`OrganizerCore::syncOverwrite`, `SyncOverwriteDialog`).

use std::path::{Path, PathBuf};

use strata_core::{Instance, ModKind, ModList, ModMeta};
use strata_vfs::conflicts::Area;

use crate::mods::{ModRow, RowKind};
use crate::Result;

/// Which mods the CSV holds (the dialog's three radio buttons).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsvRows {
    All,
    /// "Only active (checked) mods from your current profile".
    Active,
    /// "All currently visible mods in the mod list": the names the window passes.
    Visible,
}

/// The columns, in the order MO2 writes them (not the order of its checkboxes).
pub const CSV_FIELDS: [&str; 10] = [
    "#Mod_Priority",
    "#Mod_Status",
    "#Mod_Name",
    "#Note",
    "#Primary_Category",
    "#Nexus_ID",
    "#Mod_Nexus_URL",
    "#Mod_Version",
    "#Install_Date",
    "#Download_File_Name",
];

/// A mod's folder, when it has one.
fn mod_dir(instance: &Instance, name: &str) -> Option<PathBuf> {
    let dir = instance.paths.mods.join(name);
    (!name.is_empty() && !name.contains('/') && dir.is_dir()).then_some(dir)
}

/// `CSVBuilder` with `TYPE_STRING` quoted always: `"` doubled inside.
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// The CSV MO2 makes: a header of the chosen field names (unquoted), then one row per mod by
/// ascending priority (Overwrite and backups left out), strings quoted, the Nexus ID bare,
/// CRLF line ends. `fields` are names of [`CSV_FIELDS`]; others are ignored.
pub fn export_csv(instance: &Instance, rows: &[ModRow], which: CsvRows, visible: &[String], fields: &[String]) -> String {
    let chosen: Vec<&str> = CSV_FIELDS.iter().copied().filter(|f| fields.iter().any(|c| c == f)).collect();
    let mut out = chosen.join(",");
    out.push_str("\r\n");
    let mut by_priority: Vec<&ModRow> =
        rows.iter().filter(|r| r.priority.is_some() && !matches!(r.kind, RowKind::Overwrite | RowKind::Backup)).collect();
    by_priority.sort_by_key(|r| r.priority);
    for row in by_priority {
        let enabled = row.enabled || row.kind == RowKind::Foreign;
        match which {
            CsvRows::Active if !enabled => continue,
            CsvRows::Visible if !visible.iter().any(|v| v == &row.name) => continue,
            _ => {}
        }
        let meta = || mod_dir(instance, &row.name).and_then(|d| ModMeta::open(&d).ok());
        let cells: Vec<String> = chosen
            .iter()
            .map(|field| match *field {
                "#Mod_Priority" => quote(&format!("{:04}", row.priority.unwrap_or(0))),
                "#Mod_Status" => quote(if enabled { "+" } else { "-" }),
                "#Mod_Name" => quote(&row.name),
                "#Note" => quote(&row.comments.replace(',', "")),
                // foreign mods have no primary category ("Non-MO" is only what the column shows)
                "#Primary_Category" => quote(if row.kind == RowKind::Foreign { "" } else { &row.category }),
                "#Nexus_ID" => row.nexus_id.unwrap_or(0).to_string(),
                "#Mod_Nexus_URL" => quote(&match row.nexus_id {
                    Some(id) => format!("https://www.nexusmods.com/{}/mods/{id}", instance.game.nexus_name()),
                    None => String::new(),
                }),
                "#Mod_Version" => quote(&strata_install::version::canonical(&row.version)),
                "#Install_Date" => quote(&row.install_time.map_or(String::new(), |t| {
                    let l = strata_core::LocalTime::at(t);
                    format!("{:04}/{:02}/{:02} {:02}:{:02}:{:02}", l.year, l.month, l.day, l.hour, l.minute, l.second)
                })),
                "#Download_File_Name" => quote(&meta().map(|m| m.installation_file()).unwrap_or_default()),
                _ => String::new(),
            })
            .collect();
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

/// "Auto assign categories": every regular mod whose Nexus category (`nexusCategory`, else
/// `category` of its download's `.meta`) maps to an existing instance category through
/// `nexuscatmap.dat` gets that category alone, as primary. MO2 also adds category 0
/// ("None") to mods it can't map; Strata leaves those as they are. Returns the mods changed.
pub fn auto_assign_categories(instance: &Instance, list: &ModList) -> Result<Vec<String>> {
    let mut changed = Vec::new();
    for entry in list.entries() {
        let (ModKind::Regular, Some(dir)) = (entry.kind, entry.path.as_ref()) else { continue };
        let Ok(mut meta) = ModMeta::open(dir) else { continue };
        let mut nexus = meta.nexus_category();
        if nexus == 0 {
            let file = meta.installation_file();
            if !file.is_empty() {
                let path = instance.paths.downloads.join(format!("{file}.meta"));
                nexus = std::fs::read(&path)
                    .ok()
                    .and_then(|d| strata_ini::Document::parse(&d).string("", "category"))
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(0);
            }
        }
        let id = strata_install::install::nexus_category(&instance.root, nexus);
        if id == 0 {
            continue;
        }
        let id = i32::try_from(id).unwrap_or(0);
        if meta.categories() == [id] && meta.primary_category() == Some(id) {
            continue;
        }
        meta.set_categories(Some(id), &[]);
        meta.save()?;
        changed.push(entry.name.clone());
    }
    Ok(changed)
}

/// One file of Overwrite and the mods it could go back to (`SyncOverwriteDialog::readTree`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncFile {
    /// Relative to Overwrite, `/`-separated (`Root/...` for the game folder's files).
    pub path: String,
    /// The enabled mods that have this file as a loose file, the one that would win without
    /// Overwrite first (MO2's default choice). Empty: "<don't sync>" only.
    pub mods: Vec<String>,
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if rel.is_empty() && name == "meta.ini" {
            continue;
        }
        let path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            walk(&e.path(), &path, out);
        } else {
            out.push(path);
        }
    }
}

/// `path` under `base` with each part matched without case, when it exists.
fn find_without_case(base: &Path, path: &str) -> Option<PathBuf> {
    let mut at = base.to_owned();
    for part in path.split('/') {
        let direct = at.join(part);
        at = if direct.symlink_metadata().is_ok() {
            direct
        } else {
            std::fs::read_dir(&at)
                .ok()?
                .filter_map(|e| e.ok())
                .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?
                .path()
        };
    }
    at.is_file().then_some(at)
}

/// The data folder's name in a mod path (`Data/x` in the merged game folder is `x` in a mod).
fn area_of(path: &str) -> (Area, &str) {
    match path.split_once('/') {
        Some((first, rest)) if first.eq_ignore_ascii_case("Root") => (Area::Root, rest),
        _ => (Area::Data, path),
    }
}

/// Every file in Overwrite with the mods that provide it.
pub fn sync_files(instance: &Instance, profile: &strata_core::Profile) -> Result<Vec<SyncFile>> {
    let mut paths = Vec::new();
    walk(&instance.paths.overwrite, "", &mut paths);
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let index = crate::conflicts::build_index(instance, profile)?;
    Ok(paths
        .into_iter()
        .map(|path| {
            let (area, inner) = area_of(&path);
            let mut mods: Vec<String> = index
                .origins(area, inner)
                .unwrap_or_default()
                .iter()
                .rev() // winner first
                .filter(|p| p.archive.is_none() && !p.layer.starts_with('<'))
                .map(|p| p.layer.to_owned())
                .filter(|m| mod_dir(instance, m).is_some_and(|d| find_without_case(&d, &path).is_some()))
                .collect();
            mods.dedup();
            SyncFile { path, mods }
        })
        .collect())
}

/// `SyncOverwriteDialog::applyTo`: each file moves from Overwrite onto the same file in the
/// chosen mod (its spelling there kept); folders of Overwrite left empty are removed (the
/// `Root` folder stays). Returns how many files moved and the errors, one per failed file.
pub fn sync_apply(instance: &Instance, moves: &[(String, String)]) -> (usize, Vec<String>) {
    let mut moved = 0;
    let mut errors = Vec::new();
    for (path, name) in moves {
        if path.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
            errors.push(format!("{path}: not a path inside overwrite"));
            continue;
        }
        let source = instance.paths.overwrite.join(path);
        let Some(target) = mod_dir(instance, name).and_then(|d| find_without_case(&d, path)) else {
            errors.push(format!("{path}: {name} has no such file"));
            continue;
        };
        match std::fs::rename(&source, &target) {
            Ok(()) => moved += 1,
            Err(e) => errors.push(format!("failed to move {} to {}: {e}", source.display(), target.display())),
        }
        // the folders the file was in, when they are empty now
        let mut dir = source.parent().map(Path::to_path_buf);
        while let Some(d) = dir {
            if d == instance.paths.overwrite || d == instance.paths.overwrite.join("Root") || std::fs::remove_dir(&d).is_err() {
                break;
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    (moved, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_instance::TestInstance;

    #[test]
    fn csv_as_mo2_writes_it() {
        let t = TestInstance::new("+a2\r\n+a1\r\n");
        t.meta("a1", "[General]\r\nversion=1.2\r\ncomments=\"hi, you\"\r\nmodid=42\r\n");
        let mut s = crate::Session::open(&t.root, None).unwrap();
        let rows = s.mods();
        let fields: Vec<String> = ["#Mod_Name", "#Mod_Priority", "#Nexus_ID", "#Note", "#Mod_Nexus_URL", "#Mod_Version"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let csv = export_csv(s.instance(), &rows, CsvRows::All, &[], &fields);
        let mut lines = csv.split("\r\n");
        assert_eq!(lines.next().unwrap(), "#Mod_Priority,#Mod_Name,#Note,#Nexus_ID,#Mod_Nexus_URL,#Mod_Version");
        let a1 = csv.lines().find(|l| l.contains("\"a1\"")).unwrap();
        assert!(a1.ends_with(",\"a1\",\"hi you\",42,\"https://www.nexusmods.com/skyrimspecialedition/mods/42\",\"1.2.0.0\""), "{a1}");
        assert!(!csv.contains("Overwrite"));
        assert_eq!(quote("5\" figure"), "\"5\"\" figure\"");
        let visible = export_csv(s.instance(), &rows, CsvRows::Visible, &["a1".into()], &fields);
        assert_eq!(visible.lines().count(), 2);
    }

    #[test]
    fn auto_assign_maps_nexus_categories() {
        let t = TestInstance::new("+a2\r\n+a1\r\n");
        std::fs::write(t.root.as_path().join("categories.dat"), "1|Animations|51|0\n7|Patches|79|0\n").unwrap();
        std::fs::write(t.root.as_path().join("nexuscatmap.dat"), "1|Animation|51\n7|Patches|79\n").unwrap();
        t.meta("a1", "[General]\r\nnexusCategory=79\r\ncategory=\"1,\"\r\n");
        t.meta("a2", "[General]\r\nnexusCategory=999\r\ncategory=\"1,\"\r\n");
        let s = crate::Session::open(&t.root, None).unwrap();
        let changed = auto_assign_categories(s.instance(), s.mod_list()).unwrap();
        assert_eq!(changed, ["a1"]);
        let m = ModMeta::open(&mod_dir(s.instance(), "a1").unwrap()).unwrap();
        assert_eq!((m.categories(), m.primary_category()), (vec![7], Some(7)));
        assert_eq!(auto_assign_categories(s.instance(), s.mod_list()).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn overwrite_files_go_back_to_the_mods_that_have_them() {
        let t = TestInstance::new("+a2\r\n+a1\r\n");
        let ow = t.root.as_path().join("overwrite");
        std::fs::create_dir_all(ow.join("SKSE/Plugins")).unwrap();
        std::fs::write(ow.join("SKSE/Plugins/a.ini"), "new").unwrap();
        std::fs::write(ow.join("lonely.txt"), "x").unwrap();
        std::fs::write(ow.join("meta.ini"), "").unwrap();
        let a1 = t.root.as_path().join("mods/a1");
        std::fs::create_dir_all(a1.join("skse/plugins")).unwrap();
        std::fs::write(a1.join("skse/plugins/A.ini"), "old").unwrap();
        let s = crate::Session::open(&t.root, None).unwrap();
        let files = sync_files(s.instance(), s.profile()).unwrap();
        let get = |p: &str| files.iter().find(|f| f.path == p).unwrap().mods.clone();
        assert_eq!(files.len(), 2, "{files:?}");
        assert_eq!(get("SKSE/Plugins/a.ini"), ["a1"]);
        assert!(get("lonely.txt").is_empty());
        let (moved, errors) = sync_apply(s.instance(), &[("SKSE/Plugins/a.ini".into(), "a1".into())]);
        assert_eq!((moved, errors.len()), (1, 0));
        assert_eq!(std::fs::read_to_string(a1.join("skse/plugins/A.ini")).unwrap(), "new");
        assert!(!ow.join("SKSE").exists(), "emptied folders are removed");
        assert!(ow.join("lonely.txt").exists());
    }
}
