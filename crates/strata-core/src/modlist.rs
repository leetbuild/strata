//! The mod list of a profile: the mods on disk, ordered and switched by `modlist.txt`.
//!
//! [`ModList`] mirrors MO2's `Profile` mod status (`m_ModStatus`, `m_ModIndexByPriority`):
//! loading follows `Profile::refreshModStatus`, the operations follow `Profile::setModEnabled`,
//! `setModsEnabled`, `setModPriority` and `ModList::changeModPriority`, and saving follows
//! `Profile::doWriteModlist`. Each operation returns the [`ModListEvent`]s it caused.

use std::path::{Path, PathBuf};

use crate::events::ModListEvent;
use crate::separators::{DropPlan, SeparatorTree, enclosing_separator_nest, priority_after_separator, priority_for_mod_in_separator};
use crate::write::Transaction;
use crate::{Error, Instance, ListedState, ModListFile, ModMeta, Profile, Result, new_mod_meta};

/// What kind of entry a mod is (`ModInfo::EModType` and friends).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModKind {
    Regular,
    /// A folder named `<Name>_separator`: a heading in the list, never a layer.
    Separator,
    /// A plugin in the game's own Data folder that is not a primary plugin
    /// (`GamebryoUnmangedMods`), named `Unmanaged: <plugin without extension>`. No folder.
    /// Always enabled.
    Foreign,
    /// A folder named `...backup<n>`: kept after an install replaced a mod. Never enabled,
    /// never moved.
    Backup,
}

/// One mod of a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModEntry {
    pub name: String,
    pub kind: ModKind,
    pub enabled: bool,
    /// 0 is the lowest priority, as in MO2's left pane.
    pub priority: usize,
    /// The mod's folder; `None` for foreign mods.
    pub path: Option<PathBuf>,
    /// False for a mod that `modlist.txt` doesn't name (MO2 adds it on top, disabled).
    pub listed: bool,
    /// A separator's level (`Sub-Separator\nest` in its `meta.ini`); 0 for other mods.
    pub nest: u32,
}

/// `ModInfo::isSeparatorName`: the name ends in `_separator`.
pub fn is_separator_name(name: &str) -> bool {
    name.ends_with("_separator")
}

/// `ModInfo::isBackupName`: the name ends in `backup` and optional digits.
pub fn is_backup_name(name: &str) -> bool {
    name.trim_end_matches(|c: char| c.is_ascii_digit()).ends_with("backup")
}

/// `MOBase::fixDirectoryName`: what a name typed by the user becomes as a folder name, or
/// `None` when nothing usable is left.
pub fn fix_directory_name(name: &str) -> Option<String> {
    let simplified = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut temp = simplified(name);
    while temp.ends_with('.') {
        temp.pop();
    }
    temp.retain(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'));
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&temp.as_str()) {
        temp.clear();
    }
    let temp = simplified(&temp);
    (!temp.is_empty()).then_some(temp)
}

fn kind_of(name: &str) -> ModKind {
    // ModInfo::createFrom checks backups first.
    if is_backup_name(name) {
        ModKind::Backup
    } else if is_separator_name(name) {
        ModKind::Separator
    } else {
        ModKind::Regular
    }
}

/// Whether a Strata FUSE mount sits at `path` (`/proc/self/mountinfo`, type `fuse.strata`).
fn is_strata_mount(path: &Path) -> bool {
    let wanted = path.to_string_lossy();
    let Ok(text) = std::fs::read_to_string("/proc/self/mountinfo") else { return false };
    text.lines().any(|line| {
        let mountpoint = line.split(' ').nth(4).unwrap_or("");
        let mountpoint = mountpoint.replace("\\040", " ").replace("\\011", "\t").replace("\\012", "\n").replace("\\134", "\\");
        mountpoint == wanted && line.split(" - ").nth(1).is_some_and(|rest| rest.starts_with("fuse.strata "))
    })
}

