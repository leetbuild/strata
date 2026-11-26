//! Swapping the tree of a live mount (`refresh`): the new tree keeps the old node ids for
//! every path both trees have, because the kernel knows nodes by those ids (inode = id + 1)
//! and keeps them in its caches and open files.
//!
//! Paths that went away keep their id as a deleted node (lookups of a stale inode answer
//! "no such file"); new paths get fresh ids after the old ones. The result also says what
//! the kernel must forget: files whose bytes now come from another real file, and names
//! that appeared or went away in a folder.

use super::{Kind, Node, NodeId, ROOT, Tree, fold};

/// What changed in a refresh, for the kernel's caches.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Refreshed {
    /// Files that now read from another real file (or stopped existing): their cached
    /// data and attributes must go.
    pub changed: Vec<NodeId>,
    /// `(folder, name)` entries that appeared or went away: cached lookups (and cached
    /// "no such file" answers) must go.
    pub entries: Vec<(NodeId, String)>,
}

impl Tree {
    /// Whether the node can be found from the root (its folder lists it).
    fn reachable(&self, mut id: NodeId) -> bool {
        while id != ROOT {
            let node = &self.nodes[id as usize];
            if self.child(node.parent, &fold(&node.name)) != Some(id) {
                return false;
            }
            id = node.parent;
        }
        true
    }

    /// `new`, renumbered to keep `self`'s ids (see the module doc). Folders keep their id
    /// only as folders and files only as files: a path that changed kind gets a new id.
    pub fn rebased(&self, mut new: Tree) -> (Tree, Refreshed) {
        let old_len = self.nodes.len();
        // new id -> final id
        let mut map: Vec<Option<NodeId>> = vec![None; new.nodes.len()];
        map[ROOT as usize] = Some(ROOT);
        let mut taken = vec![false; old_len];
        taken[ROOT as usize] = true;
        let mut next = old_len as NodeId;

        // Parents come before their children in `new.nodes` (a node is added under an
        // existing folder), so one pass in id order maps every parent first.
        for id in 1..new.nodes.len() {
            let node = &new.nodes[id];
            let parent = map[node.parent as usize].unwrap_or(ROOT);
            let reuse = (parent as usize) < old_len
                && self.child(parent, &fold(&node.name)).is_some_and(|old| {
                    let old_node = &self.nodes[old as usize];
                    !taken[old as usize] && old_node.is_dir() == node.is_dir()
                });
            let final_id = if reuse {
                let old = self.child(parent, &fold(&node.name)).unwrap_or(ROOT);
                taken[old as usize] = true;
                old
            } else {
                next += 1;
                next - 1
            };
            map[id] = Some(final_id);
        }

        let total = next as usize;
        // Old slots no longer used stay as deleted nodes with their old name and parent.
        let mut nodes: Vec<Node> = self
            .nodes
            .iter()
            .map(|n| Node { name: n.name.clone(), parent: n.parent, kind: Kind::Gone, shadowed: false })
            .collect();
        nodes.resize(total, Node { name: "".into(), parent: ROOT, kind: Kind::Gone, shadowed: false });
        for (id, node) in new.nodes.drain(..).enumerate() {
            let final_id = map[id].unwrap_or(ROOT) as usize;
            let parent = map[node.parent as usize].unwrap_or(ROOT);
            let kind = match node.kind {
                Kind::Dir { children } => Kind::Dir {
                    children: children.into_iter().map(|(name, child)| (name, map[child as usize].unwrap_or(ROOT))).collect(),
                },
                other => other,
            };
            nodes[final_id] = Node { name: node.name, parent, kind, shadowed: node.shadowed };
        }
        new.nodes = nodes;

        // What the kernel must forget.
        let mut refreshed = Refreshed::default();
        for id in 0..old_len {
            let id = id as NodeId;
            // a node a higher layer's file replaced (a folder's old children) was never visible
            if !self.reachable(id) {
                continue;
            }
            let (before, after) = (self.node(id), new.node(id));
            match (before, after) {
                (Some(b), Some(_)) if !b.is_dir() && self.real_path(id) != new.real_path(id) => refreshed.changed.push(id),
                (Some(b), None) if !b.is_dir() => refreshed.changed.push(id),
                _ => {}
            }
            if let (Some(b), Some(a)) = (before, after)
                && b.is_dir()
                && a.is_dir()
            {
                let names = |tree: &Tree| -> Vec<(String, NodeId)> {
                    tree.children(id).map(|(child, n)| (fold(&n.name), child)).collect()
                };
                let (was, now) = (names(self), names(&new));
                for (name, child) in &was {
                    if !now.iter().any(|(n, c)| n == name && c == child) {
                        refreshed.entries.push((id, self.nodes[*child as usize].name.to_string()));
                    }
                }
                for (name, child) in &now {
                    if !was.iter().any(|(n, c)| n == name && c == child) {
                        refreshed.entries.push((id, new.nodes[*child as usize].name.to_string()));
                    }
                }
            }
        }
        (new, refreshed)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use strata_game::{Layer, LayerKind, Mapping};

    use super::super::{Options, Tree};
    use super::Refreshed;

    fn put(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, path.to_string_lossy().as_bytes()).unwrap();
    }

