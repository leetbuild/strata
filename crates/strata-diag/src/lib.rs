//! Diagnostics: the problems list (MO2's "Problems" dialog).
//!
//! Each check is the counterpart of an MO2 diagnose plugin and names it: `diagnose_basic`
//! (missing masters, overwrite not empty, nitpick, profile tweaks, font configuration,
//! alternate game sources), `form43_checker`,
//! `script_extender_plugin_checker`. The checks take plain inputs (the plugin list, a
//! folder, the text of a log), so the caller decides where they come from.

use std::path::Path;

use strata_core::plugins::PluginList;

/// One problem, as the Problems dialog lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// A stable name for scripts: `missing-masters`, `form43`, ...
    pub id: &'static str,
    /// The one-line description (MO2's `shortDescription`).
    pub summary: String,
    /// One line per item (a plugin, a file), plain text.
    pub details: Vec<String>,
}

/// The `Basic diagnosis plugin` settings (`DiagnoseBasic::settings`), from `[Plugins]` of
/// `ModOrganizer.ini`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasicSettings {
    /// Default **false**: Leet change 8 ("no overwrite notification") overrides MO2's true.
    pub check_overwrite: bool,
    pub check_missing_masters: bool,
    /// The nitpick check (`check_conflict`).
    pub check_conflict: bool,
    pub ow_ignore_empty: bool,
    pub ow_ignore_log: bool,
    /// `check_font`: the font configuration names fonts that aren't installed.
    pub check_font: bool,
    /// `check_alternategames` (MO2's default false): active mods from another game.
    pub check_alternate_games: bool,
}

impl Default for BasicSettings {
    fn default() -> Self {
        Self {
            check_overwrite: false,
            check_missing_masters: true,
            check_conflict: true,
            ow_ignore_empty: false,
            ow_ignore_log: false,
            check_font: true,
            check_alternate_games: false,
        }
    }
}

impl BasicSettings {
    /// Reads the settings from the instance's ini, keeping the defaults for keys it lacks.
    pub fn from_ini(ini: &strata_ini::Document) -> Self {
        let d = Self::default();
        let get = |key: &str, default: bool| {
            ini.value("Plugins", &format!("Basic diagnosis plugin/{key}")).map_or(default, |v| v.to_bool())
        };
        Self {
            check_overwrite: get("check_overwrite", d.check_overwrite),
            check_missing_masters: get("check_missingmasters", d.check_missing_masters),
            check_conflict: get("check_conflict", d.check_conflict),
            ow_ignore_empty: get("ow_ignore_empty", d.ow_ignore_empty),
            ow_ignore_log: get("ow_ignore_log", d.ow_ignore_log),
            check_font: get("check_font", d.check_font),
            check_alternate_games: get("check_alternategames", d.check_alternate_games),
        }
    }
}

/// `DiagnoseBasic::missingMasters`: masters that enabled plugins need but that are not
/// enabled, each with the plugins that need it.
pub fn missing_masters(list: &PluginList) -> Option<Problem> {
    // master (lower case) -> (master as named, children)
    let mut missing: std::collections::BTreeMap<String, (String, Vec<String>)> = Default::default();
    for p in list.plugins() {
        for master in &p.missing_masters {
            let entry = missing.entry(master.to_lowercase()).or_insert_with(|| (master.clone(), Vec::new()));
            entry.1.push(p.name.clone());
        }
    }
    (!missing.is_empty()).then(|| Problem {
        id: "missing-masters",
        summary: "Missing Masters".into(),
        details: missing.into_values().map(|(master, children)| format!("{master}: needed by {}", children.join(", "))).collect(),
    })
}

/// `Form43Checker`: `.esp` and `.esm` files (not `.esl`) whose form version is below the
/// game's (Skyrim SE: 44), enabled or not. Plugins without a readable form version are
/// skipped, as MO2's `-1`.
pub fn form43(list: &PluginList, game_form_version: u16) -> Option<Problem> {
    let details: Vec<String> = list
        .plugins()
        .filter(|p| {
            let n = p.name.to_lowercase();
            n.ends_with(".esp") || n.ends_with(".esm")
        })
        .filter_map(|p| p.form_version().filter(|v| *v < game_form_version).map(|v| format!("{} (form {v})", p.name)))
        .collect();
    (!details.is_empty()).then(|| Problem {
        id: "form43",
        summary: "Form 43 (or lower) plugin detected".into(),
        details,
    })
}

/// One plugin line of the script extender's log that counts as a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedPlugin {
    /// The DLL's path as the log writes it (a Windows path).
    pub path: String,
    /// What the log says: `reported as incompatible during load`, `couldn't load (error 126)`...
    pub status: String,
}

