//! `Tree::rename`, in named steps: check the source, look at what stands at the new name,
//! plan where every moved file and folder goes, put the journal's steps in order, carry them
//! out, then make the tree follow what is on disk.

use std::io;
use std::path::PathBuf;

use super::journal::Step;
use super::{FileSource, Kind, Node, NodeId, ROOT, TEMP_COUNTER, TEMP_PREFIX, Tree, copy_contents, err, find_dir_ci, fold};

/// What stands at the new name, and what replacing it takes.
struct Target {
    /// The entry there (a live one or a whiteout's tombstone).
    node: Option<NodeId>,
    /// A read-only layer has the path (a whiteout stood there, or the replaced entry hides one).
    lower: bool,
    /// The replaced entry is a folder: the moved folder is spelled as asked, not as its real one.
    is_dir: bool,
    /// The replaced file's real path, when it lives in a writable layer.
    file: Option<PathBuf>,
    /// The replaced folder's real folders in the writable layers.
    remove_dirs: Vec<Step>,
    /// A whiteout the replacement adds (a replaced folder that hides a lower one).
    whiteout_add: Option<String>,
    /// A whiteout the replacement ends (a file moved onto a whited-out path).
    whiteout_remove: Option<String>,
}

/// The new place: the target layer, the real folder of the new parent there, and the name.
struct Destination {
    layer: u16,
    base: PathBuf,
    /// The new parent's mount path, folded.
    base_folded: String,
    name: String,
    /// A folder is replaced (see [`Target::is_dir`]).
    replaces_dir: bool,
}

/// Where the moved entries go.
struct Moves {
    /// Every moved folder with its real folder in the target layer.
    dirs: Vec<(NodeId, PathBuf)>,
    /// The files' moves (from the old real file, or from a copy made under a temp name).
    moves: Vec<Step>,
    /// Old files in writable layers to delete once their copy is in place.
    sources: Vec<Step>,
    /// Every moved file with its new real folder.
    files: Vec<(NodeId, PathBuf)>,
}

impl Tree {
    /// Moves `name` in `parent` to `new_name` in `new_parent`, replacing a file or an
    /// empty folder there.
    ///
    /// The result always lives in a writable layer: a file already in one is renamed on
    /// disk, a file of a read-only layer is copied and whited out. Folders move with
    /// everything in them. A change of more than one step on disk is planned first and
    /// written to the journal (`journal.rs`), then done; the tree changes last.
    pub fn rename(&mut self, parent: NodeId, name: &str, new_parent: NodeId, new_name: &str) -> io::Result<()> {
        let (id, layer) = self.rename_source(parent, name, new_parent, new_name)?;
        let is_dir = self.nodes[id as usize].is_dir();
        let Some(target) = self.rename_target(id, is_dir, new_parent, new_name)? else {
            // The same entry under another spelling: names compare without case, so there
            // is nothing to move. The first spelling stays.
            return Ok(());
        };
        let old_path = self.path(id);
        let lower_has_source = self.subtree_has_lower(id);
        let to = Destination {
            layer,
            base: self.writable_dir_path(layer, new_parent)?,
            base_folded: fold(&self.path(new_parent)),
            name: new_name.to_owned(),
            replaces_dir: target.is_dir,
        };
        let moves = self.plan_moves(id, &to)?;
        let stale = self.stale_dirs(&old_path);

        // The journal's order: the replaced entry, the new folders, the moves, the old
        // copies, the emptied folders, the whiteouts.
        let mut steps = target.remove_dirs.clone();
        // The replaced file goes, unless the moved file lands on that very path.
        if let Some(real) = target.file.clone().filter(|real| *real != to.base.join(new_name) || is_dir) {
            steps.push(Step::Remove(real));
        }
        steps.extend(moves.dirs.iter().map(|(_, real)| Step::MakeDir(real.clone())));
        steps.extend(moves.moves.iter().cloned());
        steps.extend(moves.sources.iter().cloned());
        steps.extend(stale.iter().map(|(_, real)| Step::RemoveDir(real.clone())));
        let mut add: Vec<String> = target.whiteout_add.iter().cloned().collect();
        if lower_has_source {
            // the old place keeps the read-only layers' files hidden
            add.push(old_path.clone());
        }
        let remove: Vec<String> = target.whiteout_remove.iter().cloned().collect();
        if !add.is_empty() || !remove.is_empty() {
            steps.push(Step::Whiteouts { add, remove });
        }
        self.carry_out(&steps)?;

        self.follow_rename(id, &to, new_parent, &target, moves, &stale)?;
        if lower_has_source {
            self.add_child(parent, name, Kind::Gone, true);
        }
        Ok(())
    }

