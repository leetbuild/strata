//! The settings dialog (`SettingsDialog`, `settingsdialog.ui`): which widget shows which
//! setting, and storing what the user changed the way each MO2 settings tab's `update()`
//! does. The settings themselves (keys, types, defaults) are `strata_core::settings`.

use std::path::{Path, PathBuf};

use strata_core::settings::{self, Kind};
use strata_core::Instance;
use strata_ini::Value;

use crate::Result;

/// A widget of MO2's form and the setting it edits. Widgets of the form not named here
/// (language, MO2's own updates, Steam login, crash dump type, the plugin list) have no
/// meaning on Linux or belong to later work; the dialog disables them.
pub const WIDGETS: &[(&str, &str, &str)] = &[
    // General tab
    ("compactBox", "Settings", "compact_downloads"),
    ("showMetaBox", "Settings", "meta_downloads"),
    ("hideDownloadInstallBox", "Settings", "autohide_downloads"),
    ("localINIs", "Settings", "profile_local_inis"),
    ("localSaves", "Settings", "profile_local_saves"),
    ("automaticArchiveInvalidation", "Settings", "profile_archive_invalidation"),
    ("centerDialogs", "Settings", "center_dialogs"),
    ("changeGameConfirmation", "Settings", "show_change_game_confirmation"),
    ("showMenubarOnAlt", "Settings", "show_menubar_on_alt"),
    ("doubleClickPreviews", "Settings", "double_click_previews"),
    // Theme / mod list tab
    ("styleBox", "Settings", "style"),
    ("colorSeparatorsBox", "Settings", "colorSeparatorScrollbars"),
    ("displayForeignBox", "Settings", "display_foreign"),
    ("saveFiltersBox", "Settings", "save_filters"),
    ("checkUpdateInstallBox", "Settings", "autocheck_update_install"),
    ("autoCollapseDelayBox", "Settings", "auto_collapse_on_hover"),
    ("collapsibleSeparatorsAscBox", "Settings", "collapsible_separators_asc"),
    ("collapsibleSeparatorsDscBox", "Settings", "collapsible_separators_dsc"),
    ("collapsibleSeparatorsHighlightToBox", "Settings", "collapsible_separators_conflicts_to"),
    ("collapsibleSeparatorsHighlightFromBox", "Settings", "collapsible_separators_conflicts_from"),
    ("collapsibleSeparatorsIconsConflictsBox", "Settings", "collapsible_separators_icons_1"),
    ("collapsibleSeparatorsIconsFlagsBox", "Settings", "collapsible_separators_icons_2"),
    ("collapsibleSeparatorsIconsContentsBox", "Settings", "collapsible_separators_icons_3"),
    ("collapsibleSeparatorsIconsVersionBox", "Settings", "collapsible_separators_icons_7"),
    ("collapsibleSeparatorsPerProfileBox", "Settings", "collapsible_separators_per_profile"),
    // Paths tab
    ("baseDirEdit", "Settings", "base_directory"),
    ("downloadDirEdit", "Settings", "download_directory"),
    ("modDirEdit", "Settings", "mod_directory"),
    ("cacheDirEdit", "Settings", "cache_directory"),
    ("profilesDirEdit", "Settings", "profiles_directory"),
    ("overwriteDirEdit", "Settings", "overwrite_directory"),
    // Nexus tab
    ("endorsementBox", "Settings", "endorsement_integration"),
    ("trackedBox", "Settings", "tracked_integration"),
    ("categoryMappingsBox", "Settings", "category_mappings"),
    ("hideAPICounterBox", "Settings", "hide_api_counter"),
    // Workarounds tab
    ("forceEnableBox", "Settings", "force_enable_core_files"),
    ("enableArchiveParsingBox", "Settings", "archive_parsing_experimental"),
    ("lockGUIBox", "Settings", "lock_gui"),
    ("offlineBox", "Settings", "offline_mode"),
    ("proxyBox", "Settings", "use_proxy"),
    ("useCustomBrowser", "Settings", "use_custom_browser"),
    ("browserCommand", "Settings", "custom_browser"),
    ("execBlacklistBtn", "Settings", "executable_blacklist"),
    ("skipFileSuffixBtn", "Settings", "skip_file_suffixes"),
    ("strataEnvironmentBtn", "Settings", "strata_environment"),
    ("skipDirectoriesBtn", "Settings", "skip_directories"),
    // Diagnostics tab
    ("logLevelBox", "Settings", "log_level"),
    ("lootLogLevel", "Settings", "loot_log_level"),
    ("dumpsMaxEdit", "Settings", "crash_dumps_max"),
    // Leet's settings, in a group the dialog adds to the theme tab (Leet changes 8, 9, 11)
    ("leetAutoEnable", "Settings", "auto_enable_install"),
    ("leetCheckOverwrite", "Plugins", "Basic diagnosis plugin/check_overwrite"),
    ("leetSubSeparatorAlwaysShowSize", "Plugins", "Sub-Separator/alwaysShowSize"),
    ("leetSubSeparatorIndentation", "Plugins", "Sub-Separator/indentation"),
    ("leetSubSeparatorInvertAlt", "Plugins", "Sub-Separator/invertAltModifierWhenSeparatorIsCollapsed"),
    ("leetCheckFont", "Plugins", "Basic diagnosis plugin/check_font"),
    ("leetCheckAlternateGames", "Plugins", "Basic diagnosis plugin/check_alternategames"),
    // third-party MO2 plugins, built in
    ("builtinCrashLogLabeler", "PluginPersistance", "Crash Log Labeler/enabled"),
    ("builtinCrashLogOffline", "Plugins", "Crash Log Labeler/offline_mode"),
    ("builtinCrashLogViewer", "PluginPersistance", "Crash Log Viewer/enabled"),
];

