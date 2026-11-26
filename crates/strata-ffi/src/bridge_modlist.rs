//! The `modlist` area's bridge (round 2): the profiles, executables and settings dialogs
//! and the delete confirmation. Each function opens the instance in `root` itself (the
//! dialogs are short-lived), does one thing through `strata_core` / `strata_ui::modlist`,
//! and catches panics (a panic becomes the error "internal error: ...").

use std::path::Path;

use strata_core::settings::Kind;
use strata_core::{DeleteChoice, Executable, Instance, ModKind};
use strata_ui::modlist::{executables, profiles, settings};
use crate::{guarded, quiet};

#[cxx::bridge(namespace = "strata::modlist")]
mod ffi {
    struct ProfileRow {
        name: String,
        selected: bool,
        local_saves: bool,
        local_settings: bool,
        archive_invalidation: bool,
    }

    struct ExecutableRow {
        title: String,
        binary: String,
        arguments: String,
        working_directory: String,
        steam_app_id: String,
        toolbar: bool,
        own_icon: bool,
        hide: bool,
        /// "Create files in mod": the mod, or empty.
        output_mod: String,
    }

    struct SettingRow {
        /// The object name of the widget in MO2's `settingsdialog.ui`.
        widget: String,
        /// 0 bool, 1 int, 2 text, 3 path, 4 list (one item per line).
        kind: u8,
        value: String,
    }

    /// A row of the Theme tab's colour table.
    struct ColorRow {
        caption: String,
        /// `#rrggbb` or `#aarrggbb` (QColor reads both).
        color: String,
    }

    struct SettingPair {
        widget: String,
        value: String,
    }

    extern "Rust" {
        fn profile_rows(root: &str) -> Result<Vec<ProfileRow>>;
        /// Returns the new profile's name (as fixed by `fixDirectoryName`).
        fn profile_create(root: &str, name: &str, defaults: bool) -> Result<String>;
        fn profile_copy(root: &str, from: &str, name: &str) -> Result<String>;
        fn profile_rename(root: &str, old: &str, new_name: &str) -> Result<()>;
        /// To the desktop's trash.
        fn profile_delete(root: &str, name: &str) -> Result<()>;
        /// `remove`: when switching off, the saves (or inis) go to the trash.
        fn profile_set_local_saves(root: &str, name: &str, on: bool, remove: bool) -> Result<()>;
        fn profile_set_local_inis(root: &str, name: &str, on: bool, remove: bool) -> Result<()>;
        fn profile_set_invalidation(root: &str, name: &str, on: bool) -> Result<()>;
        /// Selects the profile; a live deploy is unmounted and mounted again. Returns
        /// whether that happened.
        fn profile_select(root: &str, name: &str) -> Result<bool>;

        fn executable_rows(root: &str, profile: &str) -> Result<Vec<ExecutableRow>>;
        /// `original`: the titles when the dialog opened (their output mods are replaced).
        fn executables_commit(root: &str, profile: &str, original: Vec<String>, rows: Vec<ExecutableRow>) -> Result<bool>;
        /// The stored form of a binary picked in a file dialog (`Z:\...`).
        fn executable_binary_for_storage(host_path: &str) -> String;
        /// The host path of a stored binary (for the file dialog's start folder); empty
        /// when it can't be resolved.
        fn executable_host_path(root: &str, stored: &str) -> String;
        /// The new title for a "New Executable" whose binary was picked; empty: keep it.
        fn executable_title_for_binary(titles: Vec<String>, current: &str, binary: &str) -> String;
        /// The profile's regular mods, for the output mod box.
        fn executable_mod_names(root: &str, profile: &str) -> Result<Vec<String>>;

        fn setting_rows(root: &str) -> Result<Vec<SettingRow>>;
        fn settings_store(root: &str, values: Vec<SettingPair>) -> Result<bool>;
        fn reset_dialog_choices(root: &str) -> Result<()>;
        /// The instance's game folder, for the Paths tab's "Managed Game".
        fn game_folder(root: &str) -> String;
        /// The six highlight colours (MO2's colour table), stored or MO2's defaults.
        fn highlight_colors(root: &str) -> Vec<ColorRow>;

        /// The remembered answer of a delete confirmation: 0 none, 1 trash, 2 permanent.
        fn delete_choice(root: &str, action: &str) -> u8;
        /// Remembers an answer (0 forgets).
        fn set_delete_choice(root: &str, action: &str, choice: u8) -> Result<()>;
    }
}

