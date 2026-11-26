//! The conflict index: for every file, all the layers that provide it, and per layer the conflict
//! flags MO2 shows in the mod list.
//!
//! MO2 keeps two structures (Leet change 5): the Data structure (`DirectoryRefresher::refresh`:
//! the game's Data folder as origin `data`, then every enabled mod and overwrite by priority) and
//! the Root structure (`OrganizerCore::rebuildRootStructure`: only the enabled mods' `Root`
//! folders; the game folder is not indexed and overwrite is skipped). Both drop the top-level
//! `meta.ini`, `readme.txt` and `fomod/` (`DirectoryRefresher::cleanStructure`). Unlike the
//! overlay, MO2's structures keep `*.mohidden` files and `.git` folders, so the index does too.
//!
//! With MO2's archive parsing (`[Settings] archive_parsing_experimental`, see [`ArchiveRules`]),
//! the files inside the mods' BSA/BA2 archives join the Data structure too
//! (`DirectoryEntry::addFromAllBSAs`): a loose file always beats an archived one, archives among
//! themselves go by their plugin's load order.
//!
//! The flags follow `ModInfoWithConflictInfo::doConflictCheck`, quirks included (see
//! [`ConflictIndex::flags`]). The index is built from its own scan, apart from the merged tree,
//! so it never slows a mount. A changed layer list ([`ConflictIndex::update`]) rescans only the
//! layers it has never seen and recomputes only the files of the layers that were added, removed
//! or moved (the lesson of Leet change 12: no disk work for a toggle or a drag).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use strata_archive::GameArchive;
use strata_game::{Layer, LayerKind};

use crate::tree::{Options, fold, scan_layer};

/// Which of MO2's two structures a file is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Area {
    /// Under the game's Data folder (paths are relative to it).
    Data,
    /// The game folder itself, from the mods' `Root` folders (paths relative to it).
    Root,
}

/// A conflict flag (`ModInfoWithConflictInfo::EConflictType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictState {
    #[default]
    None,
    /// Wins files over other mods and loses none.
    Overwrite,
    /// Loses files to other mods and wins none.
    Overwritten,
    /// Both.
    Mixed,
    /// Every file it has is won by another mod (loose files only).
    Redundant,
}

/// MO2's archive parsing: which archives count and how they are ordered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveRules {
    /// The archive file names the profile loads, exactly as written (MO2 reads them from the
    /// profile's `archives.txt`, `OrganizerCore::enabledArchives`, and compares with case).
    pub enabled: HashSet<String>,
    /// Plugin file names in load order (`GamePlugins::getLoadOrder`: the profile's
    /// `loadorder.txt`).
    pub load_order: Vec<String>,
}

impl ArchiveRules {
    /// The archive's place: the load-order index of the plugin it belongs to (`<plugin> - x.bsa`
    /// or `<plugin>.bsa`, any case; the last match wins), or -1 (`DirectoryEntry::addFromAllBSAs`).
    pub fn order_of(&self, archive: &str) -> i32 {
        let name = archive.to_lowercase();
        let mut order = -1;
        for (i, plugin) in self.load_order.iter().enumerate() {
            let lower = plugin.to_lowercase();
            let stem = lower.rsplit_once('.').map_or(lower.as_str(), |(s, _)| s);
            if name.starts_with(&format!("{stem} - ")) || name.starts_with(&format!("{stem}.")) {
                // MO2 looks the plugin up again, so a name listed twice gives its first place.
                order = self.load_order.iter().position(|p| p == plugin).unwrap_or(i) as i32;
            }
        }
        order
    }
}

/// A layer's conflicts as MO2 computes them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Flags {
    /// Loose files against loose files (and redundancy).
    pub state: ConflictState,
    /// Layers this one wins loose files from (MO2's `m_OverwriteList`), lowest first.
    pub overwrites: Vec<String>,
    /// Layers that win loose files from this one (`m_OverwrittenList`), lowest first.
    pub overwritten_by: Vec<String>,
    /// Archived files against archived files (`m_ArchiveConflictState`; never redundant).
    pub archive_state: ConflictState,
    pub archive_overwrites: Vec<String>,
    pub archive_overwritten_by: Vec<String>,
    /// Loose files against archived ones (`m_ArchiveConflictLooseState`): "overwrite" = this
    /// layer's loose files beat other layers' archives, "overwritten" = other layers' loose files
    /// beat this layer's archives.
    pub archive_loose_state: ConflictState,
    pub archive_loose_overwrites: Vec<String>,
    pub archive_loose_overwritten_by: Vec<String>,
    /// A file or folder of the layer ends in `.mohidden` (`FLAG_HIDDEN_FILES`).
    pub hidden_files: bool,
}

