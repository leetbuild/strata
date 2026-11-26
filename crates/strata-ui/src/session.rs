//! One opened instance and profile, as the window sees it.

use std::path::{Path, PathBuf};

use strata_core::separators::DropTarget;
use strata_core::write::Transaction;
use strata_core::{Categories, Color, Executable, Instance, ModList, ModListEvent, ModMeta, Profile, Trash};

use crate::data::DataView;
use crate::downloads::{self, DownloadRow};
use crate::mods::{self, MetaCache, ModRow};
use crate::plugins::{self, PluginView};
use crate::saves::{self, SaveRow};
use crate::{Result, style};

/// The instance and profile the window shows. Holds the core's objects (and a cache of
/// what the rows show from each `meta.ini`); every view is computed from them on request,
/// every change goes through the core's operations, which return their events.
#[derive(Debug)]
pub struct Session {
    instance: Instance,
    profile: Profile,
    list: ModList,
    categories: Categories,
    metas: MetaCache,
}

impl Session {
    /// Opens the instance in `root` and its `profile` (the selected one for `None`). An
    /// interrupted multi-file write of an earlier run is finished or rolled back first, as
    /// the CLI does.
    pub fn open(root: &Path, profile: Option<&str>) -> Result<Self> {
        let instance = Instance::open(root)?;
        strata_core::write::recover(&instance.strata_dir())?;
        let profile = instance.profile(profile)?;
        let list = ModList::load(&instance, &profile)?;
        let categories = Categories::open(&instance.root)?;
        Ok(Self { instance, profile, list, categories, metas: MetaCache::default() })
    }

    /// Reads the instance and profile again from disk (after a change made elsewhere).
    pub fn reload(&mut self) -> Result<()> {
        *self = Self::open(&self.instance.root.clone(), Some(&self.profile.name.clone()))?;
        Ok(())
    }

    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// `ModOrganizer.ini`: where the window stores its layout.
    pub fn ini_path(&self) -> PathBuf {
        self.instance.root.join("ModOrganizer.ini")
    }

    /// The instance's profile names, sorted.
    pub fn profile_names(&self) -> Result<Vec<String>> {
        Ok(self.instance.profile_names()?)
    }

    /// The style the instance names, or the default ([`style::default_style`]: `riki.qss`
    /// when it is in a search folder, else `vs15 Dark.qss`).
    pub fn style(&self) -> String {
        style::instance_style(&self.instance.ini).unwrap_or_else(|| {
            let dirs = style::search_dirs(Some(&self.instance.root), &style::program_dir(), style::data_home().as_deref());
            style::default_style(&dirs).to_owned()
        })
    }

    /// The mod list's rows (see [`mods::rows`]).
    pub fn mods(&mut self) -> Vec<ModRow> {
        mods::rows(&self.instance, &self.list, &self.categories, &mut self.metas)
    }

    /// The filter panel's entries (`FilterList::refresh`): the special criteria, one
    /// `<Contains ...>` per content, then the categories some mod has (with their parents),
    /// as a flat list in `categories.dat`'s order.
    pub fn filter_entries(&mut self) -> Vec<crate::filter::PanelEntry> {
        use crate::filter::{PanelEntry, PanelKind, Special};
        let mut out: Vec<PanelEntry> =
            Special::ALL.iter().map(|s| PanelEntry { kind: PanelKind::Special, id: *s as i32, name: s.name().to_owned() }).collect();
        for c in crate::content::Content::ALL {
            out.push(PanelEntry { kind: PanelKind::Content, id: c as i32, name: format!("<Contains {}>", c.name()) });
        }
        let mut used = std::collections::HashSet::new();
        for row in self.mods() {
            for mut id in row.categories {
                // parents too, so they show; a cycle stops at the first repeat
                while id != 0 && used.insert(id) {
                    id = self.categories.get(id).map_or(0, |c| c.parent);
                }
            }
        }
        for c in &self.categories.list {
            if used.contains(&c.id) {
                out.push(PanelEntry { kind: PanelKind::Category, id: c.id, name: c.name.clone() });
            }
        }
        out
    }