fn open(root: &str) -> Result<Instance, String> {
    Instance::open(Path::new(root)).map_err(|e| e.to_string())
}

fn text<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn profile_rows(root: &str) -> Result<Vec<ffi::ProfileRow>, String> {
    guarded(|| {
        let rows = profiles::rows(&open(root)?).map_err(text)?;
        Ok(rows
            .into_iter()
            .map(|r| ffi::ProfileRow {
                name: r.name,
                selected: r.selected,
                local_saves: r.local_saves,
                local_settings: r.local_settings,
                archive_invalidation: r.archive_invalidation,
            })
            .collect())
    })
}

fn profile_create(root: &str, name: &str, defaults: bool) -> Result<String, String> {
    guarded(|| Ok(open(root)?.create_profile(name, defaults).map_err(text)?.0.name))
}

fn profile_copy(root: &str, from: &str, name: &str) -> Result<String, String> {
    guarded(|| Ok(open(root)?.copy_profile(from, name).map_err(text)?.0.name))
}

fn profile_rename(root: &str, old: &str, new_name: &str) -> Result<(), String> {
    guarded(|| open(root)?.rename_profile(old, new_name).map(|_| ()).map_err(text))
}

fn profile_delete(root: &str, name: &str) -> Result<(), String> {
    guarded(|| profiles::delete(&open(root)?, name).map_err(text))
}

fn trash_if(delete: bool) -> Result<Option<strata_core::Trash>, String> {
    if delete { strata_core::Trash::home().map(Some).map_err(text) } else { Ok(None) }
}

fn profile_set_local_saves(root: &str, name: &str, on: bool, remove: bool) -> Result<(), String> {
    guarded(|| {
        let instance = open(root)?;
        let profile = instance.profile(Some(name)).map_err(text)?;
        instance.set_local_saves(&profile, on, remove, trash_if(remove)?.as_ref()).map(|_| ()).map_err(text)
    })
}

fn profile_set_local_inis(root: &str, name: &str, on: bool, remove: bool) -> Result<(), String> {
    guarded(|| {
        let instance = open(root)?;
        let profile = instance.profile(Some(name)).map_err(text)?;
        instance.set_local_settings(&profile, on, remove, trash_if(remove)?.as_ref()).map(|_| ()).map_err(text)
    })
}

fn profile_set_invalidation(root: &str, name: &str, on: bool) -> Result<(), String> {
    guarded(|| profiles::set_archive_invalidation(&open(root)?, name, on).map(|_| ()).map_err(text))
}

fn profile_select(root: &str, name: &str) -> Result<bool, String> {
    guarded(|| strata_cli::select_profile(Path::new(root), name, None).map(|s| s.redeployed).map_err(|e| format!("{e:#}")))
}

fn executable_rows(root: &str, profile: &str) -> Result<Vec<ffi::ExecutableRow>, String> {
    guarded(|| {
        let instance = open(root)?;
        let profile = instance.profile((!profile.is_empty()).then_some(profile)).map_err(text)?;
        let rows = executables::rows(&instance, &profile).map_err(text)?;
        Ok(rows
            .into_iter()
            .map(|r| ffi::ExecutableRow {
                title: r.exe.title,
                binary: r.exe.binary,
                arguments: r.exe.arguments,
                working_directory: r.exe.working_directory,
                steam_app_id: r.exe.steam_app_id,
                toolbar: r.exe.toolbar,
                own_icon: r.exe.own_icon,
                hide: r.exe.hide,
                output_mod: r.output_mod,
            })
            .collect())
    })
}

