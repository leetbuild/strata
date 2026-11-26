//! The `cxx` bridge: what the shell can ask the core. Every function here catches panics
//! (unwinding into C++ is undefined behaviour): a function that returns `Result` turns a
//! panic into an error, the others into an empty value. The logic lives in `strata-ui`;
//! this file only converts.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::pin::Pin;

use strata_ini::Value;
use strata_ui::mods::RowKind;
use crate::{guarded, quiet};

#[cxx::bridge(namespace = "strata")]
mod ffi {
    /// One row of the mod list (`strata_ui::mods::ModRow`). Absent numbers are -1.
    struct ModRow {
        name: String,
        display_name: String,
        /// 0 regular, 1 separator, 2 foreign, 3 backup, 4 overwrite.
        kind: u8,
        enabled: bool,
        checkable: bool,
        priority: i64,
        has_color: bool,
        /// 0xRRGGBBAA.
        color: u32,
        nest: i32,
        stored_size: bool,
        version: String,
        /// `newestVersion`; `update` 0 unknown, 1 current, 2 update available, 3 downgrade.
        newest_version: String,
        update: u8,
        category: String,
        nexus_id: i64,
        comments: String,
        install_time: i64,
        has_files: bool,
        /// Index of the parent separator in the same list (ascending priority), -1 at the top.
        parent: i64,
        /// The Flags column (without the conflict index's "hidden files"): icons and texts.
        flag_icons: Vec<String>,
        flag_texts: Vec<String>,
        /// The long notes (the notes flag's tooltip).
        notes: String,
        /// The Content column: ids (`strata_ui::content::Content`), icons ("" = none), names.
        contents: Vec<u8>,
        content_icons: Vec<String>,
        content_names: Vec<String>,
        /// Every existing category id; endorsed, tracked, valid game data (the filters).
        categories: Vec<i32>,
        endorsed: bool,
        tracked: bool,
        valid: bool,
    }

    /// An entry of the filter panel; `kind` 0 special, 1 content, 2 category.
    struct FilterEntry {
        kind: u8,
        id: i32,
        name: String,
    }

    /// A ticked filter entry; `kind` 0 special or category, 1 content.
    struct Criterion {
        kind: u8,
        id: i32,
        inverse: bool,
    }

    /// What the filter criteria look at for one mod (`strata_ui::filter::Facts`).
    struct Facts {
        enabled: bool,
        always_enabled: bool,
        separator: bool,
        backup: bool,
        foreign: bool,
        overwrite: bool,
        categories: Vec<i32>,
        conflicted: bool,
        hidden_files: bool,
        endorsed: bool,
        tracked: bool,
        valid: bool,
        nexus_id: i64,
        contents: Vec<u8>,
        update_available: bool,
    }

    /// One file of Overwrite for "Sync to Mods..." (`strata_ui::listtools::SyncFile`).
    struct SyncFile {
        path: String,
        mods: Vec<String>,
    }

    /// A mod's conflicts (`strata_ui::conflicts::ConflictRow`): four icon slots ("" = empty).
    #[derive(Default)]
    struct ConflictRow {
        icons: Vec<String>,
        texts: Vec<String>,
        hidden_files: bool,
    }

    /// What a selection highlights, by mod name.
    #[derive(Default)]
    struct Markers {
        overwriting: Vec<String>,
        overwritten: Vec<String>,
        archive_overwriting: Vec<String>,
        archive_overwritten: Vec<String>,
    }

    /// The highlight colours, 0xRRGGBBAA.
    #[derive(Default)]
    struct MarkerColors {
        overwriting_loose: u32,
        overwritten_loose: u32,
        overwriting_archive: u32,
        overwritten_archive: u32,
    }

    /// A change event (`strata_core::ModListEvent`). `kind`: 0 state (names, enabled),
    /// 1 moved (name, from, to), 2 nest (name, to = level), 3 renamed (name -> new_name),
    /// 4 added (name, to = priority), 5 meta (name), 6 overwrite, 7 removed (name, from = priority).
    struct ModEvent {
        kind: u8,
        names: Vec<String>,
        enabled: bool,
        name: String,
        new_name: String,
        from: i64,
        to: i64,
    }

    /// A row of the plugin list (`strata_ui::plugins::PluginRow`).
    struct PluginRow {
        name: String,
        origin: String,
        enabled: bool,
        force_loaded: bool,
        force_disabled: bool,
        priority: i64,
        index: String,
        bold: bool,
        italic: bool,
        locked: bool,
        icons: Vec<String>,
        tooltip: String,
    }

    struct ExecutableRow {
        title: String,
        binary: String,
        /// Shown on the toolbar (`Executable::ShowInToolbar`).
        toolbar: bool,
        /// "Hide in user interface": not in the executables list.
        hide: bool,
    }

    struct DownloadRow {
        file_name: String,
        display_name: String,
        status: String,
        size: u64,
        file_time: i64,
        mod_name: String,
        version: String,
        nexus_id: String,
        game: String,
        hidden: bool,
    }

    /// An archive of the Archives tab (`strata_ui::plugins::ArchiveRow`).
    struct ArchiveRow {
        name: String,
        origin: String,
        loaded: bool,
    }

    /// A save (`strata_ui::saves::SaveRow`); `created` -1 when unreadable; `screenshot` RGBA.
    struct SaveRow {
        name: String,
        file: String,
        path: String,
        created: i64,
        tooltip: String,
        missing: Vec<String>,
        screenshot: Vec<u8>,
        width: u32,
        height: u32,
    }