/// One provider of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider<'a> {
    pub layer: &'a str,
    /// The archive the file is in, or `None` for a loose file.
    pub archive: Option<&'a str>,
}

/// One file of a layer with everyone who provides it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileConflict<'a> {
    pub area: Area,
    /// Relative to the area's folder, `/`-separated, in the first spelling seen.
    pub path: &'a str,
    /// Lowest first, as MO2 sorts them (archived files below loose ones): the last one wins.
    pub origins: Vec<Provider<'a>>,
}

impl FileConflict<'_> {
    /// The path as MO2's Conflicts tabs show it: `/meshes/x.nif`, or `/Root/x.dll` for Root files.
    pub fn shown(&self) -> String {
        match self.area {
            Area::Data => format!("/{}", self.path),
            Area::Root => format!("/Root/{}", self.path),
        }
    }

    pub fn winner(&self) -> &str {
        self.origins.last().map_or("", |p| p.layer)
    }
}

type FileId = u32;
type SlotId = u32;

/// Where a layer has a file: loose, or in one of its archives (index into `Slot::archives`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    file: FileId,
    archive: Option<u32>,
}

/// One provider in a file's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Origin {
    slot: SlotId,
    archive: Option<u32>,
}

/// A layer the index has scanned. Kept when the layer leaves the list, so enabling it again costs
/// no disk work.
#[derive(Debug)]
struct Slot {
    id: String,
    kind: LayerKind,
    /// At most one entry per file: a loose file hides the same path in the layer's archives, and
    /// the first archive by name hides the others (MO2's `FileEntry::addOrigin` keeps the first
    /// origin it gets per mod, and the loose walk comes first).
    entries: Vec<Entry>,
    /// The archives read, with their load-order place.
    archives: Vec<(String, i32)>,
    hidden_files: bool,
    /// Position in the active list, `None` while the layer is not in it.
    pos: Option<u32>,
    computed: Computed,
}

#[derive(Debug)]
struct FileName {
    area: Area,
    path: Box<str>,
}

/// The conflict index of one layer list (`docs/ARCHITECTURE.md`, "Conflicts index").
#[derive(Debug)]
pub struct ConflictIndex {
    /// The Data folder's name in the mount (`Game::data_dir`), folded.
    data_dir: String,
    archives: Option<ArchiveRules>,
    slots: Vec<Slot>,
    by_key: HashMap<(String, PathBuf), SlotId>,
    /// The active layers, lowest priority first.
    active: Vec<SlotId>,
    names: Vec<FileName>,
    lookup: HashMap<(Area, Box<str>), FileId>,
    /// Per file: the active providers, sorted as MO2's `FileEntry::sortOrigins`.
    origins: Vec<Vec<Origin>>,
}

/// One scanned file before interning: area, folded path, path as spelled, archive.
type Scanned = (Area, String, String, Option<u32>);

/// What scanning a layer gives.
struct LayerScan {
    files: Vec<Scanned>,
    archives: Vec<(String, i32)>,
    hidden: bool,
}

impl ConflictIndex {
    /// The index of `layers` (lowest priority first, as the overlay gets them). `data_dir` is the
    /// game's Data folder name (`Data` for Skyrim). `archives`: MO2's archive parsing, when on.
    pub fn build(layers: &[Layer], data_dir: &str, archives: Option<ArchiveRules>) -> Self {
        let mut index = Self {
            data_dir: fold(data_dir),
            archives,
            slots: Vec::new(),
            by_key: HashMap::new(),
            active: Vec::new(),
            names: Vec::new(),
            lookup: HashMap::new(),
            origins: Vec::new(),
        };
        index.update(layers);
        index
    }