fn executables_commit(root: &str, profile: &str, original: Vec<String>, rows: Vec<ffi::ExecutableRow>) -> Result<bool, String> {
    guarded(|| {
        let mut instance = open(root)?;
        let profile = instance.profile((!profile.is_empty()).then_some(profile)).map_err(text)?;
        let rows: Vec<executables::ExecutableRow> = rows
            .into_iter()
            .map(|r| executables::ExecutableRow {
                exe: Executable {
                    title: r.title,
                    binary: r.binary,
                    arguments: r.arguments,
                    working_directory: r.working_directory,
                    steam_app_id: r.steam_app_id,
                    toolbar: r.toolbar,
                    own_icon: r.own_icon,
                    hide: r.hide,
                },
                output_mod: r.output_mod,
            })
            .collect();
        executables::commit(&mut instance, &profile, &original, &rows).map_err(text)
    })
}

fn executable_binary_for_storage(host_path: &str) -> String {
    quiet(|| executables::binary_for_storage(Path::new(host_path)))
}

fn executable_host_path(root: &str, stored: &str) -> String {
    quiet(|| {
        let Ok(instance) = open(root) else { return String::new() };
        let pfx = instance.proton.prefix.as_ref().map(|p| p.join("pfx"));
        strata_core::winepath::to_host(stored, pfx.as_deref()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
    })
}

fn executable_title_for_binary(titles: Vec<String>, current: &str, binary: &str) -> String {
    quiet(|| {
        let list: Vec<Executable> = titles.into_iter().map(|title| Executable { title, ..Default::default() }).collect();
        executables::title_for_binary(&list, current, binary).unwrap_or_default()
    })
}

fn executable_mod_names(root: &str, profile: &str) -> Result<Vec<String>, String> {
    guarded(|| {
        let instance = open(root)?;
        let profile = instance.profile((!profile.is_empty()).then_some(profile)).map_err(text)?;
        let mods = instance.mod_list(&profile).map_err(text)?;
        Ok(mods.into_iter().filter(|m| m.kind == ModKind::Regular).map(|m| m.name).collect())
    })
}

fn setting_rows(root: &str) -> Result<Vec<ffi::SettingRow>, String> {
    guarded(|| {
        Ok(settings::rows(&open(root)?)
            .into_iter()
            .map(|r| ffi::SettingRow {
                widget: r.widget,
                kind: match r.kind {
                    Kind::Bool => 0,
                    Kind::Int => 1,
                    Kind::Text => 2,
                    Kind::Path => 3,
                    Kind::List => 4,
                },
                value: r.value,
            })
            .collect())
    })
}

fn settings_store(root: &str, values: Vec<ffi::SettingPair>) -> Result<bool, String> {
    guarded(|| {
        let mut instance = open(root)?;
        let values: Vec<(String, String)> = values.into_iter().map(|p| (p.widget, p.value)).collect();
        settings::store(&mut instance, &values).map_err(text)
    })
}

fn reset_dialog_choices(root: &str) -> Result<(), String> {
    guarded(|| open(root)?.reset_dialog_choices().map(|_| ()).map_err(text))
}

fn game_folder(root: &str) -> String {
    quiet(|| open(root).map(|i| i.game_path.to_string_lossy().into_owned()).unwrap_or_default())
}

fn highlight_colors(root: &str) -> Vec<ffi::ColorRow> {
    quiet(|| {
        open(root)
            .map(|i| {
                settings::highlight_colors(&i)
                    .into_iter()
                    .map(|(caption, color)| ffi::ColorRow { caption: caption.to_owned(), color: color.name() })
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn delete_choice(root: &str, action: &str) -> u8 {
    quiet(|| match open(root).ok().and_then(|i| i.delete_choice(action)) {
        None => 0,
        Some(DeleteChoice::Trash) => 1,
        Some(DeleteChoice::Permanent) => 2,
    })
}

fn set_delete_choice(root: &str, action: &str, choice: u8) -> Result<(), String> {
    guarded(|| {
        let choice = match choice {
            1 => Some(DeleteChoice::Trash),
            2 => Some(DeleteChoice::Permanent),
            _ => None,
        };
        open(root)?.set_delete_choice(action, choice).map(|_| ()).map_err(text)
    })
}
