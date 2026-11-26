//! The merged tree: every layer's files as one folder, and the changes a mounted game makes.
//!
//! Nothing here knows FUSE. The tree answers "which real file is this path", and carries out
//! writes the way `spec/03-vfs.md` wants them: new files and copies of changed files go to the
//! overwrite layer, deletions of lower-layer files become whiteouts.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use strata_game::{Layer, LayerKind, Mapping};

mod journal;
mod rebase;
mod rename;
pub use rebase::Refreshed;
use journal::Step;

/// Index of a node in the tree. The root is 0.
pub type NodeId = u32;

/// The root folder of the mount.
pub const ROOT: NodeId = 0;

/// Files and folders the overlay never shows (MO2's `skip_file_suffixes` and `skip_directories`).
#[derive(Debug, Clone)]
pub struct Options {
    /// File name endings to hide, lower case. MO2's default: `.mohidden`.
    pub skip_file_suffixes: Vec<String>,
    /// Folder names to hide, lower case. MO2's default: `.git`.
    pub skip_directories: Vec<String>,
    /// Delete what an interrupted copy-up left in overwrite. Only a mount sets this: a
    /// build for reading (`strata tree`) never writes.
    pub clean_temp_files: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { skip_file_suffixes: vec![".mohidden".into()], skip_directories: vec![".git".into()], clean_temp_files: false }
    }
}

/// Where a file's bytes are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSource {
    /// Index into the tree's layers.
    pub layer: u16,
    /// Index into the tree's table of real folders.
    dir: u32,
    /// The file's name on disk when it is spelled differently from the node's name.
    real_name: Option<Box<str>>,
    /// Edits go to this file itself instead of a copy in overwrite (`Mapping::write_back`).
    pub write_back: bool,
}

/// What a node is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Dir {
        /// Case-folded name to node.
        children: HashMap<Box<str>, NodeId>,
    },
    File(FileSource),
    /// A whiteout or a deleted node: the path does not exist, whatever lower layers hold.
    Gone,
}

/// One file or folder of the merged tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The name as shown: the first spelling seen, lowest layer first.
    pub name: Box<str>,
    pub parent: NodeId,
    pub kind: Kind,
    /// A lower layer also has this path, so deleting it needs a whiteout.
    pub shadowed: bool,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, Kind::Dir { .. })
    }
}

/// The merged tree.
#[derive(Debug)]
pub struct Tree {
    nodes: Vec<Node>,
    layers: Vec<Layer>,
    real_dirs: Vec<PathBuf>,
    /// The layers programs' writes may land in, highest first (none: a read-only tree).
    writable: Vec<u16>,
    /// The first `Overwrite` layer from the bottom: whiteouts hide what is below it.
    whiteout_floor: Option<u16>,
    /// Folders that exist in the writable layers: (layer, case-folded mount path) to real folder.
    writable_dirs: HashMap<(u16, String), PathBuf>,
    /// Whiteouts: case-folded mount path to the path as written in the file.
    whiteouts: BTreeMap<String, String>,
    whiteout_file: Option<PathBuf>,
    /// Where changes of several steps are written down first (`journal.rs`); none for a
    /// tree without a whiteout file or whose writable layers are only reachable through
    /// this process's handles.
    journal: Option<PathBuf>,
}