    struct DataEntry {
        name: String,
        is_dir: bool,
        origin: String,
        size: u64,
        modified: i64,
        /// Inside a loaded archive (the "Archives" box).
        from_archive: bool,
    }

    #[derive(Default)]
    struct StoredList {
        found: bool,
        items: Vec<String>,
    }

    /// The instance's folders, for the "open folder" menu.
    #[derive(Default)]
    struct Folders {
        instance: String,
        mods: String,
        downloads: String,
        overwrite: String,
        profile: String,
        game: String,
    }

    /// A value read from `ModOrganizer.ini`.
    #[derive(Default)]
    struct StoredValue {
        found: bool,
        bytes: Vec<u8>,
    }

    /// A value to store in `ModOrganizer.ini`. `kind` 0: plain text (numbers, `true`);
    /// 1: `@ByteArray(...)` (window states); 2: a Qt value already in its ini form, such as
    /// `@Size(32 32)`, written as it is; 3: a list of strings, `\n`-separated in `bytes`,
    /// written as `QSettings` writes a `QVariantList`.
    struct SettingValue {
        section: String,
        key: String,
        bytes: Vec<u8>,
        kind: u8,
    }

    extern "Rust" {
        /// Strata's version, e.g. `0.1.0`.
        fn version() -> String;
        /// The game's Nexus page ("" for a game Strata doesn't know): "Visit Nexus".
        fn nexus_game_url(game_name: &str) -> String;
        /// The archive types the installers take (a drop on the mod list installs them).
        fn archive_extensions() -> Vec<String>;

        /// The instance folder the window opens (`strata_ui::config::choose_instance`);
        /// `given` is `-i`'s value or empty.
        fn choose_instance(given: &str) -> Result<String>;
        /// Stores `root` as the last instance opened.
        fn remember_instance(root: &str) -> Result<()>;

        /// The stylesheet `name` (`Night Eyes.qss`) in the folders `strata_ui::style` searches;
        /// empty when not found.
        fn find_stylesheet(name: &str, instance_root: &str, program_dir: &str) -> String;
        /// Those folders, first match wins.
        fn stylesheet_dirs(instance_root: &str, program_dir: &str) -> Vec<String>;
        /// The sheet used when the instance names none: `riki.qss` when found, else `vs15 Dark.qss`.
        fn default_stylesheet(instance_root: &str, program_dir: &str) -> String;

        /// The mod list filter (`strata_ui::filter::matches`). Empty `comments` / `category`
        /// and a negative `nexus_id` stand for hidden columns or no value.
        fn mod_matches_filter(filter: &str, name: &str, comments: &str, category: &str, nexus_id: i64) -> bool;

        /// The filter panel's verdict for each mod (`strata_ui::filter::matches_criteria`); `all`: And, else Or;
        /// `separators` 0 filter, 1 show, 2 hide.
        fn criteria_matches(criteria: Vec<Criterion>, all: bool, separators: u8, facts: Vec<Facts>) -> Vec<bool>;

        type Session;
        /// Opens the instance in `root` and `profile` (empty: the selected one).
        fn open_session(root: &str, profile: &str) -> Result<Box<Session>>;
        fn root(self: &Session) -> String;
        fn game_name(self: &Session) -> String;
        fn profile_name(self: &Session) -> String;
        fn profile_names(self: &Session) -> Result<Vec<String>>;
        fn folders(self: &Session) -> Folders;
        /// The instance's style (`[Settings] style`) or the default.
        fn style(self: &Session) -> String;
        /// `[Settings] display_foreign` (MO2's default: true).
        fn display_foreign(self: &Session) -> bool;
        fn mods(self: Pin<&mut Session>) -> Result<Vec<ModRow>>;
        fn filter_entries(self: Pin<&mut Session>) -> Vec<FilterEntry>;

        // Mod list operations (strata_core's, through strata_ui::Session). Each saves the
        // list and returns the core's change events. Names are internal names.
        fn set_enabled(self: Pin<&mut Session>, names: Vec<String>, enabled: bool) -> Result<Vec<ModEvent>>;
        /// `priority` -1 = the end (the bottom of the list).
        fn move_mods(self: Pin<&mut Session>, names: Vec<String>, priority: i64) -> Result<Vec<ModEvent>>;
        fn send_to_separator(self: Pin<&mut Session>, names: Vec<String>, separator: &str) -> Result<Vec<ModEvent>>;
        /// `parent` "" = the top level; `row` -1 = onto the separator `parent` itself.
        fn drop_mods(
            self: Pin<&mut Session>,
            names: Vec<String>,
            parent: &str,
            row: i64,
            below_expanded_separator: bool,
        ) -> Result<Vec<ModEvent>>;
        fn rename_mod(self: Pin<&mut Session>, old: &str, new_name: &str) -> Result<Vec<ModEvent>>;
        /// `at` "" = the end of the list.
        fn create_separator(self: Pin<&mut Session>, name: &str, at: &str) -> Result<Vec<ModEvent>>;
        /// Ctrl+Up / Ctrl+Down: the mods `offset` priorities further.
        fn shift_mods(self: Pin<&mut Session>, names: Vec<String>, offset: i64) -> Result<Vec<ModEvent>>;
        /// Space: every enabled one off, every disabled one on.
        fn toggle_mods(self: Pin<&mut Session>, names: Vec<String>) -> Result<Vec<ModEvent>>;
        /// A separator's Sub-Separator level.
        fn set_nest(self: Pin<&mut Session>, name: &str, nest: u32) -> Result<Vec<ModEvent>>;
        /// Overwrite's content into a new mod `name` (`create`) or an existing one.
        fn move_overwrite(self: Pin<&mut Session>, name: &str, create: bool) -> Result<Vec<ModEvent>>;
        /// Everything in overwrite to the trash (or for good).
        fn clear_overwrite(self: Pin<&mut Session>, trash: bool) -> Result<Vec<ModEvent>>;
        /// "Export to csv...": `which` 0 all, 1 active, 2 the `visible` names; `fields` of
        /// `#Mod_Priority`, `#Mod_Status`, `#Mod_Name`, `#Note`, `#Primary_Category`,
        /// `#Nexus_ID`, `#Mod_Nexus_URL`, `#Mod_Version`, `#Install_Date`, `#Download_File_Name`.
        fn export_csv(self: Pin<&mut Session>, which: u8, visible: Vec<String>, fields: Vec<String>) -> String;
        /// "Auto assign categories": the mods whose categories changed.
        fn auto_assign_categories(self: Pin<&mut Session>) -> Result<Vec<String>>;
        /// "Sync to Mods...": Overwrite's files (paths relative to it) and, per file, the mods
        /// that have it, the default first (empty: none).
        fn sync_overwrite_files(self: &Session) -> Result<Vec<SyncFile>>;
        /// Moves each `paths[i]` into `mods[i]`; the messages of files that failed.
        fn sync_overwrite(self: &Session, paths: Vec<String>, mods: Vec<String>) -> Result<Vec<String>>;
        /// The overwrite dialog: delete (paths relative to overwrite), rename, new folder
        /// (returns its relative path).
        fn remove_from_overwrite(self: &Session, paths: Vec<String>, trash: bool) -> Result<()>;
        fn rename_in_overwrite(self: &Session, path: &str, new_name: &str) -> Result<()>;
        fn new_folder_in_overwrite(self: &Session, dir: &str) -> Result<String>;
        /// "Create empty mod": `at` "" = on top, else above that mod / inside that separator.
        fn create_empty_mod(self: Pin<&mut Session>, name: &str, at: &str) -> Result<Vec<ModEvent>>;
        /// A folder dropped on the mod list: a new mod `name` with a copy of its content.
        fn create_mod_from_folder(self: Pin<&mut Session>, name: &str, folder: &str) -> Result<Vec<ModEvent>>;
        fn set_comments(self: Pin<&mut Session>, name: &str, text: &str) -> Result<Vec<ModEvent>>;
        /// `rgba` 0xRRGGBBAA; `has_color` false clears it.
        fn set_color(self: Pin<&mut Session>, name: &str, has_color: bool, rgba: u32) -> Result<Vec<ModEvent>>;
        /// Removes mods: to the user's trash, or for good (`trash` false).
        fn remove_mods(self: Pin<&mut Session>, names: Vec<String>, trash: bool) -> Result<Vec<ModEvent>>;
        /// The profile row's backup buttons: a backup of `modlist.txt` (its file name) or of
        /// the load order files (how many); the backups of either (suffixes, newest first);
        /// restoring one.
        fn backup_mod_list(self: &Session) -> Result<String>;
        fn backup_plugins(self: &Session) -> Result<usize>;
        fn backups(self: &Session, mods: bool) -> Result<Vec<String>>;
        fn restore_backup(self: Pin<&mut Session>, mods: bool, suffix: &str) -> Result<()>;
        /// Makes the session's profile the instance's selected one.
        fn select_profile(self: Pin<&mut Session>, name: &str) -> Result<()>;
        /// The profile a live deploy shows ("" when nothing is deployed).
        fn deployed_profile(self: &Session) -> Result<String>;
        /// A mod's folder ("" for foreign mods).
        fn mod_path(self: &Session, name: &str) -> String;
        /// Sends the profile's layers to a live deploy of it; a line for the log.
        fn refresh_deploy(self: &Session) -> Result<String>;
        /// The plugin list (scans the merged Data folder: on a worker, own session).
        fn build_plugins(self: &Session) -> Result<Box<Plugins>>;
        /// The Sort button: LOOT on the profile's plugins (on a worker); a line for the log.
        fn sort_plugins(self: &Session) -> Result<String>;
        fn executables(self: &Session) -> Vec<ExecutableRow>;
        fn downloads(self: &Session) -> Vec<DownloadRow>;
        /// The saves, with the plugins each misses from `plugins` (on a worker).
        fn saves(self: &Session, plugins: &Plugins) -> Result<Vec<SaveRow>>;
        /// Scans the merged tree for the Data tab. Slow on big instances: call it off the
        /// UI thread, on a session of its own.
        fn build_data_view(self: &Session) -> Result<Box<DataView>>;
        fn load_setting(self: &Session, section: &str, key: &str) -> StoredValue;
        /// A list of strings (`strata_ui::settings::string_list`); `found` false when absent
        /// or not a list.
        fn load_string_list(self: &Session, section: &str, key: &str) -> StoredList;
        /// Stores the values in `ModOrganizer.ini`; returns whether the file changed.
        fn store_settings(self: &Session, values: Vec<SettingValue>) -> Result<bool>;

        /// The conflict index of the profile. Slow on big instances: on a worker, with a
        /// session of its own.
        fn build_conflicts(self: &Session) -> Result<Box<Conflicts>>;
        fn marker_colors(self: &Session) -> MarkerColors;

        type Plugins;
        fn rows(self: &Plugins) -> Vec<PluginRow>;
        /// The Archives tab, grouped by mod in priority order.
        fn archives(self: &Plugins) -> Vec<ArchiveRow>;
        /// Saves `plugins.txt` / `loadorder.txt`; returns the plugins that changed.
        fn set_enabled(self: Pin<&mut Plugins>, names: Vec<String>, enabled: bool) -> Result<Vec<String>>;
        /// Every plugin but the forced ones; saves; returns the plugins that changed.
        fn set_enabled_all(self: Pin<&mut Plugins>, enabled: bool) -> Result<Vec<String>>;
        /// `priority` -1 = the bottom.
        fn send_to_priority(self: Pin<&mut Plugins>, names: Vec<String>, priority: i64) -> Result<()>;
        fn set_locked(self: Pin<&mut Plugins>, name: &str, lock: bool) -> Result<()>;

        type Conflicts;
        /// After the layers changed (`session` = the profile as it is now on disk): only the
        /// mods concerned are recomputed; returns their names.
        fn update(self: Pin<&mut Conflicts>, session: &Session) -> Result<Vec<String>>;
        fn row(self: &Conflicts, name: &str) -> ConflictRow;
        fn markers(self: &Conflicts, names: Vec<String>) -> Markers;

        type DataView;
        /// The entries of a folder of the merged Data folder ("" = Data).
        fn children(self: &DataView, path: &str) -> Vec<DataEntry>;
        /// A folder of the Data tab (`area` 0) or the Root tab (1) with the tab's filter,
        /// "conflicts only" and "Archives" (after `load_archives`).
        fn query(self: &DataView, area: u8, path: &str, filter: &str, conflicts_only: bool, archives: bool) -> Vec<DataEntry>;
        /// Reads the loaded archives' file lists for the "Archives" box. Slow: on a worker.
        fn load_archives(self: Pin<&mut DataView>, session: &Session) -> Result<()>;
        fn file_count(self: &DataView) -> usize;
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn non_empty(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}

fn nexus_game_url(game_name: &str) -> String {
    quiet(|| strata_ui::links::nexus_game_url(game_name).unwrap_or_default())
}

fn archive_extensions() -> Vec<String> {
    quiet(|| strata_install::installers::ARCHIVE_EXTENSIONS.iter().map(|e| (*e).to_owned()).collect())
}

fn version() -> String {
    quiet(|| env!("CARGO_PKG_VERSION").to_owned())
}

fn choose_instance(given: &str) -> Result<String, String> {
    guarded(|| {
        let cwd = std::env::current_dir().map_err(|e| format!("cannot read the current folder: {e}"))?;
        let config = strata_ui::config::config_file();
        strata_ui::config::choose_instance(non_empty(given).map(Path::new), &cwd, config.as_deref())
            .map(|p| text(&p))
            .map_err(|e| e.to_string())
    })
}

fn remember_instance(root: &str) -> Result<(), String> {
    guarded(|| {
        let Some(config) = strata_ui::config::config_file() else { return Ok(()) };
        strata_ui::config::remember_instance(&config, Path::new(root)).map_err(|e| e.to_string())?;
        // the instance manager lists it from now on, wherever it is
        strata_ui::instances::remember(&config, Path::new(root)).map_err(|e| e.to_string())
    })
}

fn search_dirs(instance_root: &str, program_dir: &str) -> Vec<std::path::PathBuf> {
    let home = strata_ui::style::data_home();
    strata_ui::style::search_dirs(non_empty(instance_root).map(Path::new), Path::new(program_dir), home.as_deref())
}

fn default_stylesheet(instance_root: &str, program_dir: &str) -> String {
    quiet(|| strata_ui::style::default_style(&search_dirs(instance_root, program_dir)).to_owned())
}

fn find_stylesheet(name: &str, instance_root: &str, program_dir: &str) -> String {
    quiet(|| {
        strata_ui::style::find(name, &search_dirs(instance_root, program_dir)).map(|p| text(&p)).unwrap_or_default()
    })
}

fn stylesheet_dirs(instance_root: &str, program_dir: &str) -> Vec<String> {
    quiet(|| search_dirs(instance_root, program_dir).iter().map(|p| text(p)).collect())
}

fn mod_matches_filter(filter: &str, name: &str, comments: &str, category: &str, nexus_id: i64) -> bool {
    catch_unwind(|| {
        let categories = [category];
        strata_ui::filter::matches(
            filter,
            strata_ui::filter::Fields {
                name,
                comments: non_empty(comments),
                categories: non_empty(category).map(|_| &categories[..]),
                nexus_id: (nexus_id > 0).then_some(nexus_id),
            },
        )
    })
    .unwrap_or(true)
}

/// The core's events, for C++.
fn events(result: strata_ui::Result<Vec<strata_core::ModListEvent>>) -> Result<Vec<ffi::ModEvent>, String> {
    use strata_core::ModListEvent as E;
    let list = result.map_err(|e| e.to_string())?;
    let blank = || ffi::ModEvent {
        kind: 0,
        names: Vec::new(),
        enabled: false,
        name: String::new(),
        new_name: String::new(),
        from: -1,
        to: -1,
    };
    let as_i64 = |v: usize| i64::try_from(v).unwrap_or(i64::MAX);
    Ok(list
        .into_iter()
        .map(|e| match e {
            E::StateChanged { names, enabled } => ffi::ModEvent { kind: 0, names, enabled, ..blank() },
            E::Moved { name, from, to } => ffi::ModEvent { kind: 1, name, from: as_i64(from), to: as_i64(to), ..blank() },
            E::NestChanged { name, nest } => ffi::ModEvent { kind: 2, name, to: i64::from(nest), ..blank() },
            E::Renamed { from, to } => ffi::ModEvent { kind: 3, name: from, new_name: to, ..blank() },
            E::Added { name, priority } => ffi::ModEvent { kind: 4, name, to: as_i64(priority), ..blank() },
            E::MetaChanged { name } => ffi::ModEvent { kind: 5, name, ..blank() },
            E::OverwriteChanged => ffi::ModEvent { kind: 6, ..blank() },
            E::Removed { name, priority } => ffi::ModEvent { kind: 7, name, from: as_i64(priority), ..blank() },
        })
        .collect())
}

fn criteria_matches(criteria: Vec<ffi::Criterion>, all: bool, separators: u8, facts: Vec<ffi::Facts>) -> Vec<bool> {
    use strata_ui::filter::{Criterion, Facts, Separators, matches_criteria};
    let n = facts.len();
    catch_unwind(AssertUnwindSafe(|| {
        let criteria: Vec<Criterion> = criteria
            .iter()
            .map(|c| match c.kind {
                1 => Criterion::Content { id: u8::try_from(c.id).unwrap_or(u8::MAX), inverse: c.inverse },
                _ => Criterion::Category { id: c.id, inverse: c.inverse },
            })
            .collect();
        let separators = match separators {
            1 => Separators::Show,
            2 => Separators::Hide,
            _ => Separators::Filter,
        };
        facts
            .into_iter()
            .map(|f| {
                let facts = Facts {
                    enabled: f.enabled,
                    always_enabled: f.always_enabled,
                    separator: f.separator,
                    backup: f.backup,
                    foreign: f.foreign,
                    overwrite: f.overwrite,
                    categories: f.categories,
                    conflicted: f.conflicted,
                    hidden_files: f.hidden_files,
                    endorsed: f.endorsed,
                    tracked: f.tracked,
                    valid: f.valid,
                    nexus_id: f.nexus_id,
                    contents: f.contents,
                    update_available: f.update_available,
                };
                matches_criteria(&criteria, all, separators, &facts)
            })
            .collect()
    }))
    .unwrap_or_else(|_| vec![true; n])
}

/// An opened instance and profile (`strata_ui::Session`).
pub struct Session(strata_ui::Session);

fn open_session(root: &str, profile: &str) -> Result<Box<Session>, String> {
    guarded(|| {
        strata_ui::Session::open(Path::new(root), non_empty(profile)).map(|s| Box::new(Session(s))).map_err(|e| e.to_string())
    })
}

fn opt(value: Option<i64>) -> i64 {
    value.unwrap_or(-1)
}

fn index(value: Option<usize>) -> i64 {
    value.and_then(|v| i64::try_from(v).ok()).unwrap_or(-1)
}

impl Session {
    fn root(&self) -> String {
        quiet(|| text(&self.0.instance().root))
    }

    fn game_name(&self) -> String {
        quiet(|| self.0.instance().game.name().to_owned())
    }

    fn profile_name(&self) -> String {
        quiet(|| self.0.profile().name.clone())
    }

    fn folders(&self) -> ffi::Folders {
        quiet(|| {
            let instance = self.0.instance();
            ffi::Folders {
                instance: text(&instance.root),
                mods: text(&instance.paths.mods),
                downloads: text(&instance.paths.downloads),
                overwrite: text(&instance.paths.overwrite),
                profile: text(&self.0.profile().dir),
                game: text(&instance.game_path),
            }
        })
    }

    fn profile_names(&self) -> Result<Vec<String>, String> {
        guarded(|| self.0.profile_names().map_err(|e| e.to_string()))
    }

    fn style(&self) -> String {
        quiet(|| self.0.style())
    }

    fn display_foreign(&self) -> bool {
        // InterfaceSettings::displayForeign: true unless the ini says otherwise.
        catch_unwind(AssertUnwindSafe(|| {
            self.0.instance().ini.value("Settings", "display_foreign").is_none_or(|v| v.to_bool())
        }))
        .unwrap_or(true)
    }

    fn mods(self: Pin<&mut Self>) -> Result<Vec<ffi::ModRow>, String> {
        let this = self.get_mut();
        guarded(|| {
            let rows = this.0.mods();
            Ok(rows
                .into_iter()
                .map(|r| ffi::ModRow {
                    kind: match r.kind {
                        RowKind::Regular => 0,
                        RowKind::Separator => 1,
                        RowKind::Foreign => 2,
                        RowKind::Backup => 3,
                        RowKind::Overwrite => 4,
                    },
                    enabled: r.enabled,
                    checkable: r.checkable,
                    priority: index(r.priority),
                    has_color: r.color.is_some(),
                    color: r.color.map_or(0, u32::from_be_bytes),
                    nest: r.nest,
                    stored_size: r.stored_size,
                    version: r.version,
                    newest_version: r.newest_version,
                    update: match r.update {
                        None => 0,
                        Some(std::cmp::Ordering::Equal) => 1,
                        Some(std::cmp::Ordering::Less) => 2,
                        Some(std::cmp::Ordering::Greater) => 3,
                    },
                    category: r.category,
                    nexus_id: opt(r.nexus_id),
                    comments: r.comments,
                    install_time: opt(r.install_time),
                    has_files: r.has_files,
                    parent: index(r.parent),
                    flag_icons: r.flags.iter().map(|f| f.icon().to_owned()).collect(),
                    flag_texts: r.flags.iter().map(|f| f.text().to_owned()).collect(),
                    notes: r.notes,
                    contents: r.contents.iter().map(|c| *c as u8).collect(),
                    content_icons: r.contents.iter().map(|c| c.icon().unwrap_or("").to_owned()).collect(),
                    content_names: r.contents.iter().map(|c| c.name().to_owned()).collect(),
                    categories: r.categories,
                    endorsed: r.endorsed,
                    tracked: r.tracked,
                    valid: r.valid,
                    name: r.name,
                    display_name: r.display_name,
                })
                .collect())
        })
    }

    fn filter_entries(self: Pin<&mut Self>) -> Vec<ffi::FilterEntry> {
        let this = self.get_mut();
        quiet(|| {
            use strata_ui::filter::PanelKind;
            this.0
                .filter_entries()
                .into_iter()
                .map(|e| ffi::FilterEntry {
                    kind: match e.kind {
                        PanelKind::Special => 0,
                        PanelKind::Content => 1,
                        PanelKind::Category => 2,
                    },
                    id: e.id,
                    name: e.name,
                })
                .collect()
        })
    }

    fn set_enabled(self: Pin<&mut Self>, names: Vec<String>, enabled: bool) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| events(this.0.set_enabled(&names, enabled)))
    }

