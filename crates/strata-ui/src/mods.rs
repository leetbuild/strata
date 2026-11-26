//! The rows of the mod list (left pane), as `ModList::data` shows them, nested under their
//! separators by `strata_core::separators::SeparatorTree` (`ModListByPriorityProxy::buildTree`,
//! Leet change 9).

use std::collections::HashMap;
use std::path::Path;
use std::time::UNIX_EPOCH;

use strata_core::separators::SeparatorTree;
use strata_core::{Categories, Endorsed, Instance, ModKind, ModList, ModMeta};

use crate::content::{self, Content};

/// A mod flag (`ModInfo::EFlag`, the Flags column), in MO2's order of `getFlags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModFlag {
    Backup,
    /// A file or folder ends in `.mohidden` (from the conflict index).
    HiddenFiles,
    NotEndorsed,
    Tracked,
    /// "No valid game data".
    Invalid,
    Notes,
}

impl ModFlag {
    /// `ModFlagIconDelegate::getFlagIcon`.
    pub fn icon(self) -> &'static str {
        match self {
            ModFlag::Backup => ":/MO/gui/emblem_backup",
            ModFlag::HiddenFiles => ":/MO/gui/emblem_hidden_files",
            ModFlag::NotEndorsed => ":/MO/gui/emblem_notendorsed",
            ModFlag::Tracked => ":/MO/gui/tracked",
            ModFlag::Invalid => ":/MO/gui/problem",
            ModFlag::Notes => ":/MO/gui/emblem_notes",
        }
    }

    /// `ModList::getFlagText` (notes show the notes themselves).
    pub fn text(self) -> &'static str {
        match self {
            ModFlag::Backup => "Backup",
            ModFlag::HiddenFiles => "Contains hidden files",
            ModFlag::NotEndorsed => "Not endorsed yet",
            ModFlag::Tracked => "Tracked on Nexus",
            ModFlag::Invalid => "No valid game data",
            ModFlag::Notes => "Notes",
        }
    }
}

/// A colour as 8-bit red, green, blue, alpha.
pub type Rgba = [u8; 4];

/// What a row is. `Overwrite` is the pseudo mod for the overwrite folder (`ModInfoOverwrite`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Regular,
    Separator,
    Foreign,
    Backup,
    Overwrite,
}

/// One row of the mod list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModRow {
    /// The mod's internal name (its folder, `Tools_separator` for a separator).
    pub name: String,
    /// What the Name column shows: separators without `_separator` (`ModList::getDisplayName`).
    pub display_name: String,
    pub kind: RowKind,
    pub enabled: bool,
    /// Whether the row has a checkbox: regular mods only (`canBeEnabled`).
    pub checkable: bool,
    /// The Priority column; `None` for backups and Overwrite (`hasAutomaticPriority`).
    pub priority: Option<usize>,
    pub color: Option<Rgba>,
    /// The separator level (0 for everything but sub-separators).
    pub nest: i32,
    /// Whether the separator's item count is drawn although "always show size" is off.
    pub stored_size: bool,
    pub version: String,
    /// `newestVersion` (what the last update check found), "" when unknown.
    pub newest_version: String,
    /// The installed version against the newest (`updateAvailable` = Less, `downgradeAvailable`
    /// = Greater); `None` without a newest version.
    pub update: Option<std::cmp::Ordering>,
    /// The primary category's name; `Non-MO` for foreign mods.
    pub category: String,
    /// The Nexus ID column; `None` when 0 or less.
    pub nexus_id: Option<i64>,
    /// The Notes column (`comments` in `meta.ini`).
    pub comments: String,
    /// The Installation column: the mod folder's creation time, seconds since 1970 (MO2
    /// reads `QFileInfo::birthTime` of the folder).
    pub install_time: Option<i64>,
    /// Overwrite only: whether the folder has anything in it (MO2 then draws its name in
    /// dark red, else dark grey).
    pub has_files: bool,
    /// The row's parent separator (an index into the same list) when the list is shown by
    /// ascending priority; `None` at the top level.
    pub parent: Option<usize>,
    /// The Flags column, without the conflict index's (hidden files).
    pub flags: Vec<ModFlag>,
    /// The Content column.
    pub contents: Vec<Content>,
    /// `notes` (the long notes; the Flags column's tooltip).
    pub notes: String,
    /// Every category id the mod has that exists (the filter panel's categories).
    pub categories: Vec<i32>,
    /// `endorsed` says endorsed; `tracked` is set (the filter panel's `<Endorsed>`, `<Tracked>`).
    pub endorsed: bool,
    pub tracked: bool,
    /// Valid game data (or `validated`): no "no valid game data" flag.
    pub valid: bool,
}