/// The plugin lines of a script extender log (`skse64.log`) that `ScriptExtenderPluginChecker`
/// reports: `plugin <path> (<info> <name> <version>) <status>` with a status other than
/// `loaded correctly` or `no version data`, and `couldn't load plugin <path> (Error ...)`.
///
/// MO2 also has a pattern for "does not appear to be an SKSE plugin", but its regex never
/// matches a whole line (its `|` splits the whole pattern), so those lines are never reported;
/// Strata does the same.
pub fn script_extender_failures(log: &str) -> Vec<FailedPlugin> {
    let mut out: Vec<FailedPlugin> = Vec::new();
    for line in log.lines() {
        let line = line.trim_end();
        let found = if let Some(rest) = line.strip_prefix("couldn't load plugin ") {
            // "<path> (Error <code>[: details])"
            rest.rfind(" (Error ").map(|i| FailedPlugin {
                path: rest[..i].to_owned(),
                status: format!("couldn't load ({})", rest[i + 2..].trim_end_matches(')')),
            })
        } else if let Some(rest) = line.strip_prefix("plugin ") {
            normal_line(rest).filter(|f| f.status != "loaded correctly" && f.status != "no version data")
        } else {
            None
        };
        if let Some(f) = found
            && !out.contains(&f)
        {
            out.push(f);
        }
    }
    out
}

/// `<path> (<8 hex> <name> <8 hex>) <status>[ <code>][ (handle <n>)]`, as MO2's regex reads it.
fn normal_line(rest: &str) -> Option<FailedPlugin> {
    // The last " (XXXXXXXX " that starts a well-formed info block.
    let hex8 = |s: &str| s.len() == 8 && s.bytes().all(|b| b.is_ascii_hexdigit());
    let mut search = rest.len();
    while let Some(open) = rest[..search].rfind(" (") {
        let after = &rest[open + 2..];
        if after.len() > 9 && hex8(&after[..8]) && after.as_bytes()[8] == b' ' {
            let close_rel = after.find(") ")?;
            let inner = &after[..close_rel];
            let version = inner.rsplit(' ').next()?;
            if hex8(version) && inner.len() >= 18 {
                let mut status = after[close_rel + 2..].trim().to_owned();
                if let Some(i) = status.rfind(" (handle ")
                    && status.ends_with(')')
                {
                    status.truncate(i);
                }
                // A trailing error code (`<status> <digits>`) is not part of the status.
                if let Some((head, code)) = status.rsplit_once(' ')
                    && !code.is_empty()
                    && code.bytes().all(|b| b.is_ascii_digit())
                {
                    status = head.to_owned();
                }
                return Some(FailedPlugin { path: rest[..open].to_owned(), status });
            }
        }
        search = open;
    }
    None
}

/// `ScriptExtenderPluginChecker` as a problem: the failures whose DLL still exists in the
/// merged game folder (`origin_of` gives the mod it comes from; a DLL that is gone since
/// the game ran is not reported, as in MO2).
pub fn script_extender(log: &str, origin_of: impl Fn(&str) -> Option<String>) -> Option<Problem> {
    let details: Vec<String> = script_extender_failures(log)
        .into_iter()
        .filter_map(|f| {
            let origin = origin_of(&f.path)?;
            let name = f.path.rsplit(['\\', '/']).next().unwrap_or(&f.path).to_owned();
            Some(format!("{name} ({origin}): {}", f.status))
        })
        .collect();
    (!details.is_empty()).then(|| Problem {
        id: "script-extender",
        summary: "Script extender log reports incompatible plugins.".into(),
        details,
    })
}

/// `DiagnoseBasic::overwriteFiles`: overwrite holds anything (with `ow_ignore_empty`, only
/// files count; with `ow_ignore_log`, `.log` files don't either).
pub fn overwrite(dir: &Path, settings: &BasicSettings) -> Option<Problem> {
    if !settings.check_overwrite {
        return None;
    }
    let mut details = Vec::new();
    if settings.ow_ignore_empty || settings.ow_ignore_log {
        collect_files(dir, dir, settings.ow_ignore_log, &mut details);
    } else {
        // QDir::count() with the default filter: every entry but hidden ones.
        let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
        details = entries.map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with('.')).collect();
    }
    details.sort();
    (!details.is_empty()).then(|| Problem {
        id: "overwrite",
        summary: "There are files in your Overwrite mod directory".into(),
        details,
    })
}

/// `DiagnoseBasic::checkEmpty`, collecting the files that make the folder not empty.
fn collect_files(base: &Path, dir: &Path, ignore_logs: bool, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() {
            collect_files(base, &path, ignore_logs, out);
        } else if kind.is_file() && !(ignore_logs && is_log(&e.file_name().to_string_lossy())) {
            out.push(path.strip_prefix(base).unwrap_or(&path).to_string_lossy().into_owned());
        }
    }
}

