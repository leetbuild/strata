//! The Plugins tab: the core's plugin list (`strata_core::plugins`, MO2's `PluginList`) over
//! the plugins of the merged Data folder, and what each row shows (`PluginList::data`).

use crate::html::escape;
use std::path::PathBuf;

use strata_core::plugins::{AvailablePlugin, Plugin, PluginList};
use strata_core::{Instance, Profile};

use crate::{Error, Result};

/// One row of the plugin list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRow {
    pub name: String,
    /// The mod the plugin comes from (`<game>` for the game's own).
    pub origin: String,
    pub enabled: bool,
    /// Enforced by the game: can't be disabled or moved (greyed).
    pub force_loaded: bool,
    /// Can't be enabled (dark red).
    pub force_disabled: bool,
    pub priority: usize,
    /// The Mod Index column (`FE:001` for light plugins, empty when not loaded).
    pub index: String,
    /// Bold: a master (by extension or flag) or `.esl` (`PluginList::fontData`).
    pub bold: bool,
    /// Italic: light (flag or extension).
    pub italic: bool,
    pub locked: bool,
    /// The Flags column's icons (`PluginList::iconData`, the parts the core knows).
    pub icons: Vec<&'static str>,
    /// The Name column's tooltip (HTML, `PluginList::tooltipData`).
    pub tooltip: String,
}

/// One archive of the Archives tab (`MainWindow::updateBSAList`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveRow {
    pub name: String,
    /// The mod it comes from; `<Unmanaged>` for the game's own.
    pub origin: String,
    /// The origin's place in the layers (the tab groups by mod, in priority order).
    pub origin_priority: usize,
    /// The game loads it: a default archive of the game, `Update.bsa`, or an archive named
    /// after an active plugin (`hasAssociatedPlugin`).
    pub loaded: bool,
}

/// The profile's plugin list, editable, and the archives of the merged Data folder.
pub struct PluginView {
    list: PluginList,
    profile_dir: PathBuf,
    archives: Vec<(String, String, usize)>,
    /// The names of the files at the top of the merged Data folder, lower case (a plugin's
    /// ini and archives are looked up there, as `PluginList::refresh` does).
    top_files: std::collections::HashSet<String>,
}

impl PluginView {
    /// The plugins of the merged Data folder (scanned like `strata list plugins`: on a
    /// worker) in the profile's order.
    pub fn build(instance: &Instance, profile: &Profile) -> Result<Self> {
        let tree = crate::profile_tree::merged_tree(instance, profile)?;
        let available = crate::profile_tree::available_plugins(instance, &tree);
        let mut archives = Vec::new();
        let mut top_files = std::collections::HashSet::new();
        if let Some(data) = tree.resolve(instance.game.data_dir()) {
            for (id, node) in tree.children(data) {
                if node.is_dir() {
                    continue;
                }
                let Some(layer) = tree.layer_of(id) else { continue };
                let lower = node.name.to_lowercase();
                top_files.insert(lower.clone());
                if lower.ends_with(".bsa") || lower.ends_with(".ba2") {
                    let origin = if layer.kind == strata_game::LayerKind::Game { crate::data::UNMANAGED.to_owned() } else { layer.id.clone() };
                    let priority = tree.layers().iter().position(|l| l.id == layer.id).unwrap_or(0);
                    archives.push((node.name.to_string(), origin, priority));
                }
            }
        }
        let list = instance.plugin_list(profile, available)?;
        Ok(Self { list, profile_dir: profile.dir.clone(), archives, top_files })
    }

    pub fn list(&self) -> &PluginList {
        &self.list
    }

    /// The Archives tab: grouped by mod in priority order, then by name.
    pub fn archives(&self) -> Vec<ArchiveRow> {
        let active: Vec<String> = self
            .list
            .plugins()
            .filter(|p| p.enabled)
            .map(|p| p.name.to_lowercase().rsplit_once('.').map_or(String::new(), |(stem, _)| stem.to_owned()))
            .collect();
        let mut rows: Vec<ArchiveRow> = self
            .archives
            .iter()
            .map(|(name, origin, priority)| {
                let lower = name.to_lowercase();
                // Skyrim.ini's default sResourceArchiveList names "Skyrim - *.bsa"
                let default = origin == crate::data::UNMANAGED && lower.starts_with("skyrim - ");
                let loaded = default || lower == "update.bsa" || active.iter().any(|stem| !stem.is_empty() && lower.starts_with(stem.as_str()));
                ArchiveRow { name: name.clone(), origin: origin.clone(), origin_priority: *priority, loaded }
            })
            .collect();
        rows.sort_by_cached_key(|r| (r.origin_priority, r.name.to_lowercase()));
        rows
    }

