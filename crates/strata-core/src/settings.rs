//! The instance settings MO2's settings dialog writes (`settings.cpp`, `settingsdialog*.cpp`),
//! those that mean something on Linux, and writing `ModOrganizer.ini` one line at a time.
//!
//! Every write re-reads the file first, so keys another part of Strata (the window's
//! layout) wrote meanwhile are kept, then changes only the lines it must and writes the
//! file atomically when something changed.

use std::path::Path;

use strata_ini::{Document, Value};

use crate::{Error, Instance, Result, write::write_if_changed};

/// What kind of value a setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Text,
    /// A folder, stored as text; `%BASE_DIR%` may stand for the base folder.
    Path,
    /// A comma list (`QStringList`).
    List,
}

/// One setting: where MO2 keeps it, its type and default (the MO2 getter's), and the
/// settings dialog page it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setting {
    pub page: &'static str,
    pub section: &'static str,
    pub key: &'static str,
    pub kind: Kind,
    pub default: &'static str,
}

const fn s(page: &'static str, section: &'static str, key: &'static str, kind: Kind, default: &'static str) -> Setting {
    Setting { page, section, key, kind, default }
}

/// The settings that apply on Linux, with MO2 2.5.2's defaults (`settings.cpp` getters;
/// the Senjay picks of Leet change 8 for `compact_downloads`, `meta_downloads`,
/// `profile_archive_invalidation`, `center_dialogs`; Leet change 11 for
/// `auto_enable_install`). Left out: Steam login, language (Strata is English only),
/// update checks of MO2 itself, the Windows crash dump types, `spawn_delay` (usvfs).
pub const SETTINGS: &[Setting] = &[
    // General
    s("General", "Settings", "style", Kind::Text, ""),
    s("General", "Settings", "profile_local_inis", Kind::Bool, "true"),
    s("General", "Settings", "profile_local_saves", Kind::Bool, "false"),
    s("General", "Settings", "profile_archive_invalidation", Kind::Bool, "true"),
    s("General", "Settings", "center_dialogs", Kind::Bool, "true"),
    s("General", "Settings", "show_change_game_confirmation", Kind::Bool, "true"),
    s("General", "Settings", "show_menubar_on_alt", Kind::Bool, "true"),
    s("General", "Settings", "double_click_previews", Kind::Bool, "true"),
    s("General", "Settings", "compact_downloads", Kind::Bool, "true"),
    s("General", "Settings", "meta_downloads", Kind::Bool, "true"),
    s("General", "Settings", "autohide_downloads", Kind::Bool, "false"),
    s("General", "Settings", "auto_enable_install", Kind::Bool, "true"),
    s("General", "General", "backup_install", Kind::Bool, "false"),
    // Mod list
    s("Mod List", "Settings", "display_foreign", Kind::Bool, "true"),
    s("Mod List", "Settings", "colorSeparatorScrollbars", Kind::Bool, "true"),
    s("Mod List", "Settings", "collapsible_separators_asc", Kind::Bool, "true"),
    s("Mod List", "Settings", "collapsible_separators_dsc", Kind::Bool, "true"),
    // ModList::EColumn numbers: 1 conflicts, 2 flags, 3 content, 7 version
    s("Mod List", "Settings", "collapsible_separators_icons_1", Kind::Bool, "true"),
    s("Mod List", "Settings", "collapsible_separators_icons_2", Kind::Bool, "true"),
    s("Mod List", "Settings", "collapsible_separators_icons_3", Kind::Bool, "true"),
    s("Mod List", "Settings", "collapsible_separators_icons_7", Kind::Bool, "true"),
    s("Mod List", "Settings", "collapsible_separators_conflicts_to", Kind::Bool, "true"),
    s("Mod List", "Settings", "collapsible_separators_conflicts_from", Kind::Bool, "true"),
    s("Mod List", "Settings", "collapsible_separators_per_profile", Kind::Bool, "false"),
    s("Mod List", "Settings", "save_filters", Kind::Bool, "false"),
    s("Mod List", "Settings", "auto_collapse_on_hover", Kind::Bool, "false"),
    s("Mod List", "Settings", "autocheck_update_install", Kind::Bool, "true"),
    s("Mod List", "Settings", "filter_regex", Kind::Bool, "false"),
    s("Mod List", "Settings", "regex_case_sensitive", Kind::Bool, "false"),
    s("Mod List", "Settings", "regex_extended", Kind::Bool, "false"),
    s("Mod List", "Settings", "filter_scroll_to_selection", Kind::Bool, "false"),
    // Leet change 9: Sub-Separator (ModOrganizer.ini [Plugins] Sub-Separator\...)
    s("Mod List", "Plugins", "Sub-Separator/alwaysShowSize", Kind::Bool, "false"),
    // InterfaceSettings::subSeparatorIndentation: 0 when unset (negative = Qt's default)
    s("Mod List", "Plugins", "Sub-Separator/indentation", Kind::Int, "0"),
    s("Mod List", "Plugins", "Sub-Separator/invertAltModifierWhenSeparatorIsCollapsed", Kind::Bool, "false"),
    // Paths (Settings::paths(); empty = the default under the base folder)
    s("Paths", "Settings", "base_directory", Kind::Path, ""),
    s("Paths", "Settings", "download_directory", Kind::Path, "%BASE_DIR%/downloads"),
    s("Paths", "Settings", "mod_directory", Kind::Path, "%BASE_DIR%/mods"),
    s("Paths", "Settings", "cache_directory", Kind::Path, "%BASE_DIR%/webcache"),
    s("Paths", "Settings", "profiles_directory", Kind::Path, "%BASE_DIR%/profiles"),
    s("Paths", "Settings", "overwrite_directory", Kind::Path, "%BASE_DIR%/overwrite"),
    // Nexus (what applies without a Nexus client)
    s("Nexus", "Settings", "endorsement_integration", Kind::Bool, "true"),
    s("Nexus", "Settings", "tracked_integration", Kind::Bool, "true"),
    s("Nexus", "Settings", "category_mappings", Kind::Bool, "true"),
    s("Nexus", "Settings", "hide_api_counter", Kind::Bool, "false"),
    s("Nexus", "Settings", "offline_mode", Kind::Bool, "false"),
    s("Nexus", "Settings", "use_proxy", Kind::Bool, "false"),
    s("Nexus", "Settings", "use_custom_browser", Kind::Bool, "false"),
    s("Nexus", "Settings", "custom_browser", Kind::Text, ""),
    // Workarounds
    s("Workarounds", "Settings", "force_enable_core_files", Kind::Bool, "true"),
    s("Workarounds", "Settings", "lock_gui", Kind::Bool, "true"),
    s("Workarounds", "Settings", "archive_parsing_experimental", Kind::Bool, "false"),
    s("Workarounds", "Settings", "skip_file_suffixes", Kind::List, ".mohidden"),
    s("Workarounds", "Settings", "skip_directories", Kind::List, ".git"),
    s("Workarounds", "Settings", "executable_blacklist", Kind::Text, ""),
    // Strata's own: environment variables for every launch, one NAME=value per line
    s("Workarounds", "Settings", "strata_environment", Kind::Text, ""),
    // Diagnostics
    // log::Levels: 0 debug, 1 info, 2 warning, 3 error; lootcli::LogLevels: 0 trace ... 4 error
    s("Diagnostics", "Settings", "log_level", Kind::Int, "1"),
    s("Diagnostics", "Settings", "loot_log_level", Kind::Int, "2"),
    s("Diagnostics", "Settings", "crash_dumps_max", Kind::Int, "5"),
    // Leet change 8: the overwrite check of diagnose_basic is off by default
    s("Diagnostics", "Plugins", "Basic diagnosis plugin/check_overwrite", Kind::Bool, "false"),
    // diagnose_basic's other switches (DiagnoseBasic::settings)
    s("Diagnostics", "Plugins", "Basic diagnosis plugin/check_font", Kind::Bool, "true"),
    s("Diagnostics", "Plugins", "Basic diagnosis plugin/check_alternategames", Kind::Bool, "false"),
    // third-party MO2 plugins built in: on or off as MO2 stores a plugin's state
    // (PluginContainer::isEnabled)
    s("Plugins", "PluginPersistance", "Crash Log Labeler/enabled", Kind::Bool, "true"),
    s("Plugins", "Plugins", "Crash Log Labeler/offline_mode", Kind::Bool, "true"),
    s("Plugins", "PluginPersistance", "Crash Log Viewer/enabled", Kind::Bool, "true"),
];