/// Names compare as MO2's `FileNameComparator` does: without case.
fn same_name(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

/// A profile's mods in priority order, editable.
#[derive(Debug, Clone)]
pub struct ModList {
    /// Lowest priority first; `entries[i].priority == i`.
    entries: Vec<ModEntry>,
    /// The first `regular` entries can be moved and switched; the backups come after them.
    regular: usize,
    /// Where `modlist.txt` lives.
    file: PathBuf,
}

impl Instance {
    /// The foreign mods of the instance (`GamebryoUnmangedMods::mods`): every `.esp`,
    /// `.esm` and `.esl` in the real game Data folder that is not a primary plugin, unless
    /// `[Settings] display_foreign` is off (then only official DLC, which for Skyrim SE
    /// are all primary). Sorted like `ModInfo::ByName`.
    ///
    /// While the instance is deployed, `<game>/Data` is Strata's own mount and lists every
    /// mod's plugins; then the names come from the snapshot taken just before mounting
    /// ([`Instance::snapshot_game_plugins`]), and without one there are none.
    pub fn foreign_mods(&self) -> Vec<String> {
        let display = self.ini.value("Settings", "display_foreign").is_none_or(|v| v.to_bool());
        if !display {
            return Vec::new();
        }
        let files = if is_strata_mount(&self.game_path) {
            std::fs::read_to_string(self.game_plugins_snapshot())
                .map(|text| text.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect())
                .unwrap_or_default()
        } else {
            self.real_game_plugins()
        };
        let primary = self.primary_plugins();
        let mut names: Vec<String> = files
            .into_iter()
            .filter(|n| !primary.iter().any(|p| p.eq_ignore_ascii_case(n)))
            .map(|n| format!("Unmanaged: {}", &n[..n.len() - 4]))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup_by(|a, b| same_name(a, b));
        names
    }

    /// The plugin files in the game's Data folder on disk (wrong while it is mounted).
    fn real_game_plugins(&self) -> Vec<String> {
        let data = self.game_path.join(self.game.data_dir());
        let Ok(entries) = std::fs::read_dir(&data) else { return Vec::new() };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| {
                let lower = n.to_lowercase();
                [".esp", ".esm", ".esl"].iter().any(|ext| lower.ends_with(ext))
            })
            .collect();
        names.sort();
        names
    }

    fn game_plugins_snapshot(&self) -> PathBuf {
        self.strata_dir().join("game-plugins.txt")
    }

    /// Records the game's own plugin files for [`Instance::foreign_mods`] while mounted.
    /// Called by whoever mounts, just before; does nothing when the folder is already a
    /// Strata mount (the snapshot from that mount stays).
    pub fn snapshot_game_plugins(&self) -> Result<()> {
        if is_strata_mount(&self.game_path) {
            return Ok(());
        }
        let dir = self.strata_dir();
        std::fs::create_dir_all(&dir).map_err(|source| Error::Write { path: dir.clone(), source })?;
        let mut text = self.real_game_plugins().join("\n");
        text.push('\n');
        crate::write::write_if_changed(&self.game_plugins_snapshot(), text.as_bytes()).map(|_| ())
    }

    /// The profile's mods in priority order (lowest first). See [`ModList::load`].
    pub fn mod_list(&self, profile: &Profile) -> Result<Vec<ModEntry>> {
        Ok(ModList::load(self, profile)?.entries)
    }
}

