//! The tree the installers work on: MO2's `IFileTree` (uibase `ifiletree.h`), without Qt.
//!
//! An archive is listed once into a [`FileTree`]; installers move, copy, drop and merge
//! entries, and only at the end are the remaining files extracted to the places the tree
//! names. Each file remembers where its bytes come from ([`Source`]), so a file can be
//! copied to several places (FOMOD) and nothing is read before the end.
//!
//! Names compare without case and keep their spelling, as `FileNameComparator` does.
//! Entries are kept in `FileEntryComparator` order: folders first, then names without case.

use std::cmp::Ordering;

/// One entry of a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Dir(Dir),
    File(File),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub name: String,
    /// Where the bytes come from: the file's path in the archive (`/`-separated).
    pub source: String,
}

/// A folder: its name (empty for the top) and its entries in MO2's order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dir {
    pub name: String,
    entries: Vec<Node>,
}

/// What to do when an entry is put where one of the same name already is
/// (`IFileTree::InsertPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Leave the existing entry and fail.
    FailIfExists,
    /// The new entry replaces the old one, whatever their kinds.
    Replace,
    /// Two folders merge, a file replaces a file; a file and a folder fail.
    Merge,
}

/// The tree of one install: the top folder.
pub type FileTree = Dir;

/// The key names are compared by: the overlay's rule (`strata_vfs::tree::fold`, one character
/// at a time as Windows and MO2 compare), so an install and the deployed game agree on which
/// names are the same.
pub use strata_vfs::tree::fold;

/// `QFileInfo::suffix()`: the part after the last dot, `""` without one.
pub fn suffix(name: &str) -> &str {
    name.rfind('.').map_or("", |i| &name[i + 1..])
}

/// `QFileInfo::completeBaseName()`: the part before the last dot.
pub fn complete_base_name(name: &str) -> &str {
    name.rfind('.').map_or(name, |i| &name[..i])
}

/// Splits a path on `/` and `\` (both are separators in an `IFileTree`), dropping empty
/// parts and `.`.
pub fn split(path: &str) -> Vec<&str> {
    path.split(['/', '\\']).filter(|p| !p.is_empty() && *p != ".").collect()
}

impl Node {
    pub fn name(&self) -> &str {
        match self {
            Node::Dir(d) => &d.name,
            Node::File(f) => &f.name,
        }
    }

    pub fn is_dir(&self) -> bool {
        matches!(self, Node::Dir(_))
    }

    pub fn as_dir(&self) -> Option<&Dir> {
        match self {
            Node::Dir(d) => Some(d),
            Node::File(_) => None,
        }
    }

    fn set_name(&mut self, name: &str) {
        match self {
            Node::Dir(d) => d.name = name.to_owned(),
            Node::File(f) => f.name = name.to_owned(),
        }
    }
}

/// `FileEntryComparator`: folders before files, then names without case.
fn order(a: &Node, b: &Node) -> Ordering {
    b.is_dir().cmp(&a.is_dir()).then_with(|| fold(a.name()).cmp(&fold(b.name())))
}