    /// The rows, in priority order.
    pub fn rows(&self) -> Vec<PluginRow> {
        self.list.plugins().map(|p| self.row(p)).collect()
    }

    fn row(&self, p: &Plugin) -> PluginRow {
        let locked = self.list.locked_position(&p.name).is_some();
        let form_43 = p.form_version() == Some(43);
        let mut icons = Vec::new();
        if !p.missing_masters.is_empty() || form_43 {
            icons.push(":/MO/gui/warning");
        }
        if locked {
            icons.push(":/MO/gui/locked");
        }
        // PluginList::refresh: `<base name>.ini` beside it, and every archive whose name starts
        // with the base name (completeBaseName: up to the last dot; any mod's archive)
        let base = p.name.rsplit_once('.').map_or(p.name.as_str(), |(stem, _)| stem).to_lowercase();
        let has_ini = self.top_files.contains(&format!("{base}.ini"));
        let mut loads: Vec<&String> = self
            .archives
            .iter()
            .map(|(name, _, _)| name)
            .filter(|name| name.to_lowercase().starts_with(&base))
            .collect();
        loads.sort_by_key(|n| n.to_lowercase());
        loads.dedup();
        if has_ini {
            icons.push(":/MO/gui/attachment");
        }
        if !loads.is_empty() {
            icons.push(":/MO/gui/archive_conflict_neutral");
        }
        if p.is_light_flagged && !p.has_light_extension {
            icons.push(":/MO/gui/awaiting");
        }
        if p.header.as_ref().is_ok_and(|h| h.num_records == 0) {
            icons.push(":/MO/gui/unchecked-checkbox"); // hasNoRecords
        }
        let mut tip = format!("<b>Origin</b>: {}", escape(&p.origin));
        if p.force_loaded {
            tip.push_str("<br><b><i>This plugin can't be disabled or moved (enforced by the game).</i></b>");
        } else if p.force_disabled {
            tip.push_str("<br><b><i>This plugin can't be disabled (enforced by the game).</i></b>");
        }
        match &p.header {
            Ok(h) => {
                if !h.author.is_empty() {
                    tip.push_str(&format!("<br><b>Author</b>: {}", escape(&h.author)));
                }
                if !h.description.is_empty() {
                    tip.push_str(&format!("<br><b>Description</b>: {}", escape(&h.description)));
                }
            }
            Err(e) => tip.push_str(&format!("<br><b>Cannot read the header</b>: {}", escape(e))),
        }
        if !p.missing_masters.is_empty() {
            tip.push_str(&format!("<br><b>Missing Masters</b>: <b>{}</b>", escape(&p.missing_masters.join(", "))));
        }
        if !p.masters.is_empty() {
            tip.push_str(&format!("<br><b>Masters</b>: {}", escape(&p.masters.join(", "))));
        }
        if !loads.is_empty() {
            // PluginList::makeToolTip: the names when fewer than six
            let names = if loads.len() < 6 {
                format!("{}<br>", escape(&loads.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")))
            } else {
                String::new()
            };
            tip.push_str(&format!(
                "<br><b>Loads Archives</b>: {names}There are Archives connected to this plugin. Their assets will be \
                 added to your game, overwriting in case of conflicts following the plugin order. Loose files will \
                 always overwrite assets from Archives. (This flag only checks for Archives from the same mod as the \
                 plugin)"
            ));
        }
        if has_ini {
            tip.push_str(
                "<br><b>Loads INI settings</b>: <br>There is an ini file connected to this plugin. Its settings will \
                 be added to your game settings, overwriting in case of conflicts.",
            );
        }
        if form_43 {
            tip.push_str("<br><b>Form version 43</b>: made for Skyrim LE, not converted for Special Edition.");
        }
        PluginRow {
            name: p.name.clone(),
            origin: p.origin.clone(),
            enabled: p.enabled,
            force_loaded: p.force_loaded,
            force_disabled: p.force_disabled,
            priority: p.priority,
            index: p.index.clone(),
            bold: p.has_master_extension || p.is_master_flagged || p.has_light_extension,
            italic: p.is_light_flagged || p.has_light_extension,
            locked,
            icons,
            tooltip: tip,
        }
    }

    /// Enables or disables plugins and saves `plugins.txt` / `loadorder.txt`; returns the
    /// names that changed.
    pub fn set_enabled(&mut self, names: &[String], enabled: bool) -> Result<Vec<String>> {
        let changed = self.list.set_enabled(names, enabled)?;
        self.save()?;
        Ok(changed)
    }

    /// Enables or disables every plugin but the forced ones (MO2's "Enable all" / "Disable
    /// all") and saves; returns the names that changed.
    pub fn set_enabled_all(&mut self, enabled: bool) -> Result<Vec<String>> {
        let changed = self.list.set_enabled_all(enabled);
        self.save()?;
        Ok(changed)
    }

    /// Moves plugins to a priority (`usize::MAX` = the bottom) by MO2's rules, and saves.
    pub fn send_to_priority(&mut self, names: &[String], priority: usize) -> Result<()> {
        self.list.send_to_priority(names, priority)?;
        self.save()
    }

    /// Locks or unlocks a plugin's load order position, and saves.
    pub fn set_locked(&mut self, name: &str, lock: bool) -> Result<()> {
        self.list.set_locked(name, lock)?;
        self.save()
    }

    fn save(&self) -> Result<()> {
        self.list.save(&self.profile_dir)?;
        Ok(())
    }
}

/// The Sort button (`strata sort`, MO2's lootcli run): the masterlist is updated when the
/// network allows (the copy from before is used otherwise), libloot sorts, and the new order
/// goes through the plugin list's rules (locked plugins, masters first) into
/// `loadorder.txt` and `plugins.txt`. Returns a line for the log. Slow: on a worker.
pub fn sort(instance: &Instance, profile: &Profile) -> Result<String> {
    let done = sort_with_report(instance, profile, true, std::sync::Arc::new(strata_loot::Silent))?;
    Ok(done.line())
}

/// What the LOOT dialog shows after a sort (MO2's `Loot::Report`): lootcli's report for the
/// enabled plugins, plus what Strata adds (how many plugins moved, why the masterlist was
/// not updated). Serialised for the shell with lootcli's names.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SortOutcome {
    #[serde(flatten)]
    pub report: strata_loot::Report,
    /// Plugins whose position changed (0: the order was already sorted).
    pub moved: usize,
    /// Why the download of the masterlist failed ("" when it worked or was not tried).
    pub masterlist_note: String,
    pub stats: SortStats,
}

/// MO2's `Loot::Stats`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SortStats {
    /// Milliseconds, the whole run.
    pub time: u64,
    pub strata_version: String,
    pub loot_version: String,
}