/// The setting named `section/key`.
pub fn find(section: &str, key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.section == section && s.key == key)
}

/// How a removal is confirmed (Leet change 8, `deletechoice.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteChoice {
    /// "Move to the Recycle Bin": the desktop's trash here.
    Trash,
    Permanent,
}

/// `QMessageBox::Yes`, stored by `TaskDialog::remember` for "Move to the Recycle Bin".
const BUTTON_YES: i64 = 0x4000;
/// `QMessageBox::Ok`, for "Delete permanently".
const BUTTON_OK: i64 = 0x400;

/// The remember keys MO2 uses with `askDeleteChoice`.
pub const REMOVE_MOD: &str = "removeMod";
pub const REMOVE_MODS: &str = "removeMods";

impl Instance {
    /// Whether the window offers the built-in plugin `name`: `[PluginPersistance]
    /// <name>\enabled` (default true); for plugins with their own switch (their `isActive`)
    /// also `[Plugins] <name>\enabled` (default true).
    pub fn builtin_enabled(&self, name: &str) -> bool {
        let flag = |section: &str| self.ini.value(section, &format!("{name}/enabled")).is_none_or(|v| v.to_bool());
        #[allow(unused_mut)]
        let mut own_switch = false;
        flag("PluginPersistance") && (!own_switch || flag("Plugins"))
    }