    fn move_mods(self: Pin<&mut Self>, names: Vec<String>, priority: i64) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        let priority = usize::try_from(priority).unwrap_or(usize::MAX);
        guarded(|| events(this.0.move_mods(&names, priority)))
    }

    fn send_to_separator(self: Pin<&mut Self>, names: Vec<String>, separator: &str) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| events(this.0.send_to_separator(&names, separator)))
    }

    fn drop_mods(
        self: Pin<&mut Self>,
        names: Vec<String>,
        parent: &str,
        row: i64,
        below_expanded_separator: bool,
    ) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        let row = usize::try_from(row).ok();
        guarded(|| events(this.0.drop_mods(&names, non_empty(parent), row, below_expanded_separator)))
    }

    fn rename_mod(self: Pin<&mut Self>, old: &str, new_name: &str) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| events(this.0.rename_mod(old, new_name)))
    }

    fn create_separator(self: Pin<&mut Self>, name: &str, at: &str) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| events(this.0.create_separator(name, non_empty(at))))
    }

    fn shift_mods(self: Pin<&mut Self>, names: Vec<String>, offset: i64) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| events(this.0.shift_mods(&names, isize::try_from(offset).unwrap_or(0))))
    }

    fn toggle_mods(self: Pin<&mut Self>, names: Vec<String>) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| events(this.0.toggle_mods(&names)))
    }

    fn set_nest(self: Pin<&mut Self>, name: &str, nest: u32) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| events(this.0.set_nest(name, nest)))
    }

    fn move_overwrite(self: Pin<&mut Self>, name: &str, create: bool) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| events(this.0.move_overwrite(name, create)))
    }

    fn clear_overwrite(self: Pin<&mut Self>, trash: bool) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| events(this.0.clear_overwrite(trash)))
    }

    fn export_csv(self: Pin<&mut Self>, which: u8, visible: Vec<String>, fields: Vec<String>) -> String {
        use strata_ui::listtools::CsvRows;
        let this = self.get_mut();
        let which = match which {
            1 => CsvRows::Active,
            2 => CsvRows::Visible,
            _ => CsvRows::All,
        };
        quiet(|| this.0.export_csv(which, &visible, &fields))
    }

    fn auto_assign_categories(self: Pin<&mut Self>) -> Result<Vec<String>, String> {
        let this = self.get_mut();
        guarded(|| this.0.auto_assign_categories().map_err(|e| e.to_string()))
    }

    fn sync_overwrite_files(&self) -> Result<Vec<ffi::SyncFile>, String> {
        guarded(|| {
            Ok(self
                .0
                .sync_overwrite_files()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|f| ffi::SyncFile { path: f.path, mods: f.mods })
                .collect())
        })
    }

    fn sync_overwrite(&self, paths: Vec<String>, mods: Vec<String>) -> Result<Vec<String>, String> {
        guarded(|| {
            let moves: Vec<(String, String)> = paths.into_iter().zip(mods).collect();
            self.0.sync_overwrite(&moves).map(|(_, errors)| errors).map_err(|e| e.to_string())
        })
    }

    fn remove_from_overwrite(&self, paths: Vec<String>, trash: bool) -> Result<(), String> {
        guarded(|| self.0.remove_from_overwrite(&paths, trash).map_err(|e| e.to_string()))
    }

    fn rename_in_overwrite(&self, path: &str, new_name: &str) -> Result<(), String> {
        guarded(|| self.0.rename_in_overwrite(path, new_name).map_err(|e| e.to_string()))
    }

    fn new_folder_in_overwrite(&self, dir: &str) -> Result<String, String> {
        guarded(|| self.0.new_folder_in_overwrite(dir).map_err(|e| e.to_string()))
    }

    fn create_empty_mod(self: Pin<&mut Self>, name: &str, at: &str) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| events(this.0.create_empty_mod(name, non_empty(at))))
    }

    fn create_mod_from_folder(self: Pin<&mut Self>, name: &str, folder: &str) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| events(this.0.create_mod_from_folder(name, Path::new(folder))))
    }

    fn set_comments(self: Pin<&mut Self>, name: &str, text: &str) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| events(this.0.set_comments(name, text)))
    }

    fn set_color(self: Pin<&mut Self>, name: &str, has_color: bool, rgba: u32) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        let [r, g, b, a] = rgba.to_be_bytes();
        let color = has_color.then_some(strata_core::Color { r, g, b, a });
        guarded(|| events(this.0.set_color(name, color)))
    }

    fn remove_mods(self: Pin<&mut Self>, names: Vec<String>, trash: bool) -> Result<Vec<ffi::ModEvent>, String> {
        let this = self.get_mut();
        guarded(|| {
            let bin = if trash { Some(strata_core::Trash::home().map_err(|e| e.to_string())?) } else { None };
            events(this.0.remove_mods(&names, bin.as_ref()))
        })
    }

    fn backup_mod_list(&self) -> Result<String, String> {
        guarded(|| self.0.backup_mod_list().map_err(|e| e.to_string()))
    }

    fn backup_plugins(&self) -> Result<usize, String> {
        guarded(|| self.0.backup_plugins().map_err(|e| e.to_string()))
    }

    fn backups(&self, mods: bool) -> Result<Vec<String>, String> {
        guarded(|| self.0.backups(mods).map_err(|e| e.to_string()))
    }

    fn restore_backup(self: Pin<&mut Self>, mods: bool, suffix: &str) -> Result<(), String> {
        let this = self.get_mut();
        guarded(|| this.0.restore_backup(mods, suffix).map_err(|e| e.to_string()))
    }

    fn select_profile(self: Pin<&mut Self>, name: &str) -> Result<(), String> {
        let this = self.get_mut();
        guarded(|| this.0.select_profile(name).map_err(|e| e.to_string()))
    }

    fn deployed_profile(&self) -> Result<String, String> {
        guarded(|| self.0.deployed_profile().map(Option::unwrap_or_default).map_err(|e| e.to_string()))
    }

    fn mod_path(&self, name: &str) -> String {
        quiet(|| self.0.mod_path(name).map(|p| text(&p)).unwrap_or_default())
    }

    fn refresh_deploy(&self) -> Result<String, String> {
        guarded(|| self.0.refresh_deploy().map(|o| o.to_string()).map_err(|e| e.to_string()))
    }

    fn build_conflicts(&self) -> Result<Box<Conflicts>, String> {
        guarded(|| {
            strata_ui::conflict_flags::ConflictView::build(self.0.instance(), self.0.profile())
                .map(|v| Box::new(Conflicts(v)))
                .map_err(|e| e.to_string())
        })
    }

    fn marker_colors(&self) -> ffi::MarkerColors {
        quiet(|| {
            let c = strata_ui::conflict_flags::MarkerColors::of(self.0.instance());
            let pack = |c: strata_core::Color| u32::from_be_bytes([c.r, c.g, c.b, c.a]);
            ffi::MarkerColors {
                overwriting_loose: pack(c.overwriting_loose),
                overwritten_loose: pack(c.overwritten_loose),
                overwriting_archive: pack(c.overwriting_archive),
                overwritten_archive: pack(c.overwritten_archive),
            }
        })
    }

    fn build_plugins(&self) -> Result<Box<Plugins>, String> {
        guarded(|| self.0.plugin_view().map(|v| Box::new(Plugins(v))).map_err(|e| e.to_string()))
    }

    fn sort_plugins(&self) -> Result<String, String> {
        guarded(|| self.0.sort_plugins().map_err(|e| e.to_string()))
    }

    fn executables(&self) -> Vec<ffi::ExecutableRow> {
        quiet(|| {
            self.0.executables().into_iter().map(|e| ffi::ExecutableRow { title: e.title, binary: e.binary, toolbar: e.toolbar, hide: e.hide }).collect()
        })
    }

    fn downloads(&self) -> Vec<ffi::DownloadRow> {
        quiet(|| {
            self.0
                .downloads()
                .into_iter()
                .map(|d| ffi::DownloadRow {
                    file_name: d.file_name,
                    display_name: d.display_name,
                    status: d.status.to_owned(),
                    size: d.size,
                    file_time: opt(d.file_time),
                    mod_name: d.mod_name,
                    version: d.version,
                    nexus_id: d.nexus_id,
                    game: d.game,
                    hidden: d.hidden,
                })
                .collect()
        })
    }

    fn saves(&self, plugins: &Plugins) -> Result<Vec<ffi::SaveRow>, String> {
        guarded(|| {
            let rows = self.0.saves(Some(&plugins.0)).map_err(|e| e.to_string())?;
            Ok(rows
                .into_iter()
                .map(|s| ffi::SaveRow {
                    name: s.name,
                    file: s.file,
                    path: text(&s.path),
                    created: opt(s.created),
                    tooltip: s.tooltip,
                    missing: s.missing,
                    screenshot: s.screenshot,
                    width: s.width,
                    height: s.height,
                })
                .collect())
        })
    }

    fn build_data_view(&self) -> Result<Box<DataView>, String> {
        guarded(|| self.0.data_view().map(|v| Box::new(DataView(v))).map_err(|e| e.to_string()))
    }

    fn load_setting(&self, section: &str, key: &str) -> ffi::StoredValue {
        quiet(|| match strata_ui::settings::load(&self.0.ini_path(), section, key) {
            Some(bytes) => ffi::StoredValue { found: true, bytes },
            None => ffi::StoredValue { found: false, bytes: Vec::new() },
        })
    }

    fn load_string_list(&self, section: &str, key: &str) -> ffi::StoredList {
        quiet(|| match strata_ui::settings::load_list(&self.0.ini_path(), section, key) {
            Some(items) => ffi::StoredList { found: true, items },
            None => ffi::StoredList::default(),
        })
    }

    fn store_settings(&self, values: Vec<ffi::SettingValue>) -> Result<bool, String> {
        guarded(|| {
            let settings: Vec<strata_ui::settings::Setting> = values
                .into_iter()
                .map(|v| strata_ui::settings::Setting {
                    section: v.section,
                    key: v.key,
                    value: match v.kind {
                        1 => Value::ByteArray(v.bytes),
                        2 => Value::Other(String::from_utf8_lossy(&v.bytes).into_owned()),
                        3 => {
                            let text = String::from_utf8_lossy(&v.bytes).into_owned();
                            let items: Vec<String> =
                                if text.is_empty() { Vec::new() } else { text.split('\n').map(str::to_owned).collect() };
                            strata_ui::settings::string_list_value(&items)
                        }
                        _ => Value::String(String::from_utf8_lossy(&v.bytes).into_owned()),
                    },
                })
                .collect();
            strata_ui::settings::store(&self.0.ini_path(), &settings).map_err(|e| e.to_string())
        })
    }
}

