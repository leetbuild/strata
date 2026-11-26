//! The metadata of an install: the download's `<archive>.meta` (read, then marked
//! installed) and the mod's `meta.ini` (written as MO2's install writes it).
//!
//! MO2's sources: `InstallationManager::install` (reading `.meta`), `doInstall` (the keys
//! written to `meta.ini`), `ModInfoRegular::addInstalledFile` + `saveMeta` (the
//! `[installedFiles]` array), `DownloadManager::markInstalled`, the installers'
//! `onInstallationEnd` (`[Plugins]` settings, `url`).
//!
//! After an install [`ModMeta::save_meta`] does what MO2's `saveMeta` does when the new mod
//! is saved: every key it knows written back in its canonical form (`version` as
//! `VersionInfo::canonicalString`, `category` as a list), the missing ones appended with
//! `readMeta`'s defaults.

use std::path::Path;

use strata_ini::{Document, Value};

use crate::{Error, Result};

/// What a download's `.meta` says (the keys `InstallationManager::install` and
/// `OrganizerCore::installArchive` read).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadMeta {
    pub game_name: String,
    pub mod_id: i64,
    pub file_id: i64,
    /// `name`, HTML in MO2 (`QTextDocument::setHtml`), here as plain text.
    pub name: String,
    pub mod_name: String,
    pub version: String,
    pub newest_version: String,
    pub category: i64,
    pub repository: String,
    pub file_category: i64,
    /// The download states `DownloadManager::refreshList` reads (Qt's `toBool`).
    pub paused: bool,
    pub installed: bool,
    pub uninstalled: bool,
    /// Hidden from the Downloads tab.
    pub removed: bool,
}

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| Error::io(path, e))
}

/// Writes `bytes` to `path` atomically, only when the content differs, keeping the file's
/// permissions (`strata_core::write`; `docs/ARCHITECTURE.md`, "Writes").
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    strata_core::write::write_if_changed(path, bytes)?;
    Ok(())
}

/// The plain text of the bit of HTML Nexus puts in `name` (entities and tags).
fn plain_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    for (entity, c) in [("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&#39;", "'"), ("&apos;", "'"), ("&nbsp;", " "), ("&amp;", "&")] {
        out = out.replace(entity, c);
    }
    out
}

impl DownloadMeta {
    /// Reads `<archive>.meta`; `None` when there is none.
    pub fn read_for(archive: &Path) -> Result<Option<Self>> {
        let path = meta_path(archive);
        if !path.is_file() {
            return Ok(None);
        }
        let doc = Document::parse(&read(&path)?);
        let s = |key| doc.string("", key).unwrap_or_default();
        let flag = |key| doc.value("", key).is_some_and(|v| v.to_bool());
        let n = |key, default| doc.string("", key).and_then(|v| v.trim().parse().ok()).unwrap_or(default);
        Ok(Some(Self {
            game_name: s("gameName"),
            mod_id: n("modID", 0),
            file_id: n("fileID", 0),
            name: plain_text(&s("name")),
            mod_name: s("modName"),
            version: s("version"),
            newest_version: s("newestVersion"),
            category: n("category", 0),
            repository: s("repository"),
            file_category: n("fileCategory", 1),
            paused: flag("paused"),
            installed: flag("installed"),
            uninstalled: flag("uninstalled"),
            removed: flag("removed"),
        }))
    }
}

/// `<archive>.meta`.
pub fn meta_path(archive: &Path) -> std::path::PathBuf {
    let mut name = archive.as_os_str().to_owned();
    name.push(".meta");
    name.into()
}

/// `DownloadManager::markInstalled`: `installed=true`, `uninstalled=false` in the `.meta`
/// (MO2 creates the file when the download had none).
pub fn mark_download_installed(archive: &Path) -> Result<()> {
    let path = meta_path(archive);
    let mut doc = if path.is_file() { Document::parse(&read(&path)?) } else { Document::parse(b"[General]\n") };
    doc.set("", "installed", &Value::String("true".into()));
    doc.set("", "uninstalled", &Value::String("false".into()));
    write_atomic(&path, &doc.to_bytes())
}

