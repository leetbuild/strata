//! The Problems dialog (MO2's `ProblemsDialog` over its diagnose plugins): every check of
//! `strata-diag` run on the profile, and the fixes the core can do.
//!
//! What each check reads: the merged tree of the profile (the overlay's index, built here,
//! no mount), its plugin list, overwrite, the profile folder, the script extender's log in
//! the prefix and the mods' `meta.ini`.

use crate::html::escape;
use std::path::Path;

use strata_core::plugins::PluginList;
use strata_core::{Instance, ModKind, ModList, ModMeta, Profile, Trash};
use strata_diag::{BasicSettings, ModSource, Problem};
use strata_vfs::Tree;

use crate::{Error, Result};

/// One problem with the fix the dialog can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemRow {
    pub problem: Problem,
    /// The Fix button's text, when there is one.
    pub fix: Option<&'static str>,
}

impl ProblemRow {
    /// The dialog's description pane (MO2's `fullDescription`): what the problem means,
    /// then its items. HTML.
    pub fn description_html(&self) -> String {
        let items: String = self.problem.details.iter().map(|d| format!("<li>{}</li>", escape(d))).collect();
        format!("{}<ul>{items}</ul>", explanation(self.problem.id))
    }
}

/// MO2's `fullDescription` texts, shortened, with what Strata does differently.
fn explanation(id: &str) -> &'static str {
    match id {
        "overwrite" => {
            "There are files in your <i>Overwrite</i> folder: usually made by a tool (xEdit, Pandora, ...) or the \
             game. Files in Overwrite win over every mod and are shared by all profiles. Move the ones you keep \
             into a mod (right click on Overwrite), or switch this warning off in the settings (Diagnostics)."
        }
        "font-config" => {
            "Your font configuration (<code>Data/interface/fontconfig.txt</code>) names a font that is not \
             installed. You may see boxes instead of letters. Most likely a font replacer is installed only in \
             part, or a mod overrides its fontconfig.txt."
        }
        "nitpick" => {
            "The nitpick SKSE plugin is installed. It is not needed with a mod manager and the two may conflict: \
             remove it."
        }
        "missing-masters" => {
            "The masters of some plugins are not enabled. The game will crash unless they are installed and \
             enabled. The fix enables the ones that are installed."
        }
        "alternate-game" => {
            "At least one active mod comes from another game's Nexus pages. Depending on the mod, its files may \
             need converting. Once it works, mark it converted in its context menu."
        }
        "profile-tweaks" => {
            "Settings of the profile's ini tweaks were changed in game or by a tool and stored in \
             <i>profile_tweaks.ini</i> in the profile folder. Copy what you want to keep into an ini tweak, then \
             delete the file: the fix moves it to the trash."
        }
        "form43" => {
            "Some plugins have the old Skyrim (LE) format (form 43). They may work, but can break things: open \
             and save them in the Creation Kit to convert them, or replace them with SE versions."
        }
        "executables-in-root" => {
            "Some executables start a file inside a mod's <i>Root</i> folder. Root files appear in the game folder \
             only through the overlay, and tools like the script extender's loader must start from there. The fix \
             points them at the same file in the game folder."
        }
        "script-extender" => {
            "Some script extender plugins failed to load (from the script extender's log of the last run). \
             Update them, or remove the mods they come from."
        }
        _ => "",
    }
}