impl ModList {
    /// Orders the mods as `Profile::refreshModStatus` does:
    ///
    /// 1. foreign mods the list doesn't name, in reverse name order (each is inserted
    ///    below all others),
    /// 2. the mods named in `modlist.txt`, bottom line first,
    /// 3. folders the list doesn't name, by name, disabled,
    /// 4. backups, by name.
    ///
    /// Names in the list without a mod are dropped (MO2 logs "mod not found"), as are
    /// listed backups. Names compare without case; when a name appears twice in another
    /// case, the first line gives the priority and the last one the state, as in MO2.
    /// Foreign mods are always enabled.
    pub fn load(instance: &Instance, profile: &Profile) -> Result<Self> {
        let dir = &instance.paths.mods;
        let entries = std::fs::read_dir(dir).map_err(|source| Error::Read { path: dir.clone(), source })?;
        // Every mod MO2 knows, sorted like ModInfo::ByName: (lower-case name, name, kind).
        let mut known: Vec<(String, String, ModKind)> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .map(|name| (name.to_lowercase(), kind_of(&name), name))
            .map(|(key, kind, name)| (key, name, kind))
            .collect();
        known.extend(instance.foreign_mods().into_iter().map(|n| (n.to_lowercase(), n, ModKind::Foreign)));
        known.sort();

        let find = |name: &str| {
            let key = name.to_lowercase();
            known.binary_search_by(|(k, _, _)| k.as_str().cmp(&key)).ok()
        };

        // (index into `known`, enabled), in list order (highest priority first).
        let mut listed: Vec<(usize, bool)> = Vec::new();
        for line in &profile.modlist.mods {
            let Some(i) = find(&line.name) else { continue };
            if known[i].2 == ModKind::Backup {
                continue;
            }
            let enabled = line.state != ListedState::Disabled;
            match listed.iter_mut().find(|(j, _)| *j == i) {
                Some(seen) => seen.1 = enabled,
                None => listed.push((i, enabled)),
            }
        }

        let entry = |i: usize, enabled: bool, listed: bool| {
            let (_, name, kind) = &known[i];
            let path = (*kind != ModKind::Foreign).then(|| dir.join(name));
            let nest = match (&path, kind) {
                (Some(path), ModKind::Separator) => ModMeta::open(path).map(|m| m.nest()).unwrap_or(0),
                _ => 0,
            };
            ModEntry {
                name: name.clone(),
                kind: *kind,
                enabled: (enabled || *kind == ModKind::Foreign) && *kind != ModKind::Backup,
                priority: 0,
                path,
                listed,
                nest,
            }
        };
        let is_listed = |i: usize| listed.iter().any(|(j, _)| *j == i);

        let mut result: Vec<ModEntry> = Vec::new();
        // Unlisted foreign mods: each one goes below everything before it.
        for i in (0..known.len()).rev().filter(|i| known[*i].2 == ModKind::Foreign && !is_listed(*i)) {
            result.push(entry(i, true, false));
        }
        for &(i, enabled) in listed.iter().rev() {
            result.push(entry(i, enabled, true));
        }
        for i in (0..known.len()).filter(|i| !matches!(known[*i].2, ModKind::Foreign | ModKind::Backup) && !is_listed(*i)) {
            result.push(entry(i, false, false));
        }
        let regular = result.len();
        for i in (0..known.len()).filter(|i| known[*i].2 == ModKind::Backup) {
            result.push(entry(i, false, false));
        }
        for (priority, e) in result.iter_mut().enumerate() {
            e.priority = priority;
        }
        Ok(Self { entries: result, regular, file: profile.dir.join("modlist.txt") })
    }

    /// Every mod, lowest priority first.
    pub fn entries(&self) -> &[ModEntry] {
        &self.entries
    }

    /// How many mods can be moved (`Profile::m_NumRegularMods`): all but the backups.
    pub fn regular_count(&self) -> usize {
        self.regular
    }

