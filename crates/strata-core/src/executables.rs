//! The executables list, `[customExecutables]` in `ModOrganizer.ini` (`ExecutablesList`,
//! `executableslist.cpp`; `Settings::setExecutables`).


use strata_ini::Value;

use crate::{Instance, Result, winepath};

/// One entry of the executables list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Executable {
    pub title: String,
    /// As stored: usually a Windows path (`Z:\...\skse64_loader.exe`).
    pub binary: String,
    pub arguments: String,
    /// As stored; empty means the binary's folder.
    pub working_directory: String,
    /// Overrides the game's Steam app id when not empty (the Creation Kit has its own).
    pub steam_app_id: String,
    /// `toolbar`: shown on the toolbar (`Executable::ShowInToolbar`).
    pub toolbar: bool,
    /// `ownicon`: use the program's own icon (`UseApplicationIcon`).
    pub own_icon: bool,
    /// `hide`: hidden from the run menu (`Hide`).
    pub hide: bool,
}

const SECTION: &str = "customExecutables";

impl Instance {
    /// The executables list, in the order MO2 shows it (`ExecutablesList::load`; entries
    /// without a title are skipped).
    pub fn executables(&self) -> Vec<Executable> {
        let get = |index: usize, key: &str| self.ini.string(SECTION, &format!("{index}/{key}")).unwrap_or_default();
        let flag = |index: usize, key: &str| self.ini.value(SECTION, &format!("{index}/{key}")).is_some_and(|v| v.to_bool());
        let size = self.ini.string(SECTION, "size").and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0);
        (1..=size)
            .map(|i| Executable {
                title: get(i, "title"),
                binary: get(i, "binary"),
                arguments: get(i, "arguments"),
                working_directory: get(i, "workingDirectory"),
                steam_app_id: get(i, "steamAppID"),
                toolbar: flag(i, "toolbar"),
                own_icon: flag(i, "ownicon"),
                hide: flag(i, "hide"),
            })
            .filter(|e| !e.title.is_empty())
            .collect()
    }

    /// `ExecutablesList::store` + `Settings::setExecutables`: nothing when the list is the
    /// same; when it got shorter the section is cleared first (Qt can't remove array
    /// elements); then `size` and each entry's keys in MO2's order (a `std::map`, so
    /// sorted). Lines that keep their value are left as they are. Returns whether the
    /// file changed.
    pub fn store_executables(&mut self, list: &[Executable]) -> Result<bool> {
        let current = self.executables();
        if current == list {
            return Ok(false);
        }
        let mut values: Vec<(String, Option<Value>)> = Vec::new();
        if current.len() > list.len() {
            for entry in self.ini.entries().filter(|e| e.section == SECTION) {
                values.push((entry.key.to_owned(), None));
            }
        }
        values.push(("size".into(), Some(Value::String(list.len().to_string()))));
        let flag = |on: bool| Some(Value::String(if on { "true" } else { "false" }.into()));
        let text = |s: &str| Some(Value::String(s.to_owned()));
        for (i, e) in list.iter().enumerate() {
            let n = i + 1;
            for (key, value) in [
                ("arguments", text(&e.arguments)),
                ("binary", text(&e.binary)),
                ("hide", flag(e.hide)),
                ("ownicon", flag(e.own_icon)),
                ("steamAppID", text(&e.steam_app_id)),
                ("title", text(&e.title)),
                ("toolbar", flag(e.toolbar)),
                ("workingDirectory", text(&e.working_directory)),
            ] {
                values.push((format!("{n}/{key}"), value));
            }
        }
        let values: Vec<(&str, &str, Option<Value>)> = values.iter().map(|(k, v)| (SECTION, k.as_str(), v.clone())).collect();
        self.store_settings(&values)
    }

    /// A binary or working directory inside a mod's `Root/` folder, rewritten to where the
    /// overlay shows it in the game folder (Leet backlog, "Executable pointing into a mod's
    /// Root\"): the loader must run from the game folder. `None` when the path is not in a
    /// mod's `Root/`. The result keeps the stored form (a Windows path stays one).
    pub fn root_path_in_game(&self, stored: &str) -> Option<String> {
        let pfx = self.proton.prefix.as_ref().map(|p| p.join("pfx"));
        let host = winepath::to_host(stored, pfx.as_deref()).ok()?;
        let rest = host.strip_prefix(&self.paths.mods).ok()?;
        let mut parts = rest.components();
        parts.next()?; // the mod
        let root = parts.next()?;
        if !root.as_os_str().to_string_lossy().eq_ignore_ascii_case("root") {
            return None;
        }
        let in_game = self.game_path.join(parts.as_path());
        let windows = stored.len() > 1 && stored.as_bytes()[1] == b':';
        Some(if windows { winepath::to_wine(&in_game) } else { in_game.to_string_lossy().into_owned() })
    }
}

/// `ExecutablesList::makeNonConflictingTitle`: `prefix`, else `prefix (1)` … `prefix (99)`.
pub fn non_conflicting_title(list: &[Executable], prefix: &str) -> Option<String> {
    let taken = |t: &str| list.iter().any(|e| e.title.eq_ignore_ascii_case(t));
    std::iter::once(prefix.to_owned()).chain((1..100).map(|i| format!("{prefix} ({i})"))).find(|t| !taken(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_made_unique() {
        let list = vec![
            Executable { title: "SKSE".into(), ..Default::default() },
            Executable { title: "skse (1)".into(), ..Default::default() },
        ];
        assert_eq!(non_conflicting_title(&list, "SKSE").as_deref(), Some("SKSE (2)"));
        assert_eq!(non_conflicting_title(&list, "xEdit").as_deref(), Some("xEdit"));
    }
}