    fn tree(root: &Path, mods: &[&str]) -> Tree {
        let layers = mods
            .iter()
            .map(|m| Layer { id: (*m).into(), kind: LayerKind::Mod, source: root.join(m), rules: vec![Mapping::new("", "")] })
            .collect();
        Tree::build(layers, Options::default(), None).unwrap()
    }

    #[test]
    fn unchanged_paths_keep_their_ids_and_changes_are_listed() {
        let tmp = tempfile::tempdir().unwrap();
        for path in ["a/x/same.txt", "a/x/over.txt", "a/gone.txt", "a/kind", "b/x/over.txt", "b/new/n.txt"] {
            put(&tmp.path().join(path));
        }
        let old = tree(tmp.path(), &["a"]);
        std::fs::remove_file(tmp.path().join("a/kind")).unwrap();
        put(&tmp.path().join("a/kind/inside.txt"));
        std::fs::remove_file(tmp.path().join("a/gone.txt")).unwrap();
        let (new, refreshed) = old.rebased(tree(tmp.path(), &["a", "b"]));

        for path in ["x", "x/same.txt", "x/over.txt"] {
            assert_eq!(new.resolve(path), old.resolve(path), "{path}");
        }
        let over = old.resolve("x/over.txt").unwrap();
        assert!(new.real_path(over).unwrap().ends_with("b/x/over.txt"));
        assert!(new.resolve("gone.txt").is_none());
        assert!(new.node(old.resolve("gone.txt").unwrap()).is_none(), "the stale id is a deleted node");
        // a file that became a folder gets a new id
        assert_ne!(new.resolve("kind"), old.resolve("kind"));
        assert!(new.resolve("new/n.txt").unwrap() as usize >= old.len());

        let gone = old.resolve("gone.txt").unwrap();
        assert!(refreshed.changed.contains(&over) && refreshed.changed.contains(&gone));
        let names: Vec<&str> = refreshed.entries.iter().map(|(_, n)| n.as_str()).collect();
        for name in ["gone.txt", "kind", "new"] {
            assert!(names.contains(&name), "{name} in {names:?}");
        }
        assert!(!names.contains(&"same.txt"));
        assert_eq!(new.files().len(), 4);
    }

    /// A folder hidden by a higher layer's file leaves unreachable nodes behind: rebuilding
    /// the same tree must report nothing.
    #[test]
    fn the_same_tree_again_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        for path in ["low/clash/inside.txt", "high/clash", "high/x/y.txt"] {
            put(&tmp.path().join(path));
        }
        let old = tree(tmp.path(), &["low", "high"]);
        let (new, refreshed) = old.rebased(tree(tmp.path(), &["low", "high"]));
        assert_eq!(refreshed, Refreshed::default());
        assert_eq!(new.files(), old.files());
    }
}