impl Dir {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_owned(), entries: Vec::new() }
    }

    /// Builds a tree from an archive listing: `(path, is_dir)` pairs, `/` or `\` separated.
    /// Folders that only appear as parents of files are created too. Entries that would land
    /// outside the mod (`..`, a drive letter) are left out, as Fluorine's
    /// `safeRelativeOutputPath` does.
    pub fn from_listing<'a>(entries: impl IntoIterator<Item = (&'a str, bool)>) -> Self {
        let mut tree = Dir::default();
        for (path, is_dir) in entries {
            let parts = split(path);
            if parts.is_empty() || parts.iter().any(|p| *p == ".." || p.contains(':')) {
                continue;
            }
            if is_dir {
                tree.add_dir(path);
            } else {
                let source = parts.join("/");
                tree.insert(path, Node::File(File { name: String::new(), source }), Policy::Replace);
            }
        }
        tree
    }

    /// The entries, in MO2's order.
    pub fn entries(&self) -> &[Node] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    fn position(&self, name: &str) -> Option<usize> {
        let key = fold(name);
        self.entries.iter().position(|e| fold(e.name()) == key)
    }

    fn put(&mut self, node: Node) {
        let at = self.entries.partition_point(|e| order(e, &node) == Ordering::Less);
        self.entries.insert(at, node);
    }

    /// The entry named `name` directly in this folder.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.position(name).map(|i| &self.entries[i])
    }

    /// The entry at `path` below this folder.
    pub fn find(&self, path: &str) -> Option<&Node> {
        let parts = split(path);
        let (last, parents) = parts.split_last()?;
        let mut dir = self;
        for part in parents {
            dir = dir.child(part)?.as_dir()?;
        }
        dir.child(last)
    }

    /// The folder at `path` (`""` is this folder).
    pub fn find_dir(&self, path: &str) -> Option<&Dir> {
        if split(path).is_empty() {
            return Some(self);
        }
        self.find(path)?.as_dir()
    }

    fn find_dir_mut(&mut self, path: &str) -> Option<&mut Dir> {
        let mut dir = self;
        for part in split(path) {
            let i = dir.position(part)?;
            dir = match &mut dir.entries[i] {
                Node::Dir(d) => d,
                Node::File(_) => return None,
            };
        }
        Some(dir)
    }

    /// Whether this folder has a sub-folder named `name` (any case).
    pub fn has_dir(&self, name: &str) -> bool {
        self.child(name).is_some_and(Node::is_dir)
    }

    /// Creates the folders of `path` that don't exist yet (`IFileTree::addDirectory`) and
    /// returns the last one. A file in the way is replaced by a folder.
    pub fn add_dir(&mut self, path: &str) -> &mut Dir {
        let mut dir = self;
        for part in split(path) {
            let i = match dir.position(part) {
                Some(i) if dir.entries[i].is_dir() => i,
                found => {
                    if let Some(i) = found {
                        dir.entries.remove(i);
                    }
                    dir.put(Node::Dir(Dir::new(part)));
                    dir.position(part).unwrap_or_default()
                }
            };
            dir = match &mut dir.entries[i] {
                Node::Dir(d) => d,
                Node::File(_) => unreachable!("just made a folder here"),
            };
        }
        dir
    }

    /// Takes the entry at `path` out of the tree.
    pub fn remove(&mut self, path: &str) -> Option<Node> {
        let parts = split(path);
        let (last, parents) = parts.split_last()?;
        let dir = self.find_dir_mut(&parents.join("/"))?;
        let i = dir.position(last)?;
        Some(dir.entries.remove(i))
    }

    /// Puts `node` at `path` (its last part is the new name; a path ending in a separator
    /// keeps the node's own name), creating the parent folders. Returns false when the
    /// policy refused.
    pub fn insert(&mut self, path: &str, mut node: Node, policy: Policy) -> bool {
        let into_folder = path.ends_with(['/', '\\']) || split(path).is_empty();
        let parts = split(path);
        let (parent, name) = if into_folder {
            (parts.join("/"), node.name().to_owned())
        } else {
            let (last, parents) = parts.split_last().expect("checked above");
            (parents.join("/"), (*last).to_owned())
        };
        node.set_name(&name);
        self.add_dir(&parent).put_with(node, policy)
    }

    /// Puts `node` directly in this folder under the given policy.
    pub fn put_with(&mut self, node: Node, policy: Policy) -> bool {
        let Some(i) = self.position(node.name()) else {
            self.put(node);
            return true;
        };
        match policy {
            Policy::FailIfExists => false,
            Policy::Replace => {
                self.entries.remove(i);
                self.put(node);
                true
            }
            Policy::Merge => match (&mut self.entries[i], node) {
                (Node::Dir(existing), Node::Dir(source)) => {
                    existing.merge(source);
                    true
                }
                (Node::File(_), node @ Node::File(_)) => {
                    self.entries.remove(i);
                    self.put(node);
                    true
                }
                _ => false,
            },
        }
    }

    /// Moves every entry of `source` into this folder (`IFileTree::merge`): folders merge,
    /// anything else replaces what is there. Returns how many entries were replaced.
    pub fn merge(&mut self, source: Dir) -> usize {
        let mut replaced = 0;
        for node in source.entries {
            match (self.position(node.name()), node) {
                (Some(i), Node::Dir(src)) if self.entries[i].is_dir() => {
                    let Node::Dir(dst) = &mut self.entries[i] else { unreachable!() };
                    replaced += dst.merge(src);
                }
                (Some(i), node) => {
                    self.entries.remove(i);
                    self.put(node);
                    replaced += 1;
                }
                (None, node) => self.put(node),
            }
        }
        replaced
    }

    /// Moves the entry at `from` to `to` (see [`Dir::insert`] for `to`).
    pub fn move_entry(&mut self, from: &str, to: &str, policy: Policy) -> bool {
        let Some(node) = self.remove(from) else { return false };
        self.insert(to, node, policy)
    }

    /// Every file below this folder: its path from here and its source.
    pub fn files(&self) -> Vec<(String, &str)> {
        let mut out = Vec::new();
        self.collect_files("", &mut out);
        out
    }

    fn collect_files<'a>(&'a self, base: &str, out: &mut Vec<(String, &'a str)>) {
        for entry in &self.entries {
            let path = if base.is_empty() { entry.name().to_owned() } else { format!("{base}/{}", entry.name()) };
            match entry {
                Node::Dir(d) => d.collect_files(&path, out),
                Node::File(f) => out.push((path, f.source.as_str())),
            }
        }
    }

    /// Every path below this folder, folders with a trailing `/`, sorted: for tests and
    /// for showing a tree.
    pub fn paths(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_paths("", &mut out);
        out.sort();
        out
    }

    fn collect_paths(&self, base: &str, out: &mut Vec<String>) {
        for entry in &self.entries {
            let path = format!("{base}{}", entry.name());
            match entry {
                Node::Dir(d) => {
                    out.push(format!("{path}/"));
                    d.collect_paths(&format!("{path}/"), out);
                }
                Node::File(_) => out.push(path),
            }
        }
    }

    /// Whether there are files anywhere below (empty folders don't count).
    pub fn has_files(&self) -> bool {
        self.entries.iter().any(|e| match e {
            Node::File(_) => true,
            Node::Dir(d) => d.has_files(),
        })
    }
}