/// The values `doInstall` writes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallMeta {
    pub game_name: String,
    pub mod_id: i64,
    pub version: String,
    pub newest_version: String,
    pub category: i64,
    pub file_category: i64,
    /// The archive's file name when it is in the downloads folder, else its full path.
    pub installation_file: String,
    pub repository: String,
    /// A page set by the installer (ModPub); written with `hasCustomURL=true` unless the
    /// mod already has a custom URL.
    pub page_url: String,
}

/// `QVariant(QColor())` as `QSettings` writes it: type 67, spec 0 (invalid), alpha 0xffff,
/// then zeros.
const INVALID_COLOR: &[u8] = br"@Variant(\0\0\0\x43\0\xff\xff\0\0\0\0\0\0\0\0)";

/// `QDateTime::currentDateTimeUtc().toString(Qt::ISODate)`: `2026-11-25T08:00:00Z`.
fn utc_now_iso() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let t = libc::time_t::try_from(now).unwrap_or(0);
    // SAFETY: `tm` is plain data; `gmtime_r` only writes it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::gmtime_r(&t, &mut tm) };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

fn string(v: impl Into<String>) -> Value {
    Value::String(v.into())
}

/// The mod's `meta.ini`, opened for the writes an install does. Unknown keys are kept.
pub struct ModMeta {
    path: std::path::PathBuf,
    doc: Document,
}

impl ModMeta {
    pub fn open(mod_dir: &Path) -> Result<Self> {
        let path = mod_dir.join("meta.ini");
        let doc = if path.is_file() { Document::parse(&read(&path)?) } else { Document::parse(b"[General]\n") };
        Ok(Self { path, doc })
    }

    /// `doInstall`'s writes. Its `merge` flag is never set in MO2 2.5.2 (`bool merge =
    /// false;`), so the version is written whenever there is one and `[installedFiles]` is
    /// cleared even when the files were merged into an existing mod.
    pub fn write_install(&mut self, m: &InstallMeta) {
        let d = &mut self.doc;
        if !m.game_name.is_empty() || d.get("", "gameName").is_none() {
            d.set("", "gameName", &string(m.game_name.clone()));
        }
        if m.mod_id != 0 || d.get("", "modid").is_none() {
            d.set("", "modid", &string(m.mod_id.to_string()));
        }
        if d.get("", "version").is_none() || !m.version.is_empty() {
            d.set("", "version", &string(m.version.clone()));
        }
        if !m.newest_version.is_empty() || d.get("", "newestVersion").is_none() {
            d.set("", "newestVersion", &string(m.newest_version.clone()));
        }
        // issue #51: never overwrite categories set by hand
        if d.get("", "category").is_none() {
            d.set("", "category", &string(m.category.to_string()));
        }
        d.set("", "nexusFileStatus", &string(m.file_category.to_string()));
        d.set("", "installationFile", &string(m.installation_file.clone()));
        d.set("", "repository", &string(m.repository.clone()));
        let custom = d.value("", "hasCustomURL").is_some_and(|v| v.to_bool());
        if !m.page_url.is_empty() && !custom {
            d.set("", "url", &string(m.page_url.clone()));
            d.set("", "hasCustomURL", &string("true"));
        }
        self.clear_section("installedFiles");
    }

    fn clear_section(&mut self, section: &str) {
        let keys: Vec<String> = self.doc.entries().filter(|e| e.section == section).map(|e| e.key.to_owned()).collect();
        for key in keys {
            self.doc.remove(section, &key);
        }
    }

    /// `ModInfoRegular::addInstalledFile` + the `[installedFiles]` array of `saveMeta`.
    pub fn add_installed_file(&mut self, mod_id: i64, file_id: i64) {
        let mut pairs: Vec<(String, String)> = Vec::new();
        let size: usize = self.doc.string("installedFiles", "size").and_then(|s| s.parse().ok()).unwrap_or(0);
        for i in 1..=size {
            let get = |k: &str| self.doc.string("installedFiles", &format!("{i}/{k}")).unwrap_or_default();
            pairs.push((get("modid"), get("fileid")));
        }
        let new = (mod_id.to_string(), file_id.to_string());
        if !pairs.contains(&new) {
            pairs.push(new);
        }
        // m_InstalledFileIDs is a std::set of (modid, fileid)
        pairs.sort_by_key(|(m, f)| (m.parse::<i64>().unwrap_or(0), f.parse::<i64>().unwrap_or(0)));
        self.clear_section("installedFiles");
        for (i, (m, f)) in pairs.iter().enumerate() {
            self.doc.set("installedFiles", &format!("{}/modid", i + 1), &string(m.clone()));
            self.doc.set("installedFiles", &format!("{}/fileid", i + 1), &string(f.clone()));
        }
        self.doc.set("installedFiles", "size", &string(pairs.len().to_string()));
    }