/// Test builds only (feature `crash-hooks`): stops here for good when `STRATA_CRASH_AT`
/// names this point (`name` or `name#n` for its n-th time), after writing the point's name
/// to the file `STRATA_CRASH_MARK`, so a test can kill vfsd at that step.
#[cfg(feature = "crash-hooks")]
pub(crate) fn crash_point(name: &str) {
    use std::sync::Mutex;
    static HITS: Mutex<BTreeMap<String, u32>> = Mutex::new(BTreeMap::new());
    let Ok(wanted) = std::env::var("STRATA_CRASH_AT") else { return };
    let (point, nth) = match wanted.split_once('#') {
        Some((point, n)) => (point.to_owned(), n.parse().unwrap_or(1)),
        None => (wanted.clone(), 1),
    };
    if point != name {
        return;
    }
    let hits = {
        let mut table = HITS.lock().unwrap_or_else(|e| e.into_inner());
        let hits = table.entry(point).or_default();
        *hits += 1;
        *hits
    };
    if hits == nth {
        if let Some(mark) = std::env::var_os("STRATA_CRASH_MARK") {
            let _ = std::fs::write(mark, name);
        }
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
}

#[cfg(not(feature = "crash-hooks"))]
#[inline(always)]
pub(crate) fn crash_point(_name: &str) {}

/// Reads a whiteout file: folded path to the path as written. A missing file is empty.
pub(crate) fn read_whiteouts(file: &Path) -> io::Result<BTreeMap<String, String>> {
    let mut whiteouts = BTreeMap::new();
    match std::fs::read_to_string(file) {
        Ok(text) => {
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                whiteouts.insert(fold(line), line.to_owned());
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(whiteouts)
}

/// Writes a whiteout file whole (a temp file renamed over).
pub(crate) fn write_whiteouts(file: &Path, whiteouts: &BTreeMap<String, String>) -> io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = String::from(
        "# Files deleted through Strata's overlay that still exist in a mod or the game folder.\n\
         # One path per line, relative to the game folder. Remove a line to bring the file back.\n",
    );
    for path in whiteouts.values() {
        text.push_str(path);
        text.push('\n');
    }
    let temp = file.with_extension("txt.tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, file)
}

/// Case folding for names: the overlay is case-insensitive and case-preserving.
///
/// Character by character, as Windows compares names (one UTF-16 unit at a time through its
/// case table; MO2's `CharLowerBuffW` works the same way): no context, so `Σ` is always
/// `σ` (`str::to_lowercase` makes a word-final one `ς`, and "ΟΔΟΣ" would then miss
/// "οδοσ"), and a character whose lower case is longer than itself (`İ`) stays as it is.
/// The characters where this still differs from Windows' table are listed in
/// `examples/casefold.rs`; no name in the corpora has one.
pub fn fold(name: &str) -> String {
    if name.is_ascii() {
        return name.to_ascii_lowercase();
    }
    name.chars()
        .map(|c| {
            let mut lower = c.to_lowercase();
            match (lower.next(), lower.next()) {
                (Some(l), None) => l,
                _ => c,
            }
        })
        .collect()
}

fn err(code: i32) -> io::Error {
    io::Error::from_raw_os_error(code)
}

/// One scanned folder of a layer.
pub(crate) struct ScanDir {
    /// Where it appears in the mount ("" = the root), in this layer's spelling.
    pub(crate) target: String,
    real: PathBuf,
    pub(crate) files: Vec<String>,
    write_back: bool,
}

/// A file made by an interrupted copy-up; never shown, removed at the next build.
const TEMP_PREFIX: &str = ".strata-tmp-";

fn find_dir_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let exact = parent.join(name);
    if exact.is_dir() {
        return Some(exact);
    }
    let wanted = fold(name);
    std::fs::read_dir(parent).ok()?.filter_map(|e| e.ok()).find_map(|e| {
        let path = e.path();
        (fold(&e.file_name().to_string_lossy()) == wanted && path.is_dir()).then_some(path)
    })
}

/// Every folder of `layer` its rules place in the mount, with the files in each (also used by
/// the conflict index, `conflicts.rs`).
pub(crate) fn scan_layer(layer: &Layer, options: &Options) -> Vec<ScanDir> {
    let mut out = Vec::new();
    for rule in &layer.rules {
        // The folder the rule maps, found without regard to case.
        let mut real = layer.source.clone();
        let mut found = true;
        for part in rule.from.split('/').filter(|p| !p.is_empty()) {
            match find_dir_ci(&real, part) {
                Some(next) => real = next,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if found {
            scan_dir(&real, rule.to.trim_matches('/'), true, rule, layer, options, &mut out);
        }
    }
    out
}

fn scan_dir(
    real: &Path,
    target: &str,
    top: bool,
    rule: &Mapping,
    layer: &Layer,
    options: &Options,
    out: &mut Vec<ScanDir>,
) {
    let Ok(entries) = std::fs::read_dir(real) else { return };
    let mut files = Vec::new();
    let mut subdirs = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        // Names that are not UTF-8 cannot be matched without regard to case: leave them out.
        let Ok(name) = entry.file_name().into_string() else { continue };
        let folded = fold(&name);
        if top && !rule.covers(&name) {
            continue;
        }
        let Ok(mut file_type) = entry.file_type() else { continue };
        if file_type.is_symlink() {
            match std::fs::metadata(entry.path()) {
                Ok(meta) => file_type = meta.file_type(),
                Err(_) => continue, // a broken link
            }
        }
        if file_type.is_dir() {
            if !options.skip_directories.contains(&folded) {
                subdirs.push(name);
            }
        } else if file_type.is_file() {
            let hidden = options.skip_file_suffixes.iter().any(|s| folded.ends_with(s.as_str()));
            let temp = layer.kind.is_writable() && name.starts_with(TEMP_PREFIX);
            if temp && options.clean_temp_files {
                let _ = std::fs::remove_file(entry.path());
            } else if !hidden && !temp {
                files.push(name);
            }
        }
    }
    out.push(ScanDir { target: target.to_owned(), real: real.to_owned(), files, write_back: rule.write_back });
    for name in subdirs {
        let child_target = if target.is_empty() { name.clone() } else { format!("{target}/{name}") };
        scan_dir(&real.join(&name), &child_target, false, rule, layer, options, out);
    }
}

impl Tree {
    /// Builds the merged tree from `layers` (lowest priority first).
    ///
    /// `whiteout_file` is `<instance>/strata/whiteouts.txt`: read now, rewritten on every
    /// change. `None` keeps whiteouts in memory only.
    pub fn build(layers: Vec<Layer>, options: Options, whiteout_file: Option<PathBuf>) -> io::Result<Self> {
        if layers.len() > usize::from(u16::MAX) {
            return Err(io::Error::other("too many layers"));
        }
        // A mount first finishes what a crash interrupted (a build for reading never writes).
        if let (Some(file), true) = (&whiteout_file, options.clean_temp_files) {
            journal::replay(file)?;
        }
        let scans: Vec<Vec<ScanDir>> = layers.par_iter().map(|layer| scan_layer(layer, &options)).collect();

        let whiteouts = match &whiteout_file {
            Some(file) => read_whiteouts(file)?,
            None => BTreeMap::new(),
        };
        let loaded = whiteouts.len();
        let journal = whiteout_file.as_deref().map(journal::path_for).filter(|_| {
            layers.iter().filter(|l| l.kind.is_writable()).all(|l| !l.source.starts_with("/proc/self/fd"))
        });

        let whiteout_floor = layers.iter().position(|l| l.kind == LayerKind::Overwrite).map(|i| i as u16);
        let writable: Vec<u16> =
            layers.iter().enumerate().rev().filter(|(_, l)| l.kind.is_writable()).map(|(i, _)| i as u16).collect();
        let mut tree = Self {
            nodes: vec![Node {
                name: "".into(),
                parent: ROOT,
                kind: Kind::Dir { children: HashMap::new() },
                shadowed: false,
            }],
            layers,
            real_dirs: Vec::new(),
            writable,
            whiteout_floor,
            writable_dirs: HashMap::new(),
            whiteouts,
            whiteout_file,
            journal,
        };

        let mut whiteouts_applied = false;
        for (index, scan) in scans.into_iter().enumerate() {
            let index = index as u16;
            if Some(index) == whiteout_floor {
                tree.apply_whiteouts();
                whiteouts_applied = true;
            }
            for dir in scan {
                tree.merge_dir(index, dir);
            }
        }
        if !whiteouts_applied {
            tree.apply_whiteouts();
        }
        // A mount writes back the whiteouts the build ended (their path exists in overwrite
        // again, e.g. after a crash between making a file and saving the list).
        if options.clean_temp_files && tree.whiteouts.len() != loaded {
            tree.save_whiteouts()?;
        }
        Ok(tree)
    }

    fn apply_whiteouts(&mut self) {
        let paths: Vec<String> = self.whiteouts.keys().cloned().collect();
        for path in paths {
            if let Some(id) = self.resolve(&path)
                && id != ROOT {
                    self.nodes[id as usize].kind = Kind::Gone;
                    self.nodes[id as usize].shadowed = true;
                }
        }
    }

    fn merge_dir(&mut self, layer: u16, dir: ScanDir) {
        let is_writable = self.writable.contains(&layer);
        let above_floor = self.whiteout_floor.is_some_and(|floor| layer >= floor);
        // Find or make the folder's node.
        let mut at = ROOT;
        let mut folded_path = String::new();
        for part in dir.target.split('/').filter(|p| !p.is_empty()) {
            let key = fold(part);
            if !folded_path.is_empty() {
                folded_path.push('/');
            }
            folded_path.push_str(&key);
            at = match self.child(at, &key) {
                Some(id) => {
                    let node = &mut self.nodes[id as usize];
                    if !node.is_dir() {
                        // A folder over a lower layer's file (or a whiteout): the folder wins.
                        node.kind = Kind::Dir { children: HashMap::new() };
                        node.shadowed = true;
                    }
                    id
                }
                None => self.add_child(at, part, Kind::Dir { children: HashMap::new() }, false),
            };
            if !is_writable {
                // For a folder, `shadowed` means: a layer that is not written to has it.
                self.nodes[at as usize].shadowed = true;
            }
        }
        if is_writable {
            self.writable_dirs.insert((layer, folded_path.clone()), dir.real.clone());
        }

        let dir_id = self.real_dirs.len() as u32;
        self.real_dirs.push(dir.real);
        for name in dir.files {
            let key = fold(&name);
            match self.child(at, &key) {
                Some(id) => {
                    let was_gone = self.nodes[id as usize].kind == Kind::Gone;
                    let node = &mut self.nodes[id as usize];
                    let real_name = (*node.name != *name).then(|| name.as_str().into());
                    node.kind = Kind::File(FileSource { layer, dir: dir_id, real_name, write_back: dir.write_back });
                    node.shadowed = true;
                    if was_gone && above_floor {
                        // The path exists in overwrite again: its whiteout has done its job.
                        let path = if folded_path.is_empty() { key.clone() } else { format!("{folded_path}/{key}") };
                        self.whiteouts.remove(&path);
                    }
                }
                None => {
                    let source = FileSource { layer, dir: dir_id, real_name: None, write_back: dir.write_back };
                    self.add_child(at, &name, Kind::File(source), false);
                }
            }
        }
    }

    fn add_child(&mut self, parent: NodeId, name: &str, kind: Kind, shadowed: bool) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node { name: name.into(), parent, kind, shadowed });
        if let Kind::Dir { children } = &mut self.nodes[parent as usize].kind {
            children.insert(fold(name).into(), id);
        }
        id
    }

    /// The child entry for an already folded name, whiteouts included.
    fn child(&self, parent: NodeId, folded: &str) -> Option<NodeId> {
        match &self.nodes.get(parent as usize)?.kind {
            Kind::Dir { children } => children.get(folded).copied(),
            _ => None,
        }
    }

    /// The layers the tree was built from.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// How many nodes the tree holds (deleted ones included).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    /// The node, unless it is deleted or out of range.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id as usize).filter(|n| n.kind != Kind::Gone)
    }

    /// The entry named `name` (any case) in folder `parent`.
    pub fn lookup(&self, parent: NodeId, name: &str) -> Option<NodeId> {
        self.child(parent, &fold(name)).filter(|id| self.node(*id).is_some())
    }

    /// The node at a mount-relative path (`Data/Meshes/x.nif`, any case; "" = the root).
    pub fn resolve(&self, path: &str) -> Option<NodeId> {
        path.split('/').filter(|p| !p.is_empty()).try_fold(ROOT, |at, part| self.lookup(at, part))
    }

    /// The entries of a folder: `(id, node)`, in no particular order.
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = (NodeId, &Node)> {
        let children = match self.nodes.get(id as usize).map(|n| &n.kind) {
            Some(Kind::Dir { children }) => Some(children),
            _ => None,
        };
        children.into_iter().flat_map(|c| c.values()).filter_map(|id| self.node(*id).map(|n| (*id, n)))
    }

    /// The node's mount-relative path, as shown.
    pub fn path(&self, mut id: NodeId) -> String {
        let mut parts = Vec::new();
        while id != ROOT {
            let node = &self.nodes[id as usize];
            parts.push(&*node.name);
            id = node.parent;
        }
        parts.reverse();
        parts.join("/")
    }

    /// The real file behind a file node.
    pub fn real_path(&self, id: NodeId) -> Option<PathBuf> {
        let node = self.node(id)?;
        match &node.kind {
            Kind::File(src) => {
                Some(self.real_dirs[src.dir as usize].join(src.real_name.as_deref().unwrap_or(&node.name)))
            }
            _ => None,
        }
    }

    /// The layer a file comes from.
    pub fn layer_of(&self, id: NodeId) -> Option<&Layer> {
        match &self.node(id)?.kind {
            Kind::File(src) => self.layers.get(usize::from(src.layer)),
            _ => None,
        }
    }

    /// Every file as `(mount-relative path, node id)`, sorted by path without regard to case.
    pub fn files(&self) -> Vec<(String, NodeId)> {
        let mut out = Vec::new();
        let mut stack = vec![(ROOT, String::new())];
        while let Some((id, prefix)) = stack.pop() {
            for (child, node) in self.children(id) {
                let path = if prefix.is_empty() { node.name.to_string() } else { format!("{prefix}/{}", node.name) };
                if node.is_dir() {
                    stack.push((child, path));
                } else {
                    out.push((path, child));
                }
            }
        }
        out.sort_by_cached_key(|(path, _)| fold(path));
        out
    }

    // ---- changes -------------------------------------------------------------------

    fn is_writable(&self, layer: u16) -> bool {
        self.writable.contains(&layer)
    }

    /// Whether `layer`'s rules place an entry `name` of folder `parent` in that layer.
    fn accepts(&self, layer: u16, parent: NodeId, name: &str) -> bool {
        let rules = &self.layers[usize::from(layer)].rules;
        let parent_path = fold(&self.path(parent));
        let full = if parent_path.is_empty() { fold(name) } else { format!("{parent_path}/{}", fold(name)) };
        // A rule that maps straight to this entry (`"" -> Data`, `saves -> Saves`).
        if rules.iter().any(|r| fold(r.to.trim_matches('/')) == full) {
            return true;
        }
        // Otherwise the rule covering `parent`: the one with the longest `to` under which it lies.
        let covering = rules
            .iter()
            .filter(|r| {
                let to = fold(r.to.trim_matches('/'));
                to.is_empty() || parent_path == to || parent_path.starts_with(&format!("{to}/"))
            })
            .max_by_key(|r| r.to.trim_matches('/').len());
        match covering {
            Some(rule) => {
                // The rule's own lists decide, by the entry directly inside the rule's
                // folder that the new path lies in (itself, or the folder it is under).
                let to = fold(rule.to.trim_matches('/'));
                let below = if to.is_empty() { full.as_str() } else { &full[to.len() + 1..] };
                rule.covers(below.split('/').next().unwrap_or(below))
            }
            None => false,
        }
    }

    /// The layer a new entry `name` in folder `parent` goes to: the highest writable layer
    /// whose rules take it. `EROFS` without writable layers, `EACCES` when none takes it.
    fn target_layer(&self, parent: NodeId, name: &str) -> io::Result<u16> {
        if self.writable.is_empty() {
            return Err(err(libc::EROFS));
        }
        self.writable.iter().copied().find(|layer| self.accepts(*layer, parent, name)).ok_or_else(|| err(libc::EACCES))
    }

    /// The real folder in `layer` for the folder node `id`, created if needed. Every level
    /// made is remembered, so `rmdir` finds it again.
    fn writable_dir_for(&mut self, layer: u16, id: NodeId) -> io::Result<PathBuf> {
        let folded = fold(&self.path(id));
        if let Some(real) = self.writable_dirs.get(&(layer, folded.clone())) {
            return Ok(real.clone());
        }
        let node = &self.nodes[id as usize];
        let (name, parent) = (node.name.to_string(), node.parent);
        let direct = self.layers[usize::from(layer)].rules.iter().any(|r| fold(r.to.trim_matches('/')) == folded);
        let real = if direct || id == ROOT {
            self.writable_dir_path(layer, id)?
        } else {
            if !self.accepts(layer, parent, &name) {
                return Err(err(libc::EACCES));
            }
            let parent_real = self.writable_dir_for(layer, parent)?;
            find_dir_ci(&parent_real, &name).unwrap_or_else(|| parent_real.join(&name))
        };
        std::fs::create_dir_all(&real)?;
        self.writable_dirs.insert((layer, folded), real.clone());
        Ok(real)
    }

    /// Where the folder node `id` is or would be in `layer`, without making anything.
    /// `EACCES` when the layer's rules do not take it.
    fn writable_dir_path(&self, layer: u16, id: NodeId) -> io::Result<PathBuf> {
        let folded = fold(&self.path(id));
        if let Some(real) = self.writable_dirs.get(&(layer, folded.clone())) {
            return Ok(real.clone());
        }
        let source = self.layers[usize::from(layer)].source.clone();
        // A rule may map straight to this folder (`"" -> Data`): then it is the rule's source.
        let direct = self.layers[usize::from(layer)]
            .rules
            .iter()
            .find(|r| fold(r.to.trim_matches('/')) == folded)
            .map(|r| r.from.clone());
        match direct {
            Some(from) => {
                let mut real = source;
                for part in from.split('/').filter(|p| !p.is_empty()) {
                    real = find_dir_ci(&real, part).unwrap_or_else(|| real.join(part));
                }
                Ok(real)
            }
            None if id == ROOT => Err(err(libc::EACCES)),
            None => {
                let node = &self.nodes[id as usize];
                if !self.accepts(layer, node.parent, &node.name) {
                    return Err(err(libc::EACCES));
                }
                let parent_real = self.writable_dir_path(layer, node.parent)?;
                Ok(find_dir_ci(&parent_real, &node.name).unwrap_or_else(|| parent_real.join(&*node.name)))
            }
        }
    }

    fn save_whiteouts(&self) -> io::Result<()> {
        match &self.whiteout_file {
            Some(file) => write_whiteouts(file, &self.whiteouts),
            None => Ok(()),
        }
    }

    fn register_dir(&mut self, real: PathBuf) -> u32 {
        if let Some(i) = self.real_dirs.iter().rposition(|d| *d == real) {
            return i as u32;
        }
        self.real_dirs.push(real);
        (self.real_dirs.len() - 1) as u32
    }

    /// Detaches `id` from its parent's entries.
    fn detach(&mut self, id: NodeId) {
        let (parent, key) = {
            let node = &self.nodes[id as usize];
            (node.parent, fold(&node.name))
        };
        if let Kind::Dir { children } = &mut self.nodes[parent as usize].kind
            && children.get(key.as_str()) == Some(&id) {
                children.remove(key.as_str());
            }
    }

    fn attach(&mut self, id: NodeId, parent: NodeId) {
        let key: Box<str> = fold(&self.nodes[id as usize].name).into();
        if let Kind::Dir { children } = &mut self.nodes[parent as usize].kind {
            children.insert(key, id);
        }
    }

    /// Makes room for a new entry `name` in `parent`: fails if one exists, and reports
    /// whether a read-only layer has that path (a whiteout stood there).
    fn claim(&mut self, parent: NodeId, name: &str) -> io::Result<bool> {
        if !self.node(parent).is_some_and(Node::is_dir) {
            return Err(err(libc::ENOTDIR));
        }
        if name.is_empty() || name.contains('/') || name.starts_with(TEMP_PREFIX) {
            return Err(err(libc::EINVAL));
        }
        match self.child(parent, &fold(name)) {
            Some(id) if self.nodes[id as usize].kind != Kind::Gone => Err(err(libc::EEXIST)),
            Some(id) => {
                self.detach(id);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Adds a new file `name` to folder `parent` and creates it, empty, with `mode`.
    /// Returns the node and the real path.
    ///
    /// The file exists before a whiteout of its path goes: a crash in between leaves both,
    /// and the next build shows the file (`merge_dir`).
    pub fn create_file(&mut self, parent: NodeId, name: &str, mode: u32) -> io::Result<(NodeId, PathBuf)> {
        use std::os::unix::fs::OpenOptionsExt;
        if self.lookup(parent, name).is_some() {
            return Err(err(libc::EEXIST));
        }
        if name.is_empty() || name.contains('/') || name.starts_with(TEMP_PREFIX) {
            return Err(err(libc::EINVAL));
        }
        let layer = self.target_layer(parent, name)?;
        let real_dir = self.writable_dir_for(layer, parent)?;
        let real = real_dir.join(name);
        std::fs::File::options().write(true).create(true).truncate(true).mode(mode).open(&real)?;
        let shadowed = match self.claim(parent, name) {
            Ok(shadowed) => shadowed,
            Err(e) => {
                let _ = std::fs::remove_file(&real);
                return Err(e);
            }
        };
        let dir = self.register_dir(real_dir);
        let source = FileSource { layer, dir, real_name: None, write_back: false };
        let id = self.add_child(parent, name, Kind::File(source), shadowed);
        if shadowed {
            crash_point("create");
            // A file whiteout ends when the file exists again.
            let path = fold(&self.path(id));
            if self.whiteouts.remove(&path).is_some() {
                self.save_whiteouts()?;
            }
        }
        Ok((id, real))
    }

    /// Adds a new folder `name` to folder `parent`, creating it on disk.
    pub fn create_dir(&mut self, parent: NodeId, name: &str) -> io::Result<NodeId> {
        if self.lookup(parent, name).is_some() {
            return Err(err(libc::EEXIST));
        }
        let layer = self.target_layer(parent, name)?;
        let shadowed = self.claim(parent, name)?;
        let id = self.add_child(parent, name, Kind::Dir { children: HashMap::new() }, shadowed);
        if let Err(e) = self.writable_dir_for(layer, id) {
            self.detach(id);
            self.nodes[id as usize].kind = Kind::Gone;
            return Err(e);
        }
        Ok(id)
    }

    /// Makes file `id` writable: a file of a read-only layer is copied to a writable one
    /// first (unless its rule says edits go back to the file itself). Returns the real path.
    pub fn make_writable(&mut self, id: NodeId) -> io::Result<PathBuf> {
        let current = self.real_path(id).ok_or_else(|| err(libc::ENOENT))?;
        let Some(Kind::File(src)) = self.node(id).map(|n| n.kind.clone()) else { return Err(err(libc::EISDIR)) };
        if self.writable.is_empty() {
            return Err(err(libc::EROFS));
        }
        if self.is_writable(src.layer) || src.write_back {
            return Ok(current);
        }
        let (name, parent) = {
            let node = &self.nodes[id as usize];
            (node.name.to_string(), node.parent)
        };
        let layer = self.target_layer(parent, &name)?;
        let real_dir = self.writable_dir_for(layer, parent)?;
        let target = real_dir.join(&name);
        copy_file(&current, &target)?;
        let dir = self.register_dir(real_dir);
        let node = &mut self.nodes[id as usize];
        node.kind = Kind::File(FileSource { layer, dir, real_name: None, write_back: false });
        node.shadowed = true;
        Ok(target)
    }

    /// Hides `id` and, when a read-only layer still has the path, records a whiteout.
    fn remove_node(&mut self, id: NodeId, lower_has_it: bool) -> io::Result<()> {
        if lower_has_it {
            let path = self.path(id);
            self.whiteouts.insert(fold(&path), path);
            self.nodes[id as usize].kind = Kind::Gone;
            self.nodes[id as usize].shadowed = true;
            self.save_whiteouts()
        } else {
            self.detach(id);
            self.nodes[id as usize].kind = Kind::Gone;
            Ok(())
        }
    }

    /// Deletes file `name` from folder `parent`. A file in a writable layer is really
    /// deleted; a file of a mod or the game is hidden by a whiteout and stays on disk.
    pub fn unlink(&mut self, parent: NodeId, name: &str) -> io::Result<()> {
        let id = self.lookup(parent, name).ok_or_else(|| err(libc::ENOENT))?;
        let Kind::File(src) = self.nodes[id as usize].kind.clone() else { return Err(err(libc::EISDIR)) };
        if self.writable.is_empty() {
            return Err(err(libc::EROFS));
        }
        let ours = self.is_writable(src.layer);
        let lower_has_it = !ours || self.nodes[id as usize].shadowed;
        // The whiteout first: a crash before the file goes leaves both, which the next
        // build reads as "not deleted"; the other order would show the lower file.
        let path = self.path(id);
        if lower_has_it {
            self.whiteouts.insert(fold(&path), path.clone());
            self.save_whiteouts()?;
            crash_point("unlink");
        }
        if ours {
            let real = self.real_path(id).ok_or_else(|| err(libc::ENOENT))?;
            match std::fs::remove_file(&real) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    if lower_has_it {
                        self.whiteouts.remove(&fold(&path));
                        let _ = self.save_whiteouts();
                    }
                    return Err(e);
                }
            }
        }
        if !lower_has_it {
            self.detach(id);
        }
        self.nodes[id as usize].kind = Kind::Gone;
        self.nodes[id as usize].shadowed = lower_has_it;
        Ok(())
    }

    /// Deletes the empty folder `name` from folder `parent`.
    pub fn rmdir(&mut self, parent: NodeId, name: &str) -> io::Result<()> {
        let id = self.lookup(parent, name).ok_or_else(|| err(libc::ENOENT))?;
        if !self.nodes[id as usize].is_dir() {
            return Err(err(libc::ENOTDIR));
        }
        if self.writable.is_empty() {
            return Err(err(libc::EROFS));
        }
        if self.children(id).next().is_some() {
            return Err(err(libc::ENOTEMPTY));
        }
        let folded = fold(&self.path(id));
        for layer in self.writable.clone() {
            // Remembered, or made by a rename's moves (which remembers only what it moved).
            let real = match self.writable_dirs.remove(&(layer, folded.clone())) {
                Some(real) => Some(real),
                None => self.writable_dir_path(layer, id).ok(),
            };
            if let Some(real) = real {
                // Only succeeds when the real folder is empty too (hidden files keep it).
                let _ = std::fs::remove_dir(real);
            }
        }
        let lower_has_it = self.nodes[id as usize].shadowed;
        self.remove_node(id, lower_has_it)
    }

    /// Does `steps` in order: through the journal when there is more than one, so a crash
    /// in the middle is finished at the next mount. Whiteout changes are saved when their
    /// step comes.
    fn carry_out(&mut self, steps: &[Step]) -> io::Result<()> {
        let journal = self.journal.clone().filter(|_| steps.len() > 1);
        if let Some(journal) = &journal {
            if journal.exists() {
                // An earlier change failed half-way and waits for the next mount to finish it.
                return Err(io::Error::other(format!(
                    "{} is pending: unmount and mount again to finish it",
                    journal.display()
                )));
            }
            journal::write(journal, steps)?;
        }
        for step in steps {
            crash_point("rename-step");
            journal::run(step, &mut self.whiteouts)?;
            if matches!(step, Step::Whiteouts { .. }) {
                self.save_whiteouts()?;
            }
        }
        if let Some(journal) = &journal {
            journal::clear(journal)?;
        }
        Ok(())
    }

    /// The files and folders at and under `id` (deleted ones left out).
    fn collect(&self, id: NodeId, files: &mut Vec<NodeId>, dirs: &mut Vec<NodeId>) {
        match &self.nodes[id as usize].kind {
            Kind::File(_) => files.push(id),
            Kind::Dir { children } => {
                dirs.push(id);
                for child in children.values() {
                    self.collect(*child, files, dirs);
                }
            }
            Kind::Gone => {}
        }
    }

    /// Whether `id` or anything under it comes from (or hides) a read-only layer.
    fn subtree_has_lower(&self, id: NodeId) -> bool {
        let node = &self.nodes[id as usize];
        node.shadowed
            || match &node.kind {
                Kind::File(src) => !self.is_writable(src.layer),
                Kind::Dir { children } => children.values().any(|c| self.subtree_has_lower(*c)),
                Kind::Gone => true,
            }
    }
}

/// Numbers the temp files of one process.
static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Copies `from` to `to` through a temporary file, so `to` is whole or absent.
fn copy_file(from: &Path, to: &Path) -> io::Result<()> {
    let dir = to.parent().ok_or_else(|| err(libc::EINVAL))?;
    let temp = dir.join(format!(
        "{TEMP_PREFIX}{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = copy_contents(from, &temp).and_then(|()| {
        crash_point("copy-up");
        std::fs::rename(&temp, to)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Copies the bytes, keeps the modification time and makes the copy writable by its owner.
fn copy_contents(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::copy(from, to)?;
    let meta = std::fs::metadata(from)?;
    let mut perms = meta.permissions();
    perms.set_mode(perms.mode() | 0o600);
    std::fs::set_permissions(to, perms)?;
    if let Ok(modified) = meta.modified() {
        std::fs::File::options().write(true).open(to)?.set_modified(modified)?;
    }
    Ok(())
}
