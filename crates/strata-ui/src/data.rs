//! The Data tab: the merged `Data` folder with each file's origin (MO2's `FileTreeModel`).
//!
//! Built from the same merged tree the overlay mounts (`strata-vfs`), so the tab shows
//! exactly what the game sees. The shell asks for one folder at a time as the user opens
//! it, so only the visible part is turned into rows.

use std::collections::HashMap;
use std::time::UNIX_EPOCH;

use strata_core::{Instance, Profile};
use strata_game::LayerKind;
use strata_vfs::{NodeId, Tree};

use crate::Result;

/// What the Mod column shows for files of the game itself (`UnmanagedModName`).
pub const UNMANAGED: &str = "<Unmanaged>";

/// One file or folder in the Data tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataEntry {
    pub name: String,
    pub is_dir: bool,
    /// The mod that wins the file, `<Unmanaged>` for the game's own, "Overwrite" for
    /// overwrite. Empty for folders.
    pub origin: String,
    /// Bytes; 0 for folders.
    pub size: u64,
    /// Seconds since 1970; `None` when the file cannot be read.
    pub modified: Option<i64>,
    /// Inside a loaded archive (the "Archives" box), not a loose file.
    pub from_archive: bool,
}

/// The merged tree of a profile, for browsing.
pub struct DataView {
    tree: Tree,
    /// The folder the Data tab shows (`Data` under the Game Root).
    data: Option<NodeId>,
    /// The files of the loaded archives that no loose file hides, by folder (lower case,
    /// `/`-separated, "" = Data); `None` until [`DataView::load_archives`].
    archived: Option<HashMap<String, Vec<DataEntry>>>,
}

impl DataView {
    /// Scans the profile's layers (on a 1,200-mod list: about 0.2 s; run it off the UI
    /// thread). Builds for reading only: nothing is written.
    pub fn build(instance: &Instance, profile: &Profile) -> Result<Self> {
        let tree = crate::profile_tree::merged_tree(instance, profile)?;
        let data = tree.resolve(instance.game.data_dir());
        Ok(Self { tree, data, archived: None })
    }