    /// Switches to a new layer list (a mod toggled, moved, a profile changed). Returns the ids of
    /// the layers whose flags changed, with layers that joined or left the list. The archive
    /// rules stay those given to [`build`](Self::build): a changed plugin order needs a new build.
    pub fn update(&mut self, layers: &[Layer]) -> Vec<String> {
        // Slots for the new list; scan the layers never seen, in parallel.
        let key = |l: &Layer| (l.id.clone(), l.source.clone());
        let unseen: Vec<&Layer> = {
            let mut seen = HashSet::new();
            layers.iter().filter(|l| !self.by_key.contains_key(&key(l)) && seen.insert(key(l))).collect()
        };
        let scans: Vec<LayerScan> =
            unseen.par_iter().map(|l| scan(l, &self.data_dir, self.archives.as_ref())).collect();
        for (layer, scan) in unseen.into_iter().zip(scans) {
            let mut have = HashSet::new();
            let mut entries = Vec::new();
            for (area, folded, spelled, archive) in scan.files {
                let file = self.intern(area, folded, spelled);
                if have.insert(file) {
                    entries.push(Entry { file, archive });
                }
            }
            let slot = self.slots.len() as SlotId;
            self.slots.push(Slot {
                id: layer.id.clone(),
                kind: layer.kind,
                entries,
                archives: scan.archives,
                hidden_files: scan.hidden,
                pos: None,
                computed: Computed::default(),
            });
            self.by_key.insert(key(layer), slot);
        }
        self.origins.resize_with(self.names.len(), Vec::new);
        let new_active: Vec<SlotId> = layers.iter().map(|l| self.by_key[&key(l)]).collect();

        // Which layers changed place: added, removed, or out of the longest run that kept its order.
        let old_pos: HashMap<SlotId, u32> = self.active.iter().enumerate().map(|(i, s)| (*s, i as u32)).collect();
        let new_set: HashSet<SlotId> = new_active.iter().copied().collect();
        let removed: Vec<SlotId> = self.active.iter().copied().filter(|s| !new_set.contains(s)).collect();
        let added: Vec<SlotId> = new_active.iter().copied().filter(|s| !old_pos.contains_key(s)).collect();
        let kept: Vec<SlotId> = new_active.iter().copied().filter(|s| old_pos.contains_key(s)).collect();
        let in_order = longest_increasing(&kept.iter().map(|s| old_pos[s]).collect::<Vec<_>>());
        let moved: Vec<SlotId> =
            kept.iter().enumerate().filter(|(i, _)| !in_order.contains(i)).map(|(_, s)| *s).collect();

        let mut affected: Vec<FileId> = Vec::new();
        let mut is_affected = vec![false; self.names.len()];
        for s in removed.iter().chain(&added).chain(&moved) {
            for e in &self.slots[*s as usize].entries {
                if !std::mem::replace(&mut is_affected[e.file as usize], true) {
                    affected.push(e.file);
                }
            }
        }
        let before: HashMap<SlotId, Flags> = affected
            .iter()
            .flat_map(|f| self.origins[*f as usize].iter().map(|o| o.slot))
            .collect::<HashSet<_>>()
            .into_iter()
            .map(|s| (s, self.flags_of(s)))
            .collect();

        for s in &removed {
            self.slots[*s as usize].pos = None;
        }
        for (i, s) in new_active.iter().enumerate() {
            self.slots[*s as usize].pos = Some(i as u32);
        }
        self.active = new_active;
        for &f in &affected {
            self.origins[f as usize].retain(|o| self.slots[o.slot as usize].pos.is_some());
        }
        for &s in &added {
            for e in &self.slots[s as usize].entries {
                self.origins[e.file as usize].push(Origin { slot: s, archive: e.archive });
            }
        }
        let mut recompute: HashSet<SlotId> = HashSet::new();
        for &f in &affected {
            let mut list = std::mem::take(&mut self.origins[f as usize]);
            list.sort_unstable_by_key(|o| self.sort_key(*o));
            recompute.extend(list.iter().map(|o| o.slot));
            self.origins[f as usize] = list;
        }
        for &s in &removed {
            self.slots[s as usize].computed = Computed::default();
        }
        let results: Vec<(SlotId, Computed)> = recompute.par_iter().map(|s| (*s, self.compute(*s))).collect();
        for (s, c) in results {
            self.slots[s as usize].computed = c;
        }

        let mut changed: Vec<String> = Vec::new();
        for s in recompute.iter().chain(&removed) {
            if before.get(s) != Some(&self.flags_of(*s)) || added.contains(s) || removed.contains(s) {
                changed.push(self.slots[*s as usize].id.clone());
            }
        }
        changed.sort();
        changed.dedup();
        changed
    }

