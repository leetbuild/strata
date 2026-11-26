//! Creating, copying, renaming and deleting profiles, and switching local saves and local
//! inis (`ProfilesDialog`, `Profile` in `profile.cpp`, `GameSkyrimSE::initializeProfile`).

use std::path::{Path, PathBuf};

use strata_ini::{Document, Value};

use crate::events::ProfileEvent;
use crate::trash::{self, Trash};
use crate::write::write_if_changed;
use crate::{Error, Instance, Profile, Result, fix_directory_name};

/// The file `name` in `dir`, matched without case (Fluorine's `resolveFileCaseInsensitive`:
/// profiles made on Windows often hold `skyrim.ini` where the game says `Skyrim.ini`).
pub(crate) fn find_any_case(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

/// `GameGamebryo::copyToProfile`: unless the profile already has `target` (any case), copy
/// `source_name` from `source_dir` there, or create it empty when that fails.
fn copy_to_profile(source_dir: Option<&Path>, source_name: &str, profile: &Path, target: &str) -> Result<()> {
    if find_any_case(profile, target).is_some() {
        return Ok(());
    }
    let data = source_dir.and_then(|d| find_any_case(d, source_name)).and_then(|f| std::fs::read(f).ok()).unwrap_or_default();
    write_if_changed(&profile.join(target), &data).map(|_| ())
}

/// What `IPluginGame::initializeProfile` sets up.
#[derive(Debug, Clone, Copy, Default)]
struct Init {
    /// `MODS`: `plugins.txt` from the prefix.
    mods: bool,
    /// `CONFIGURATION`: the game's inis from the prefix's `My Games` (or the game's
    /// default ini).
    configuration: bool,
    /// `PREFER_DEFAULTS`: the main ini from the game folder's default.
    prefer_defaults: bool,
}

impl Instance {
    /// The prefix's `Documents/My Games/<game>` and `AppData/Local/<game>`, when the
    /// instance names a prefix.
    pub(crate) fn prefix_game_dirs(&self) -> (Option<PathBuf>, Option<PathBuf>) {
        let Some(prefix) = &self.proton.prefix else { return (None, None) };
        let user = prefix.join("pfx/drive_c/users/steamuser");
        (
            Some(user.join("Documents/My Games").join(self.game.documents_dir())),
            Some(user.join("AppData/Local").join(self.game.appdata_dir())),
        )
    }

    /// `GameSkyrimSE::initializeProfile` for `dir`. Files come from the instance's prefix;
    /// without one they are created empty, as MO2 does when a copy fails. The main ini's
    /// default is `<main ini stem>_default.ini` in the game folder (`Skyrim_default.ini`).
    fn initialize_profile(&self, dir: &Path, init: Init) -> Result<()> {
        let (my_games, appdata) = self.prefix_game_dirs();
        if init.mods {
            copy_to_profile(appdata.as_deref(), "plugins.txt", dir, "plugins.txt")?;
        }
        if init.configuration {
            let inis = self.game.ini_files();
            if let Some(main) = inis.first() {
                let has_main = my_games.as_deref().and_then(|d| find_any_case(d, main)).is_some();
                if init.prefer_defaults || !has_main {
                    let default = format!("{}_default.ini", main.trim_end_matches(".ini"));
                    copy_to_profile(Some(&self.game_path), &default, dir, main)?;
                } else {
                    copy_to_profile(my_games.as_deref(), main, dir, main)?;
                }
            }
            for ini in inis.iter().skip(1) {
                copy_to_profile(my_games.as_deref(), ini, dir, ini)?;
            }
        }
        Ok(())
    }

    /// A `[Settings]` flag of `ModOrganizer.ini` with MO2's default.
    fn setting_flag(&self, key: &str, default: bool) -> bool {
        self.ini.value("Settings", key).map_or(default, |v| v.to_bool())
    }

    fn check_profile_name(&self, name: &str) -> Result<(String, PathBuf)> {
        let fixed = fix_directory_name(name).ok_or_else(|| Error::InvalidName { name: name.to_owned() })?;
        let dir = self.paths.profiles.join(&fixed);
        let taken = self.profile_names().unwrap_or_default().iter().any(|n| n.eq_ignore_ascii_case(&fixed));
        if taken || dir.exists() {
            return Err(Error::Exists { path: dir });
        }
        Ok((fixed, dir))
    }

    /// A new profile (`Profile::Profile(name, ..., useDefaultSettings)`): the folder with
    /// an empty `modlist.txt` and `archives.txt`, `plugins.txt` and the game's inis
    /// (`initializeProfile` with `CONFIGURATION | MODS | SAVEGAMES`), and `settings.ini`
    /// from the instance's defaults (`findProfileSettings`: `profile_local_saves` off,
    /// `profile_local_inis` on, `profile_archive_invalidation` on). The folder is built
    /// under a temp name and renamed into place, so a failure leaves nothing.
    pub fn create_profile(&self, name: &str, prefer_defaults: bool) -> Result<(Profile, Vec<ProfileEvent>)> {
        let (fixed, dir) = self.check_profile_name(name)?;
        let building = self.paths.profiles.join(format!(".{fixed}.strata-new"));
        let _ = std::fs::remove_dir_all(&building);
        std::fs::create_dir(&building).map_err(|source| Error::Write { path: building.clone(), source })?;
        let result = (|| {
            for file in ["modlist.txt", "archives.txt"] {
                write_if_changed(&building.join(file), b"")?;
            }
            self.initialize_profile(&building, Init { mods: true, configuration: true, prefer_defaults })?;
            let flag = |on: bool| if on { "true" } else { "false" };
            // Skyrim SE has no archive invalidation feature: the setting is just stored.
            let settings = format!(
                "[General]\nLocalSaves={}\nLocalSettings={}\nAutomaticArchiveInvalidation={}\n",
                flag(self.setting_flag("profile_local_saves", false)),
                flag(self.setting_flag("profile_local_inis", true)),
                flag(self.setting_flag("profile_archive_invalidation", true)),
            );
            write_if_changed(&building.join("settings.ini"), settings.as_bytes())?;
            std::fs::rename(&building, &dir).map_err(|source| Error::Write { path: dir.clone(), source })
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_dir_all(&building);
            return Err(e);
        }
        Ok((Profile::open(&fixed, &dir)?, vec![ProfileEvent::Created { name: fixed }]))
    }

    /// A copy of a profile (`Profile::createPtrFrom`): every file and folder of it (saves and
    /// backups included, `copyDir`), then `plugins.txt` from the prefix if the copy has
    /// none. Built under a temp name, renamed into place.
    pub fn copy_profile(&self, from: &str, name: &str) -> Result<(Profile, Vec<ProfileEvent>)> {
        let source = self.profile(Some(from))?;
        let (fixed, dir) = self.check_profile_name(name)?;
        let building = self.paths.profiles.join(format!(".{fixed}.strata-new"));
        let _ = std::fs::remove_dir_all(&building);
        let result = (|| {
            copy_dir(&source.dir, &building)?;
            self.initialize_profile(&building, Init { mods: true, ..Init::default() })?;
            std::fs::rename(&building, &dir).map_err(|source| Error::Write { path: dir.clone(), source })
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_dir_all(&building);
            return Err(e);
        }
        Ok((Profile::open(&fixed, &dir)?, vec![ProfileEvent::Created { name: fixed }]))
    }

    /// Makes `name` the selected profile: `[General] selected_profile` in
    /// `ModOrganizer.ini`, written as MO2 writes it (`@ByteArray(name)`), only that line.
    pub fn select_profile(&mut self, name: &str) -> Result<Vec<ProfileEvent>> {
        let profile = self.profile(Some(name))?;
        if profile.name == self.selected_profile {
            return Ok(Vec::new());
        }
        let value = Value::ByteArray(profile.name.clone().into_bytes());
        self.store_settings(&[("", "selected_profile", Some(value))])?;
        self.selected_profile.clone_from(&profile.name);
        Ok(vec![ProfileEvent::Selected { name: profile.name }])
    }

    /// Renames a profile (`ProfilesDialog::on_renameButton_clicked`, `Profile::rename`).
    /// The selected profile can't be renamed, as in MO2.
    pub fn rename_profile(&self, old: &str, new: &str) -> Result<Vec<ProfileEvent>> {
        let profile = self.profile(Some(old))?;
        if profile.name == self.selected_profile {
            return Err(Error::ActiveProfile { name: profile.name, action: "it can't be renamed" });
        }
        let fixed = fix_directory_name(new).ok_or_else(|| Error::InvalidName { name: new.to_owned() })?;
        if fixed == profile.name {
            return Ok(Vec::new());
        }
        let dir = self.paths.profiles.join(&fixed);
        let other_case = fixed.eq_ignore_ascii_case(&profile.name);
        if !other_case && (dir.exists() || self.profile_names()?.iter().any(|n| n.eq_ignore_ascii_case(&fixed))) {
            return Err(Error::Exists { path: dir });
        }
        let mut txn = crate::write::Transaction::new(&self.strata_dir())?;
        txn.rename(&profile.dir, &dir)?;
        txn.commit()?;
        Ok(vec![ProfileEvent::Renamed { from: profile.name, to: fixed }])
    }

    /// Deletes a profile with its local saves (`on_removeProfileButton_clicked`), to the
    /// trash unless `trash` is `None`. The selected profile can't be deleted.
    pub fn delete_profile(&self, name: &str, trash: Option<&Trash>) -> Result<Vec<ProfileEvent>> {
        let profile = self.profile(Some(name))?;
        if profile.name == self.selected_profile {
            return Err(Error::ActiveProfile { name: profile.name, action: "it can't be deleted" });
        }
        trash::remove(&profile.dir, trash)?;
        Ok(vec![ProfileEvent::Removed { name: profile.name }])
    }

    /// `Profile::enableLocalSaves`: on makes `saves/`; off keeps it unless `delete_saves`
    /// (then it goes to `trash`, or for good). Stores `LocalSaves`.
    pub fn set_local_saves(&self, profile: &Profile, on: bool, delete_saves: bool, trash: Option<&Trash>) -> Result<Vec<ProfileEvent>> {
        let saves = profile.dir.join("saves");
        if on {
            if !saves.is_dir() {
                std::fs::create_dir(&saves).map_err(|source| Error::Write { path: saves.clone(), source })?;
            }
        } else if delete_saves && saves.exists() {
            trash::remove(&saves, trash)?;
        }
        store_setting(profile, "LocalSaves", on)
    }

    /// `Profile::enableLocalSettings`: on copies the game's inis into the profile where
    /// missing (`initializeProfile(CONFIGURATION)`); off keeps them unless `delete_inis`.
    /// Stores `LocalSettings`.
    pub fn set_local_settings(&self, profile: &Profile, on: bool, delete_inis: bool, trash: Option<&Trash>) -> Result<Vec<ProfileEvent>> {
        if on {
            self.initialize_profile(&profile.dir, Init { configuration: true, ..Init::default() })?;
        } else if delete_inis {
            for ini in self.game.ini_files() {
                if let Some(file) = find_any_case(&profile.dir, ini) {
                    trash::remove(&file, trash)?;
                }
            }
        }
        store_setting(profile, "LocalSettings", on)
    }
}

/// `Profile::storeSetting("", key, on)` in `settings.ini`, written only when it changes.
fn store_setting(profile: &Profile, key: &str, on: bool) -> Result<Vec<ProfileEvent>> {
    let value = Value::String(if on { "true" } else { "false" }.into());
    if store_profile_settings(profile, &[("", key, Some(value))])? {
        Ok(vec![ProfileEvent::SettingsChanged { name: profile.name.clone() }])
    } else {
        Ok(Vec::new())
    }
}

fn read_settings(profile: &Profile) -> Result<Document> {
    let path = profile.dir.join("settings.ini");
    match std::fs::read(&path) {
        Ok(data) => Ok(Document::parse(&data)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Document::parse(b"[General]\n")),
        Err(source) => Err(Error::Read { path, source }),
    }
}

/// `Profile::setting(section, key)`: a value of the profile's `settings.ini` ("" =
/// `[General]`; `custom_overwrites` holds the executables' "create files in mod").
pub fn profile_setting(profile: &Profile, section: &str, key: &str) -> Result<Option<Value>> {
    Ok(read_settings(profile)?.value(section, key))
}

/// `Profile::storeSetting` / `removeSetting` for several values (`None` removes), written
/// once and only when the file changed. Returns whether it changed.
pub fn store_profile_settings(profile: &Profile, values: &[(&str, &str, Option<Value>)]) -> Result<bool> {
    let mut doc = read_settings(profile)?;
    let mut changed = false;
    for (section, key, value) in values {
        changed |= match value {
            Some(v) => doc.set(section, key, v),
            None => doc.remove(section, key),
        };
    }
    if changed {
        write_if_changed(&profile.dir.join("settings.ini"), &doc.to_bytes())?;
    }
    Ok(changed)
}

/// Copies a folder tree (`copyDir`). Files are copied with `std::fs::copy`, which on btrfs
/// shares the data (copy_file_range). Symlinks are copied as links.
fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir(to).map_err(|source| Error::Write { path: to.to_owned(), source })?;
    for entry in std::fs::read_dir(from).map_err(|source| Error::Read { path: from.to_owned(), source })? {
        let entry = entry.map_err(|source| Error::Read { path: from.to_owned(), source })?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type().map_err(|e| Error::Read { path: source.clone(), source: e })?;
        if kind.is_dir() {
            copy_dir(&source, &target)?;
        } else if kind.is_symlink() {
            let link = std::fs::read_link(&source).map_err(|e| Error::Read { path: source.clone(), source: e })?;
            std::os::unix::fs::symlink(link, &target).map_err(|e| Error::Write { path: target.clone(), source: e })?;
        } else {
            std::fs::copy(&source, &target).map_err(|e| Error::Write { path: target.clone(), source: e })?;
        }
    }
    Ok(())
}