    /// `IModInterface::setUrl`, as the FOMOD installer calls it when the mod has no URL.
    pub fn set_url_if_empty(&mut self, url: &str) {
        if !url.is_empty() && self.doc.string("", "url").unwrap_or_default().is_empty() {
            self.doc.set("", "url", &string(url));
            self.doc.set("", "hasCustomURL", &string("true"));
        }
    }

    /// `installationFile`, as the bundle installer resets it to the outer archive.
    pub fn set_installation_file(&mut self, file: &str) {
        self.doc.set("", "installationFile", &string(file));
    }

    /// `[Plugins] <installer>\<key>` (`setPluginSetting`); `clear` first removes the
    /// installer's old settings (`clearPluginSettings`).
    pub fn set_plugin_settings(&mut self, installer: &str, settings: &[(String, String)], clear: bool) {
        if clear {
            let prefix = format!("{installer}/");
            let keys: Vec<String> = self
                .doc
                .entries()
                .filter(|e| e.section == "Plugins" && e.key.starts_with(&prefix))
                .map(|e| e.key.to_owned())
                .collect();
            for key in keys {
                self.doc.remove("Plugins", &key);
            }
        }
        for (key, value) in settings {
            self.doc.set("Plugins", &format!("{installer}/{key}"), &string(value.clone()));
        }
    }

    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        self.doc.string(section, key)
    }

    /// `pluginSettings(installer)`: the `[Plugins] <installer>\<key>` pairs, sorted by key
    /// (a `std::map`).
    pub fn plugin_settings(&self, installer: &str) -> Vec<(String, String)> {
        let prefix = format!("{installer}/");
        let mut out: Vec<(String, String)> = self
            .doc
            .entries()
            .filter(|e| e.section == "Plugins")
            .filter_map(|e| e.key.strip_prefix(&prefix).map(|k| (k.to_owned(), e.value().to_text())))
            .collect();
        out.sort();
        out
    }

    /// `ModInfoRegular::readMeta` then `saveMeta`, as MO2 runs them on the mod it just
    /// installed: each key of `saveMeta` is written in its order (an existing key keeps its
    /// line, a missing one is appended), with the value `readMeta` makes of the file.
    /// `categories` are the instance's (`readMeta` drops ids that are not in it).
    pub fn save_meta(&mut self, categories: &strata_core::Categories) {
        let d = &self.doc;
        let text = |k: &str| d.string("", k).unwrap_or_default();
        let int = |k: &str, default: i64| d.string("", k).map_or(default, |v| v.trim().parse::<i64>().unwrap_or(0));
        let flag = |k: &str| d.value("", k).is_some_and(|v| v.to_bool());

        // category: the first known id is the primary one, the rest follow (a std::set)
        let mut primary = -1i64;
        let mut others = std::collections::BTreeSet::new();
        let listed = match d.value("", "category") {
            Some(Value::List(items)) => items,
            Some(v) => v.to_text().split(',').map(str::to_owned).collect(),
            None => Vec::new(),
        };
        for (i, item) in listed.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).enumerate() {
            let Ok(id) = item.parse::<i32>() else { continue };
            if id > 0 && categories.get(id).is_some() {
                if i == 0 {
                    primary = i64::from(id);
                }
                others.insert(i64::from(id));
            }
        }
        others.remove(&primary);
        let category = format!("{primary},{}", others.iter().map(i64::to_string).collect::<Vec<_>>().join(","));

        let mod_id = int("modid", -1);
        let url = text("url");
        let has_custom_url = if d.get("", "hasCustomURL").is_some() {
            flag("hasCustomURL")
        } else {
            // MO2 leaves the flag uninitialised for an id-less mod without a URL: false here
            mod_id <= 0 && !url.is_empty()
        };
        let iso = |k: &str| text(k);
        let last_modified = d.string("", "nexusLastModified").unwrap_or_else(utc_now_iso);
        let repository = d.string("", "repository").unwrap_or_else(|| "Nexus".to_owned());
        let tracked = if flag("tracked") { "1" } else { "0" };
        let values: Vec<(&str, Value)> = vec![
            ("category", string(category)),
            ("newestVersion", string(crate::version::canonical(&text("newestVersion")))),
            ("ignoredVersion", string(crate::version::canonical(&text("ignoredVersion")))),
            ("version", string(crate::version::canonical(&text("version")))),
            ("installationFile", string(text("installationFile"))),
            ("repository", string(repository)),
            ("gameName", string(text("gameName"))),
            ("modid", string(mod_id.to_string())),
            ("comments", string(text("comments"))),
            ("notes", string(text("notes"))),
            ("nexusDescription", string(text("nexusDescription"))),
            ("url", string(url)),
            ("hasCustomURL", string(has_custom_url.to_string())),
            ("nexusFileStatus", string(int("nexusFileStatus", 1).to_string())),
            ("lastNexusQuery", string(iso("lastNexusQuery"))),
            ("lastNexusUpdate", string(iso("lastNexusUpdate"))),
            ("nexusLastModified", string(last_modified)),
            ("nexusCategory", string(int("nexusCategory", 0).to_string())),
            ("converted", string(flag("converted").to_string())),
            ("validated", string(flag("validated").to_string())),
            ("color", d.value("", "color").unwrap_or_else(|| strata_ini::codec::decode_value(INVALID_COLOR))),
            ("tracked", string(tracked)),
        ];
        for (key, value) in values {
            self.doc.set("", key, &value);
        }
    }

    pub fn save(&self) -> Result<()> {
        write_atomic(&self.path, &self.doc.to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> InstallMeta {
        InstallMeta {
            game_name: "SkyrimSE".into(),
            mod_id: 84743,
            version: "2.3.4.0".into(),
            newest_version: "2.3.4.0".into(),
            category: 32,
            file_category: 1,
            installation_file: "AVG-84743-2-3-4-0.7z".into(),
            repository: "Nexus".into(),
            page_url: String::new(),
        }
    }

    #[test]
    fn fresh_meta_ini_has_mo2s_keys_in_mo2s_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = ModMeta::open(dir.path()).unwrap();
        m.write_install(&meta());
        m.add_installed_file(84743, 797626);
        m.save().unwrap();
        let text = std::fs::read_to_string(dir.path().join("meta.ini")).unwrap();
        assert_eq!(
            text,
            "[General]\ngameName=SkyrimSE\nmodid=84743\nversion=2.3.4.0\nnewestVersion=2.3.4.0\ncategory=32\n\
             nexusFileStatus=1\ninstallationFile=AVG-84743-2-3-4-0.7z\nrepository=Nexus\n\
             [installedFiles]\n1\\modid=84743\n1\\fileid=797626\nsize=1\n"
        );
    }

    #[test]
    fn reinstall_keeps_categories_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("meta.ini"),
            "[General]\ncategory=\"5,\"\nnotes=mine\nversion=3.0\n[installedFiles]\n1\\modid=1\n1\\fileid=2\nsize=1\n",
        )
        .unwrap();
        let mut m = ModMeta::open(dir.path()).unwrap();
        m.write_install(&meta());
        assert_eq!(m.get("", "category").as_deref(), Some("5,"));
        assert_eq!(m.get("", "notes").as_deref(), Some("mine"));
        assert_eq!(m.get("", "version").as_deref(), Some("2.3.4.0"));
        assert!(m.get("installedFiles", "size").is_none(), "doInstall clears the array");
        m.add_installed_file(1, 2);
        m.add_installed_file(84743, 9);
        m.add_installed_file(1, 2);
        assert_eq!(m.get("installedFiles", "size").as_deref(), Some("2"));
        assert_eq!(m.get("installedFiles", "2/modid").as_deref(), Some("84743"));
    }

    #[test]
    fn download_meta_is_read_and_marked() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.7z");
        std::fs::write(
            meta_path(&archive),
            "[General]\ngameName=skyrimse\nmodID=12\nfileID=34\nname=\"A &amp; B <b>x</b>\"\nmodName=AB\ncategory=32\ninstalled=false\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(meta_path(&archive), std::fs::Permissions::from_mode(0o644)).unwrap();
        let m = DownloadMeta::read_for(&archive).unwrap().unwrap();
        assert_eq!((m.mod_id, m.file_id, m.name.as_str(), m.file_category), (12, 34, "A & B x", 1));
        assert!(!m.installed && !m.paused && !m.removed && !m.uninstalled);
        mark_download_installed(&archive).unwrap();
        let text = std::fs::read_to_string(meta_path(&archive)).unwrap();
        assert!(text.contains("installed=true") && text.contains("uninstalled=false"));
        // the .meta keeps its permissions (a temp file alone would make it 0600)
        assert_eq!(std::fs::metadata(meta_path(&archive)).unwrap().permissions().mode() & 0o777, 0o644);
        assert!(DownloadMeta::read_for(&archive).unwrap().unwrap().installed);
    }

    /// The `meta.ini` MO2 2.5.2 left for "Bonk" in a real instance (installed from its download):
    /// `doInstall`'s keys, then `saveMeta`'s.
    #[test]
    fn save_meta_matches_what_mo2_left() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("meta.ini"),
            "[General]\r\nmodid=0\r\nversion=d2026.11.9\r\nnewestVersion=\r\ncategory=0\r\n\
             installationFile=Bonk - Bumping NPCs can start brawls.zip\r\ngameName=SkyrimSE\r\n\
             nexusFileStatus=1\r\nrepository=Nexus\r\n\r\n[installedFiles]\r\n1\\modid=0\r\n1\\fileid=0\r\nsize=1\r\n",
        )
        .unwrap();
        let mut m = ModMeta::open(dir.path()).unwrap();
        m.save_meta(&strata_core::Categories::open(dir.path()).unwrap());
        m.save().unwrap();
        let got = std::fs::read_to_string(dir.path().join("meta.ini")).unwrap();
        let got: Vec<&str> = got.lines().filter(|l| !l.starts_with("nexusLastModified=")).collect();
        let want = "[General]\nmodid=0\nversion=d2026.11.9.0\nnewestVersion=\ncategory=\"-1,\"\n\
             installationFile=Bonk - Bumping NPCs can start brawls.zip\ngameName=SkyrimSE\n\
             nexusFileStatus=1\nrepository=Nexus\nignoredVersion=\ncomments=\nnotes=\nnexusDescription=\nurl=\n\
             hasCustomURL=false\nlastNexusQuery=\nlastNexusUpdate=\nnexusCategory=0\nconverted=false\n\
             validated=false\ncolor=@Variant(\\0\\0\\0\\x43\\0\\xff\\xff\\0\\0\\0\\0\\0\\0\\0\\0)\ntracked=0\n\n\
             [installedFiles]\n1\\modid=0\n1\\fileid=0\nsize=1";
        assert_eq!(got.join("\n"), want);
        // saving again changes nothing (the timestamp stays)
        let before = std::fs::read(dir.path().join("meta.ini")).unwrap();
        let mut m = ModMeta::open(dir.path()).unwrap();
        m.save_meta(&strata_core::Categories::open(dir.path()).unwrap());
        assert_eq!(m.doc.to_bytes(), before);
    }

    #[test]
    fn save_meta_keeps_known_categories_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("categories.dat"), "8|Armour|28|0\n12|Weapons|55|0\n").unwrap();
        std::fs::write(dir.path().join("meta.ini"), "[General]\ncategory=\"99,12,8\"\nmodid=5\nurl=x\n").unwrap();
        let mut m = ModMeta::open(dir.path()).unwrap();
        m.save_meta(&strata_core::Categories::open(dir.path()).unwrap());
        assert_eq!(m.get("", "category").as_deref(), Some("-1,8,12"));
        // a valid mod id: the old URL is kept but not used
        assert_eq!(m.get("", "hasCustomURL").as_deref(), Some("false"));
        std::fs::write(dir.path().join("meta.ini"), "[General]\ncategory=12\n").unwrap();
        let mut m = ModMeta::open(dir.path()).unwrap();
        m.save_meta(&strata_core::Categories::open(dir.path()).unwrap());
        assert_eq!(m.get("", "category").as_deref(), Some("12,"));
    }
}