    fn intern(&mut self, area: Area, folded: String, spelled: String) -> FileId {
        if let Some(id) = self.lookup.get(&(area, folded.as_str().into())) {
            return *id;
        }
        let id = self.names.len() as FileId;
        self.names.push(FileName { area, path: spelled.into() });
        self.lookup.insert((area, folded.into()), id);
        id
    }

    /// `FileEntry::sortOrigins`: archived files first, by their plugin's load order (none = last
    /// of them), then loose files by priority. Archives in the same place go by priority (MO2's
    /// sort leaves that open).
    fn sort_key(&self, o: Origin) -> (bool, i32, Option<u32>) {
        let slot = &self.slots[o.slot as usize];
        match o.archive {
            Some(a) => (false, slot_order(slot, a), slot.pos),
            None => (true, 0, slot.pos),
        }
    }

    /// `doConflictCheck` for one layer.
    fn compute(&self, s: SlotId) -> Computed {
        let slot = &self.slots[s as usize];
        let mut c = Computed::default();
        if slot.kind == LayerKind::Game || slot.entries.is_empty() {
            return c;
        }
        let pos = |x: SlotId| self.slots[x as usize].pos;
        let order = |o: &Origin| o.archive.map(|a| slot_order(&self.slots[o.slot as usize], a));
        let mut provides_anything = false;
        for entry in &slot.entries {
            let origins = &self.origins[entry.file as usize];
            let Some((winner, alternatives)) = origins.split_last() else { continue };
            // No other provider, or the nearest one below is the game's Data folder (MO2's base
            // origin, loose files only): not a conflict.
            let base = self.names[entry.file as usize].area == Area::Data
                && alternatives.last().is_some_and(|a| self.slots[a.slot as usize].kind == LayerKind::Game);
            if alternatives.is_empty() || base {
                provides_anything = true;
                continue;
            }
            let mine_archived = entry.archive.is_some();
            let my_order = entry.archive.map(|a| slot_order(slot, a));
            if winner.slot == s {
                provides_anything = true;
            } else if winner.archive.is_some() {
                c.archive_overwritten_by.insert(winner.slot);
            } else if mine_archived {
                c.archive_loose_overwritten_by.insert(winner.slot);
            } else {
                c.overwritten_by.insert(winner.slot);
            }
            for alt in alternatives.iter().filter(|a| a.slot != s) {
                // The game's Data folder lands in these lists too, as in MO2 (where it is index
                // UINT_MAX): a mod losing a file that the game also has is "mixed".
                match (alt.archive.is_some(), mine_archived) {
                    (false, false) if pos(s) > pos(alt.slot) => c.overwrites.insert(alt.slot),
                    (false, false) => c.overwritten_by.insert(alt.slot),
                    (false, true) => c.archive_loose_overwritten_by.insert(alt.slot),
                    (true, false) => c.archive_loose_overwrites.insert(alt.slot),
                    (true, true) if my_order > order(alt) => c.archive_overwrites.insert(alt.slot),
                    (true, true) if my_order < order(alt) => c.archive_overwritten_by.insert(alt.slot),
                    (true, true) => false,
                };
            }
        }
        let state = |over: &BTreeSet<SlotId>, under: &BTreeSet<SlotId>| match (over.is_empty(), under.is_empty()) {
            (false, false) => ConflictState::Mixed,
            (false, true) => ConflictState::Overwrite,
            (true, false) => ConflictState::Overwritten,
            (true, true) => ConflictState::None,
        };
        c.state = if provides_anything { state(&c.overwrites, &c.overwritten_by) } else { ConflictState::Redundant };
        c.archive_state = state(&c.archive_overwrites, &c.archive_overwritten_by);
        c.archive_loose_state = state(&c.archive_loose_overwrites, &c.archive_loose_overwritten_by);
        c
    }