    /// Reads the file lists of the archives the game loads (the Archives tab's ticked ones)
    /// for the "Archives" box (MO2's archive parsing, `FileTreeModel` with `Archives`). A
    /// loose file wins over an archived one; between archives the one loaded later wins:
    /// the game's default archives first, then by the load order of the plugin each belongs
    /// to. Shown as "<mod> (<archive>)". An archive that cannot be read is left out.
    pub fn load_archives(&mut self, instance: &Instance, profile: &Profile) -> Result<()> {
        let Some(data) = self.data else { return Ok(()) };
        let plugins = crate::plugins::PluginView::build(instance, profile)?;
        let enabled: Vec<(String, usize)> = plugins
            .list()
            .plugins()
            .filter(|p| p.enabled)
            .map(|p| (p.name.rsplit_once('.').map_or(p.name.as_str(), |(stem, _)| stem).to_lowercase(), p.priority))
            .collect();
        let mut loaded: Vec<(i64, crate::plugins::ArchiveRow)> = plugins
            .archives()
            .into_iter()
            .filter(|a| a.loaded)
            .map(|a| {
                let lower = a.name.to_lowercase();
                let order = enabled
                    .iter()
                    .filter(|(stem, _)| !stem.is_empty() && lower.starts_with(stem.as_str()))
                    .map(|(_, p)| i64::try_from(*p).unwrap_or(i64::MAX))
                    .max()
                    .unwrap_or(-1);
                (order, a)
            })
            .collect();
        loaded.sort_by_key(|(order, a)| (*order, a.name.to_lowercase()));
        // path (lower) -> (path as stored, origin label, size); later archives replace
        let mut files: HashMap<String, (String, String, u64)> = HashMap::new();
        for (_, archive) in &loaded {
            let Some(real) = self.tree.lookup(data, &archive.name).and_then(|id| self.tree.real_path(id)) else { continue };
            let Ok(bsa) = strata_archive::GameArchive::open(&real) else { continue };
            let label = format!("{} ({})", archive.origin, archive.name);
            for f in bsa.files() {
                let path = f.path.replace('\\', "/");
                files.insert(path.to_lowercase(), (path, label.clone(), f.stored_size));
            }
        }
        let mut archived: HashMap<String, Vec<DataEntry>> = HashMap::new();
        let mut folders: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        for (path, origin, size) in files.into_values() {
            let mut at = Some(data);
            for part in path.split('/') {
                at = at.and_then(|id| self.tree.lookup(id, part));
            }
            if at.is_some() {
                continue; // a loose file (or folder) of that name wins
            }
            let (folder, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
            archived.entry(folder.to_lowercase()).or_default().push(DataEntry {
                name: name.to_owned(),
                is_dir: false,
                origin,
                size,
                modified: None,
                from_archive: true,
            });
            // every folder above it, so the tree can be opened down to it
            let parts: Vec<&str> = folder.split('/').filter(|p| !p.is_empty()).collect();
            for depth in 0..parts.len() {
                folders.insert((parts[..depth].join("/").to_lowercase(), parts[depth].to_owned()));
            }
        }
        for (parent, name) in folders {
            archived.entry(parent).or_default().push(DataEntry { name, is_dir: true, origin: String::new(), size: 0, modified: None, from_archive: true });
        }
        self.archived = Some(archived);
        Ok(())
    }

    /// How many files the merged Data folder holds (the status line of the tab).
    pub fn file_count(&self) -> usize {
        let Some(data) = self.data else { return 0 };
        let prefix = format!("{}/", self.tree.path(data).to_lowercase());
        self.tree.files().iter().filter(|(path, _)| path.to_lowercase().starts_with(&prefix)).count()
    }

    /// The entries of the folder `path` (relative to Data, `/`-separated, any case; "" is
    /// Data itself): folders first, then files, each by name without regard to case, as
    /// MO2's tree sorts them. An unknown path gives no entries.
    pub fn children(&self, path: &str) -> Vec<DataEntry> {
        self.query(Area::Data, path, &Query::default())
    }

    /// The entries of a folder of `area` that pass `query`: a file by its own name, a folder
    /// when something below it passes (FileTreeModel's filter and "conflicts only").
    ///
    /// The Root tab (Leet change 5) shows only what the mods' `Root` folders bring: the
    /// game folder minus `Data`, files of the game itself and of overwrite left out.
    pub fn query(&self, area: Area, path: &str, query: &Query) -> Vec<DataEntry> {
        let base = match area {
            Area::Data => self.data,
            Area::Root => Some(strata_vfs::tree::ROOT),
        };
        let Some(base) = base else { return Vec::new() };
        let folder = path.split('/').filter(|p| !p.is_empty()).try_fold(base, |at, name| self.tree.lookup(at, name));
        let filter = query.filter.to_lowercase();
        let Some(folder) = folder else {
            // a folder only archives have
            if area == Area::Data && query.archives && !query.conflicts_only {
                let mut entries = self.archived_entries(path, &filter);
                entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase()));
                return entries;
            }
            return Vec::new();
        };
        let mut entries: Vec<DataEntry> = self
            .tree
            .children(folder)
            .filter(|(id, _)| !(area == Area::Root && folder == base && Some(*id) == self.data))
            .filter(|(id, _)| self.passes(*id, area, &filter, query.conflicts_only))
            .map(|(id, node)| self.entry(id, node))
            .collect();
        if area == Area::Data && query.archives && !query.conflicts_only {
            entries.extend(self.archived_entries(path, &filter));
        }
        entries.sort_by_cached_key(|e| (!e.is_dir, e.name.to_lowercase()));
        entries.dedup_by(|a, b| a.is_dir && b.is_dir && a.name.eq_ignore_ascii_case(&b.name));
        entries
    }

    /// What the loaded archives add to the folder `path` (files no loose file hides, and the
    /// folders that lead to them), filtered as the tree is.
    fn archived_entries(&self, path: &str, filter: &str) -> Vec<DataEntry> {
        let Some(archived) = &self.archived else { return Vec::new() };
        let key = path.split('/').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("/").to_lowercase();
        let below = |folder: &str| {
            let prefix = format!("{folder}/");
            archived.iter().any(|(k, list)| {
                (k == folder || k.starts_with(&prefix)) && list.iter().any(|e| !e.is_dir && e.name.to_lowercase().contains(filter))
            })
        };
        archived
            .get(&key)
            .into_iter()
            .flatten()
            .filter(|e| {
                if e.is_dir {
                    let folder = if key.is_empty() { e.name.to_lowercase() } else { format!("{key}/{}", e.name.to_lowercase()) };
                    filter.is_empty() || below(&folder)
                } else {
                    filter.is_empty() || e.name.to_lowercase().contains(filter)
                }
            })
            .cloned()
            .collect()
    }

    /// Whether a file passes, or a folder holds a file that does.
    fn passes(&self, id: NodeId, area: Area, filter: &str, conflicts_only: bool) -> bool {
        let Some(node) = self.tree.node(id) else { return false };
        if node.is_dir() {
            return self.tree.children(id).any(|(child, _)| self.passes(child, area, filter, conflicts_only));
        }
        // OrganizerCore::rebuildRootStructure: only the mods' Root folders, not the game
        // folder itself nor overwrite
        if area == Area::Root && self.tree.layer_of(id).is_some_and(|l| matches!(l.kind, LayerKind::Game | LayerKind::Overwrite)) {
            return false;
        }
        // a file a lower layer also has: MO2 lists the losers as alternatives
        (!conflicts_only || node.shadowed) && (filter.is_empty() || node.name.to_lowercase().contains(filter))
    }

    fn entry(&self, id: NodeId, node: &strata_vfs::tree::Node) -> DataEntry {
        let is_dir = node.is_dir();
        let origin = if is_dir {
            String::new()
        } else {
            match self.tree.layer_of(id) {
                Some(layer) if layer.kind == LayerKind::Game => UNMANAGED.to_owned(),
                Some(layer) if layer.kind == LayerKind::Overwrite => "Overwrite".to_owned(),
                Some(layer) => layer.id.clone(),
                None => String::new(),
            }
        };
        let meta = if is_dir { None } else { self.tree.real_path(id).and_then(|p| std::fs::metadata(p).ok()) };
        DataEntry {
            name: node.name.to_string(),
            is_dir,
            origin,
            size: meta.as_ref().map_or(0, |m| m.len()),
            modified: meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .and_then(|d| i64::try_from(d.as_secs()).ok()),
            from_archive: false,
        }
    }
}