/// `RE_LOG_FILE`: `.*[.]log[0-9]*$`.
fn is_log(name: &str) -> bool {
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit());
    trimmed.ends_with(".log")
}

/// `DiagnoseBasic::nitpickInstalled`: `SKSE/Plugins/nitpick.dll` is in the merged Data folder.
pub fn nitpick(settings: &BasicSettings, origin: Option<String>) -> Option<Problem> {
    let origin = origin.filter(|_| settings.check_conflict)?;
    Some(Problem {
        id: "nitpick",
        summary: "Nitpick installed".into(),
        details: vec![format!("SKSE/Plugins/nitpick.dll ({origin}): not needed with a mod manager, and may conflict with it")],
    })
}

/// `PROBLEM_PROFILETWEAKS`: the profile has a `profile_tweaks.ini`.
pub fn profile_tweaks(profile_dir: &Path) -> Option<Problem> {
    let path = profile_dir.join("profile_tweaks.ini");
    path.is_file().then(|| Problem {
        id: "profile-tweaks",
        summary: "INI Tweaks overwritten".into(),
        details: vec![format!("{}: settings changed in game or by a tool; copy what you want to keep into an ini tweak, then delete it", path.display())],
    })
}

/// `DiagnoseBasic::invalidFontConfig` (Skyrim LE and SE only; the caller decides): a
/// `fontlib "<path>"` line of `Data/interface/fontconfig.txt` names a file that
/// `resolves` (a path relative to Data, `\\` as written) doesn't find. The three fonts of
/// `Skyrim - Interface.bsa` are never reported. MO2 reads the file in text mode (`\r\n`
/// becomes `\n`) and matches each line with `^fontlib "([^"]*)"$`; the line end is
/// dropped before matching here (whether MO2's `std::regex` lets `$` match before the
/// `\n` depends on the C++ library; the check's intent is the line without it).
pub fn invalid_font_config(config: &str, mut resolves: impl FnMut(&str) -> bool) -> Option<Problem> {
    const DEFAULT_FONTS: [&str; 3] = ["interface\\fonts_console.swf", "interface\\fonts_en.swf", "interface\\fonts_cclub.swf"];
    let missing: Vec<String> = config
        .split('\n')
        .filter_map(|line| line.trim_end_matches('\r').strip_prefix("fontlib \"")?.strip_suffix('"'))
        .filter(|path| !path.contains('"'))
        // FileNameComparator::CaseSensitivity is case-insensitive on Windows
        .filter(|path| !DEFAULT_FONTS.iter().any(|d| d.eq_ignore_ascii_case(path)))
        .filter(|path| !resolves(path))
        .map(|path| format!("{path}: not installed"))
        .collect();
    (!missing.is_empty()).then(|| Problem {
        id: "font-config",
        summary: "Your font configuration may be broken".into(),
        details: missing,
    })
}

/// One mod for [`alternate_game_sources`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModSource {
    pub name: String,
    pub active: bool,
    /// `meta.ini` `gameName` (empty: not set, the managed game).
    pub game_name: String,
    /// `meta.ini` `converted` ("Mark as converted/working").
    pub converted: bool,
}

/// `DiagnoseBasic::alternateGame`: active mods whose `gameName` is neither the managed
/// game's short name nor one of its primary sources (`ModInfoRegular`'s `m_IsAlternate`),
/// unless marked converted (`ModList::state`'s `STATE_ALTERNATE`).
pub fn alternate_game_sources(mods: &[ModSource], short_name: &str, primary_sources: &[&str]) -> Option<Problem> {
    let details: Vec<String> = mods
        .iter()
        .filter(|m| m.active && !m.converted && !m.game_name.is_empty())
        .filter(|m| !m.game_name.eq_ignore_ascii_case(short_name))
        .filter(|m| !primary_sources.iter().any(|s| s.eq_ignore_ascii_case(&m.game_name)))
        .map(|m| format!("{} (from {}): mark it converted once it works", m.name, m.game_name))
        .collect();
    (!details.is_empty()).then(|| Problem {
        id: "alternate-game",
        summary: "At least one unverified mod is using an alternative game source".into(),
        details,
    })
}

/// Leet backlog ("Executable pointing into a mod's Root\\"): executables whose binary or
/// start folder is inside a mod's `Root/` folder. They must run from the game folder,
/// where the overlay shows those files. `found`: (title, the path as stored, the path in
/// the game folder).
pub fn executables_in_root(found: &[(String, String, String)]) -> Option<Problem> {
    let details: Vec<String> = found.iter().map(|(title, stored, game)| format!("{title}: {stored} → {game}")).collect();
    (!details.is_empty()).then(|| Problem {
        id: "executables-in-root",
        summary: "Executables point into a mod's Root folder".into(),
        details,
    })
}

#[cfg(test)]
mod tests;
