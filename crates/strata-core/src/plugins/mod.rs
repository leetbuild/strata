//! The plugin list of a profile: every plugin of the merged Data folder, its order, its
//! state, its masters (`docs/ARCHITECTURE.md`, `PluginList`).
//!
//! A port of MO2 2.5.2's `PluginList` (`modorganizer/src/pluginlist.cpp`) and of the plugin
//! list files of `CreationGamePlugins` / `GamebryoGamePlugins` (Fluorine
//! `libs/game_bethesda`). The functions keep MO2's names in their docs, and their bodies follow
//! MO2's step by step, odd corners included, so that the order Strata computes is the order
//! MO2 would show for the same files. Where MO2's result is unspecified (it walks a hash map),
//! Strata picks a fixed order.
//!
//! The list knows nothing about the overlay: the caller hands it the plugins of the merged
//! Data folder ([`AvailablePlugin`]).

mod files;
mod saves;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use strata_game::PluginListFormat;
use strata_game::plugin::{self, PluginHeader};

pub use files::{LockedOrder, parse_locked_order};
pub use saves::{MissingPlugin, SaveEntry};

use crate::{Error, Instance, MODLIST_HEADER, Profile, Result};

/// A plugin file in the merged Data folder (`PluginList::refresh` takes these from the
/// directory structure's top level).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailablePlugin {
    /// The file name as the merged folder shows it.
    pub name: String,
    /// The layer it comes from (a mod's name, `<game>`, `<overwrite>`).
    pub origin: String,
    /// The real file.
    pub path: PathBuf,
}

/// One plugin of the list (`PluginList::ESPInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct Plugin {
    pub name: String,
    pub origin: String,
    pub path: PathBuf,
    /// The TES4 header, or why it could not be read (then MO2 sees no flags, no masters).
    pub header: std::result::Result<PluginHeader, String>,
    pub has_master_extension: bool,
    pub has_light_extension: bool,
    pub is_master_flagged: bool,
    /// The light flag, only where the game supports light plugins.
    pub is_light_flagged: bool,
    /// A primary plugin with `force_enable_core_files` on: always enabled, never moved.
    pub force_loaded: bool,
    /// Can never be enabled (an `.esl` in a game without light plugins).
    pub force_disabled: bool,
    pub enabled: bool,
    /// 0 = loads first. Every plugin has one, enabled or not.
    pub priority: usize,
    /// Position among the enabled plugins; `None` when disabled.
    pub load_order: Option<usize>,
    /// The form id prefix the game gives it, as MO2 shows it: `0A`, `FE:00C`; empty when
    /// disabled.
    pub index: String,
    /// The masters as MO2 keeps them: each once, sorted without case.
    pub masters: Vec<String>,
    /// Masters that are not enabled (absent or disabled), for an enabled plugin
    /// (`masterUnset`). Sorted like `masters`.
    pub missing_masters: Vec<String>,
}

impl Plugin {
    /// A master for ordering: `.esm`, `.esl` or the master flag (MO2 never counts the light
    /// flag alone).
    pub fn is_master_like(&self) -> bool {
        self.has_light_extension || self.has_master_extension || self.is_master_flagged
    }

    /// Loads in the light (FE) space.
    pub fn is_light(&self) -> bool {
        self.has_light_extension || self.is_light_flagged
    }

    /// The form version from the header, if read.
    pub fn form_version(&self) -> Option<u16> {
        self.header.as_ref().ok().and_then(|h| h.form_version)
    }
}

/// The game's side of the plugin list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRules {
    /// The plugins the game always loads, in order (`IPluginGame::primaryPlugins`; Skyrim
    /// SE adds the Creation Club list).
    pub primary: Vec<String>,
    /// `[Settings] force_enable_core_files` (default true): primary plugins are always
    /// enabled and first.
    pub force_enable_core_files: bool,
    pub light_plugins_supported: bool,
    /// How `plugins.txt` is read and written.
    pub format: PluginListFormat,
}