    /// The entry to move and the layer it goes to, or the error a rename gets first.
    fn rename_source(&self, parent: NodeId, name: &str, new_parent: NodeId, new_name: &str) -> io::Result<(NodeId, u16)> {
        let id = self.lookup(parent, name).ok_or_else(|| err(libc::ENOENT))?;
        if !self.node(new_parent).is_some_and(Node::is_dir) {
            return Err(err(libc::ENOTDIR));
        }
        if new_name.is_empty() || new_name.contains('/') || new_name.starts_with(TEMP_PREFIX) {
            return Err(err(libc::EINVAL));
        }
        let layer = self.target_layer(new_parent, new_name)?;
        // A folder cannot move into itself.
        let mut up = new_parent;
        while up != ROOT {
            if up == id {
                return Err(err(libc::EINVAL));
            }
            up = self.nodes[up as usize].parent;
        }
        Ok((id, layer))
    }

    /// What stands at the new name. `None` when it is the moved entry itself.
    fn rename_target(&self, id: NodeId, is_dir: bool, new_parent: NodeId, new_name: &str) -> io::Result<Option<Target>> {
        let node = self.child(new_parent, &fold(new_name));
        let mut target = Target {
            node,
            lower: false,
            is_dir: node.is_some_and(|t| t != id && self.nodes[t as usize].is_dir()),
            file: None,
            remove_dirs: Vec::new(),
            whiteout_add: None,
            whiteout_remove: None,
        };
        match node {
            Some(t) if t == id => return Ok(None),
            Some(t) if self.nodes[t as usize].kind == Kind::Gone => {
                if !is_dir {
                    // a file at a whited-out path: the whiteout has done its job
                    target.whiteout_remove = Some(self.path(t));
                }
                target.lower = true;
            }
            Some(t) => {
                let target_is_dir = self.nodes[t as usize].is_dir();
                match (is_dir, target_is_dir) {
                    (false, true) => return Err(err(libc::EISDIR)),
                    (true, false) => return Err(err(libc::ENOTDIR)),
                    (true, true) if self.children(t).next().is_some() => return Err(err(libc::ENOTEMPTY)),
                    _ => {}
                }
                target.lower = self.subtree_has_lower(t);
                if target_is_dir {
                    let folded = fold(&self.path(t));
                    for l in &self.writable {
                        if let Some(real) = self.writable_dirs.get(&(*l, folded.clone())) {
                            target.remove_dirs.push(Step::RemoveDir(real.clone()));
                        }
                    }
                    if target.lower {
                        // as `rmdir` does: the lower layers' folder stays hidden
                        target.whiteout_add = Some(self.path(t));
                    }
                } else if let Kind::File(src) = &self.nodes[t as usize].kind
                    && self.is_writable(src.layer)
                {
                    target.file = self.real_path(t);
                }
            }
            None => {}
        }
        Ok(Some(target))
    }

    /// The real folder in the target layer for `node` (the moved entry or a folder under
    /// it), found without regard to case where it already exists. A replaced folder's real
    /// one goes first, so the new one is spelled as asked.
    fn planned_dir(&self, id: NodeId, node: NodeId, to: &Destination) -> PathBuf {
        let old_path = self.path(id);
        let path = self.path(node);
        let below = &path[old_path.len()..]; // "" or "/sub/..."
        let mut folded =
            if to.base_folded.is_empty() { fold(&to.name) } else { format!("{}/{}", to.base_folded, fold(&to.name)) };
        let mut real = match self.writable_dirs.get(&(to.layer, folded.clone())) {
            _ if to.replaces_dir => to.base.join(&to.name),
            Some(cached) => cached.clone(),
            None => find_dir_ci(&to.base, &to.name).unwrap_or_else(|| to.base.join(&to.name)),
        };
        for part in below.split('/').filter(|p| !p.is_empty()) {
            folded = format!("{folded}/{}", fold(part));
            real = match self.writable_dirs.get(&(to.layer, folded.clone())) {
                Some(cached) => cached.clone(),
                None => find_dir_ci(&real, part).unwrap_or_else(|| real.join(part)),
            };
        }
        real
    }