/// The plugin list (`strata_ui::plugins::PluginView`).
pub struct Plugins(strata_ui::plugins::PluginView);

impl Plugins {
    fn rows(&self) -> Vec<ffi::PluginRow> {
        quiet(|| {
            self.0
                .rows()
                .into_iter()
                .map(|p| ffi::PluginRow {
                    name: p.name,
                    origin: p.origin,
                    enabled: p.enabled,
                    force_loaded: p.force_loaded,
                    force_disabled: p.force_disabled,
                    priority: index(Some(p.priority)),
                    index: p.index,
                    bold: p.bold,
                    italic: p.italic,
                    locked: p.locked,
                    icons: p.icons.iter().map(|i| (*i).to_owned()).collect(),
                    tooltip: p.tooltip,
                })
                .collect()
        })
    }

    fn archives(&self) -> Vec<ffi::ArchiveRow> {
        quiet(|| {
            self.0
                .archives()
                .into_iter()
                .map(|a| ffi::ArchiveRow { name: a.name, origin: a.origin, loaded: a.loaded })
                .collect()
        })
    }

    fn set_enabled(self: Pin<&mut Self>, names: Vec<String>, enabled: bool) -> Result<Vec<String>, String> {
        let this = self.get_mut();
        guarded(|| this.0.set_enabled(&names, enabled).map_err(|e| e.to_string()))
    }