/// The profile files the list is read from; `None` for a file that does not exist.
#[derive(Debug, Clone, Default)]
pub struct PluginFiles {
    pub plugins_txt: Option<Vec<u8>>,
    pub loadorder_txt: Option<Vec<u8>>,
    pub lockedorder_txt: Option<Vec<u8>>,
}

/// The plugin list (`PluginList`).
#[derive(Debug, Clone)]
pub struct PluginList {
    /// In MO2's row order (the order plugins were found), not load order.
    plugins: Vec<Plugin>,
    /// Lower-case name to row.
    by_name: HashMap<String, usize>,
    /// Priority to row.
    by_priority: Vec<usize>,
    /// `m_LockedOrder`: lower-case name to (name as written, load order position).
    locked: BTreeMap<String, (String, i64)>,
    rules: PluginRules,
}

fn key(name: &str) -> String {
    name.to_lowercase()
}

fn contains(list: &[String], name: &str) -> bool {
    list.iter().any(|p| p.eq_ignore_ascii_case(name) || key(p) == key(name))
}

/// Sorted without case and each name once, like MO2's `std::set<QString, FileNameComparator>`.
fn name_set(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for name in names {
        map.entry(key(&name)).or_insert(name);
    }
    map.into_values().collect()
}

/// The lines of a list file as `MOBase::forEachLineInFile` gives them: split at CR or LF,
/// trimmed, blank and `#` lines skipped.
fn lines(data: &[u8]) -> impl Iterator<Item = String> + '_ {
    data.split(|b| *b == b'\n' || *b == b'\r')
        .map(|l| l.trim_ascii())
        .filter(|l| !l.is_empty() && l[0] != b'#')
        .map(|l| String::from_utf8_lossy(l).into_owned())
}

impl PluginList {
    /// Builds the list as `PluginList::refresh` does on a fresh start:
    /// the plugins found, `readPluginLists` (load order from `loadorder.txt`, or from
    /// `plugins.txt` without it; states from `plugins.txt`), `fixPrimaryPlugins`,
    /// `fixPluginRelationships`, `testMasters`, `readLockedOrderFrom`, `refreshLoadOrder`.
    ///
    /// `available` is the merged Data folder's plugins; names are unique without case.
    /// Headers are read from each plugin's real file.
    pub fn load(available: Vec<AvailablePlugin>, rules: PluginRules, files: &PluginFiles) -> Self {
        let mut available = available;
        // MO2 walks a hash map here, so its row order is unspecified; ours is by name.
        available.sort_by_key(|a| key(&a.name));
        available.dedup_by(|a, b| key(&a.name) == key(&b.name));
        let plugins = available.into_iter().map(|a| Self::make_plugin(a, &rules)).collect();
        let mut list = Self { plugins, by_name: HashMap::new(), by_priority: Vec::new(), locked: BTreeMap::new(), rules };
        // fixPriorities: every new plugin has priority -1, so they keep row order.
        for (i, p) in list.plugins.iter_mut().enumerate() {
            p.priority = i;
        }
        list.update_indices();
        list.read_plugin_lists(files);
        list.fix_primary_plugins();
        list.fix_plugin_relationships();
        list.test_masters();
        list.update_indices();
        if let Some(data) = &files.lockedorder_txt {
            list.read_locked_order(data);
        }
        list.refresh_load_order();
        list.test_masters();
        list
    }