impl SortOutcome {
    /// The line for the log.
    pub fn line(&self) -> String {
        let note = if self.masterlist_note.is_empty() { String::new() } else { format!(" (masterlist not updated: {})", self.masterlist_note) };
        if self.moved == 0 { format!("the load order is already sorted{note}") } else { format!("sorted: {} plugins moved{note}", self.moved) }
    }
}

/// [`sort`], telling `observer` each step and returning the report. The masterlist is
/// downloaded first when `update` (MO2: once a session, never in offline mode);
/// `STRATA_LOOT_OFFLINE` (any value) never downloads (the tests).
pub fn sort_with_report(
    instance: &Instance,
    profile: &Profile,
    update: bool,
    observer: std::sync::Arc<dyn strata_loot::Observer>,
) -> Result<SortOutcome> {
    let dir = strata_loot::data_dir(instance.game.loot_name());
    sort_in(instance, profile, &dir, update && std::env::var_os("STRATA_LOOT_OFFLINE").is_none(), observer)
}

/// [`sort_with_report`] with the masterlist in `dir`, downloaded first when `update`.
fn sort_in(
    instance: &Instance,
    profile: &Profile,
    dir: &std::path::Path,
    update: bool,
    observer: std::sync::Arc<dyn strata_loot::Observer>,
) -> Result<SortOutcome> {
    use strata_loot::{Level, Progress};
    let started = std::time::Instant::now();
    let loot_err = |e: strata_loot::Error| Error::Loot(e.to_string());
    observer.progress(Progress::CheckingMasterlistExistence);
    let view = PluginView::build(instance, profile)?;
    // runLoot starts with savePluginList: the profile's files then hold the list as shown
    // (a mod enabled since the last write adds plugins), which is what libloot must read
    view.save()?;
    let loot_name = instance.game.loot_name();
    let masterlist = dir.join("masterlist.yaml");
    let mut note = String::new();
    if update {
        observer.progress(Progress::UpdatingMasterlist);
        match strata_loot::update_masterlist_with(loot_name, dir, observer.as_ref()) {
            Ok(()) => {}
            Err(strata_loot::Error::Cancelled) => return Err(loot_err(strata_loot::Error::Cancelled)),
            Err(e) if masterlist.is_file() => {
                observer.log(Level::Warning, &format!("{e}; using the masterlist from before"));
                note = e.to_string();
            }
            Err(e) => return Err(loot_err(e)),
        }
    }
    // the user's LOOT userlist is in the prefix, where lootcli reads it under Proton
    let userlist = instance.proton.prefix.as_ref().map(|p| {
        p.join("pfx/drive_c/users/steamuser/AppData/Local/LOOT/games").join(instance.game.appdata_dir()).join("userlist.yaml")
    });
    if observer.cancelled() {
        return Err(loot_err(strata_loot::Error::Cancelled));
    }
    // the merged game folder, case-insensitive (MO2 runs lootcli in its VFS); a private mount
    // lives until `game` is dropped at the end of this function, whatever happens
    let game = crate::loot_view::LootView::open(instance, profile)?;
    let level = if game.how == crate::loot_view::How::NoFuse { Level::Warning } else { Level::Info };
    observer.log(level, &game.how.to_string());
    let input = strata_loot::SortInput {
        loot_name: loot_name.to_owned(),
        game_path: game.game_path.clone(),
        local_path: profile.dir.clone(),
        data_paths: game.data_paths.clone(),
        plugins: view.list.plugins().map(|p| (p.name.clone(), p.path.clone())).collect(),
        masterlist,
        prelude: Some(dir.join("prelude.yaml")),
        userlist,
    };
    let sorted = strata_loot::sort_with_report(&input, observer.clone()).map_err(loot_err)?;
    if observer.cancelled() {
        return Err(loot_err(strata_loot::Error::Cancelled));
    }
    observer.progress(Progress::WritingLoadorder);
    let mut loadorder = format!("{}\r\n", strata_core::MODLIST_HEADER);
    for name in &sorted.order {
        loadorder.push_str(name);
        loadorder.push_str("\r\n");
    }
    let files = strata_core::plugins::PluginFiles {
        plugins_txt: Some(view.list.plugins_txt()),
        loadorder_txt: Some(loadorder.into_bytes()),
        lockedorder_txt: Some(view.list.lockedorder_txt()),
    };
    let available =
        view.list.plugins().map(|p| AvailablePlugin { name: p.name.clone(), origin: p.origin.clone(), path: p.path.clone() }).collect();
    let new = PluginList::load(available, instance.plugin_rules(), &files);
    let moved = new.plugins().zip(view.list.plugins()).filter(|(a, b)| a.name != b.name).count();
    new.save(&profile.dir)?;
    // Loot::reportPlugin: lootcli doesn't know which plugins are enabled; MO2 drops the others
    let enabled: std::collections::HashSet<String> = new.plugins().filter(|p| p.enabled).map(|p| p.name.to_lowercase()).collect();
    let mut report = sorted.report;
    report.plugins.retain(|p| enabled.contains(&p.name.to_lowercase()));
    observer.progress(Progress::Done);
    Ok(SortOutcome {
        report,
        moved,
        masterlist_note: note,
        stats: SortStats {
            time: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            strata_version: env!("CARGO_PKG_VERSION").to_owned(),
            loot_version: strata_loot::libloot_version(),
        },
    })
}