/// A widget's current value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRow {
    pub widget: String,
    pub kind: Kind,
    /// Booleans as `true`/`false`, lists one item per line, paths shown resolved (with
    /// `%BASE_DIR%` and the defaults filled in, as the Paths tab shows them).
    pub value: String,
}

fn text_of(value: &Value) -> String {
    match value {
        Value::List(items) => items.join("\n"),
        other => other.to_text(),
    }
}

/// The base folder (`base_directory`, else the instance folder) on this machine.
fn base_dir(instance: &Instance) -> PathBuf {
    instance.paths.base.clone()
}

/// Every widget's value.
pub fn rows(instance: &Instance) -> Vec<SettingRow> {
    WIDGETS
        .iter()
        .filter_map(|(widget, section, key)| {
            let def = settings::find(section, key)?;
            let mut value = text_of(&instance.setting(section, key));
            if def.kind == Kind::Path {
                value = match *key {
                    "base_directory" => base_dir(instance).to_string_lossy().into_owned(),
                    _ => value.replace("%BASE_DIR%", &base_dir(instance).to_string_lossy()),
                };
            }
            if def.kind == Kind::Bool {
                value = Value::String(value).to_bool().to_string();
            }
            Some(SettingRow { widget: (*widget).to_owned(), kind: def.kind, value })
        })
        .collect()
}

/// Stores what the dialog shows (all widgets, changed or not; unchanged lines stay as they
/// are). Paths follow `PathsSettingsTab::update`: a folder that doesn't exist is created;
/// a folder that is the default under the base folder is stored as "not set"; the base
/// folder is "not set" when it is the instance folder. Returns whether the file changed.
pub fn store(instance: &mut Instance, values: &[(String, String)]) -> Result<bool> {
    let base = values
        .iter()
        .find(|(w, _)| w == "baseDirEdit")
        .map_or_else(|| base_dir(instance), |(_, v)| PathBuf::from(v.trim()));
    let mut changes: Vec<(&'static str, &'static str, Option<Value>)> = Vec::new();
    for (widget, raw) in values {
        let Some((_, section, key)) = WIDGETS.iter().find(|(w, _, _)| w == widget) else { continue };
        let Some(def) = settings::find(section, key) else { continue };
        let value = match def.kind {
            Kind::Bool => Some(Value::String(if raw == "true" { "true" } else { "false" }.into())),
            Kind::Int => Some(Value::String(raw.trim().parse::<i64>().map_or_else(|_| def.default.to_owned(), |n| n.to_string()))),
            Kind::Text => Some(Value::String(raw.clone())),
            Kind::List => Some(Value::List(raw.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect())),
            Kind::Path => store_path(instance, key, raw.trim(), &base)?,
        };
        changes.push((section, key, value));
    }
    Ok(instance.store_settings(&changes)?)
}

fn store_path(instance: &Instance, key: &str, shown: &str, base: &Path) -> Result<Option<Value>> {
    if key == "base_directory" {
        return Ok((Path::new(shown) != instance.root.as_path() && !shown.is_empty()).then(|| Value::String(shown.to_owned())));
    }
    let default_name = settings::find("Settings", key).map(|s| s.default.trim_start_matches("%BASE_DIR%/")).unwrap_or("");
    let real = PathBuf::from(shown.replace("%BASE_DIR%", &base.to_string_lossy()));
    if shown.is_empty() || real == base.join(default_name) {
        return Ok(None);
    }
    if !real.is_dir() {
        std::fs::create_dir_all(&real).map_err(|source| crate::Error::Write { path: real.clone(), source })?;
    }
    // A Windows path stays Windows (MO2 and Fluorine read either).
    let stored = if shown.len() > 1 && shown.as_bytes()[1] == b':' { shown.to_owned() } else { real.to_string_lossy().into_owned() };
    Ok(Some(Value::String(stored)))
}