    /// A setting's value: the stored one, or the default from [`SETTINGS`] (empty text
    /// for a key the table doesn't know).
    pub fn setting(&self, section: &str, key: &str) -> Value {
        match self.ini.value(section, key) {
            Some(v) => v,
            None => match find(section, key) {
                Some(s) if s.kind == Kind::List => Value::List(s.default.split(',').map(str::to_owned).collect()),
                Some(s) => Value::String(s.default.to_owned()),
                None => Value::String(String::new()),
            },
        }
    }

    /// A boolean setting as Qt's `toBool` reads it, with the table's default.
    pub fn setting_bool(&self, section: &str, key: &str) -> bool {
        self.setting(section, key).to_bool()
    }

    /// Stores values in `ModOrganizer.ini` (`None` removes the key) through
    /// [`store_instance_ini`], and keeps the re-read document. Returns whether it changed.
    pub fn store_settings(&mut self, values: &[(&str, &str, Option<Value>)]) -> Result<bool> {
        let (changed, doc) = store_instance_ini(&self.root, values)?;
        self.ini = doc;
        Ok(changed)
    }

    /// The remembered answer of a delete confirmation (`[DialogChoices] <action>`), if
    /// "remember" was ticked.
    pub fn delete_choice(&self, action: &str) -> Option<DeleteChoice> {
        let value = self.ini.value("DialogChoices", action)?.to_text().trim().parse::<i64>().ok()?;
        match value {
            BUTTON_YES => Some(DeleteChoice::Trash),
            BUTTON_OK => Some(DeleteChoice::Permanent),
            _ => None,
        }
    }

    /// Remembers (or with `None` forgets) the answer of a delete confirmation.
    pub fn set_delete_choice(&mut self, action: &str, choice: Option<DeleteChoice>) -> Result<bool> {
        let value = choice.map(|c| Value::String(match c {
            DeleteChoice::Trash => BUTTON_YES,
            DeleteChoice::Permanent => BUTTON_OK,
        }.to_string()));
        self.store_settings(&[("DialogChoices", action, value)])
    }