    /// The mod's priority, by name (any case).
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name).or_else(|| self.entries.iter().position(|e| same_name(&e.name, name)))
    }

    /// Like [`ModList::index_of`] for many names, failing on the first unknown one.
    pub fn indices_of(&self, names: &[impl AsRef<str>]) -> Result<Vec<usize>> {
        names
            .iter()
            .map(|n| self.index_of(n.as_ref()).ok_or_else(|| Error::NoSuchMod { name: n.as_ref().to_owned() }))
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<&ModEntry> {
        self.index_of(name).map(|i| &self.entries[i])
    }

    /// `Profile::setModsEnabled`: foreign mods stay enabled, backups stay disabled; mods
    /// already in the wanted state are left alone. Separators can be switched (the window
    /// has no checkbox for them, but "enable all" switches them, so lists hold `+` and `-`
    /// separators). Returns one event naming the mods that changed, or none.
    pub fn set_enabled(&mut self, names: &[impl AsRef<str>], enabled: bool) -> Result<Vec<ModListEvent>> {
        let indices = self.indices_of(names)?;
        let mut changed = Vec::new();
        for i in indices {
            let e = &mut self.entries[i];
            let fixed = (e.kind == ModKind::Foreign && !enabled) || (e.kind == ModKind::Backup && enabled);
            if !fixed && e.enabled != enabled {
                e.enabled = enabled;
                changed.push(e.name.clone());
            }
        }
        Ok(if changed.is_empty() { Vec::new() } else { vec![ModListEvent::StateChanged { names: changed, enabled }] })
    }

    /// `Profile::setModPriority`: moves one mod to `priority` (clamped to the regular
    /// mods), shifting the mods in between. Backups don't move. Returns the move, if any.
    pub fn set_priority(&mut self, name: &str, priority: usize) -> Result<Vec<ModListEvent>> {
        let from = self.index_of(name).ok_or_else(|| Error::NoSuchMod { name: name.to_owned() })?;
        Ok(self.move_index(from, priority).into_iter().collect())
    }

    fn move_index(&mut self, from: usize, priority: usize) -> Option<ModListEvent> {
        if from >= self.regular || self.regular == 0 {
            return None;
        }
        let to = priority.min(self.regular - 1);
        if to == from {
            return None;
        }
        let entry = self.entries.remove(from);
        self.entries.insert(to, entry);
        let (low, high) = (from.min(to), from.max(to));
        for (i, e) in self.entries.iter_mut().enumerate().take(high + 1).skip(low) {
            e.priority = i;
        }
        Some(ModListEvent::Moved { name: self.entries[to].name.clone(), from, to })
    }

    /// `ModList::changeModPriority` for a drop of several mods at `priority` (the priority
    /// of the row they are dropped on; `usize::MAX` = the end): the mods that go down move
    /// first, highest first, each to `priority`; then, if any goes up, the target becomes
    /// one less and those move, lowest first. With one mod this is
    /// [`ModList::set_priority`]: the mod gets exactly `priority` ("Send to priority",
    /// `strata move --to`). A drop wants [`ModList::drop_mods_at`].
    pub fn move_mods(&mut self, names: &[impl AsRef<str>], priority: usize) -> Result<Vec<ModListEvent>> {
        let indices = self.indices_of(names)?;
        if let [one] = indices.as_slice() {
            let name = self.entries[*one].name.clone();
            return self.set_priority(&name, priority);
        }
        self.drop_mods_at(names, priority)
    }

    /// A drop at the row of `priority` (`usize::MAX` = the end): [`ModList::move_mods`]'s
    /// rule for one mod as well, as MO2's `changeModsPriority` does: a mod going up lands just
    /// before that row, not after it (one mod dropped further down the list, or "Send to
    /// separator" from above it, went one row too far).
    pub fn drop_mods_at(&mut self, names: &[impl AsRef<str>], priority: usize) -> Result<Vec<ModListEvent>> {
        let indices = self.indices_of(names)?;
        let mut moving: Vec<String> = Vec::new();
        for i in indices {
            if !moving.contains(&self.entries[i].name) {
                moving.push(self.entries[i].name.clone());
            }
        }
        let mut events = Vec::new();
        let mut target = priority;
        let position = |list: &Self, name: &str| list.entries.iter().position(|e| e.name == name).unwrap_or(0);

        moving.sort_by_key(|n| std::cmp::Reverse(position(self, n)));
        for name in &moving {
            let from = position(self, name);
            if from > target {
                events.extend(self.move_index(from, target));
            }
        }
        moving.sort_by_key(|n| position(self, n));
        if moving.iter().any(|n| position(self, n) < target) {
            target -= 1;
        }
        for name in &moving {
            let from = position(self, name);
            if from < target {
                events.extend(self.move_index(from, target));
            }
        }
        Ok(events)
    }

    /// `ModList::shiftModsPriority`: moves each mod by `offset` (the "move up / down"
    /// actions), the ones nearest the target end first so they don't pass each other.
    pub fn shift_mods(&mut self, names: &[impl AsRef<str>], offset: isize) -> Result<Vec<ModListEvent>> {
        let indices = self.indices_of(names)?;
        let mut moving: Vec<String> = indices.iter().map(|i| self.entries[*i].name.clone()).collect();
        let position = |list: &Self, name: &str| list.entries.iter().position(|e| e.name == name).unwrap_or(0);
        moving.sort_by_key(|n| position(self, n));
        if offset > 0 {
            moving.reverse();
        }
        let mut events = Vec::new();
        for name in &moving {
            let from = position(self, name);
            let to = from.saturating_add_signed(offset);
            events.extend(self.move_index(from, to));
        }
        Ok(events)
    }

    /// Sets a separator's level (`SubSeparator::setNest`): its `meta.ini` is staged in
    /// `txn`. Returns the event, if the level changed.
    pub fn set_nest(&mut self, name: &str, nest: u32, txn: &mut Transaction) -> Result<Vec<ModListEvent>> {
        let i = self.index_of(name).ok_or_else(|| Error::NoSuchMod { name: name.to_owned() })?;
        let entry = &mut self.entries[i];
        if entry.kind != ModKind::Separator {
            return Err(Error::Refused { what: format!("\"{}\" is not a separator; only separators have a level", entry.name) });
        }
        let Some(path) = &entry.path else { return Ok(Vec::new()) };
        let mut meta = ModMeta::open(path)?;
        entry.nest = nest;
        if !meta.set_nest(nest) {
            return Ok(Vec::new());
        }
        txn.write(meta.path(), &meta.to_bytes())?;
        Ok(vec![ModListEvent::NestChanged { name: entry.name.clone(), nest }])
    }

    /// The separator tree of the list as it is now.
    pub fn tree(&self) -> SeparatorTree {
        SeparatorTree::build(&self.entries)
    }

    /// Carries out a drop planned by [`SeparatorTree::plan_drop`]: the dropped separators'
    /// levels first (staged in `txn`), then the move, as `dropMimeData` does.
    pub fn apply_drop(&mut self, plan: &DropPlan, dropped: &[usize], txn: &mut Transaction) -> Result<Vec<ModListEvent>> {
        let names: Vec<String> = dropped.iter().filter_map(|p| self.entries.get(*p)).map(|e| e.name.clone()).collect();
        let mut events = Vec::new();
        for (p, nest) in &plan.nests {
            let name = self.entries[*p].name.clone();
            events.extend(self.set_nest(&name, *nest, txn)?);
        }
        events.extend(self.drop_mods_at(&names, plan.priority)?);
        Ok(events)
    }

    /// Adds a mod that was just created on disk where `Profile::refreshModStatus` puts a
    /// mod the list doesn't know: on top of the regular mods, disabled.
    fn push_new(&mut self, name: &str, kind: ModKind, path: PathBuf) -> ModListEvent {
        let at = self.regular;
        self.entries.insert(at, ModEntry { name: name.to_owned(), kind, enabled: false, priority: at, path: Some(path), listed: false, nest: 0 });
        self.regular += 1;
        for (i, e) in self.entries.iter_mut().enumerate().skip(at) {
            e.priority = i;
        }
        ModListEvent::Added { name: name.to_owned(), priority: at }
    }

    /// Creates an empty mod (`ModListViewActions::createEmptyMod` + `OrganizerCore::createMod`):
    /// the folder and MO2's initial `meta.ini`, then on top of the list. With `at`, it is
    /// moved to that mod's priority (just above it on screen), or for a separator after its
    /// own mods (`priority_for_mod_in_separator`; MO2's `findInstallPriority` went to its end). The caller saves the list.
    pub fn create_mod(&mut self, mods_dir: &Path, name: &str, at: Option<&str>) -> Result<Vec<ModListEvent>> {
        let fixed = fix_directory_name(name).ok_or_else(|| Error::InvalidName { name: name.to_owned() })?;
        let priority = match at {
            Some(target) => {
                let p = self.index_of(target).ok_or_else(|| Error::NoSuchMod { name: target.to_owned() })?;
                Some(if self.entries[p].kind == ModKind::Separator {
                    priority_for_mod_in_separator(&self.entries, p).unwrap_or(usize::MAX)
                } else {
                    p
                })
            }
            None => None,
        };
        let mut events = vec![self.make_folder(mods_dir, &fixed, kind_of(&fixed))?];
        if let Some(p) = priority {
            events.extend(self.set_priority(&fixed, p)?);
        }
        Ok(events)
    }

    /// Creates a separator (`ModListViewActions::createSeparator`), `name` without the
    /// `_separator` suffix. With `at` a separator: at its end, one level deeper. With `at`
    /// a mod: just above it, at the level of the separator the mod is in (splitting it).
    /// Without `at`: at the end of the list, level 0. The new `meta.ini` holds the level
    /// (written directly: the folder is new). The caller saves the list.
    pub fn create_separator(&mut self, mods_dir: &Path, name: &str, at: Option<&str>) -> Result<Vec<ModListEvent>> {
        let fixed = fix_directory_name(name).ok_or_else(|| Error::InvalidName { name: name.to_owned() })?;
        let full = format!("{fixed}_separator");
        let (priority, nest) = match at {
            Some(target) => {
                let p = self.index_of(target).ok_or_else(|| Error::NoSuchMod { name: target.to_owned() })?;
                if self.entries[p].kind == ModKind::Separator {
                    (priority_after_separator(&self.entries, p), self.entries[p].nest + 1)
                } else {
                    (Some(p), enclosing_separator_nest(&self.entries, p))
                }
            }
            None => (None, 0),
        };
        let mut events = vec![self.make_folder(mods_dir, &full, ModKind::Separator)?];
        if nest > 0 {
            let i = self.index_of(&full).unwrap_or(0);
            let path = mods_dir.join(&full);
            let mut meta = ModMeta::open(&path)?;
            meta.set_nest(nest);
            meta.save()?;
            self.entries[i].nest = nest;
        }
        if let Some(p) = priority {
            events.extend(self.set_priority(&full, p)?);
        }
        Ok(events)
    }

    fn make_folder(&mut self, mods_dir: &Path, name: &str, kind: ModKind) -> Result<ModListEvent> {
        if self.index_of(name).is_some() {
            return Err(Error::Exists { path: mods_dir.join(name) });
        }
        let path = mods_dir.join(name);
        std::fs::create_dir(&path).map_err(|source| match source.kind() {
            std::io::ErrorKind::AlreadyExists => Error::Exists { path: path.clone() },
            _ => Error::Write { path: path.clone(), source },
        })?;
        crate::write::write_if_changed(&path.join("meta.ini"), new_mod_meta())?;
        Ok(self.push_new(name, kind, path))
    }

    /// Removes mods (`ModList::removeRowForce` → `ModInfo::removeMod`): each folder goes to
    /// `trash`, or is deleted for good with `None` (Leet change 8), and the mod leaves the
    /// list. Foreign mods have no folder and can't be removed. Other profiles' lists drop
    /// the name the next time they are read, as in MO2. The caller saves the list.
    pub fn remove_mods(&mut self, names: &[impl AsRef<str>], trash: Option<&crate::Trash>) -> Result<Vec<ModListEvent>> {
        let indices = self.indices_of(names)?;
        let mut doomed: Vec<String> = Vec::new();
        for i in indices {
            let e = &self.entries[i];
            if e.kind == ModKind::Foreign || e.path.is_none() {
                return Err(Error::Refused { what: format!("\"{}\" belongs to the game and can't be removed", e.name) });
            }
            if !doomed.contains(&e.name) {
                doomed.push(e.name.clone());
            }
        }
        let mut events = Vec::new();
        for name in doomed {
            let Some(i) = self.entries.iter().position(|e| e.name == name) else { continue };
            if let Some(path) = &self.entries[i].path {
                crate::trash::remove(path, trash)?;
            }
            self.entries.remove(i);
            if i < self.regular {
                self.regular -= 1;
            }
            for (p, e) in self.entries.iter_mut().enumerate().skip(i) {
                e.priority = p;
            }
            events.push(ModListEvent::Removed { name, priority: i });
        }
        Ok(events)
    }

    /// Renames a mod (`ModList::renameMod`, `ModInfoRegular::setName`,
    /// `Profile::renameModInAllProfiles`): the folder, and the mod's line in every
    /// profile's `modlist.txt`, in one transaction. A separator keeps its suffix
    /// (`makeInternalName`). Foreign mods and backups can't be renamed.
    pub fn rename_mod(&mut self, instance: &Instance, old: &str, new: &str) -> Result<Vec<ModListEvent>> {
        let i = self.index_of(old).ok_or_else(|| Error::NoSuchMod { name: old.to_owned() })?;
        let entry = self.entries[i].clone();
        let Some(from) = entry.path.clone().filter(|_| !matches!(entry.kind, ModKind::Foreign | ModKind::Backup)) else {
            return Err(Error::Refused { what: format!("\"{}\" is not a mod folder and can't be renamed", entry.name) });
        };
        let mut fixed = fix_directory_name(new).ok_or_else(|| Error::InvalidName { name: new.to_owned() })?;
        if entry.kind == ModKind::Separator && !is_separator_name(&fixed) {
            fixed.push_str("_separator");
        }
        if fixed == entry.name {
            return Ok(Vec::new());
        }
        if let Some(other) = self.index_of(&fixed)
            && other != i
        {
            return Err(Error::Exists { path: instance.paths.mods.join(&fixed) });
        }
        let mut txn = Transaction::new(&instance.strata_dir())?;
        txn.rename(&from, &instance.paths.mods.join(&fixed))?;
        for profile in instance.profile_names()? {
            let file = instance.paths.profiles.join(&profile).join("modlist.txt");
            if let Ok(data) = std::fs::read(&file)
                && let Some(renamed) = rename_in_list(&data, &entry.name, &fixed)
            {
                txn.write(&file, &renamed)?;
            }
        }
        txn.commit()?;
        let e = &mut self.entries[i];
        e.name.clone_from(&fixed);
        e.path = Some(instance.paths.mods.join(&fixed));
        Ok(vec![ModListEvent::Renamed { from: entry.name, to: fixed }])
    }

    /// `Profile::doWriteModlist`: the header, then every mod but the backups from the
    /// highest priority down, `*` for foreign mods, `+`/`-` for the others.
    pub fn to_file(&self) -> ModListFile {
        let mods = self.entries[..self.regular]
            .iter()
            .rev()
            .map(|e| crate::ListedMod {
                name: e.name.clone(),
                state: match (e.kind, e.enabled) {
                    (ModKind::Foreign, _) => ListedState::Foreign,
                    (_, true) => ListedState::Enabled,
                    (_, false) => ListedState::Disabled,
                },
            })
            .collect();
        ModListFile { mods }
    }

    /// `modlist.txt`'s path.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Writes `modlist.txt` if its content changed (atomically). Returns whether it was
    /// written. Every mod is then listed, so `listed` becomes true for all.
    pub fn save(&mut self) -> Result<bool> {
        let written = crate::write::write_if_changed(&self.file, &self.to_file().to_bytes())?;
        self.mark_listed();
        Ok(written)
    }

    /// Stages `modlist.txt` in a transaction (with other files that change with it).
    pub fn stage(&mut self, txn: &mut Transaction) -> Result<()> {
        txn.write(&self.file, &self.to_file().to_bytes())?;
        self.mark_listed();
        Ok(())
    }

    fn mark_listed(&mut self) {
        for e in &mut self.entries[..self.regular] {
            e.listed = true;
        }
    }
}