/// The Theme tab's colour table (MO2's `ColorTable::load`): the six highlight colours, each
/// from `[Settings]` (`ColorSettings`), else MO2's default. Shown, not edited, in Strata.
pub fn highlight_colors(instance: &Instance) -> Vec<(&'static str, strata_core::Color)> {
    let c = |r, g, b| strata_core::Color { r, g, b, a: 64 };
    let rows = [
        ("Is overwritten (loose files)", "overwrittenLooseFilesColor", c(0, 255, 0)),
        ("Is overwriting (loose files)", "overwritingLooseFilesColor", c(255, 0, 0)),
        ("Is overwritten (archives)", "overwrittenArchiveFilesColor", c(0, 255, 255)),
        ("Is overwriting (archives)", "overwritingArchiveFilesColor", c(255, 0, 255)),
        ("Mod contains selected plugin", "containsPluginColor", c(0, 0, 255)),
        ("Plugin is contained in selected mod", "containedColor", c(0, 0, 255)),
    ];
    rows.into_iter()
        .map(|(caption, key, default)| {
            let stored = instance.ini.value("Settings", key).as_ref().and_then(strata_core::color_from_value);
            (caption, stored.unwrap_or(default))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(root: &Path) -> Instance {
        std::fs::create_dir_all(root.join("profiles/P")).unwrap();
        std::fs::write(root.join("profiles/P/modlist.txt"), "").unwrap();
        std::fs::write(
            root.join("ModOrganizer.ini"),
            "[General]\ngameName=Skyrim Special Edition\nselected_profile=@ByteArray(P)\ngamePath=/g\n\n[Settings]\nlock_gui=false\nskip_directories=.git, .svn\n",
        )
        .unwrap();
        Instance::open(root).unwrap()
    }

    #[test]
    fn the_colour_table_shows_stored_colours_or_mo2s_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        instance(tmp.path());
        // as MO2 (QSettings) wrote it in a real instance: quoted because of the `;`
        let ini = tmp.path().join("ModOrganizer.ini");
        let text = std::fs::read_to_string(&ini).unwrap()
            + "containedColor=\"@Variant(\\0\\0\\0\\x43\\x1\\xff\\xff\\xad\\xad\\x37\\x37;;\\0\\0)\"\n";
        std::fs::write(&ini, text).unwrap();
        let inst = Instance::open(tmp.path()).unwrap();
        let red = strata_core::Color { r: 0xad, g: 0x37, b: 0x3b, a: 255 };
        let rows = highlight_colors(&inst);
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0].1, strata_core::Color { r: 0, g: 255, b: 0, a: 64 });
        assert_eq!(rows[5], ("Plugin is contained in selected mod", red));
    }

    #[test]
    fn every_widget_names_a_known_setting() {
        for (widget, section, key) in WIDGETS {
            assert!(settings::find(section, key).is_some(), "{widget}: {section}/{key}");
        }
    }

    #[test]
    fn rows_show_values_and_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = instance(tmp.path());
        let rows = rows(&inst);
        let get = |w: &str| rows.iter().find(|r| r.widget == w).unwrap().value.clone();
        assert_eq!(get("lockGUIBox"), "false");
        assert_eq!(get("displayForeignBox"), "true", "MO2's default");
        assert_eq!(get("skipDirectoriesBtn"), ".git\n.svn");
        assert_eq!(get("modDirEdit"), format!("{}/mods", tmp.path().display()));
        assert_eq!(get("logLevelBox"), "1");
    }

    #[test]
    fn storing_changes_only_what_changed_and_paths_as_mo2() {
        let tmp = tempfile::tempdir().unwrap();
        let mut inst = instance(tmp.path());
        let before = std::fs::read_to_string(tmp.path().join("ModOrganizer.ini")).unwrap();
        // the dialog shown and closed with OK: values equal to the defaults are written
        // (as MO2 does), the default paths are not
        let all: Vec<(String, String)> = rows(&inst).into_iter().map(|r| (r.widget, r.value)).collect();
        store(&mut inst, &all).unwrap();
        let after = std::fs::read_to_string(tmp.path().join("ModOrganizer.ini")).unwrap();
        assert!(after.starts_with(&before[..before.find("lock_gui").unwrap()]), "{after}");
        assert!(!after.contains("mod_directory") && !after.contains("base_directory"), "{after}");
        assert!(after.contains("skip_directories=.git, .svn\n"));

        let elsewhere = tmp.path().join("elsewhere/mods");
        store(&mut inst, &[("modDirEdit".into(), elsewhere.to_string_lossy().into_owned()), ("lockGUIBox".into(), "true".into())]).unwrap();
        assert!(elsewhere.is_dir(), "a new folder is made");
        let after = std::fs::read_to_string(tmp.path().join("ModOrganizer.ini")).unwrap();
        assert!(after.contains(&format!("mod_directory={}\n", elsewhere.display())) && after.contains("lock_gui=true\n"), "{after}");
        let back = format!("{}/mods", tmp.path().display());
        store(&mut inst, &[("modDirEdit".into(), back)]).unwrap();
        assert!(!std::fs::read_to_string(tmp.path().join("ModOrganizer.ini")).unwrap().contains("mod_directory"));
    }
}