#[cfg(test)]
pub(crate) fn listing(paths: &[&str]) -> FileTree {
    Dir::from_listing(paths.iter().map(|p| (p.trim_end_matches('/'), p.ends_with('/'))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_builds_parents_and_finds_without_case() {
        let tree = listing(&["Data/Meshes/a.nif", "readme.txt", "Empty/"]);
        assert_eq!(tree.paths(), ["Data/", "Data/Meshes/", "Data/Meshes/a.nif", "Empty/", "readme.txt"]);
        assert!(tree.find("data\\MESHES\\A.NIF").is_some());
        assert!(tree.find_dir("DATA").is_some());
        assert_eq!(tree.entries()[0].name(), "Data", "folders come first");
    }

    #[test]
    fn entries_outside_the_mod_are_dropped() {
        let tree = listing(&["../evil.dll", "a/../../b.txt", "C:/x.txt", "ok.esp"]);
        assert_eq!(tree.paths(), ["ok.esp"]);
    }

    #[test]
    fn order_is_folders_first_then_names_without_case() {
        let tree = listing(&["b.txt", "A.txt", "z/", "C/"]);
        let names: Vec<_> = tree.entries().iter().map(Node::name).collect();
        assert_eq!(names, ["C", "z", "A.txt", "b.txt"]);
    }

    #[test]
    fn merge_replaces_files_and_merges_folders() {
        let mut a = listing(&["x/1.txt", "x/2.txt", "f"]);
        let b = listing(&["X/2.TXT", "x/3.txt", "f/"]);
        let replaced = a.merge(b);
        assert_eq!(replaced, 2);
        assert_eq!(a.paths(), ["f/", "x/", "x/1.txt", "x/2.TXT", "x/3.txt"]);
    }

    #[test]
    fn insert_into_a_folder_keeps_the_name_and_policies_apply() {
        let mut tree = listing(&["a.dll", "Root/x.dll"]);
        let node = tree.remove("a.dll").unwrap();
        assert!(tree.insert("Root/", node, Policy::Merge));
        assert_eq!(tree.paths(), ["Root/", "Root/a.dll", "Root/x.dll"]);
        let other = listing(&["x.dll"]).entries()[0].clone();
        assert!(!tree.insert("root/X.dll", other.clone(), Policy::FailIfExists));
        assert!(tree.insert("root/renamed.dll", other, Policy::FailIfExists));
        assert!(tree.find("Root/renamed.dll").is_some());
    }

    #[test]
    fn suffix_and_base_name_follow_qfileinfo() {
        assert_eq!(suffix("a.tar.gz"), "gz");
        assert_eq!(complete_base_name("a.tar.gz"), "a.tar");
        assert_eq!(suffix("noext"), "");
    }
}