    /// `WidgetSettings::resetQuestionButtons`: forgets every remembered answer (the settings
    /// dialog's "Reset Dialog Choices").
    pub fn reset_dialog_choices(&mut self) -> Result<bool> {
        let keys: Vec<String> = self.ini.entries().filter(|e| e.section == "DialogChoices").map(|e| e.key.to_owned()).collect();
        let values: Vec<(&str, &str, Option<Value>)> = keys.iter().map(|k| ("DialogChoices", k.as_str(), None)).collect();
        self.store_settings(&values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_ini_rereads_changes_one_line_and_writes_only_on_change() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(store_instance_ini(tmp.path(), &[("Settings", "a", None)]).is_err(), "an instance ini must exist");
        let path = tmp.path().join("ModOrganizer.ini");
        std::fs::write(&path, "[General]\r\ngameName=x\r\n\r\n[Settings]\r\nold=1\r\n").unwrap();
        let v = |s: &str| Some(Value::String(s.into()));
        let (changed, doc) = store_instance_ini(tmp.path(), &[("Settings", "new", v("2")), ("Settings", "old", None)]).unwrap();
        assert!(changed);
        assert_eq!(doc.string("Settings", "new").as_deref(), Some("2"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[General]\r\ngameName=x\r\n\r\n[Settings]\r\nnew=2\r\n");
        assert!(!store_instance_ini(tmp.path(), &[("Settings", "new", v("2"))]).unwrap().0);
        // a file Strata keeps itself is created on the first change, folders included
        let own = tmp.path().join("cfg/strata/strata.ini");
        assert!(store_ini(&own, &[("General", "k", v("1"))], Missing::Create).unwrap().0);
        assert!(own.is_file());
    }

    #[test]
    fn every_setting_is_unique_and_typed() {
        for (i, a) in SETTINGS.iter().enumerate() {
            assert!(SETTINGS[i + 1..].iter().all(|b| (a.section, a.key) != (b.section, b.key)), "{}", a.key);
            if a.kind == Kind::Bool {
                assert!(matches!(a.default, "true" | "false"), "{}", a.key);
            }
        }
    }
}

/// What [`store_ini`] does when the file is not there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// An error (an instance's `ModOrganizer.ini` must exist).
    Fail,
    /// Start from an empty document; the file is created when a value changes.
    Create,
}

/// The one way Strata changes a QSettings ini it shares with MO2 (`ModOrganizer.ini`) or
/// keeps itself: re-read the file (someone may have changed it since it was opened), set or
/// (with `None`) remove each `[section] key`, and write it atomically only when the bytes
/// changed (`write_if_changed`). Untouched lines keep their bytes. Returns whether the file
/// changed and the document as it is now.
pub fn store_ini(path: &Path, values: &[(&str, &str, Option<Value>)], missing: Missing) -> Result<(bool, Document)> {
    let mut doc = match std::fs::read(path) {
        Ok(data) => Document::parse(&data),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && missing == Missing::Create => Document::default(),
        Err(source) => return Err(Error::Read { path: path.to_owned(), source }),
    };
    let mut changed = false;
    for (section, key, value) in values {
        changed |= match value {
            Some(v) => doc.set(section, key, v),
            None => doc.remove(section, key),
        };
    }
    if changed {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty() && missing == Missing::Create) {
            std::fs::create_dir_all(dir).map_err(|source| Error::Write { path: dir.to_owned(), source })?;
        }
        write_if_changed(path, &doc.to_bytes())?;
    }
    Ok((changed, doc))
}

/// [`store_ini`] on `<root>/ModOrganizer.ini`, which must exist: what every area calls to
/// change an instance's settings.
pub fn store_instance_ini(root: &Path, values: &[(&str, &str, Option<Value>)]) -> Result<(bool, Document)> {
    store_ini(&root.join("ModOrganizer.ini"), values, Missing::Fail)
}