    /// "Backup mod list" (`on_saveModsButton_clicked`): `modlist.txt.<date>`; returns the
    /// backup's name.
    pub fn backup_mod_list(&self) -> Result<String> {
        let copy = strata_core::backups::create_backup(&self.profile.dir.join("modlist.txt"), &strata_core::backups::now_suffix())?;
        Ok(copy.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
    }

    /// "Backup load order" (`on_saveButton_clicked`): the plugin files that exist.
    pub fn backup_plugins(&self) -> Result<usize> {
        let suffix = strata_core::backups::now_suffix();
        let mut made = 0;
        for name in strata_core::backups::PLUGIN_FILES {
            let file = self.profile.dir.join(name);
            if file.is_file() {
                strata_core::backups::create_backup(&file, &suffix)?;
                made += 1;
            }
        }
        Ok(made)
    }

    /// The backups of `modlist.txt` (`mods`) or of `plugins.txt` (the load order), newest
    /// first: their suffixes.
    pub fn backups(&self, mods: bool) -> Result<Vec<String>> {
        let file = self.profile.dir.join(if mods { "modlist.txt" } else { "plugins.txt" });
        Ok(strata_core::backups::list_backups(&file)?.into_iter().map(|b| b.suffix).collect())
    }

    /// Restores a backup (`on_restoreModsButton_clicked` / `on_restoreButton_clicked`); the
    /// mod list is read again after a mod list restore.
    pub fn restore_backup(&mut self, mods: bool, suffix: &str) -> Result<()> {
        if mods {
            strata_core::backups::restore_backup(&self.profile.dir.join("modlist.txt"), suffix)?;
            self.profile = self.instance.profile(Some(&self.profile.name))?;
            self.list = ModList::load(&self.instance, &self.profile)?;
        } else {
            for name in strata_core::backups::PLUGIN_FILES {
                let file = self.profile.dir.join(name);
                let backup = file.with_file_name(format!("{name}.{suffix}"));
                if backup.is_file() {
                    strata_core::backups::restore_backup(&file, suffix)?;
                }
            }
        }
        Ok(())
    }

    /// Makes `name` the instance's selected profile (`selected_profile` in
    /// `ModOrganizer.ini`, as MO2's profile combo does).
    pub fn select_profile(&mut self, name: &str) -> Result<()> {
        self.instance.select_profile(name)?;
        Ok(())
    }

    /// The profile a live deploy of this instance shows, if one runs.
    pub fn deployed_profile(&self) -> Result<Option<String>> {
        crate::live::deployed_profile(&self.instance)
    }

    /// The core's mod list.
    pub fn mod_list(&self) -> &ModList {
        &self.list
    }

    fn after(&mut self, events: Vec<ModListEvent>) -> Result<Vec<ModListEvent>> {
        if !events.is_empty() {
            self.list.save()?;
        }
        for e in &events {
            match e {
                ModListEvent::MetaChanged { name } => self.metas.forget(name),
                ModListEvent::Renamed { from, to } => {
                    self.metas.forget(from);
                    self.metas.forget(to);
                }
                _ => {}
            }
        }
        Ok(events)
    }

    /// Switches mods on or off (the checkbox, "Enable/Disable selected").
    pub fn set_enabled(&mut self, names: &[String], enabled: bool) -> Result<Vec<ModListEvent>> {
        let events = self.list.set_enabled(names, enabled)?;
        self.after(events)
    }

    /// Moves mods to `priority` (`usize::MAX` = the end): "Send to top / bottom / priority".
    pub fn move_mods(&mut self, names: &[String], priority: usize) -> Result<Vec<ModListEvent>> {
        let events = self.list.move_mods(names, priority)?;
        self.after(events)
    }

    /// "Send to separator": after `separator`'s own mods, before its first sub-separator
    /// (`priority_for_mod_in_separator`; MO2's `priorityAfterSeparator` put them inside the
    /// last sub-separator).
    pub fn send_to_separator(&mut self, names: &[String], separator: &str) -> Result<Vec<ModListEvent>> {
        let entries = self.list.entries();
        let s = self.list.index_of(separator).ok_or_else(|| strata_core::Error::NoSuchMod { name: separator.to_owned() })?;
        let target = strata_core::separators::priority_for_mod_in_separator(entries, s).unwrap_or(usize::MAX);
        let events = self.list.drop_mods_at(names, target)?;
        self.after(events)
    }

    /// A drag and drop by ascending priority (`ModListByPriorityProxy::dropMimeData`):
    /// `parent` is the separator dropped into (its name; `None` = the top level), `row` the
    /// row under it (`None` = onto the separator itself). Returns no events when the drop
    /// is not allowed.
    pub fn drop_mods(&mut self, names: &[String], parent: Option<&str>, row: Option<usize>, below_expanded_separator: bool) -> Result<Vec<ModListEvent>> {
        let dropped = self.list.indices_of(names)?;
        let parent = match parent {
            Some(name) => Some(self.list.index_of(name).ok_or_else(|| strata_core::Error::NoSuchMod { name: name.to_owned() })?),
            None => None,
        };
        let target = DropTarget { parent, row, below_expanded_separator };
        let Some(plan) = self.list.tree().plan_drop(self.list.entries(), target, &dropped) else { return Ok(Vec::new()) };
        let mut txn = Transaction::new(&self.instance.strata_dir())?;
        let events = self.list.apply_drop(&plan, &dropped, &mut txn)?;
        self.list.stage(&mut txn)?;
        txn.commit()?;
        for e in &events {
            if let ModListEvent::NestChanged { name, .. } = e {
                self.metas.forget(name);
            }
        }
        Ok(events)
    }

    /// "Move up / down" (Ctrl+Up / Ctrl+Down, `ModListView::moveSelection` →
    /// `ModList::shiftModsPriority`): each mod `offset` priorities further.
    pub fn shift_mods(&mut self, names: &[String], offset: isize) -> Result<Vec<ModListEvent>> {
        let events = self.list.shift_mods(names, offset)?;
        self.after(events)
    }

    /// Space on the selection (`ModList::toggleState`): each enabled mod is disabled and
    /// each disabled one enabled.
    pub fn toggle_mods(&mut self, names: &[String]) -> Result<Vec<ModListEvent>> {
        let (on, off): (Vec<String>, Vec<String>) =
            names.iter().cloned().partition(|n| self.list.get(n).is_some_and(|e| e.enabled));
        let mut events = self.list.set_enabled(&off, true)?;
        events.extend(self.list.set_enabled(&on, false)?);
        self.after(events)
    }

    /// A separator's Sub-Separator level (the Ctrl+Shift+Enter dialog,
    /// `SubSeparator::SeparatorDialog`).
    pub fn set_nest(&mut self, name: &str, nest: u32) -> Result<Vec<ModListEvent>> {
        let mut txn = Transaction::new(&self.instance.strata_dir())?;
        let events = self.list.set_nest(name, nest, &mut txn)?;
        txn.commit()?;
        self.metas.forget(name);
        Ok(events)
    }

    /// Overwrite's context menu (`ModListContextMenu::addOverwriteActions`): its content
    /// into a new mod (`createModFromOverwrite`, `create` true) or into an existing regular
    /// mod (`moveOverwriteContentToExistingMod`). Refused while deployed: the overlay writes
    /// into Overwrite (as `strata overwrite`).
    pub fn move_overwrite(&mut self, name: &str, create: bool) -> Result<Vec<ModListEvent>> {
        self.refuse_overwrite_change_while_deployed()?;
        let mut events = Vec::new();
        let mut name = name.to_owned();
        if create {
            let made = self.list.create_mod(&self.instance.paths.mods, &name, None)?;
            if let Some(ModListEvent::Added { name: fixed, .. }) = made.first() {
                name = fixed.clone();
            }
            events.extend(self.after(made)?);
        }
        let entry = self.list.get(&name).ok_or_else(|| strata_core::Error::NoSuchMod { name: name.clone() })?;
        if entry.kind != strata_core::ModKind::Regular {
            return Err(crate::Error::Invalid(format!("\"{}\" is not a regular mod", entry.name)));
        }
        let path = entry.path.clone().ok_or_else(|| crate::Error::Invalid(format!("\"{name}\" has no folder")))?;
        events.extend(self.instance.move_overwrite_to(&path)?);
        self.metas.forget(&name);
        Ok(events)
    }

    /// "Clear Overwrite..." (`clearOverwrite`): everything in it to the trash, or for good.
    pub fn clear_overwrite(&mut self, trash: bool) -> Result<Vec<ModListEvent>> {
        self.refuse_overwrite_change_while_deployed()?;
        let bin = if trash { Some(Trash::home()?) } else { None };
        Ok(self.instance.clear_overwrite(bin.as_ref())?)
    }

    /// The overwrite dialog (`OverwriteInfoDialog`): entries of Overwrite (paths relative to
    /// it) to the trash or for good. MO2 never deletes a game mapping's folder there; the
    /// Root folder (Leet change 5) is kept the same way. Refused while deployed.
    pub fn remove_from_overwrite(&self, paths: &[String], trash: bool) -> Result<()> {
        self.refuse_overwrite_change_while_deployed()?;
        let bin = if trash { Some(Trash::home()?) } else { None };
        for rel in paths {
            let path = self.overwrite_path(rel)?;
            strata_core::trash::remove(&path, bin.as_ref())?;
        }
        Ok(())
    }

    /// The overwrite dialog's Rename: `rel` gets the file name `new_name`.
    pub fn rename_in_overwrite(&self, rel: &str, new_name: &str) -> Result<()> {
        self.refuse_overwrite_change_while_deployed()?;
        let from = self.overwrite_path(rel)?;
        if new_name.is_empty() || new_name.contains('/') || new_name == "." || new_name == ".." {
            return Err(crate::Error::Invalid(format!("\"{new_name}\" is not a file name")));
        }
        let to = from.with_file_name(new_name);
        if to.symlink_metadata().is_ok() {
            return Err(crate::Error::Invalid(format!("\"{new_name}\" exists already")));
        }
        std::fs::rename(&from, &to).map_err(|source| crate::Error::Write { path: to, source })
    }

    /// The overwrite dialog's New Folder in the folder `rel_dir` ("" = Overwrite itself):
    /// "New Folder", else "New Folder1", "New Folder2"... (`createDirectoryTriggered`).
    /// Returns the new folder's path relative to Overwrite.
    pub fn new_folder_in_overwrite(&self, rel_dir: &str) -> Result<String> {
        self.refuse_overwrite_change_while_deployed()?;
        let dir = if rel_dir.is_empty() { self.instance.paths.overwrite.clone() } else { self.overwrite_path(rel_dir)? };
        let mut name = "New Folder".to_owned();
        let mut suffix = 1;
        while dir.join(&name).symlink_metadata().is_ok() {
            name = format!("New Folder{suffix}");
            suffix += 1;
        }
        let path = dir.join(&name);
        std::fs::create_dir(&path).map_err(|source| crate::Error::Write { path: path.clone(), source })?;
        Ok(if rel_dir.is_empty() { name } else { format!("{rel_dir}/{name}") })
    }

    /// An existing entry of Overwrite by its relative path; never Overwrite itself, nothing
    /// outside it, not its top `Root` folder.
    fn overwrite_path(&self, rel: &str) -> Result<PathBuf> {
        let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() || parts.iter().any(|p| *p == ".." || *p == ".") {
            return Err(crate::Error::Invalid(format!("\"{rel}\" is not a path inside overwrite")));
        }
        if parts.len() == 1 && parts[0].eq_ignore_ascii_case("root") {
            return Err(crate::Error::Invalid("the Root folder of overwrite stays (its files go to the game folder)".into()));
        }
        let path = parts.iter().fold(self.instance.paths.overwrite.clone(), |p, part| p.join(part));
        if path.symlink_metadata().is_err() {
            return Err(crate::Error::Invalid(format!("\"{rel}\" is not in overwrite")));
        }
        Ok(path)
    }

    /// "Export to csv..." (`listtools::export_csv`).
    pub fn export_csv(&mut self, which: crate::listtools::CsvRows, visible: &[String], fields: &[String]) -> String {
        let rows = self.mods();
        crate::listtools::export_csv(&self.instance, &rows, which, visible, fields)
    }

    /// "Auto assign categories" (`listtools::auto_assign_categories`); the mods changed.
    pub fn auto_assign_categories(&mut self) -> Result<Vec<String>> {
        let changed = crate::listtools::auto_assign_categories(&self.instance, &self.list)?;
        for name in &changed {
            self.metas.forget(name);
        }
        Ok(changed)
    }

    /// "Sync to Mods...": Overwrite's files with the mods they could go back to.
    pub fn sync_overwrite_files(&self) -> Result<Vec<crate::listtools::SyncFile>> {
        crate::listtools::sync_files(&self.instance, &self.profile)
    }

    /// Moves Overwrite's files (path, mod) into the mods. Refused while deployed. Returns
    /// how many moved and one message per file that didn't.
    pub fn sync_overwrite(&self, moves: &[(String, String)]) -> Result<(usize, Vec<String>)> {
        self.refuse_overwrite_change_while_deployed()?;
        Ok(crate::listtools::sync_apply(&self.instance, moves))
    }

    fn refuse_overwrite_change_while_deployed(&self) -> Result<()> {
        if let Some(deployed) = self.deployed_profile()? {
            return Err(crate::Error::Invalid(format!(
                "overwrite can't change while the instance is deployed (profile \"{deployed}\"): the game may be \
                 writing into it. Undeploy first."
            )));
        }
        Ok(())
    }

    /// Renames a mod (folder and every profile's list).
    pub fn rename_mod(&mut self, old: &str, new: &str) -> Result<Vec<ModListEvent>> {
        let events = self.list.rename_mod(&self.instance, old, new)?;
        // rename_mod writes the lists itself, in one transaction
        for e in &events {
            if let ModListEvent::Renamed { from, to } = e {
                self.metas.forget(from);
                self.metas.forget(to);
            }
        }
        Ok(events)
    }

    /// "Create empty mod" (`ModListViewActions::createEmptyMod`): MO2's initial `meta.ini`,
    /// on top of the list, or with `at` just above that mod / at the end of that separator.
    pub fn create_empty_mod(&mut self, name: &str, at: Option<&str>) -> Result<Vec<ModListEvent>> {
        let events = self.list.create_mod(&self.instance.paths.mods, name, at)?;
        self.after(events)
    }

    /// Creates a separator (`ModListViewActions::createSeparator`), `name` without the
    /// suffix, at `at` (see `ModList::create_separator`).
    pub fn create_separator(&mut self, name: &str, at: Option<&str>) -> Result<Vec<ModListEvent>> {
        let events = self.list.create_separator(&self.instance.paths.mods, name, at)?;
        self.after(events)
    }

    /// A folder from outside dropped on the mod list (`ModListView::onExternalFolderDropped`):
    /// a new mod `name` (MO2's initial `meta.ini`, on top of the list) holding a copy of
    /// the folder's content. As MO2's `copyDir`, a file that exists already in the new mod
    /// is kept (`QFile::copy` does not overwrite). Symbolic links are copied as links.
    pub fn create_mod_from_folder(&mut self, name: &str, folder: &Path) -> Result<Vec<ModListEvent>> {
        if !folder.is_dir() {
            return Err(crate::Error::Invalid(format!("{} is not a folder", folder.display())));
        }
        let events = self.list.create_mod(&self.instance.paths.mods, name, None)?;
        let made = events.iter().find_map(|e| match e {
            ModListEvent::Added { name, .. } => Some(name.clone()),
            _ => None,
        });
        if let Some(made) = made {
            copy_into(folder, &self.instance.paths.mods.join(made))?;
        }
        self.after(events)
    }

    /// Sets a mod's short note (the Notes column, `comments`).
    pub fn set_comments(&mut self, name: &str, text: &str) -> Result<Vec<ModListEvent>> {
        self.edit_meta(name, |m| m.set_comments(text))
    }

    /// Sets or clears a separator's colour.
    pub fn set_color(&mut self, name: &str, color: Option<Color>) -> Result<Vec<ModListEvent>> {
        self.edit_meta(name, |m| m.set_color(color))
    }

    fn edit_meta(&mut self, name: &str, edit: impl FnOnce(&mut ModMeta) -> bool) -> Result<Vec<ModListEvent>> {
        let entry = self.list.get(name).ok_or_else(|| strata_core::Error::NoSuchMod { name: name.to_owned() })?;
        let Some(dir) = entry.path.clone() else { return Ok(Vec::new()) };
        let mut meta = ModMeta::open(&dir)?;
        if !edit(&mut meta) {
            return Ok(Vec::new());
        }
        meta.save()?;
        self.after(vec![ModListEvent::MetaChanged { name: name.to_owned() }])
    }

    /// Removes mods (`ModList::remove_mods`, Leet change 8): to `trash`, or for good with
    /// `None`; the list is saved.
    pub fn remove_mods(&mut self, names: &[String], trash: Option<&Trash>) -> Result<Vec<ModListEvent>> {
        let events = self.list.remove_mods(names, trash)?;
        for name in names {
            self.metas.forget(name);
        }
        self.after(events)
    }

    /// A mod's folder (for "Open in file manager").
    pub fn mod_path(&self, name: &str) -> Option<PathBuf> {
        if name == "Overwrite" {
            return Some(self.instance.paths.overwrite.clone());
        }
        self.list.get(name).and_then(|e| e.path.clone())
    }

    /// Sends the profile's layers to a live deploy of it (slow on a big instance; the shell
    /// calls it on a worker with a session of its own).
    pub fn refresh_deploy(&self) -> Result<crate::live::Outcome> {
        crate::live::refresh(&self.instance, &self.profile)
    }


    pub fn executables(&self) -> Vec<Executable> {
        self.instance.executables()
    }

    pub fn downloads(&self) -> Vec<DownloadRow> {
        downloads::rows(&self.instance.paths.downloads)
    }

    /// The profile's plugin list (scans the merged Data folder: on a worker).
    pub fn plugin_view(&self) -> Result<PluginView> {
        PluginView::build(&self.instance, &self.profile)
    }

    /// The saves, with the plugins each misses from `plugins`.
    pub fn saves(&self, plugins: Option<&PluginView>) -> Result<Vec<SaveRow>> {
        saves::rows(&self.instance, &self.profile, plugins.map(PluginView::list))
    }

    /// The Sort button (see [`plugins::sort`]).
    pub fn sort_plugins(&self) -> Result<String> {
        plugins::sort(&self.instance, &self.profile)
    }

    /// The Sort button with MO2's LOOT dialog (see [`plugins::sort_with_report`]).
    pub fn sort_plugins_report(&self, update: bool, observer: std::sync::Arc<dyn strata_loot::Observer>) -> Result<plugins::SortOutcome> {
        plugins::sort_with_report(&self.instance, &self.profile, update, observer)
    }

    /// Scans the merged tree for the Data tab (slow on big instances: not on the UI thread).
    pub fn data_view(&self) -> Result<DataView> {
        DataView::build(&self.instance, &self.profile)
    }
}

/// Copies `from`'s content into `to`, keeping files that exist there.
fn copy_into(from: &Path, to: &Path) -> Result<()> {
    let read = |path: &Path, source| crate::Error::Read { path: path.to_owned(), source };
    let write = |path: &Path, source| crate::Error::Write { path: path.to_owned(), source };
    std::fs::create_dir_all(to).map_err(|e| write(to, e))?;
    for entry in std::fs::read_dir(from).map_err(|e| read(from, e))? {
        let entry = entry.map_err(|e| read(from, e))?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type().map_err(|e| read(&source, e))?;
        if kind.is_dir() {
            copy_into(&source, &target)?;
        } else if target.symlink_metadata().is_ok() {
            continue;
        } else if kind.is_symlink() {
            let link = std::fs::read_link(&source).map_err(|e| read(&source, e))?;
            std::os::unix::fs::symlink(link, &target).map_err(|e| write(&target, e))?;
        } else {
            std::fs::copy(&source, &target).map_err(|e| write(&target, e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_instance::TestInstance;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn modlist(t: &TestInstance) -> String {
        std::fs::read_to_string(t.path("profiles/Default/modlist.txt")).unwrap().replace("\r\n", "|")
    }

    #[test]
    fn keys_of_the_mod_list_shift_toggle_and_set_levels() {
        let t = TestInstance::new("+C\r\n+S_separator\r\n-B\r\n+A\r\n");
        let mut s = Session::open(&t.root, None).unwrap();
        // ascending: A, B, S, C; Ctrl+Down on A
        s.shift_mods(&names(&["A"]), 1).unwrap();
        assert!(modlist(&t).ends_with("|+C|+S_separator|+A|-B|"), "{}", modlist(&t));
        // Space on A (on) and B (off)
        s.toggle_mods(&names(&["A", "B"])).unwrap();
        assert!(modlist(&t).ends_with("|+C|+S_separator|-A|+B|"), "{}", modlist(&t));
        // a level for the separator
        let events = s.set_nest("S_separator", 2).unwrap();
        assert_eq!(events, vec![ModListEvent::NestChanged { name: "S_separator".into(), nest: 2 }]);
        assert!(std::fs::read_to_string(t.path("mods/S_separator/meta.ini")).unwrap().contains("nest=2"));
        assert!(s.set_nest("A", 1).is_err(), "only separators have a level");
    }

    #[test]
    fn overwrite_goes_into_a_new_or_existing_mod_or_is_cleared() {
        let t = TestInstance::new("+A\r\n");
        let write = |rel: &str| {
            let p = t.path(&format!("overwrite/{rel}"));
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, rel).unwrap();
        };
        write("SKSE/Plugins/one.log");
        let mut s = Session::open(&t.root, None).unwrap();
        let events = s.move_overwrite("From: Overwrite", true).unwrap();
        assert!(events.contains(&ModListEvent::OverwriteChanged), "{events:?}");
        assert!(t.path("mods/From Overwrite/SKSE/Plugins/one.log").is_file());
        assert!(modlist(&t).contains("|-From Overwrite|"), "{}", modlist(&t));
        write("two.ini");
        s.move_overwrite("A", false).unwrap();
        assert!(t.path("mods/A/two.ini").is_file());
        assert!(s.move_overwrite("Overwrite", false).is_err());
        write("three.txt");
        s.clear_overwrite(false).unwrap();
        assert_eq!(std::fs::read_dir(t.path("overwrite")).unwrap().count(), 0);
    }

    #[test]
    fn the_overwrite_dialog_deletes_renames_and_makes_folders() {
        let t = TestInstance::new("+A\r\n");
        for rel in ["SKSE/Plugins/one.log", "Root/enbcache/c.bin", "loose.txt"] {
            let p = t.path(&format!("overwrite/{rel}"));
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, rel).unwrap();
        }
        let s = Session::open(&t.root, None).unwrap();
        assert_eq!(s.new_folder_in_overwrite("").unwrap(), "New Folder");
        assert_eq!(s.new_folder_in_overwrite("").unwrap(), "New Folder1");
        assert_eq!(s.new_folder_in_overwrite("SKSE").unwrap(), "SKSE/New Folder");
        s.rename_in_overwrite("loose.txt", "renamed.txt").unwrap();
        assert!(t.path("overwrite/renamed.txt").is_file());
        assert!(s.rename_in_overwrite("renamed.txt", "../out.txt").is_err());
        s.remove_from_overwrite(&names(&["SKSE/Plugins/one.log", "New Folder"]), false).unwrap();
        assert!(!t.path("overwrite/SKSE/Plugins/one.log").exists() && !t.path("overwrite/New Folder").exists());
        assert!(s.remove_from_overwrite(&names(&["Root"]), false).is_err(), "Root stays");
        assert!(s.remove_from_overwrite(&names(&["../mods"]), false).is_err());
        assert!(s.remove_from_overwrite(&names(&[""]), false).is_err());
        assert!(t.path("overwrite/Root/enbcache/c.bin").is_file());
    }

    #[test]
    fn an_empty_mod_is_made_above_a_mod_or_inside_a_separator() {
        let t = TestInstance::new("+C\r\n+S_separator\r\n+A\r\n");
        let mut s = Session::open(&t.root, None).unwrap();
        s.create_empty_mod("Above A", Some("A")).unwrap();
        s.create_empty_mod("In S", Some("S_separator")).unwrap();
        assert!(t.path("mods/Above A/meta.ini").is_file());
        // ascending: Above A, A, S, C, In S (S holds C: its end is after it)
        assert!(modlist(&t).ends_with("|-In S|+C|+S_separator|+A|-Above A|"), "{}", modlist(&t));
    }

    #[test]
    fn a_dropped_folder_becomes_a_new_mod_with_its_content() {
        let t = TestInstance::new("+A\r\n");
        let outside = t.path("outside/Some Folder");
        std::fs::create_dir_all(outside.join("textures")).unwrap();
        std::fs::write(outside.join("textures/x.dds"), "x").unwrap();
        std::fs::write(outside.join("meta.ini"), "[General]\nmodid=99\n").unwrap();
        let mut s = Session::open(&t.root, None).unwrap();
        let events = s.create_mod_from_folder("Copied: Mod", &outside).unwrap();
        assert!(matches!(events.as_slice(), [ModListEvent::Added { name, .. }] if name == "Copied Mod"), "{events:?}");
        assert_eq!(std::fs::read_to_string(t.path("mods/Copied Mod/textures/x.dds")).unwrap(), "x");
        let meta = std::fs::read_to_string(t.path("mods/Copied Mod/meta.ini")).unwrap();
        assert!(!meta.contains("modid=99"), "MO2's own meta.ini is kept: {meta}");
        assert!(modlist(&t).contains("|-Copied Mod|"), "{}", modlist(&t));
        assert!(s.create_mod_from_folder("Other", &t.path("outside/none")).is_err());
    }

    #[test]
    fn operations_go_through_the_core_and_land_in_modlist_txt() {
        let t = TestInstance::new("+C\r\n+S_separator\r\n-B\r\n+A\r\n");
        let mut s = Session::open(&t.root, None).unwrap();

        let events = s.set_enabled(&names(&["B"]), true).unwrap();
        assert_eq!(events, vec![ModListEvent::StateChanged { names: names(&["B"]), enabled: true }]);
        assert!(modlist(&t).contains("|+B|"));
        assert!(s.set_enabled(&names(&["B"]), true).unwrap().is_empty(), "no change, no event");

        // A (priority 0) to the end
        s.move_mods(&names(&["A"]), usize::MAX).unwrap();
        assert!(modlist(&t).ends_with("|+A|+C|+S_separator|+B|"), "{}", modlist(&t));

        // C onto the separator: at its end
        let events = s.drop_mods(&names(&["C"]), Some("S_separator"), None, false).unwrap();
        assert!(matches!(events.as_slice(), [ModListEvent::Moved { .. }]), "{events:?}");
        let rows = s.mods();
        let order: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(order, ["B", "S_separator", "A", "C", "Overwrite"], "onto the separator = at its end");
        assert_eq!(rows[2].parent, Some(1));
        assert_eq!(rows[3].parent, Some(1));

        // a drop past Overwrite is refused without an error
        assert!(s.drop_mods(&names(&["B"]), None, Some(4), false).unwrap().is_empty());
    }

    #[test]
    fn meta_edits_show_in_the_rows_and_removed_mods_go_to_the_trash() {
        let t = TestInstance::new("+A\r\n+S_separator\r\n");
        let mut s = Session::open(&t.root, None).unwrap();
        assert_eq!(s.mods()[0].color, None);
        let events = s.set_color("S_separator", Some(Color { r: 1, g: 2, b: 3, a: 255 })).unwrap();
        assert_eq!(events, vec![ModListEvent::MetaChanged { name: "S_separator".into() }]);
        assert_eq!(s.mods()[0].color, Some([1, 2, 3, 255]), "the cache forgot the old value");
        s.set_comments("A", "check later").unwrap();
        assert_eq!(s.mods()[1].comments, "check later");

        let trash = Trash::at(t.path("trash"));
        let events = s.remove_mods(&names(&["A"]), Some(&trash)).unwrap();
        assert!(matches!(events.as_slice(), [ModListEvent::Removed { .. }]), "{events:?}");
        assert!(!t.path("mods/A").exists());
        assert!(t.path("trash/files/A").exists());
        assert!(!modlist(&t).contains("A|"));
        assert_eq!(s.mods().len(), 2);
    }

    #[test]
    fn the_filter_panel_lists_the_specials_contents_and_used_categories() {
        let t = TestInstance::new("+A\r\n+B\r\n");
        t.meta("A", "[General]\r\ncategory=\"2,\"\r\n");
        let mut s = Session::open(&t.root, None).unwrap();
        let names: Vec<String> = s.filter_entries().into_iter().map(|e| e.name).collect();
        assert_eq!(names[0], "<Active>");
        assert!(names.contains(&"<Contains Textures>".to_owned()));
        assert_eq!(names.last().map(String::as_str), Some("Buildings"));
        assert!(!names.contains(&"Skyrim Special Edition".to_owned()), "a category no mod has");
    }

    #[test]
    fn backups_are_made_listed_and_restored() {
        let t = TestInstance::new("+A\r\n+B\r\n");
        let mut s = Session::open(&t.root, None).unwrap();
        let name = s.backup_mod_list().unwrap();
        assert!(name.starts_with("modlist.txt."));
        let suffix = s.backups(true).unwrap()[0].clone();
        s.set_enabled(&names(&["A"]), false).unwrap();
        assert!(modlist(&t).contains("-A"));
        s.restore_backup(true, &suffix).unwrap();
        assert!(modlist(&t).contains("+A"));
        assert!(s.mods().iter().find(|r| r.name == "A").unwrap().enabled, "the list was read again");
        assert_eq!(s.backup_plugins().unwrap(), 0, "no plugin files yet");
    }

    #[test]
    fn separators_are_created_and_mods_renamed() {
        let t = TestInstance::new("+A\r\n");
        let mut s = Session::open(&t.root, None).unwrap();
        s.create_separator("Tools", None).unwrap();
        assert!(t.path("mods/Tools_separator/meta.ini").exists());
        s.rename_mod("A", "A2").unwrap();
        assert!(t.path("mods/A2").is_dir());
        assert!(modlist(&t).contains("+A2|"));
        let rows = s.mods();
        assert!(rows.iter().any(|r| r.display_name == "Tools"));
        assert!(rows.iter().any(|r| r.name == "A2"));
    }
}