fn install_time(dir: &Path) -> Option<i64> {
    let created = std::fs::metadata(dir).ok()?.created().ok()?;
    created.duration_since(UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok())
}

fn has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

/// What a row shows from one mod's `meta.ini` and folder, read once and kept until the mod
/// changes (`MetaChanged`, `Renamed`), so that a move rebuilds the rows without the disk.
#[derive(Debug, Clone, Default)]
pub struct CachedMeta {
    color: Option<Rgba>,
    stored_size: bool,
    version: String,
    newest_version: String,
    update: Option<std::cmp::Ordering>,
    comments: String,
    nexus_id: i64,
    /// The primary category (`ModMeta::primary_category`: the first id, when valid).
    primary: Option<i32>,
    install_time: Option<i64>,
    notes: String,
    /// `endorsed` says "not endorsed" (not unknown, not "never").
    not_endorsed: bool,
    is_endorsed: bool,
    category_ids: Vec<i32>,
    tracked: bool,
    validated: bool,
    scan: content::Scan,
}

impl CachedMeta {
    fn read(dir: &Path) -> Self {
        let scan = content::scan(dir);
        let Ok(meta) = ModMeta::open(dir) else {
            return Self { install_time: install_time(dir), scan, ..Self::default() };
        };
        let primary = meta.primary_category();
        Self {
            color: meta.color().map(|c| [c.r, c.g, c.b, c.a]),
            stored_size: meta
                .plugin_setting(strata_core::SUB_SEPARATOR, "maxSize")
                .and_then(|v| v.to_text().trim().parse::<i64>().ok())
                .is_some_and(|n| n >= 0),
            version: meta.version(),
            newest_version: meta.newest_version(),
            update: strata_install::version::update_state(
                &meta.version(),
                &meta.newest_version(),
                &meta.document().string("", "ignoredVersion").unwrap_or_default(),
                meta.document().string("", "nexusFileStatus").and_then(|s| s.trim().parse().ok()).unwrap_or(1),
            ),
            comments: meta.comments(),
            nexus_id: meta.mod_id(),
            primary,
            install_time: install_time(dir),
            notes: meta.notes(),
            not_endorsed: meta.endorsed() == Endorsed::False,
            is_endorsed: meta.endorsed() == Endorsed::True,
            category_ids: meta.categories(),
            tracked: meta.tracked(),
            validated: meta.validated(),
            scan,
        }
    }
}

/// [`CachedMeta`] per mod name.
#[derive(Debug, Default)]
pub struct MetaCache(HashMap<String, CachedMeta>);