/// Which of the two tabs (`FileTreeModel::Source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Data,
    Root,
}

/// The tab's filter box and "conflicts only".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub filter: String,
    pub conflicts_only: bool,
    /// The "Archives" box: also the files inside loaded archives ([`DataView::load_archives`]).
    pub archives: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_instance::TestInstance;

    fn put(dir: &std::path::Path, rel: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    #[test]
    fn data_and_root_tabs_with_filter_and_conflicts_only() {
        let t = TestInstance::new("+B\r\n+A\r\n");
        put(&t.folder("A"), "textures/same.dds");
        put(&t.folder("B"), "textures/same.dds");
        put(&t.folder("A"), "meshes/only.nif");
        put(&t.folder("A"), "Root/enbseries.ini");
        put(&t.path("Game Root"), "SkyrimSE.exe");
        put(&t.path("overwrite"), "Root/d3dx9_42.log");
        let (instance, profile) = t.open();
        let view = DataView::build(&instance, &profile).unwrap();

        let names = |e: Vec<DataEntry>| e.into_iter().map(|e| e.name).collect::<Vec<_>>();
        assert_eq!(names(view.children("")), ["meshes", "textures"]);
        let conflicts = Query { conflicts_only: true, ..Query::default() };
        assert_eq!(names(view.query(Area::Data, "", &conflicts)), ["textures"]);
        let only = Query { filter: "ONLY".into(), ..Query::default() };
        assert_eq!(names(view.query(Area::Data, "", &only)), ["meshes"]);
        assert_eq!(view.children("textures")[0].origin, "B", "B has the higher priority");

        // Root: the mod's Root files, not the game's own, not Data
        assert_eq!(names(view.query(Area::Root, "", &Query::default())), ["enbseries.ini"]);
    }

    #[test]
    fn the_archives_box_adds_what_loaded_archives_hold_under_loose_files() {
        use strata_archive::pack::{PackFile, PackOptions, write_bsa};
        let t = TestInstance::new("+B\r\n+A\r\n");
        put(&t.folder("A"), "textures/same.dds");
        put(&t.folder("A"), "src/same.dds");
        put(&t.folder("A"), "src/packed.dds");
        put(&t.folder("A"), "src/deep.nif");
        let file = |path: &str, src: &str| PackFile { path: path.into(), source: t.folder("A").join(src) };
        let bsa = |name: &str, files: &[PackFile]| write_bsa(&t.folder("A").join(name), files, PackOptions { version: 105, compress: false }).unwrap();
        bsa("a.bsa", &[file("textures\\same.dds", "src/same.dds"), file("textures\\packed.dds", "src/packed.dds"), file("meshes\\deep\\x.nif", "src/deep.nif")]);
        bsa("unloaded.bsa", &[file("textures\\never.dds", "src/same.dds")]);
        std::fs::write(t.folder("A").join("a.esp"), crate::plugins::tests_plugin_bytes()).unwrap();
        std::fs::write(t.path("profiles/Default/plugins.txt"), "*a.esp\r\n").unwrap();
        let (instance, profile) = t.open();
        let mut view = DataView::build(&instance, &profile).unwrap();
        view.load_archives(&instance, &profile).unwrap();
        let with = Query { archives: true, ..Query::default() };
        let names = |e: Vec<DataEntry>| e.into_iter().map(|e| e.name).collect::<Vec<_>>();
        let textures = view.query(Area::Data, "textures", &with);
        assert_eq!(names(textures.clone()), ["packed.dds", "same.dds"]);
        assert_eq!(textures[0].origin, "A (a.bsa)");
        assert_eq!(textures[1].origin, "A", "the loose file wins");
        assert!(!names(view.query(Area::Data, "textures", &Query::default())).contains(&"packed.dds".to_owned()));
        // folders only the archive has open down to the file
        assert!(names(view.query(Area::Data, "", &with)).contains(&"meshes".to_owned()));
        assert_eq!(names(view.query(Area::Data, "meshes/deep", &with)), ["x.nif"]);
        let filtered = Query { filter: "x.nif".into(), archives: true, ..Query::default() };
        assert_eq!(names(view.query(Area::Data, "", &filtered)), ["meshes"]);
    }
}
