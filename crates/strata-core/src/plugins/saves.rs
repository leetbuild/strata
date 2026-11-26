//! The Saves tab: where a profile's saves are, and which of a save's plugins are missing.

use std::path::{Path, PathBuf};

use strata_game::SaveGame;

use super::{PluginList, key};
use crate::{Error, Instance, Profile, Result};

/// One save of the list.
#[derive(Debug, Clone)]
pub struct SaveEntry {
    pub path: PathBuf,
    /// The save, or why it could not be read (MO2 leaves such files out; Strata shows them
    /// with the reason).
    pub save: std::result::Result<SaveGame, String>,
    /// The script extender's co-save (`.skse`) exists next to it.
    pub has_cosave: bool,
}

/// A plugin a save needs that the profile does not load (`GamebryoSaveGameInfo::
/// getMissingAssets`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingPlugin {
    pub name: String,
    /// The mods that have the plugin (enabled or not, `<overwrite>` too): enabling one of
    /// them brings it back. Empty when no mod has it.
    pub providers: Vec<String>,
}

impl Instance {
    /// The folder the profile's saves are in (`SavesTab::currentSavesDir`): the profile's
    /// `saves` with local saves; otherwise `SLocalSavePath` from `[General]` of the game's
    /// first ini (the profile's copy with local settings, the prefix's without), under the
    /// game's documents folder; otherwise `<documents>/Saves`. `None` when the save folder
    /// is in a prefix the instance does not name.
    pub fn saves_dir(&self, profile: &Profile) -> Option<PathBuf> {
        if profile.local_saves {
            return Some(profile.dir.join("saves"));
        }
        let documents = self
            .proton
            .prefix
            .as_ref()?
            .join("pfx/drive_c/users/steamuser/Documents/My Games")
            .join(self.game.documents_dir());
        let ini_name = self.game.ini_files().first()?;
        let ini = if profile.local_settings { find_ci(&profile.dir, ini_name) } else { find_ci(&documents, ini_name) };
        let local = ini.and_then(|p| std::fs::read(p).ok()).and_then(|data| ini_value(&data, "General", "SLocalSavePath"));
        Some(match local {
            Some(rel) => documents.join(rel.replace('\\', "/").trim_end_matches('/')),
            None => documents.join("Saves"),
        })
    }

    /// The profile's saves, newest first (`SavesTab::refreshSaveList`). `plugins`: also read
    /// each save's plugin lists (slower: the body is decompressed).
    pub fn saves(&self, profile: &Profile, plugins: bool) -> Result<Vec<SaveEntry>> {
        let Some(dir) = self.saves_dir(profile) else { return Ok(Vec::new()) };
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(Error::Read { path: dir, source }),
        };
        let mut saves: Vec<SaveEntry> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ess")))
            .map(|path| SaveEntry {
                save: SaveGame::read(&path, plugins).map_err(|e| e.to_string()),
                has_cosave: path.with_extension("skse").is_file(),
                path,
            })
            .collect();
        // Newest first; unreadable saves last.
        saves.sort_by_key(|s| std::cmp::Reverse(s.save.as_ref().map(|g| g.created).unwrap_or(i64::MIN)));
        Ok(saves)
    }

    /// The plugins `save` needs that `list` doesn't load, each with the mods that have it:
    /// a plugin that is off names the mod it comes from; then every mod folder (in priority
    /// order, lowest first, as `allModsByProfilePriority`) and overwrite holding a file of that
    /// name is added.
    pub fn missing_plugins(&self, profile: &Profile, list: &PluginList, save: &SaveGame) -> Result<Vec<MissingPlugin>> {
        let mut missing: Vec<MissingPlugin> = Vec::new();
        for name in save.plugins.iter().chain(&save.light_plugins) {
            if missing.iter().any(|m| key(&m.name) == key(name)) {
                continue;
            }
            match list.get(name) {
                Some(p) if p.enabled => {}
                Some(p) => missing.push(MissingPlugin { name: name.clone(), providers: vec![p.origin.clone()] }),
                None => missing.push(MissingPlugin { name: name.clone(), providers: Vec::new() }),
            }
        }
        if missing.is_empty() {
            return Ok(missing);
        }
        let mut folders: Vec<(String, PathBuf)> =
            self.mod_list(profile)?.into_iter().filter_map(|m| m.path.map(|p| (m.name, p))).collect();
        folders.push(("<overwrite>".into(), self.paths.overwrite.clone()));
        for (name, dir) in folders {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for file in entries.filter_map(|e| e.ok()) {
                let file = file.file_name().to_string_lossy().into_owned();
                if let Some(m) = missing.iter_mut().find(|m| key(&m.name) == key(&file))
                    && !m.providers.contains(&name)
                {
                    m.providers.push(name.clone());
                }
            }
        }
        Ok(missing)
    }
}

/// The file `name` in `dir`, any case.
fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
        p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name))
    })
}

/// A value of a Windows-style ini as `GetPrivateProfileString` finds it: section and key
/// without case, the value trimmed. Empty values count as absent (MO2 tests the length).
fn ini_value(data: &[u8], section: &str, key: &str) -> Option<String> {
    let text = String::from_utf8_lossy(data);
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix('[') {
            current = rest.split(']').next().unwrap_or("").trim().to_owned();
        } else if current.eq_ignore_ascii_case(section)
            && let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case(key)
        {
            let v = v.trim();
            return (!v.is_empty()).then(|| v.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_values_are_found_without_case() {
        let ini = b"[Display]\r\nSLocalSavePath=No\r\n[general]\r\n sLocalSavePath = Saves\\Mine\\ \r\n";
        assert_eq!(ini_value(ini, "General", "SLocalSavePath").as_deref(), Some("Saves\\Mine\\"));
        assert_eq!(ini_value(b"[General]\r\nSLocalSavePath=\r\n", "General", "SLocalSavePath"), None);
    }
}