/// `Profile::renameModInList`: the list with every line naming `old` (exactly) renamed to
/// `new`, or `None` when no line does (the file is then not written). Like MO2, it drops
/// lines without a name, keeps `#` lines, and writes `\r\n` line ends.
pub fn rename_in_list(data: &[u8], old: &str, new: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut renamed = false;
    for line in data.split_inclusive(|b| *b == b'\n') {
        if line.first() == Some(&b'#') {
            out.extend_from_slice(line);
            continue;
        }
        let text = String::from_utf8_lossy(line);
        let mut chars = text.chars();
        let Some(spec) = chars.next() else { continue };
        let name = chars.as_str().trim();
        if name.is_empty() {
            continue;
        }
        let name = if name == old {
            renamed = true;
            new
        } else {
            name
        };
        out.extend_from_slice(spec.to_string().as_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    renamed.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_classified_as_mo2_does() {
        assert!(is_separator_name("Tools_separator"));
        assert!(!is_separator_name("separator"));
        assert!(is_backup_name("SkyUI backup"));
        assert!(is_backup_name("SkyUIbackup12"));
        assert!(!is_backup_name("backup of SkyUI"));
        assert_eq!(kind_of("Old backup_separator"), ModKind::Separator);
        assert_eq!(kind_of("Tools_separator backup"), ModKind::Backup);
    }

    #[test]
    fn directory_names_are_fixed_as_mo2_does() {
        assert_eq!(fix_directory_name("  My   Mod.. ").as_deref(), Some("My Mod"));
        assert_eq!(fix_directory_name("a<b>:c\"d/e\\f|g?h*").as_deref(), Some("abcdefgh"));
        assert_eq!(fix_directory_name("CON"), None);
        assert_eq!(fix_directory_name(" ... "), None);
        assert_eq!(fix_directory_name("x : y").as_deref(), Some("x y"));
    }

    fn list(names: &[(&str, ModKind)]) -> ModList {
        let entries: Vec<ModEntry> = names
            .iter()
            .enumerate()
            .map(|(i, (name, kind))| ModEntry {
                name: (*name).to_owned(),
                kind: *kind,
                enabled: false,
                priority: i,
                path: None,
                listed: true,
                nest: 0,
            })
            .collect();
        let regular = entries.iter().filter(|e| e.kind != ModKind::Backup).count();
        ModList { entries, regular, file: PathBuf::new() }
    }

    fn order(list: &ModList) -> String {
        list.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(" ")
    }

    fn mods(names: &str) -> ModList {
        let mut items: Vec<(&str, ModKind)> = names.split(' ').map(|n| (n, ModKind::Regular)).collect();
        items.push(("Z backup", ModKind::Backup));
        list(&items)
    }

    #[test]
    fn set_priority_shifts_the_mods_in_between_and_clamps() {
        let mut l = mods("A B C D E");
        let events = l.set_priority("B", 3).unwrap();
        assert_eq!(order(&l), "A C D B E Z backup");
        assert_eq!(events, [ModListEvent::Moved { name: "B".into(), from: 1, to: 3 }]);
        l.set_priority("e", 0).unwrap();
        assert_eq!(order(&l), "E A C D B Z backup");
        // past the regular mods: clamped below the backups
        l.set_priority("E", 99).unwrap();
        assert_eq!(order(&l), "A C D B E Z backup");
        assert!(l.set_priority("Z backup", 0).unwrap().is_empty(), "backups never move");
        assert!(l.entries.iter().enumerate().all(|(i, e)| e.priority == i));
    }

    /// `ModList::changeModPriority`, worked by hand from the MO2 source.
    #[test]
    fn moving_several_mods_follows_change_mod_priority() {
        // A drop "on" D (priority 3) of B and F: F goes down first to 3, then B goes up
        // to 3 - 1 = 2.
        let mut l = mods("A B C D E F");
        l.move_mods(&["B", "F"], 3).unwrap();
        assert_eq!(order(&l), "A C B F D E Z backup");
        // all going up: the target is one less
        let mut l = mods("A B C D E F");
        l.move_mods(&["A", "B"], 4).unwrap();
        assert_eq!(order(&l), "C D A B E F Z backup");
        // all going down
        let mut l = mods("A B C D E F");
        l.move_mods(&["E", "F"], 1).unwrap();
        assert_eq!(order(&l), "A E F B C D Z backup");
        // to the end
        let mut l = mods("A B C D E F");
        l.move_mods(&["A", "C"], usize::MAX).unwrap();
        assert_eq!(order(&l), "B D E F A C Z backup");
    }

    #[test]
    fn shifting_moves_each_mod_by_the_offset() {
        let mut l = mods("A B C D");
        l.shift_mods(&["B", "C"], 1).unwrap();
        assert_eq!(order(&l), "A D B C Z backup");
        l.shift_mods(&["A", "B"], -1).unwrap();
        assert_eq!(order(&l), "A B D C Z backup");
    }

    #[test]
    fn foreign_mods_stay_enabled_and_backups_disabled() {
        let mut l = list(&[("Unmanaged: X", ModKind::Foreign), ("A", ModKind::Regular), ("A backup", ModKind::Backup)]);
        l.entries[0].enabled = true;
        assert!(l.set_enabled(&["Unmanaged: X"], false).unwrap().is_empty());
        assert!(l.set_enabled(&["A backup"], true).unwrap().is_empty());
        let events = l.set_enabled(&["a", "A"], true).unwrap();
        assert_eq!(events, [ModListEvent::StateChanged { names: vec!["A".into()], enabled: true }]);
        assert!(matches!(l.set_enabled(&["nope"], true), Err(Error::NoSuchMod { .. })));
    }

    #[test]
    fn renaming_in_a_list_follows_rename_mod_in_list() {
        let data = b"# header\r\n+Old\r\n-Other\r\n\r\n*Old\r\n+old\r\n";
        assert_eq!(rename_in_list(data, "Old", "New").unwrap(), b"# header\r\n+New\r\n-Other\r\n*New\r\n+old\r\n");
        assert_eq!(rename_in_list(data, "Gone", "New"), None);
    }

    #[test]
    fn the_file_lists_the_highest_priority_first_without_backups() {
        let mut l = list(&[("Unmanaged: X", ModKind::Foreign), ("A", ModKind::Regular), ("S_separator", ModKind::Separator), ("A backup", ModKind::Backup)]);
        l.entries[1].enabled = true;
        assert_eq!(
            l.to_file().to_bytes(),
            b"# This file was automatically generated by Mod Organizer.\r\n-S_separator\r\n+A\r\n*Unmanaged: X\r\n"
        );
    }
}