    /// `PluginList::ESPInfo::ESPInfo`.
    fn make_plugin(a: AvailablePlugin, rules: &PluginRules) -> Plugin {
        let force_loaded = rules.force_enable_core_files && contains(&rules.primary, &a.name);
        // refresh(): `filename.endsWith(".esl")`, case-sensitive in MO2.
        let force_disabled = !rules.light_plugins_supported && a.name.ends_with(".esl");
        let header = PluginHeader::read(&a.path).map_err(|e| e.to_string());
        let (master_flag, light_flag, masters) = match &header {
            Ok(h) => (h.is_master_flagged(), rules.light_plugins_supported && h.is_light_flagged(), h.masters.clone()),
            Err(_) => (false, false, Vec::new()),
        };
        Plugin {
            has_master_extension: plugin::has_master_extension(&a.name) && header.is_ok(),
            has_light_extension: plugin::has_light_extension(&a.name) && header.is_ok(),
            is_master_flagged: master_flag,
            is_light_flagged: light_flag,
            masters: name_set(masters),
            header,
            force_loaded,
            force_disabled,
            enabled: force_loaded,
            priority: 0,
            load_order: None,
            index: String::new(),
            missing_masters: Vec::new(),
            name: a.name,
            origin: a.origin,
            path: a.path,
        }
    }

    /// The plugins in priority order.
    pub fn plugins(&self) -> impl Iterator<Item = &Plugin> {
        self.by_priority.iter().map(|&row| &self.plugins[row])
    }

