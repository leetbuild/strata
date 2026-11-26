//! The Downloads tab's actions on a downloaded archive and its `.meta` (MO2's
//! `DownloadManager`).
//!
//! - hide / unhide: `removeFile(deleteFile = false)` writes `removed=true`,
//!   `restoreDownload` writes `removed=false`;
//! - delete: `removeFile(deleteFile = true)`: the archive and its `.meta` to the trash
//!   (or for good);
//! - `markInstalled` (`installed=true`, `uninstalled=false`) is [`crate::meta`]'s;
//!   `markUninstalled` writes `uninstalled=true`;
//! - query info: MO2 takes the mod id from the file name (`interpretNexusFileName`), then asks
//!   Nexus. Offline, Strata stores what the file name gives: `modID`, `gameName` and the mod
//!   name guess, without overwriting values the `.meta` already has.

use std::path::Path;

use strata_core::Trash;
use strata_ini::{Document, Value};

use crate::meta::{meta_path, write_atomic};
use crate::name::interpret_nexus_file_name;
use crate::Result;

fn open_meta(archive: &Path) -> Result<Document> {
    let path = meta_path(archive);
    Ok(match std::fs::read(&path) {
        Ok(data) => Document::parse(&data),
        Err(_) => Document::parse(b"[General]\n"),
    })
}

fn set(archive: &Path, values: &[(&str, &str)]) -> Result<()> {
    let mut doc = open_meta(archive)?;
    for (key, value) in values {
        doc.set("", key, &Value::String((*value).to_owned()));
    }
    write_atomic(&meta_path(archive), &doc.to_bytes())
}

/// Hide (`removed=true`) or show again (`removed=false`).
pub fn set_hidden(archive: &Path, hidden: bool) -> Result<()> {
    set(archive, &[("removed", if hidden { "true" } else { "false" })])
}

/// `markUninstalled`.
pub fn mark_uninstalled(archive: &Path) -> Result<()> {
    set(archive, &[("uninstalled", "true")])
}

/// Deletes the archive and its `.meta`: to `trash`, or for good without one.
pub fn delete(archive: &Path, trash: Option<&Trash>) -> Result<()> {
    let meta = meta_path(archive);
    strata_core::trash::remove(archive, trash)?;
    if meta.exists() {
        strata_core::trash::remove(&meta, trash)?;
    }
    Ok(())
}

/// What [`query_info_offline`] found.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Queried {
    pub mod_id: Option<i64>,
    pub mod_name: String,
}

/// Query info without Nexus: the mod id and name the file name carries go into the `.meta`
/// where it has none (`gameName` gets the game's short name).
pub fn query_info_offline(archive: &Path, game_short_name: &str) -> Result<Queried> {
    let file = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (name, id) = interpret_nexus_file_name(&file);
    let mut doc = open_meta(archive)?;
    let empty = |doc: &Document, key: &str| doc.string("", key).is_none_or(|v| v.is_empty() || v == "0");
    if let Some(id) = id
        && empty(&doc, "modID")
    {
        doc.set("", "modID", &Value::String(id.to_string()));
    }
    if empty(&doc, "gameName") {
        doc.set("", "gameName", &Value::String(game_short_name.to_owned()));
    }
    let name = name.trim().to_owned();
    if !name.is_empty() && empty(&doc, "modName") {
        doc.set("", "modName", &Value::String(name.clone()));
    }
    write_atomic(&meta_path(archive), &doc.to_bytes())?;
    Ok(Queried { mod_id: id, mod_name: name })
}


#[cfg(test)]
mod tests {
    use super::*;

    fn text(p: &Path) -> String {
        std::fs::read_to_string(meta_path(p)).unwrap()
    }

    #[test]
    fn hide_unhide_uninstall_and_query() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("SkyUI_5_2_SE-12604-5-2SE.7z");
        std::fs::write(&a, "x").unwrap();
        set_hidden(&a, true).unwrap();
        assert_eq!(text(&a), "[General]\nremoved=true\n");
        set_hidden(&a, false).unwrap();
        mark_uninstalled(&a).unwrap();
        assert_eq!(text(&a), "[General]\nremoved=false\nuninstalled=true\n");
        let q = query_info_offline(&a, "SkyrimSE").unwrap();
        assert_eq!(q, Queried { mod_id: Some(12604), mod_name: "SkyUI_5_2_SE".into() });
        assert!(text(&a).ends_with("modID=12604\ngameName=SkyrimSE\nmodName=SkyUI_5_2_SE\n"));
        std::fs::write(meta_path(&a), "[General]\nmodID=1\nmodName=Mine\n").unwrap();
        query_info_offline(&a, "SkyrimSE").unwrap();
        assert!(text(&a).starts_with("[General]\nmodID=1\nmodName=Mine\n"), "nothing overwritten");
    }

    #[test]
    fn delete_to_the_trash() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.7z");
        std::fs::write(&a, "x").unwrap();
        std::fs::write(meta_path(&a), "[General]\n").unwrap();
        let trash = Trash::at(d.path().join("Trash"));
        delete(&a, Some(&trash)).unwrap();
        assert!(!a.exists() && !meta_path(&a).exists());
        assert!(d.path().join("Trash/files/a.7z").is_file() && d.path().join("Trash/files/a.7z.meta").is_file());
    }

}