    fn flags_of(&self, s: SlotId) -> Flags {
        let slot = &self.slots[s as usize];
        let ids = |set: &BTreeSet<SlotId>| {
            let mut list: Vec<SlotId> = set.iter().copied().collect();
            list.sort_by_key(|x| self.slots[*x as usize].pos);
            list.into_iter().map(|x| self.slots[x as usize].id.clone()).collect()
        };
        let c = &slot.computed;
        Flags {
            state: c.state,
            overwrites: ids(&c.overwrites),
            overwritten_by: ids(&c.overwritten_by),
            archive_state: c.archive_state,
            archive_overwrites: ids(&c.archive_overwrites),
            archive_overwritten_by: ids(&c.archive_overwritten_by),
            archive_loose_state: c.archive_loose_state,
            archive_loose_overwrites: ids(&c.archive_loose_overwrites),
            archive_loose_overwritten_by: ids(&c.archive_loose_overwritten_by),
            hidden_files: slot.hidden_files && !slot.entries.is_empty(),
        }
    }

    fn active_slot(&self, id: &str) -> Option<SlotId> {
        self.active.iter().copied().find(|s| self.slots[*s as usize].id == id)
    }

    fn provider(&self, o: &Origin) -> Provider<'_> {
        let slot = &self.slots[o.slot as usize];
        Provider { layer: &slot.id, archive: o.archive.map(|a| slot.archives[a as usize].0.as_str()) }
    }

    fn file(&self, f: FileId) -> FileConflict<'_> {
        let name = &self.names[f as usize];
        FileConflict {
            area: name.area,
            path: &name.path,
            origins: self.origins[f as usize].iter().map(|o| self.provider(o)).collect(),
        }
    }

    /// The active layers' ids, lowest priority first.
    pub fn layer_ids(&self) -> impl Iterator<Item = &str> {
        self.active.iter().map(|s| self.slots[*s as usize].id.as_str())
    }

    /// The flags of an active layer, as `ModInfoWithConflictInfo::getConflictFlags` sees them.
    /// `None` for the game's own folder, which is not a mod.
    ///
    /// Two of MO2's quirks are kept on purpose (oracle check 3 compares with MO2's rules):
    /// a file whose nearest lower provider is the game's Data folder is no conflict at all, even
    /// when the layer loses it; and when the game's Data folder is one of several lower
    /// providers, it is listed in `overwrites` (as `<game>`).
    pub fn flags(&self, id: &str) -> Option<Flags> {
        self.active_slot(id).filter(|s| self.slots[*s as usize].kind != LayerKind::Game).map(|s| self.flags_of(s))
    }

    /// Every file of an active layer with all its providers (the mod info dialog's Conflicts
    /// tabs), Data files first, each area sorted by path.
    pub fn files_of(&self, id: &str) -> Option<Vec<FileConflict<'_>>> {
        let s = self.active_slot(id)?;
        let mut files: Vec<FileConflict<'_>> = self.slots[s as usize].entries.iter().map(|e| self.file(e.file)).collect();
        files.sort_by_cached_key(|f| (f.area, fold(f.path)));
        Some(files)
    }

    /// The providers of one file (any case), lowest first; `None` when no layer has it.
    pub fn origins(&self, area: Area, path: &str) -> Option<Vec<Provider<'_>>> {
        let f = self.lookup.get(&(area, fold(path.trim_matches('/')).into()))?;
        let list: Vec<Provider<'_>> = self.origins[*f as usize].iter().map(|o| self.provider(o)).collect();
        (!list.is_empty()).then_some(list)
    }

    /// Every file some active layer provides, in no particular order.
    pub fn all_files(&self) -> impl Iterator<Item = FileConflict<'_>> {
        (0..self.names.len() as FileId).filter(|f| !self.origins[*f as usize].is_empty()).map(|f| self.file(f))
    }

    /// How many distinct files the active layers provide.
    pub fn file_count(&self) -> usize {
        self.origins.iter().filter(|o| !o.is_empty()).count()
    }
}

/// An archive's load-order place, -1 (no plugin) sorting after every plugin's archives.
fn slot_order(slot: &Slot, archive: u32) -> i32 {
    match slot.archives[archive as usize].1 {
        o if o < 0 => i32::MAX,
        o => o,
    }
}

#[derive(Debug, Default)]
struct Computed {
    state: ConflictState,
    overwrites: BTreeSet<SlotId>,
    overwritten_by: BTreeSet<SlotId>,
    archive_state: ConflictState,
    archive_overwrites: BTreeSet<SlotId>,
    archive_overwritten_by: BTreeSet<SlotId>,
    archive_loose_state: ConflictState,
    archive_loose_overwrites: BTreeSet<SlotId>,
    archive_loose_overwritten_by: BTreeSet<SlotId>,
}