impl MetaCache {
    /// Forgets one mod (its `meta.ini` or folder changed).
    pub fn forget(&mut self, name: &str) {
        self.0.remove(name);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    fn get(&mut self, name: &str, dir: Option<&Path>) -> CachedMeta {
        match dir {
            Some(dir) => self.0.entry(name.to_owned()).or_insert_with(|| CachedMeta::read(dir)).clone(),
            None => CachedMeta::default(),
        }
    }
}

/// The mod list as the left pane shows it by ascending priority: backups first (they are
/// moved to the top, `buildTree`), then every mod by priority, then Overwrite. `parent` is
/// the separator each row sits in, as an index into the returned list.
pub fn rows(instance: &Instance, list: &ModList, categories: &Categories, cache: &mut MetaCache) -> Vec<ModRow> {
    let overwrite = &instance.paths.overwrite;
    // NexusSettings::endorsementIntegration / trackedIntegration, on unless switched off
    let setting = |key: &str| instance.ini.value("Settings", key).is_none_or(|v| v.to_bool());
    let (endorsements, tracking) = (setting("endorsement_integration"), setting("tracked_integration"));
    let entries = list.entries();
    let tree = SeparatorTree::build(entries);
    // display order: backups first, then the rest by priority
    let mut order: Vec<usize> = (0..entries.len()).filter(|p| entries[*p].kind == ModKind::Backup).collect();
    order.extend((0..entries.len()).filter(|p| entries[*p].kind != ModKind::Backup));
    let mut row_of = vec![0usize; entries.len()];
    for (row, p) in order.iter().enumerate() {
        row_of[*p] = row;
    }

    let mut rows: Vec<ModRow> = order
        .iter()
        .map(|&p| {
            let entry = &entries[p];
            let meta = cache.get(&entry.name, entry.path.as_deref());
            let kind = match entry.kind {
                ModKind::Regular => RowKind::Regular,
                ModKind::Separator => RowKind::Separator,
                ModKind::Foreign => RowKind::Foreign,
                ModKind::Backup => RowKind::Backup,
            };
            let display_name = match kind {
                RowKind::Separator => entry.name.replace("_separator", ""),
                _ => entry.name.clone(),
            };
            let category = match kind {
                RowKind::Foreign => "Non-MO".to_owned(),
                _ => meta
                    .primary
                    .filter(|id| *id > 0)
                    .and_then(|id| categories.get(id))
                    .map(|c| c.name.clone())
                    .unwrap_or_default(),
            };
            // ModInfoRegular::getFlags (and ModInfoBackup's)
            let mut flags = Vec::new();
            if kind == RowKind::Backup {
                flags.push(ModFlag::Backup);
            }
            if matches!(kind, RowKind::Regular) {
                if meta.nexus_id > 0 && meta.not_endorsed && endorsements {
                    flags.push(ModFlag::NotEndorsed);
                }
                if meta.nexus_id > 0 && meta.tracked && tracking {
                    flags.push(ModFlag::Tracked);
                }
                if !meta.scan.valid && !meta.validated {
                    flags.push(ModFlag::Invalid);
                }
                if !meta.notes.is_empty() {
                    flags.push(ModFlag::Notes);
                }
            }
            ModRow {
                categories: meta.category_ids.iter().copied().filter(|id| categories.get(*id).is_some()).collect(),
                endorsed: meta.is_endorsed,
                tracked: meta.tracked,
                valid: meta.scan.valid || meta.validated,
                flags,
                contents: if kind == RowKind::Separator { Vec::new() } else { meta.scan.contents.clone() },
                notes: meta.notes.clone(),
                display_name,
                kind,
                enabled: entry.enabled,
                checkable: kind == RowKind::Regular,
                priority: (kind != RowKind::Backup).then_some(entry.priority),
                color: meta.color,
                nest: if kind == RowKind::Separator { i32::try_from(entry.nest).unwrap_or(i32::MAX) } else { 0 },
                stored_size: meta.stored_size,
                version: meta.version,
                newest_version: meta.newest_version,
                update: if kind == RowKind::Regular { meta.update } else { None },
                category,
                nexus_id: (meta.nexus_id > 0).then_some(meta.nexus_id),
                comments: meta.comments,
                install_time: meta.install_time,
                has_files: false,
                parent: if kind == RowKind::Backup { None } else { tree.parent[p].map(|s| row_of[s]) },
                name: entry.name.clone(),
            }
        })
        .collect();

    rows.push(ModRow {
        name: "Overwrite".into(),
        display_name: "Overwrite".into(),
        kind: RowKind::Overwrite,
        enabled: true,
        checkable: false,
        priority: None,
        color: None,
        nest: 0,
        stored_size: false,
        version: String::new(),
        newest_version: String::new(),
        update: None,
        category: String::new(),
        nexus_id: None,
        comments: String::new(),
        install_time: None,
        has_files: has_files(overwrite),
        parent: None,
        flags: Vec::new(),
        contents: Vec::new(),
        notes: String::new(),
        categories: Vec::new(),
        endorsed: false,
        tracked: false,
        valid: true,
    });
    rows
}

/// How many rows each row holds directly (the `(n)` the delegate draws next to a separator).
pub fn child_counts(rows: &[ModRow]) -> HashMap<usize, usize> {
    let mut counts = HashMap::new();
    for parent in rows.iter().filter_map(|r| r.parent) {
        *counts.entry(parent).or_insert(0) += 1;
    }
    counts
}

/// Enabled regular mods: `ModListView::counters`' "active" count, which leaves out
/// separators, foreign mods, backups and Overwrite. (The "Active:" counter itself shows the
/// ones of them the filter lets through; the shell counts those in its view.)
pub fn active_count(rows: &[ModRow]) -> usize {
    rows.iter().filter(|r| r.kind == RowKind::Regular && r.enabled).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_instance::TestInstance;

    #[test]
    fn rows_follow_the_core_list_with_backups_first_and_overwrite_last() {
        let t = TestInstance::new(
            "+Top Mod\r\n-Inner\r\n+B_separator\r\n+A.1_separator\r\n+a1\r\n+A_separator\r\n+Loose\r\n",
        );
        t.meta("A_separator", "[General]\r\ncolor=@Variant(\\0\\0\\0\\x43\\x1\\xff\\xff--??FF\\0\\0)\r\n");
        t.meta("A.1_separator", "[Plugins]\r\nSub-Separator\\nest=1\r\nSub-Separator\\maxSize=3\r\n");
        t.meta("a1", "[General]\r\ncategory=\"-1,1\"\r\nversion=1.2\r\ncomments=hi\r\nmodid=42\r\n");
        t.meta("Loose", "[General]\r\ncategory=\"1,\"\r\n");
        t.folder("Old backup1");
        let (instance, profile) = t.open();
        let list = ModList::load(&instance, &profile).unwrap();
        let categories = Categories::open(&instance.root).unwrap();
        let rows = rows(&instance, &list, &categories, &mut MetaCache::default());

        let names: Vec<_> = rows.iter().map(|r| r.display_name.as_str()).collect();
        assert_eq!(names, ["Old backup1", "Loose", "A", "a1", "A.1", "B", "Inner", "Top Mod", "Overwrite"]);
        let parents: Vec<_> = rows.iter().map(|r| r.parent).collect();
        // A.1 has level 1: inside A; B (level 0) closes both
        assert_eq!(parents, [None, None, None, Some(2), Some(2), None, Some(5), Some(5), None]);
        assert_eq!(rows[2].color, Some([0x2d, 0x3f, 0x46, 0xff]));
        assert_eq!((rows[4].nest, rows[4].stored_size), (1, true));
        assert_eq!((rows[3].version.as_str(), rows[3].comments.as_str(), rows[3].nexus_id), ("1.2", "hi", Some(42)));
        assert_eq!(rows[3].category, "", "\"-1,1\" has no primary category");
        assert_eq!(rows[1].category, "Skyrim Special Edition");
        assert_eq!((rows[0].priority, rows[0].checkable), (None, false));
        assert!(!rows[6].enabled);
        // empty folders are no valid game data; separators and backups are never "invalid"
        assert_eq!(rows[1].flags, [ModFlag::Invalid]);
        assert!(rows[2].flags.is_empty());
        assert_eq!(rows[0].flags, [ModFlag::Backup]);
        assert_eq!(active_count(&rows), 3);
        assert_eq!(child_counts(&rows)[&2], 2);
    }
}