    /// Plans where everything under `id` goes. Files of another layer are copied now, under
    /// temp names in the target layer's own folder (never shown, and removed at the next
    /// mount unless the journal moves them into place); on an error the copies go again.
    fn plan_moves(&self, id: NodeId, to: &Destination) -> io::Result<Moves> {
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        self.collect(id, &mut files, &mut dirs);
        let mut out = Moves {
            dirs: dirs.iter().map(|d| (*d, self.planned_dir(id, *d, to))).collect(),
            moves: Vec::new(),
            sources: Vec::new(),
            files: Vec::new(),
        };
        let mut temps: Vec<PathBuf> = Vec::new();
        for file in files {
            let Kind::File(src) = &self.nodes[file as usize].kind else { continue };
            let old_layer = src.layer;
            let old_real = self.real_path(file).ok_or_else(|| err(libc::ENOENT))?;
            let node = &self.nodes[file as usize];
            let dir_real = if file == id {
                to.base.clone()
            } else {
                out.dirs.iter().find(|(d, _)| *d == node.parent).map(|(_, r)| r.clone()).unwrap_or_else(|| to.base.clone())
            };
            let file_name = if file == id { to.name.clone() } else { node.name.to_string() };
            let target = dir_real.join(&file_name);
            if old_layer == to.layer {
                out.moves.push(Step::Move { from: old_real, to: target });
            } else {
                let temp = self.layers[usize::from(to.layer)].source.join(format!(
                    "{TEMP_PREFIX}{}-{}",
                    std::process::id(),
                    TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ));
                if let Err(e) = copy_contents(&old_real, &temp) {
                    for temp in temps.iter().chain(std::iter::once(&temp)) {
                        let _ = std::fs::remove_file(temp);
                    }
                    return Err(e);
                }
                temps.push(temp.clone());
                out.moves.push(Step::Move { from: temp, to: target });
                if self.is_writable(old_layer) {
                    out.sources.push(Step::Remove(old_real));
                }
            }
            out.files.push((file, dir_real));
        }
        Ok(out)
    }

    /// The writable layers' folders under the old path, deepest first: emptied by the moves.
    fn stale_dirs(&self, old_path: &str) -> Vec<((u16, String), PathBuf)> {
        let old_folded = fold(old_path);
        let prefix = format!("{old_folded}/");
        let mut stale: Vec<((u16, String), PathBuf)> = self
            .writable_dirs
            .iter()
            .filter(|((_, k), _)| *k == old_folded || k.starts_with(&prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        stale.sort_by_key(|(_, real)| std::cmp::Reverse(real.as_os_str().len()));
        stale
    }

    /// After the steps are done: the tree follows what is now on disk.
    fn follow_rename(
        &mut self,
        id: NodeId,
        to: &Destination,
        new_parent: NodeId,
        target: &Target,
        moves: Moves,
        stale: &[((u16, String), PathBuf)],
    ) -> io::Result<()> {
        // The moves made the new name's folder: remember it and its parents, as
        // `writable_dir_for` would have.
        self.writable_dir_for(to.layer, new_parent)?;
        for (key, _) in stale {
            self.writable_dirs.remove(key);
        }
        if let Some(t) = target.node {
            self.detach(t);
            self.nodes[t as usize].kind = Kind::Gone;
        }
        self.detach(id);
        self.nodes[id as usize].name = to.name.as_str().into();
        self.nodes[id as usize].parent = new_parent;
        self.attach(id, new_parent);
        for (dir, real) in moves.dirs {
            self.writable_dirs.insert((to.layer, fold(&self.path(dir))), real);
            self.nodes[dir as usize].shadowed = dir == id && target.lower;
        }
        for (file, dir_real) in moves.files {
            let dir = self.register_dir(dir_real);
            let node = &mut self.nodes[file as usize];
            node.kind = Kind::File(FileSource { layer: to.layer, dir, real_name: None, write_back: false });
            node.shadowed = file == id && target.lower;
        }
        Ok(())
    }
}