    fn set_enabled_all(self: Pin<&mut Self>, enabled: bool) -> Result<Vec<String>, String> {
        let this = self.get_mut();
        guarded(|| this.0.set_enabled_all(enabled).map_err(|e| e.to_string()))
    }

    fn send_to_priority(self: Pin<&mut Self>, names: Vec<String>, priority: i64) -> Result<(), String> {
        let this = self.get_mut();
        let priority = usize::try_from(priority).unwrap_or(usize::MAX);
        guarded(|| this.0.send_to_priority(&names, priority).map_err(|e| e.to_string()))
    }

    fn set_locked(self: Pin<&mut Self>, name: &str, lock: bool) -> Result<(), String> {
        let this = self.get_mut();
        guarded(|| this.0.set_locked(name, lock).map_err(|e| e.to_string()))
    }
}

/// The conflict index (`strata_ui::conflict_flags::ConflictView`).
pub struct Conflicts(strata_ui::conflict_flags::ConflictView);

impl Conflicts {
    fn update(self: Pin<&mut Self>, session: &Session) -> Result<Vec<String>, String> {
        let this = self.get_mut();
        guarded(|| this.0.update(session.0.instance(), session.0.profile()).map_err(|e| e.to_string()))
    }

    fn row(&self, name: &str) -> ffi::ConflictRow {
        quiet(|| {
            let r = self.0.row(name);
            ffi::ConflictRow {
                icons: r.icons.iter().map(|i| i.unwrap_or("").to_owned()).collect(),
                texts: r.texts.iter().map(|t| (*t).to_owned()).collect(),
                hidden_files: r.hidden_files,
            }
        })
    }