/// `DirectoryRefresher::cleanStructure`: the top-level `meta.ini`, `readme.txt` and `fomod/` go.
fn cleaned_away(rel_folded: &str) -> bool {
    match rel_folded.split_once('/') {
        None => rel_folded == "meta.ini" || rel_folded == "readme.txt",
        Some((first, _)) => first == "fomod",
    }
}

fn is_hidden(name: &str) -> bool {
    name.len() > ".mohidden".len() && name.ends_with(".mohidden")
}

/// Scans one layer into MO2's structures: its loose files, then (with archive parsing, for mods
/// and overwrite) the files of its enabled top-level archives, and whether a name ends in
/// `.mohidden`.
fn scan(layer: &Layer, data_dir: &str, archives: Option<&ArchiveRules>) -> LayerScan {
    // MO2's structures hide nothing (the overlay's skip lists are a VFS setting).
    let options = Options { skip_file_suffixes: Vec::new(), skip_directories: Vec::new(), clean_temp_files: false };
    let mut out = LayerScan { files: Vec::new(), archives: Vec::new(), hidden: false };
    for dir in scan_layer(layer, &options) {
        let (area, rel_dir) = match dir.target.split_once('/') {
            Some((first, rest)) if fold(first) == data_dir => (Area::Data, rest),
            None if fold(&dir.target) == data_dir => (Area::Data, ""),
            _ => (Area::Root, dir.target.as_str()),
        };
        // The game folder is not in the Root structure, nor is overwrite.
        if area == Area::Root && matches!(layer.kind, LayerKind::Game | LayerKind::Overwrite) {
            continue;
        }
        let rel_folded = fold(rel_dir);
        let dir_hidden = rel_dir.split('/').any(is_hidden);
        for name in dir.files {
            let (folded, spelled) = if rel_dir.is_empty() {
                (fold(&name), name)
            } else {
                (format!("{rel_folded}/{}", fold(&name)), format!("{rel_dir}/{name}"))
            };
            if cleaned_away(&folded) {
                continue;
            }
            out.hidden |= dir_hidden || is_hidden(spelled.rsplit('/').next().unwrap_or_default());
            out.files.push((area, folded, spelled, None));
        }
    }
    if let (Some(rules), LayerKind::Mod | LayerKind::Overwrite) = (archives, layer.kind) {
        for (name, path) in mod_archives(&layer.source, rules) {
            // MO2 logs an unreadable archive and goes on without it.
            let Ok(archive) = GameArchive::open(&path) else { continue };
            let index = out.archives.len() as u32;
            out.archives.push((name.clone(), rules.order_of(&name)));
            for f in archive.files() {
                let folded = fold(&f.path);
                if cleaned_away(&folded) {
                    continue;
                }
                out.hidden |= f.path.split('/').any(is_hidden);
                out.files.push((Area::Data, folded, f.path.clone(), Some(index)));
            }
        }
    }
    out
}

/// A mod's top-level `*.bsa` / `*.ba2` that the profile loads, by name without regard to case
/// (`ModInfoRegular::archives`: `QDir::entryList`, sorted by name; `addFromAllBSAs` keeps the
/// enabled ones).
fn mod_archives(source: &Path, rules: &ArchiveRules) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(source) else { return Vec::new() };
    let mut list: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let lower = name.to_lowercase();
            let wanted = (lower.ends_with(".bsa") || lower.ends_with(".ba2")) && rules.enabled.contains(&name);
            (wanted && e.path().is_file()).then(|| (name, e.path()))
        })
        .collect();
    list.sort_by_cached_key(|(name, _)| name.to_lowercase());
    list
}

/// Indices of one longest strictly increasing subsequence of `values`.
fn longest_increasing(values: &[u32]) -> HashSet<usize> {
    // Patience sorting: tails[k] = index of the smallest tail of a run of length k + 1.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; values.len()];
    for (i, v) in values.iter().enumerate() {
        let k = tails.partition_point(|t| values[*t] < *v);
        prev[i] = k.checked_sub(1).map(|k| tails[k]);
        if k == tails.len() {
            tails.push(i);
        } else {
            tails[k] = i;
        }
    }
    let mut out = HashSet::new();
    let mut at = tails.last().copied();
    while let Some(i) = at {
        out.insert(i);
        at = prev[i];
    }
    out
}

#[cfg(test)]
mod tests;