/// A minimal plugin file for the tests of other modules.
#[cfg(test)]
pub(crate) fn tests_plugin_bytes() -> Vec<u8> {
    tests::plugin(0, &[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_instance::TestInstance;

    /// A minimal TES4 plugin: the record header and HEDR, `flags` 1 = master.
    pub(super) fn plugin(flags: u32, masters: &[&str]) -> Vec<u8> {
        let mut sub = Vec::new();
        sub.extend_from_slice(b"HEDR");
        sub.extend_from_slice(&12u16.to_le_bytes());
        sub.extend_from_slice(&1.7f32.to_le_bytes());
        sub.extend_from_slice(&0u32.to_le_bytes());
        sub.extend_from_slice(&0x800u32.to_le_bytes());
        for m in masters {
            let z = format!("{m}\0");
            sub.extend_from_slice(b"MAST");
            sub.extend_from_slice(&(z.len() as u16).to_le_bytes());
            sub.extend_from_slice(z.as_bytes());
            sub.extend_from_slice(b"DATA");
            sub.extend_from_slice(&8u16.to_le_bytes());
            sub.extend_from_slice(&0u64.to_le_bytes());
        }
        let mut rec = Vec::new();
        rec.extend_from_slice(b"TES4");
        rec.extend_from_slice(&(sub.len() as u32).to_le_bytes());
        rec.extend_from_slice(&flags.to_le_bytes());
        rec.extend_from_slice(&0u32.to_le_bytes());
        rec.extend_from_slice(&0u32.to_le_bytes());
        rec.extend_from_slice(&44u16.to_le_bytes());
        rec.extend_from_slice(&0u16.to_le_bytes());
        rec.extend_from_slice(&sub);
        rec
    }

    #[test]
    fn rows_flags_and_operations_through_the_core() {
        let t = TestInstance::new("+A\r\n");
        std::fs::write(t.folder("A").join("a.esp"), plugin(0, &["Missing.esm"])).unwrap();
        std::fs::write(t.folder("A").join("b.esp"), plugin(0, &[])).unwrap();
        let (instance, profile) = t.open();
        let mut view = PluginView::build(&instance, &profile).unwrap();
        // masters are only checked for enabled plugins
        view.set_enabled(&["a.esp".to_owned()], true).unwrap();
        let rows = view.rows();
        let a = rows.iter().find(|r| r.name == "a.esp").unwrap();
        assert_eq!(a.origin, "A");
        assert!(a.icons.contains(&":/MO/gui/warning"), "missing master");
        assert!(a.tooltip.contains("Missing Masters</b>: <b>Missing.esm"), "{}", a.tooltip);

        let changed = view.set_enabled(&["b.esp".to_owned()], true).unwrap();
        assert_eq!(changed, ["b.esp"]);
        let plugins_txt = std::fs::read_to_string(t.path("profiles/Default/plugins.txt")).unwrap();
        assert!(plugins_txt.contains("*b.esp"), "{plugins_txt}");

        view.set_locked("b.esp", true).unwrap();
        assert!(view.rows().iter().find(|r| r.name == "b.esp").unwrap().icons.contains(&":/MO/gui/locked"));
        assert!(t.path("profiles/Default/lockedorder.txt").exists());

        std::fs::write(t.folder("A").join("b.bsa"), b"").unwrap();
        std::fs::write(t.folder("A").join("other.bsa"), b"").unwrap();
        let view = PluginView::build(&instance, &profile).unwrap();
        let archives = view.archives();
        assert_eq!(archives.len(), 2);
        assert!(archives.iter().find(|a| a.name == "b.bsa").unwrap().loaded, "b.esp is enabled");
        assert!(!archives.iter().find(|a| a.name == "other.bsa").unwrap().loaded);

        // b.esp: b.bsa is its archive, B.ini its ini; a.esp has neither
        std::fs::write(t.folder("A").join("B.ini"), b"").unwrap();
        let view = PluginView::build(&instance, &profile).unwrap();
        let rows = view.rows();
        let b = rows.iter().find(|r| r.name == "b.esp").unwrap();
        assert!(b.icons.contains(&":/MO/gui/attachment") && b.icons.contains(&":/MO/gui/archive_conflict_neutral"), "{:?}", b.icons);
        assert!(b.tooltip.contains("Loads Archives</b>: b.bsa<br>") && b.tooltip.contains("Loads INI settings"), "{}", b.tooltip);
        let a = rows.iter().find(|r| r.name == "a.esp").unwrap();
        assert!(!a.icons.contains(&":/MO/gui/attachment") && !a.icons.contains(&":/MO/gui/archive_conflict_neutral"));
    }

    /// The LOOT dialog's report: the masterlist's messages for enabled plugins only (MO2's
    /// `Loot::reportPlugin`), the moved count, the files written, the steps in order.
    #[test]
    fn sort_reports_for_enabled_plugins() {
        let t = TestInstance::new("+A\r\n");
        std::fs::write(t.path("Game Root/Data/Skyrim.esm"), plugin(1, &[])).unwrap();
        for name in ["a.esp", "b.esp", "c.esp"] {
            std::fs::write(t.folder("A").join(name), plugin(0, &["Skyrim.esm"])).unwrap();
        }
        let (instance, profile) = t.open();
        let mut view = PluginView::build(&instance, &profile).unwrap();
        view.set_enabled(&["a.esp".to_owned(), "b.esp".to_owned()], true).unwrap();
        let order: Vec<String> = view.list().plugins().map(|p| p.name.clone()).collect();
        assert_eq!(order, ["Skyrim.esm", "a.esp", "b.esp", "c.esp"]);
        // a plugin the profile's files don't name yet (its mod was just installed): the list is
        // saved before sorting, as runLoot does, so libloot reads Strata's order
        std::fs::write(t.folder("A").join("0new.esp"), plugin(0, &["Skyrim.esm"])).unwrap();
        // and a plugins.txt in another order than loadorder.txt (libloadorder reads the order
        // of Skyrim SE's from plugins.txt, MO2 from loadorder.txt)
        std::fs::write(t.path("profiles/Default/plugins.txt"), "*b.esp\r\n*a.esp\r\n").unwrap();
        let loot = t.path("loot");
        std::fs::create_dir_all(&loot).unwrap();
        std::fs::write(
            loot.join("masterlist.yaml"),
            "globals:\n  - type: say\n    content: 'Hello.'\nplugins:\n  - name: a.esp\n    after: [b.esp]\n  - name: b.esp\n    msg:\n      - type: warn\n        content: 'About b.'\n  - name: c.esp\n    msg:\n      - type: say\n        content: 'About c (disabled).'\n",
        )
        .unwrap();
        struct Steps(std::sync::Mutex<Vec<strata_loot::Progress>>);
        impl strata_loot::Observer for Steps {
            fn progress(&self, step: strata_loot::Progress) {
                self.0.lock().unwrap().push(step);
            }
            fn log(&self, _: strata_loot::Level, _: &str) {}
        }
        let steps = std::sync::Arc::new(Steps(std::sync::Mutex::new(Vec::new())));
        let done = sort_in(&instance, &profile, &loot, false, steps.clone()).unwrap();
        assert_eq!(done.moved, 2);
        assert_eq!(done.line(), "sorted: 2 plugins moved");
        assert_eq!(done.report.messages.len(), 1);
        let names: Vec<&str> = done.report.plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["b.esp"], "c.esp is disabled");
        let loadorder = std::fs::read_to_string(t.path("profiles/Default/loadorder.txt")).unwrap();
        assert!(loadorder.contains("b.esp\r\na.esp"), "{loadorder}");
        let json = serde_json::to_value(&done).unwrap();
        assert_eq!(json["plugins"][0]["messages"][0]["type"], "warn");
        assert_eq!(json["moved"], 2);
        assert!(json["stats"]["lootVersion"].as_str().unwrap().starts_with("0."), "{json}");
        use strata_loot::Progress as P;
        assert_eq!(
            *steps.0.lock().unwrap(),
            [P::CheckingMasterlistExistence, P::LoadingLists, P::ReadingPlugins, P::SortingPlugins, P::ParsingLootMessages, P::WritingLoadorder, P::Done]
        );

        // sorted again: nothing moves
        let again = sort_in(&instance, &profile, &loot, false, std::sync::Arc::new(strata_loot::Silent)).unwrap();
        assert_eq!(again.line(), "the load order is already sorted");
    }

    /// The masterlist's conditions see the profile through the case-insensitive overlay (MO2
    /// runs lootcli in its VFS): `file("scripts/skse.pex")` holds for a mod's
    /// `Scripts/SKSE.pex`, `../skse64_loader.exe` for a mod's Root file. The private mount is
    /// gone afterwards, also after a failed and a cancelled sort.
    #[test]
    fn conditions_ignore_case_through_the_overlay() {
        if !crate::loot_view::fuse_available() {
            eprintln!("SKIPPED: conditions_ignore_case_through_the_overlay: /dev/fuse or fusermount3 is missing");
            return;
        }
        let t = TestInstance::new("+SKSE\r\n+A\r\n");
        std::fs::write(t.path("Game Root/Data/Skyrim.esm"), plugin(1, &[])).unwrap();
        std::fs::write(t.folder("A").join("a.esp"), plugin(0, &["Skyrim.esm"])).unwrap();
        std::fs::create_dir_all(t.folder("SKSE").join("Scripts")).unwrap();
        std::fs::write(t.folder("SKSE").join("Scripts/SKSE.pex"), "x").unwrap();
        std::fs::create_dir_all(t.folder("SKSE").join("Root")).unwrap();
        std::fs::write(t.folder("SKSE").join("Root/SKSE64_Loader.exe"), "x").unwrap();
        let (instance, profile) = t.open();
        let mut view = PluginView::build(&instance, &profile).unwrap();
        view.set_enabled(&["a.esp".to_owned()], true).unwrap();
        let loot = t.path("loot");
        std::fs::create_dir_all(&loot).unwrap();
        std::fs::write(
            loot.join("masterlist.yaml"),
            "globals:\n  - type: warn\n    content: 'SKSE scripts missing.'\n    condition: 'not file(\"scripts/skse.pex\") and file(\"../skse64_loader.exe\")'\n  - type: say\n    content: 'SKSE loader found.'\n    condition: 'file(\"../skse64_loader.exe\")'\nplugins:\n  - name: a.esp\n    msg:\n      - type: say\n        content: 'Textures there.'\n        condition: 'file(\"SCRIPTS/skse.PEX\")'\n",
        )
        .unwrap();
        let mount = crate::loot_view::mount_dir(&instance);
        let gone = |m: &std::path::Path| strata_vfs::mount::mounted_at(m).unwrap().is_none() && !m.exists();

        let done = sort_in(&instance, &profile, &loot, false, std::sync::Arc::new(strata_loot::Silent)).unwrap();
        let general: Vec<&str> = done.report.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(general, ["SKSE loader found."], "the scripts are there in another case");
        assert_eq!(done.report.plugins.len(), 1);
        assert_eq!(done.report.plugins[0].messages[0].text, "Textures there.");
        assert!(gone(&mount), "the private mount is removed");

        // a failed sort (no masterlist) and a cancelled one leave no mount either
        let empty = t.path("no-loot");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(sort_in(&instance, &profile, &empty, false, std::sync::Arc::new(strata_loot::Silent)).is_err());
        assert!(gone(&mount));
        struct CancelAt(std::sync::atomic::AtomicBool);
        impl strata_loot::Observer for CancelAt {
            fn progress(&self, step: strata_loot::Progress) {
                if step == strata_loot::Progress::ReadingPlugins {
                    self.0.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            fn log(&self, _: strata_loot::Level, _: &str) {}
            fn cancelled(&self) -> bool {
                self.0.load(std::sync::atomic::Ordering::Relaxed)
            }
        }
        let cancel = std::sync::Arc::new(CancelAt(std::sync::atomic::AtomicBool::new(false)));
        let e = sort_in(&instance, &profile, &loot, false, cancel).unwrap_err();
        assert!(e.to_string().contains("cancelled"), "{e}");
        assert!(gone(&mount));
    }
}