    fn markers(&self, names: Vec<String>) -> ffi::Markers {
        quiet(|| {
            let m = self.0.markers(&names);
            let list = |set: std::collections::HashSet<String>| set.into_iter().collect::<Vec<_>>();
            ffi::Markers {
                overwriting: list(m.overwriting),
                overwritten: list(m.overwritten),
                archive_overwriting: list(m.archive_overwriting),
                archive_overwritten: list(m.archive_overwritten),
            }
        })
    }
}

/// The merged Data folder (`strata_ui::data::DataView`).
pub struct DataView(strata_ui::data::DataView);

impl DataView {
    fn children(&self, path: &str) -> Vec<ffi::DataEntry> {
        quiet(|| {
            self.0
                .children(path)
                .into_iter()
                .map(|e| ffi::DataEntry {
                    name: e.name,
                    is_dir: e.is_dir,
                    origin: e.origin,
                    size: e.size,
                    modified: opt(e.modified),
                    from_archive: e.from_archive,
                })
                .collect()
        })
    }

    fn load_archives(self: Pin<&mut Self>, session: &Session) -> Result<(), String> {
        let this = self.get_mut();
        guarded(|| this.0.load_archives(session.0.instance(), session.0.profile()).map_err(|e| e.to_string()))
    }

    fn query(&self, area: u8, path: &str, filter: &str, conflicts_only: bool, archives: bool) -> Vec<ffi::DataEntry> {
        quiet(|| {
            let area = if area == 1 { strata_ui::data::Area::Root } else { strata_ui::data::Area::Data };
            let q = strata_ui::data::Query { filter: filter.to_owned(), conflicts_only, archives };
            self.0
                .query(area, path, &q)
                .into_iter()
                .map(|e| ffi::DataEntry {
                    name: e.name,
                    is_dir: e.is_dir,
                    origin: e.origin,
                    size: e.size,
                    modified: opt(e.modified),
                    from_archive: e.from_archive,
                })
                .collect()
        })
    }

    fn file_count(&self) -> usize {
        quiet(|| self.0.file_count())
    }
}