/// The managed game's short name and primary sources (`IPluginGame::gameShortName`,
/// `primarySources`), for the alternate-source check. `strata-game` has no short name yet;
/// these are MO2's values for the games Strata knows.
fn short_name(game: &str) -> (&'static str, &'static [&'static str]) {
    match game {
        "Skyrim" => ("Skyrim", &[]),
        "Oblivion" => ("Oblivion", &[]),
        _ => ("SkyrimSE", &[]),
    }
}

/// The profile's plugin list over the plugins of the merged Data folder.
fn plugin_list(instance: &Instance, profile: &Profile, tree: &Tree) -> Result<PluginList> {
    Ok(instance.plugin_list(profile, crate::profile_tree::available_plugins(instance, tree))?)
}

/// Every problem of the profile, in MO2's order (`diagnose_basic`'s, then form 43, then
/// the script extender's log).
pub fn collect(instance: &Instance, profile: &Profile) -> Result<Vec<ProblemRow>> {
    let tree = crate::profile_tree::merged_tree(instance, profile)?;
    let list = plugin_list(instance, profile, &tree)?;
    let settings = BasicSettings::from_ini(&instance.ini);
    let data = instance.game.data_dir();
    let origin = |path: &str| tree.resolve(path).and_then(|id| tree.layer_of(id)).map(|l| l.id.clone());

    let mut problems = Vec::new();
    problems.extend(strata_diag::overwrite(&instance.paths.overwrite, &settings));
    if settings.check_font && matches!(instance.game.name(), "Skyrim" | "Skyrim Special Edition") {
        problems.extend(font_config(&tree, data));
    }
    problems.extend(strata_diag::nitpick(&settings, origin(&format!("{data}/SKSE/Plugins/nitpick.dll"))));
    if settings.check_missing_masters {
        problems.extend(strata_diag::missing_masters(&list));
    }
    if settings.check_alternate_games {
        let (short, primary) = short_name(instance.game.name());
        problems.extend(strata_diag::alternate_game_sources(&mod_sources(instance, profile)?, short, primary));
    }
    problems.extend(strata_diag::profile_tweaks(&profile.dir));
    if let Some(version) = instance.game.form_version() {
        problems.extend(strata_diag::form43(&list, version));
    }
    problems.extend(script_extender(instance, &origin));
    problems.extend(strata_diag::executables_in_root(&executables_in_root(instance)));

    Ok(problems
        .into_iter()
        .map(|problem| {
            let fix = match problem.id {
                // DiagnoseBasic::startGuidedFix (MO2: shellDeleteQuiet)
                "profile-tweaks" => Some("Move profile_tweaks.ini to the trash"),
                // Strata's own: the masters that are there but not enabled
                "missing-masters" if !present_missing_masters(&list).is_empty() => Some("Enable the masters that are installed"),
                "executables-in-root" => Some("Point them at the game folder"),
                _ => None,
            };
            ProblemRow { problem, fix }
        })
        .collect())
}

/// `Data/interface/fontconfig.txt` of the merged tree, its fonts looked up as loose files
/// and in the BSAs at the top of Data (MO2's `resolvePath` sees the registered archives'
/// files too; Strata looks in every BSA there, loaded or not).
fn font_config(tree: &Tree, data: &str) -> Option<Problem> {
    let config = tree.resolve(&format!("{data}/interface/fontconfig.txt")).and_then(|id| tree.real_path(id))?;
    let bytes = std::fs::read(config).ok()?;
    let text: String = bytes.iter().map(|b| char::from(*b)).collect();
    let mut archived: Option<Vec<String>> = None;
    strata_diag::invalid_font_config(&text, |path| {
        let rel = path.replace('\\', "/");
        if tree.resolve(&format!("{data}/{rel}")).is_some() {
            return true;
        }
        let files = archived.get_or_insert_with(|| archive_files(tree, data));
        files.iter().any(|f| f.eq_ignore_ascii_case(&rel))
    })
}

/// The files of every `.bsa` at the top of the merged Data folder, `/`-separated.
fn archive_files(tree: &Tree, data: &str) -> Vec<String> {
    let Some(dir) = tree.resolve(data) else { return Vec::new() };
    let mut out = Vec::new();
    for (id, node) in tree.children(dir) {
        if node.is_dir() || !node.name.to_lowercase().ends_with(".bsa") {
            continue;
        }
        let Some(path) = tree.real_path(id) else { continue };
        if let Ok(archive) = strata_archive::GameArchive::open(&path) {
            out.extend(archive.files().iter().map(|f| f.path.clone()));
        }
    }
    out
}

fn mod_sources(instance: &Instance, profile: &Profile) -> Result<Vec<ModSource>> {
    let list = ModList::load(instance, profile)?;
    let mut out = Vec::new();
    for entry in list.entries().iter().filter(|e| e.kind == ModKind::Regular) {
        let Some(dir) = &entry.path else { continue };
        let meta = ModMeta::open(dir)?;
        out.push(ModSource { name: entry.name.clone(), active: entry.enabled, game_name: meta.game_name(), converted: meta.converted() });
    }
    Ok(out)
}

/// `ScriptExtenderPluginChecker`: the script extender's log in the prefix's documents
/// folder, each failed DLL reported with the mod it comes from (only DLLs still there).
fn script_extender(instance: &Instance, origin: &dyn Fn(&str) -> Option<String>) -> Option<Problem> {
    let (base, log) = instance.game.script_extender_log()?;
    let pfx = instance.proton.prefix.as_ref()?.join("pfx");
    let path = match base {
        strata_game::LogBase::Documents => {
            pfx.join("drive_c/users/steamuser/Documents/My Games").join(instance.game.documents_dir()).join(log)
        }
        strata_game::LogBase::Game => instance.game_path.join(log),
    };
    let data = std::fs::read(&path).ok()?;
    // MO2 reads it as cp1252: one byte, one character.
    let text: String = data.iter().map(|b| char::from(*b)).collect();
    strata_diag::script_extender(&text, |stored| {
        let host = strata_core::winepath::to_host(stored, Some(&pfx)).ok()?;
        origin(&relative_ci(&host, &instance.game_path)?)
    })
}

/// `path` relative to `base`, comparing without case (paths in Windows logs don't keep the
/// folders' case); empty parts (`\\` in SKSE's log) are dropped.
fn relative_ci(path: &Path, base: &Path) -> Option<String> {
    let parts = |p: &Path| -> Vec<String> {
        p.components()
            .filter_map(|c| match c {
                std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect()
    };
    let (path, base) = (parts(path), parts(base));
    if path.len() <= base.len() || !path.iter().zip(&base).all(|(a, b)| a.eq_ignore_ascii_case(b)) {
        return None;
    }
    Some(path[base.len()..].join("/"))
}

/// (title, stored path, path in the game folder) for each binary or start folder of an
/// executable that lies in a mod's `Root/` (`Instance::root_path_in_game`).
fn executables_in_root(instance: &Instance) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for e in instance.executables() {
        for stored in [&e.binary, &e.working_directory] {
            if let Some(game) = instance.root_path_in_game(stored) {
                out.push((e.title.clone(), stored.clone(), game));
            }
        }
    }
    out
}

/// Masters that enabled plugins need, that are in the list but not enabled.
fn present_missing_masters(list: &PluginList) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for plugin in list.plugins().filter(|p| p.enabled) {
        for master in &plugin.masters {
            if let Some(m) = list.get(master).filter(|m| !m.enabled && !m.force_disabled)
                && !out.iter().any(|o| o.eq_ignore_ascii_case(&m.name))
            {
                out.push(m.name.clone());
            }
        }
    }
    out
}

/// Runs the fix of problem `id` and returns what it did, in one line. Deleted files go to
/// `trash`.
pub fn fix(instance: &Instance, profile: &Profile, id: &str, trash: &Trash) -> Result<String> {
    match id {
        "profile-tweaks" => {
            let path = profile.dir.join("profile_tweaks.ini");
            trash.put(&path)?;
            Ok(format!("{} moved to the trash", path.display()))
        }
        "missing-masters" => {
            let tree = crate::profile_tree::merged_tree(instance, profile)?;
            let mut list = plugin_list(instance, profile, &tree)?;
            let masters = present_missing_masters(&list);
            if masters.is_empty() {
                return Err(Error::Invalid("none of the missing masters is installed: install them first".into()));
            }
            let changed = list.set_enabled(&masters, true)?;
            list.save(&profile.dir)?;
            Ok(format!("enabled {}", changed.join(", ")))
        }
        "executables-in-root" => {
            let mut instance = Instance::open(&instance.root)?;
            let mut list = instance.executables();
            let mut changed = Vec::new();
            for e in &mut list {
                for path in [&mut e.binary, &mut e.working_directory] {
                    if let Some(game) = instance.root_path_in_game(path) {
                        *path = game;
                        changed.push(e.title.clone());
                    }
                }
            }
            instance.store_executables(&list)?;
            changed.dedup();
            Ok(format!("pointed {} at the game folder", changed.join(", ")))
        }
        _ => Err(Error::Invalid(format!("the problem \"{id}\" has no fix Strata can do"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_instance::TestInstance;
    use strata_game::plugin::{FLAG_MASTER, build_plugin};

    #[test]
    fn profile_tweaks_and_fonts_are_reported_and_tweaks_fixed() {
        let t = TestInstance::new("+Fonts\r\n");
        std::fs::write(t.path("profiles/Default/profile_tweaks.ini"), "[Display]\r\n").unwrap();
        let fonts = t.folder("Fonts").join("interface");
        std::fs::create_dir_all(&fonts).unwrap();
        std::fs::write(fonts.join("fontconfig.txt"), "fontlib \"Interface\\fonts_en.swf\"\r\nfontlib \"Interface\\gone.swf\"\r\n").unwrap();
        let (instance, profile) = t.open();
        let rows = collect(&instance, &profile).unwrap();
        let ids: Vec<_> = rows.iter().map(|r| (r.problem.id, r.fix.is_some())).collect();
        assert_eq!(ids, [("font-config", false), ("profile-tweaks", true)]);
        assert!(rows[0].description_html().contains("<li>Interface\\gone.swf: not installed</li>"));

        let trash = Trash::at(t.path("trash"));
        fix(&instance, &profile, "profile-tweaks", &trash).unwrap();
        assert!(!t.path("profiles/Default/profile_tweaks.ini").exists());
        assert!(fix(&instance, &profile, "font-config", &trash).is_err());
    }

    #[test]
    fn executables_in_a_mods_root_are_reported_and_pointed_at_the_game() {
        let t = TestInstance::new("+SKSE\r\n");
        std::fs::create_dir_all(t.folder("SKSE").join("Root")).unwrap();
        let loader = t.path("mods/SKSE/Root/skse64_loader.exe");
        let ini = std::fs::read_to_string(t.path("ModOrganizer.ini")).unwrap()
            + &format!("[customExecutables]\r\nsize=1\r\n1\\title=SKSE\r\n1\\binary={}\r\n", loader.display());
        std::fs::write(t.path("ModOrganizer.ini"), ini).unwrap();
        let (instance, profile) = t.open();
        let rows = collect(&instance, &profile).unwrap();
        let row = rows.iter().find(|r| r.problem.id == "executables-in-root").unwrap();
        assert!(row.fix.is_some());
        let done = fix(&instance, &profile, "executables-in-root", &Trash::at(t.path("trash"))).unwrap();
        assert_eq!(done, "pointed SKSE at the game folder");
        let (instance, profile) = t.open();
        assert_eq!(instance.executables()[0].binary, t.path("Game Root/skse64_loader.exe").display().to_string());
        assert!(collect(&instance, &profile).unwrap().iter().all(|r| r.problem.id != "executables-in-root"));
    }

    #[test]
    fn log_paths_are_made_relative_without_case() {
        let rel = relative_ci(Path::new("/g/game root/DATA/SKSE//x.dll"), Path::new("/g/Game Root"));
        assert_eq!(rel.as_deref(), Some("DATA/SKSE/x.dll"));
        assert_eq!(relative_ci(Path::new("/elsewhere/x.dll"), Path::new("/g/Game Root")), None);
    }

    #[test]
    fn installed_but_disabled_masters_are_enabled_by_the_fix() {
        let t = TestInstance::new("+P\r\n");
        let data = t.folder("P");
        std::fs::write(data.join("Master.esm"), build_plugin(FLAG_MASTER, 44, &[], "", 1)).unwrap();
        std::fs::write(data.join("Child.esp"), build_plugin(0, 44, &["Master.esm", "Gone.esm"], "", 1)).unwrap();
        std::fs::write(t.path("profiles/Default/plugins.txt"), "Master.esm\r\n*Child.esp\r\n").unwrap();
        let (instance, profile) = t.open();
        let rows = collect(&instance, &profile).unwrap();
        let masters = rows.iter().find(|r| r.problem.id == "missing-masters").unwrap();
        assert!(masters.fix.is_some());
        assert_eq!(fix(&instance, &profile, "missing-masters", &Trash::at(t.path("trash"))).unwrap(), "enabled Master.esm");
        let txt = std::fs::read_to_string(t.path("profiles/Default/plugins.txt")).unwrap();
        assert!(txt.contains("*Master.esm"), "{txt}");
        // Gone.esm is still missing, but nothing installed is left to enable
        let rows = collect(&instance, &profile).unwrap();
        assert!(rows.iter().find(|r| r.problem.id == "missing-masters").is_some_and(|r| r.fix.is_none()));
    }
}