    /// A plugin by name, any case.
    pub fn get(&self, name: &str) -> Option<&Plugin> {
        self.by_name.get(&key(name)).map(|&row| &self.plugins[row])
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Whether `name` is locked, and at which load order position.
    pub fn locked_position(&self, name: &str) -> Option<i64> {
        self.locked.get(&key(name)).map(|(_, pos)| *pos)
    }

    /// `PluginList::setState`.
    fn set_state(&mut self, name: &str, active: bool) {
        if let Some(&row) = self.by_name.get(&key(name)) {
            let p = &mut self.plugins[row];
            p.enabled = (active && !p.force_disabled) || p.force_loaded;
        }
    }

    /// `PluginList::setLoadOrder`: the named plugins first, in list order; the others after
    /// them, in their old priority order.
    fn set_load_order(&mut self, order: &[String]) {
        let mut assigned = vec![false; self.plugins.len()];
        let mut next = 0;
        for name in order {
            if let Some(&row) = self.by_name.get(&key(name))
                && !assigned[row]
            {
                self.plugins[row].priority = next;
                assigned[row] = true;
                next += 1;
            }
        }
        // `use old priorities`: MO2 walks the rows, which are in name order here.
        for row in (0..self.plugins.len()).filter(|r| !assigned[*r]) {
            self.plugins[row].priority = next;
            next += 1;
        }
        self.update_indices();
    }

    /// `GamebryoGamePlugins::readPluginLists` on its first call, with the Creation games'
    /// `readPluginList`.
    ///
    /// Gamebryo games: `SkyrimGamePlugins::readPluginLists`, which always takes the order
    /// from `loadorder.txt` (without it, the order the list has) and only the states from
    /// `plugins.txt`.
    fn read_plugin_lists(&mut self, files: &PluginFiles) {
        if self.rules.format == PluginListFormat::Gamebryo {
            let order = match &files.loadorder_txt {
                Some(data) => self.read_load_order_list(data),
                None => self.plugins().map(|p| p.name.clone()).collect(),
            };
            self.set_load_order(&order);
            self.read_plugin_list_gamebryo(files.plugins_txt.as_deref());
            return;
        }
        let order = match &files.loadorder_txt {
            Some(data) => self.read_load_order_list(data),
            None => self.read_plugin_list(files.plugins_txt.as_deref()),
        };
        self.set_load_order(&order);
        self.read_plugin_list(files.plugins_txt.as_deref());
    }

    /// `SkyrimGamePlugins::readPluginList`: primary plugins on; every name in `plugins.txt`
    /// on (a line is a comment when its first byte is `#`); every other plugin off. An
    /// absent or empty file turns every non-primary plugin off.
    fn read_plugin_list_gamebryo(&mut self, data: Option<&[u8]>) {
        let primary = self.rules.primary.clone();
        for name in &primary {
            self.set_state(name, true);
        }
        let mut listed: Vec<String> = Vec::new();
        for raw in data.unwrap_or_default().split_inclusive(|b| *b == b'\n') {
            if raw.first().is_some_and(|b| *b != b'#') {
                let name = String::from_utf8_lossy(raw.trim_ascii()).into_owned();
                if !name.is_empty() {
                    self.set_state(&name, true);
                    listed.push(name);
                }
            }
        }
        let names: Vec<String> = self.plugins.iter().map(|p| p.name.clone()).collect();
        for name in names {
            if !contains(&primary, &name) && !contains(&listed, &name) {
                self.set_state(&name, false);
            }
        }
    }

    /// `GamebryoGamePlugins::readLoadOrderList`: the primary plugins, then each line of
    /// `loadorder.txt` not seen before.
    ///
    /// MO2 remembers non-primary lines in their own spelling and looks them up in lower case,
    /// so a repeated line is listed twice and leaves a gap in the priorities. Strata takes
    /// each name once (first line wins).
    fn read_load_order_list(&self, data: &[u8]) -> Vec<String> {
        let mut order: Vec<String> = self.rules.primary.clone();
        for line in lines(data) {
            if !contains(&order, &line) {
                order.push(line);
            }
        }
        order
    }

    /// `CreationGamePlugins::readPluginList`: primary plugins on, then `*name` on and `name`
    /// off for each line, and every plugin the file does not name off. Returns the primary
    /// plugins followed by the file's names (the load order when there is no
    /// `loadorder.txt`).
    fn read_plugin_list(&mut self, data: Option<&[u8]>) -> Vec<String> {
        let primary = self.rules.primary.clone();
        let mut order = primary.clone();
        for name in &primary {
            self.set_state(name, true);
        }
        // An absent or empty file (MO2 always writes a header) changes nothing more.
        let Some(data) = data.filter(|d| !d.is_empty()) else { return order };

        let mut found: Vec<String> = Vec::new();
        // QFile::readLine: lines end at LF; a line is a comment when its first byte is `#`,
        // before trimming.
        for raw in data.split_inclusive(|b| *b == b'\n') {
            let name = if raw.first().is_some_and(|b| *b != b'#') {
                String::from_utf8_lossy(raw.trim_ascii()).into_owned()
            } else {
                String::new()
            };
            if !contains(&primary, &name) {
                let (active, bare) = match name.strip_prefix('*') {
                    Some(rest) => (true, rest.to_owned()),
                    None => (false, name),
                };
                if !bare.is_empty() {
                    self.set_state(&bare, active);
                    if !contains(&order, &bare) {
                        order.push(bare.clone());
                    }
                    found.push(bare);
                }
            } else {
                // MO2 drops the first character of a bare primary plugin's name here
                // (`pluginName.remove(0, 1)`), so that plugin counts as not found below.
                found.push(name.chars().skip(1).collect());
            }
        }
        let names: Vec<String> = self.plugins.iter().map(|p| p.name.clone()).collect();
        for name in names {
            if !contains(&found, &name) {
                self.set_state(&name, false);
            }
        }
        order
    }

    /// `PluginList::fixPrimaryPlugins`: each present primary plugin is sent to its place,
    /// in the order the game names them (when `force_enable_core_files` is on).
    fn fix_primary_plugins(&mut self) {
        if !self.rules.force_enable_core_files {
            return;
        }
        let mut prio = 0;
        for name in self.rules.primary.clone() {
            if let Some(&row) = self.by_name.get(&key(&name)) {
                if self.plugins[row].priority as i64 != prio {
                    self.set_plugin_priority(row, prio);
                }
                prio += 1;
            }
        }
    }

    /// `PluginList::fixPluginRelationships`: masters above other plugins, then each plugin
    /// below its masters.
    fn fix_plugin_relationships(&mut self) {
        let master_count = self.plugins.iter().filter(|p| p.is_master_like()).count() as i64;
        for row in 0..self.plugins.len() {
            let p = &self.plugins[row];
            let prio = p.priority as i64;
            // MO2's comparisons, kept as they are: a master exactly at `master_count`, or a
            // plugin exactly there, stays.
            if (p.is_master_like() && prio > master_count) || (!p.is_master_like() && prio < master_count) {
                self.set_plugin_priority(row, master_count + 1);
            }
        }
        for row in 0..self.plugins.len() {
            let p = &self.plugins[row];
            let mut new_priority = p.priority;
            for master in &p.masters {
                if let Some(&m) = self.by_name.get(&key(master)) {
                    new_priority = new_priority.max(self.plugins[m].priority);
                }
            }
            if new_priority != self.plugins[row].priority {
                self.set_plugin_priority(row, new_priority as i64);
            }
        }
    }

    /// `PluginList::testMasters`.
    fn test_masters(&mut self) {
        let enabled: std::collections::HashSet<String> =
            self.plugins.iter().filter(|p| p.enabled).map(|p| key(&p.name)).collect();
        for p in &mut self.plugins {
            p.missing_masters =
                if p.enabled { p.masters.iter().filter(|m| !enabled.contains(&key(m))).cloned().collect() } else { Vec::new() };
        }
    }

    /// `PluginList::updateIndices` (and `generatePluginIndexes`).
    fn update_indices(&mut self) {
        self.by_name.clear();
        self.by_priority = vec![0; self.plugins.len()];
        for (row, p) in self.plugins.iter().enumerate() {
            self.by_name.insert(key(&p.name), row);
            if let Some(slot) = self.by_priority.get_mut(p.priority) {
                *slot = row;
            }
        }
        self.generate_plugin_indexes();
    }

    /// `PluginList::generatePluginIndexes`: `XX` for regular plugins, `FE:XXX` for light ones.
    fn generate_plugin_indexes(&mut self) {
        let (mut lights, mut skipped) = (0usize, 0usize);
        for l in 0..self.by_priority.len() {
            let row = self.by_priority[l];
            let light = self.rules.light_plugins_supported && self.plugins[row].is_light();
            let p = &mut self.plugins[row];
            if !p.enabled {
                p.index.clear();
                skipped += 1;
            } else if light {
                p.index = format!("{:02X}:{:03X}", 254 + lights / 4096, lights % 4096);
                lights += 1;
            } else {
                p.index = format!("{:02X}", l - lights - skipped);
            }
        }
    }

    fn is_master_like_at(&self, priority: i64) -> bool {
        self.plugins[self.by_priority[priority as usize]].is_master_like()
    }

    /// `PluginList::setPluginPriority`: moves one plugin, keeping plugins below masters,
    /// masters above plugins and below the primary plugins, plugins below their masters, and
    /// masters above the plugins that need them.
    ///
    /// Returns true when the plugin moved up: MO2 then adds one to the caller's priority
    /// (an in/out parameter), which only `changePluginPriority` keeps, so that the next
    /// plugin of a group lands below this one.
    fn set_plugin_priority(&mut self, row: usize, new_priority: i64) -> bool {
        let count = self.by_priority.len() as i64;
        if count == 0 {
            return false;
        }
        let mut target = new_priority.clamp(0, count - 1);
        if !self.plugins[row].is_master_like() {
            while target < count - 1 && self.is_master_like_at(target) {
                target += 1;
            }
        } else {
            while target > 0 && !self.is_master_like_at(target) {
                target -= 1;
            }
            while target < count - 1 && self.plugins[self.by_priority[target as usize]].force_loaded {
                target += 1;
            }
        }

        let old = self.plugins[row].priority as i64;
        if target < old {
            for master in &self.plugins[row].masters {
                if let Some(&m) = self.by_name.get(&key(master)) {
                    let mp = self.plugins[m].priority as i64;
                    if mp >= target {
                        target = mp + 1;
                    }
                }
            }
        } else if target > old {
            let name = self.plugins[row].name.clone();
            let mut i = old + 1;
            while i <= target {
                let other = &self.plugins[self.by_priority[i as usize]];
                if other.masters.iter().any(|m| key(m) == key(&name)) {
                    target = other.priority as i64 - 1;
                }
                i += 1;
            }
        }

        // MO2 guards the moves below with `.at()` and gives up on an out-of-range index.
        let mut moved_up = false;
        if target != old && (0..count).contains(&target) {
            if target > old {
                for i in old + 1..=target {
                    self.plugins[self.by_priority[i as usize]].priority -= 1;
                }
            } else {
                for i in target..old {
                    self.plugins[self.by_priority[i as usize]].priority += 1;
                }
                moved_up = true;
            }
            self.plugins[row].priority = target as usize;
        }
        self.update_indices();
        moved_up
    }

    /// `PluginList::changePluginPriority`: moves several plugins to `new_priority`, as a drop
    /// in the list does.
    fn change_plugin_priority(&mut self, mut rows: Vec<usize>, new_priority: i64) {
        let mut new_priority = new_priority;
        let (mut min, mut max) = (i64::MAX, i64::MIN);
        for p in &self.plugins {
            let prio = p.priority as i64;
            if p.force_loaded {
                new_priority = new_priority.max(prio + 1);
            }
            max = max.max(prio + 1);
            min = min.min(prio);
        }
        new_priority = new_priority.min(max).max(min);
        rows.sort_by_key(|r| self.plugins[*r].priority);
        if rows.iter().any(|r| (self.plugins[*r].priority as i64) < new_priority) {
            new_priority -= 1;
        }
        for row in rows {
            if self.set_plugin_priority(row, new_priority) {
                new_priority += 1;
            }
        }
        self.refresh_load_order();
    }

    /// `PluginList::syncLoadOrder`.
    fn sync_load_order(&mut self) {
        let mut next = 0;
        for &row in &self.by_priority {
            let p = &mut self.plugins[row];
            p.load_order = p.enabled.then(|| {
                next += 1;
                next - 1
            });
        }
    }

    fn load_order_value(&self, row: usize) -> i64 {
        self.plugins[row].load_order.map_or(-1, |l| l as i64)
    }

    /// `PluginList::refreshLoadOrder`: puts locked plugins back at their load order
    /// positions.
    fn refresh_load_order(&mut self) {
        self.sync_load_order();
        // std::map<int, QString>: one name per position, the last one in name order wins.
        let mut by_position: BTreeMap<i64, String> = BTreeMap::new();
        for (name, pos) in self.locked.values() {
            by_position.insert(*pos, name.clone());
        }
        let count = self.plugins.len() as i64;
        let mut target: i64 = 0;
        for (pos, name) in by_position {
            let Some(&row) = self.by_name.get(&key(&name)) else { continue };
            while target < count - 1 && self.load_order_value(self.by_priority[target as usize]) < pos {
                target += 1;
            }
            if target >= count {
                continue;
            }
            if self.plugins[row].priority as i64 != target {
                self.set_plugin_priority(row, target);
                self.plugins[row].load_order = usize::try_from(pos).ok();
                self.sync_load_order();
            }
        }
        self.generate_plugin_indexes();
    }

    /// `PluginList::readLockedOrderFrom`.
    fn read_locked_order(&mut self, data: &[u8]) {
        self.locked.clear();
        let count = self.by_priority.len() as i64;
        for LockedOrder { name, position } in parse_locked_order(data) {
            let Some(&row) = self.by_name.get(&key(&name)) else {
                self.locked.insert(key(&name), (name, position));
                continue;
            };
            if self.plugins[row].force_loaded {
                continue;
            }
            let mut priority = position;
            if priority >= count {
                self.locked.insert(key(&name), (name, priority));
                continue;
            }
            let taken = |locked: &BTreeMap<String, (String, i64)>, p: i64| locked.values().any(|(_, v)| *v == p);
            let free = |list: &Self, p: i64| !list.plugins[list.by_priority[p as usize]].force_loaded && !taken(&list.locked, p);
            if free(self, priority) {
                self.locked.insert(key(&name), (name, priority));
                continue;
            }
            loop {
                priority += 1;
                if priority >= count {
                    break;
                }
                if free(self, priority) {
                    self.locked.insert(key(&name), (name.clone(), priority));
                    break;
                }
            }
            if priority >= count {
                self.locked.insert(key(&name), (name, priority));
            }
        }
    }

    fn rows_of(&self, names: &[String]) -> Result<Vec<usize>> {
        names
            .iter()
            .map(|n| self.by_name.get(&key(n)).copied().ok_or_else(|| Error::NoSuchPlugin { name: n.clone() }))
            .collect()
    }

    /// Enables or disables plugins (`PluginList::setEnabled`): forced plugins are left as
    /// they are. Returns the names whose state changed.
    pub fn set_enabled(&mut self, names: &[String], enabled: bool) -> Result<Vec<String>> {
        let mut changed = Vec::new();
        for row in self.rows_of(names)? {
            let p = &mut self.plugins[row];
            if p.force_loaded || p.force_disabled || p.enabled == enabled {
                continue;
            }
            p.enabled = enabled;
            changed.push(p.name.clone());
        }
        self.after_change();
        Ok(changed)
    }

    /// Enables or disables every plugin (`PluginList::setEnabledAll`, the context menu's
    /// "Enable all" / "Disable all"): forced plugins (the primary ones, a light plugin the
    /// game cannot load) stay as they are. Returns the names whose state changed.
    pub fn set_enabled_all(&mut self, enabled: bool) -> Vec<String> {
        let mut changed = Vec::new();
        for p in &mut self.plugins {
            if p.force_loaded || p.force_disabled || p.enabled == enabled {
                continue;
            }
            p.enabled = enabled;
            changed.push(p.name.clone());
        }
        self.after_change();
        changed
    }

    /// Sends plugins to a priority (`PluginList::sendToPriority`, the context menu's "Send
    /// to..."; `usize::MAX` = the bottom). Primary plugins with force loading don't move; the
    /// others keep MO2's rules (masters above plugins, plugins below their masters, locked
    /// plugins back at their positions).
    pub fn send_to_priority(&mut self, names: &[String], priority: usize) -> Result<()> {
        let rows: Vec<usize> = self.rows_of(names)?.into_iter().filter(|r| !self.plugins[*r].force_loaded).collect();
        if !rows.is_empty() {
            self.change_plugin_priority(rows, i64::try_from(priority).unwrap_or(i64::MAX));
        }
        self.after_change();
        Ok(())
    }

    /// Locks a plugin at its current load order position, or unlocks it
    /// (`PluginList::lockESPIndex`). Forced plugins cannot be locked.
    pub fn set_locked(&mut self, name: &str, lock: bool) -> Result<()> {
        let row = self.rows_of(&[name.to_owned()])?[0];
        let p = &self.plugins[row];
        if lock {
            if !p.force_loaded {
                let pos = self.load_order_value(row);
                self.locked.insert(key(&p.name), (p.name.clone(), pos));
            }
        } else {
            self.locked.remove(&key(&p.name));
        }
        self.after_change();
        Ok(())
    }

    /// What MO2 does after a change before the next refresh shows it: the load order with
    /// locked plugins put back, the indexes, the missing masters.
    fn after_change(&mut self) {
        self.refresh_load_order();
        self.test_masters();
    }

    /// `plugins.txt` as `CreationGamePlugins::writePluginList` writes it: every plugin but
    /// the primary ones, by priority, `*` before enabled ones.
    ///
    /// Gamebryo games (`GamebryoGamePlugins::writeList`): only the enabled plugins, primary
    /// ones included, no marks.
    pub fn plugins_txt(&self) -> Vec<u8> {
        let mut out = format!("{MODLIST_HEADER}\r\n").into_bytes();
        if self.rules.format == PluginListFormat::Gamebryo {
            for p in self.plugins().filter(|p| p.enabled) {
                out.extend_from_slice(p.name.as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            return out;
        }
        for p in self.plugins() {
            if contains(&self.rules.primary, &p.name) {
                continue;
            }
            if p.enabled {
                out.push(b'*');
            }
            out.extend_from_slice(p.name.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out
    }

    /// `loadorder.txt` as `GamebryoGamePlugins::writeLoadOrderList` writes it: every
    /// plugin by priority. `None` when it would be empty (MO2 then keeps the old file).
    pub fn loadorder_txt(&self) -> Option<Vec<u8>> {
        if self.plugins.is_empty() {
            return None;
        }
        let mut out = format!("{MODLIST_HEADER}\r\n").into_bytes();
        for p in self.plugins() {
            out.extend_from_slice(p.name.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        Some(out)
    }

    /// `lockedorder.txt` as `PluginList::writeLockedOrder` writes it: `name|position`, in
    /// name order.
    pub fn lockedorder_txt(&self) -> Vec<u8> {
        let mut out = format!("{MODLIST_HEADER}\r\n").into_bytes();
        for (name, pos) in self.locked.values() {
            out.extend_from_slice(format!("{name}|{pos}\r\n").as_bytes());
        }
        out
    }

    /// Writes the three files into the profile folder (`PluginList::saveTo`), each one only
    /// when its content changed, each through a temporary file and a rename. Returns the
    /// files written.
    pub fn save(&self, profile_dir: &Path) -> Result<Vec<PathBuf>> {
        let mut written = Vec::new();
        let mut put = |name: &str, data: Vec<u8>| -> Result<()> {
            let path = profile_dir.join(name);
            if files::write_if_changed(&path, &data)? {
                written.push(path);
            }
            Ok(())
        };
        // GamebryoGamePlugins::writeList keeps the old file rather than write an empty list.
        let nothing_active = !self.plugins.iter().any(|p| p.enabled);
        if !(self.rules.format == PluginListFormat::Gamebryo && nothing_active) {
            put("plugins.txt", self.plugins_txt())?;
        }
        if let Some(data) = self.loadorder_txt() {
            put("loadorder.txt", data)?;
        }
        put("lockedorder.txt", self.lockedorder_txt())?;
        Ok(written)
    }
}

impl Instance {
    /// The game's rules for this instance's plugin list.
    pub fn plugin_rules(&self) -> PluginRules {
        PluginRules {
            primary: self.primary_plugins(),
            force_enable_core_files: self.ini.value("Settings", "force_enable_core_files").is_none_or(|v| v.to_bool()),
            light_plugins_supported: self.game.light_plugins_supported(),
            format: self.game.plugin_list_format(),
        }
    }

    /// The plugin list of `profile`, given the plugins of its merged Data folder.
    pub fn plugin_list(&self, profile: &Profile, available: Vec<AvailablePlugin>) -> Result<PluginList> {
        let read = |name: &str| -> Result<Option<Vec<u8>>> {
            let path = profile.dir.join(name);
            match std::fs::read(&path) {
                Ok(data) => Ok(Some(data)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(source) => Err(Error::Read { path, source }),
            }
        };
        let files = PluginFiles {
            plugins_txt: read("plugins.txt")?,
            loadorder_txt: read("loadorder.txt")?,
            lockedorder_txt: read("lockedorder.txt")?,
        };
        Ok(PluginList::load(available, self.plugin_rules(), &files))
    }
}

#[cfg(test)]
mod tests;
