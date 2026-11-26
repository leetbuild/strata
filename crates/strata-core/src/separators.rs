//! Separators inside separators (Leet change 9, "Sub-Separator"), as pure functions over a
//! mod list. Ported from the Leet MO2 source, which ported goldghoti's plugin:
//! `ModListByPriorityProxy::buildTree` (the tree), `canDropMimeData` / `dropMimeData` (the
//! drag rules), `modlistviewactions.cpp` (`priorityAfterSeparator`,
//! `enclosingSeparatorNest`) and `ModListView` (roll-ups, the `(n)` count).
//!
//! The tree is the one the window shows in ascending priority: a separator owns every mod
//! after it up to the next separator whose level is less than or equal to its own. In
//! descending priority MO2 shows a flat list, so there is nothing to compute.

use crate::{Color, ModEntry, ModKind};

/// A row of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    /// A mod, by priority.
    Mod(usize),
    /// The Overwrite row, always the last top-level row.
    Overwrite,
}

/// The mod list as the window's tree (`ModListByPriorityProxy` in ascending priority).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeparatorTree {
    /// Top-level rows: the backups first (MO2 puts them there), then the top-level mods
    /// and separators, then Overwrite.
    pub root: Vec<Node>,
    /// Per priority: the separator it sits in, if any.
    pub parent: Vec<Option<usize>>,
    /// Per priority: a separator's rows, in order (empty for other mods).
    pub children: Vec<Vec<usize>>,
    /// Per priority: the level used for nesting (0 for mods).
    nest: Vec<u32>,
}

/// What the user drops on: a row of the tree (`Some(row)` under `parent`, `None` = the
/// top level), or onto a separator itself (`row: None`, `parent: Some(separator)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropTarget {
    pub parent: Option<usize>,
    pub row: Option<usize>,
    /// The drop indicator is just below an expanded separator (`m_dropExpanded` with
    /// `DropPosition::BelowItem`): MO2 makes that a drop at the start of the separator.
    pub below_expanded_separator: bool,
}

/// What a drop does: move the dropped mods to `priority` (`usize::MAX` = the end) with
/// [`crate::ModList::move_mods`], after setting the levels of the dropped separators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropPlan {
    pub priority: usize,
    pub nests: Vec<(usize, u32)>,
}

impl SeparatorTree {
    /// `buildTree` in ascending priority.
    pub fn build(entries: &[ModEntry]) -> Self {
        let n = entries.len();
        let mut tree = Self { root: Vec::new(), parent: vec![None; n], children: vec![Vec::new(); n], nest: vec![0; n] };
        let mut top: Vec<Node> = Vec::new();
        let mut backups = Vec::new();
        let mut open: Option<usize> = None;
        for (p, entry) in entries.iter().enumerate() {
            match entry.kind {
                ModKind::Separator => {
                    tree.nest[p] = entry.nest;
                    // close the open separators at this level or deeper
                    while let Some(o) = open
                        && entry.nest <= tree.nest[o]
                    {
                        open = tree.parent[o];
                    }
                    tree.attach(p, open, &mut top);
                    open = Some(p);
                }
                ModKind::Backup => backups.push(Node::Mod(p)),
                _ => tree.attach(p, open, &mut top),
            }
        }
        tree.root = backups;
        tree.root.extend(top);
        tree.root.push(Node::Overwrite);
        tree
    }

    fn attach(&mut self, p: usize, parent: Option<usize>, top: &mut Vec<Node>) {
        self.parent[p] = parent;
        match parent {
            Some(s) => self.children[s].push(p),
            None => top.push(Node::Mod(p)),
        }
    }

    /// The rows under `parent` (`None` = the top level).
    pub fn rows(&self, parent: Option<usize>) -> Vec<Node> {
        match parent {
            None => self.root.clone(),
            Some(s) => self.children[s].iter().map(|c| Node::Mod(*c)).collect(),
        }
    }

    /// Everything inside a separator, nested separators and their mods included, depth
    /// first (`SubSeparator::forEachDescendant`).
    pub fn descendants(&self, separator: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self.children[separator].iter().rev().copied().collect();
        while let Some(p) = stack.pop() {
            out.push(p);
            stack.extend(self.children[p].iter().rev());
        }
        out
    }

    /// The `(n)` the delegate draws: the separator's direct rows (`rowCount`).
    pub fn row_count(&self, separator: usize) -> usize {
        self.children[separator].len()
    }

    /// The separators that contain `p`, innermost first.
    pub fn ancestors(&self, p: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut at = self.parent[p];
        while let Some(s) = at {
            out.push(s);
            at = self.parent[s];
        }
        out
    }

    /// `ModListByPriorityProxy::canDropMimeData` for a drop of mods.
    pub fn can_drop(&self, entries: &[ModEntry], target: DropTarget, dropped: &[usize]) -> bool {
        match (target.parent, target.row) {
            // onto an item: a separator other than the dropped ones
            (Some(s), None) => entries.get(s).is_some_and(|e| e.kind == ModKind::Separator) && !dropped.contains(&s),
            (None, None) => false,
            (parent, Some(row)) => {
                let len = self.rows(parent).len();
                // nothing goes past Overwrite
                row <= len && !(parent.is_none() && row == len)
            }
        }
    }

    /// `ModListByPriorityProxy::dropMimeData` in ascending priority: where the dropped mods
    /// go and the new levels of the dropped separators (one level below the separator
    /// they land in, keeping their nesting relative to each other). `None` when the drop
    /// is not allowed.
    pub fn plan_drop(&self, entries: &[ModEntry], target: DropTarget, dropped: &[usize]) -> Option<DropPlan> {
        if !self.can_drop(entries, target, dropped) {
            return None;
        }
        let mut parent = target.parent;
        let mut row = target.row;
        let mut source: Option<Node> = None;

        // dropped just below an expanded separator: at the start of that separator
        if target.below_expanded_separator
            && let Some(r) = row
            && r > 0
        {
            let rows = self.rows(parent);
            if let Some(Node::Mod(above)) = rows.get(r - 1).copied()
                && entries[above].kind == ModKind::Separator
            {
                parent = Some(above);
                row = Some(0);
                source = self.children[above].first().map(|c| Node::Mod(*c));
            }
        }

        // mods dropped onto a separator: after its own mods, not inside its last sub-separator
        // (see priority_for_mod_in_separator); dropped separators still go to its end
        if source.is_none()
            && row.is_none()
            && let Some(sep) = parent
            && dropped.iter().all(|d| entries[*d].kind != ModKind::Separator)
            && let Some(&first) = self.children[sep].iter().find(|c| entries[**c].kind == ModKind::Separator)
        {
            source = Some(Node::Mod(first));
        }

        if source.is_none() {
            let rows = self.rows(parent);
            source = match row {
                Some(r) if r < rows.len() => Some(rows[r]),
                // at the end of a separator: before whatever follows it and everything in it
                _ => self.after_subtree(parent),
            };
        }
        let priority = match source? {
            Node::Mod(p) => p,
            Node::Overwrite => usize::MAX,
        };

        // the levels of the dropped separators
        let mut nests = Vec::new();
        if !parent.is_some_and(|p| dropped.contains(&p)) {
            let parent_nest = parent.map_or(-1, |p| i64::from(self.nest[p]));
            let mut by_priority = dropped.to_vec();
            by_priority.sort_unstable();
            by_priority.dedup();
            let mut open: Vec<usize> = Vec::new();
            for p in by_priority {
                while let Some(&last) = open.last() {
                    if self.ancestors(p).contains(&last) {
                        break;
                    }
                    open.pop();
                }
                if entries[p].kind == ModKind::Separator {
                    open.push(p);
                    nests.push((p, (parent_nest + open.len() as i64) as u32));
                }
            }
        }
        Some(DropPlan { priority, nests })
    }

    /// The row after `parent` and everything inside it, going up until one exists.
    fn after_subtree(&self, parent: Option<usize>) -> Option<Node> {
        let mut item = parent?;
        loop {
            let siblings = self.rows(self.parent[item]);
            let i = siblings.iter().position(|n| *n == Node::Mod(item))?;
            if let Some(next) = siblings.get(i + 1) {
                return Some(*next);
            }
            item = self.parent[item]?;
        }
    }
}

/// `priorityAfterSeparator`: the priority right after a separator and everything nested
/// in it, i.e. of the next separator at the same level or above; `None` at the end.
/// New mods and separators created "in" a separator go there.
pub fn priority_after_separator(entries: &[ModEntry], separator: usize) -> Option<usize> {
    let nest = entries[separator].nest;
    entries.iter().skip(separator + 1).find(|e| e.kind == ModKind::Separator && e.nest <= nest).map(|e| e.priority)
}

/// Where a mod put "in" a separator goes: after the separator's own mods, before the next
/// separator of any level (its first sub-separator, or the next one after it); `None` at the
/// end. Not [`priority_after_separator`]: the list is flat, so the end of a separator whose
/// last child is a sub-separator is inside that sub-separator (a mod dropped onto "My Mods"
/// landed in its sub-separator). Separators made in a separator still go to its end.
pub fn priority_for_mod_in_separator(entries: &[ModEntry], separator: usize) -> Option<usize> {
    entries.iter().skip(separator + 1).find(|e| e.kind == ModKind::Separator).map(|e| e.priority)
}

/// `enclosingSeparatorNest`: the level of the closest separator below `p` in priority
/// (above it on screen); 0 outside of any.
pub fn enclosing_separator_nest(entries: &[ModEntry], p: usize) -> u32 {
    entries[..p].iter().rev().find(|e| e.kind == ModKind::Separator).map_or(0, |e| e.nest)
}

/// A collapsed separator's flags, conflicts or contents: its own and every descendant's,
/// merged and sorted (`ModListView::modFlags`, `conflictFlags`, `contents`).
pub fn roll_up<T: Ord + Clone>(tree: &SeparatorTree, separator: usize, of: impl Fn(usize) -> Vec<T>) -> Vec<T> {
    let mut all = of(separator);
    for p in tree.descendants(separator) {
        all.extend(of(p));
    }
    all.sort();
    all.dedup();
    all
}

/// A collapsed separator's marker colour (`ModListView::markerColor`): the average of
/// the colours of the mods inside (nested separators' own colours left out), integer
/// division per channel as MO2 does; `None` when no mod inside has one.
pub fn roll_up_color(tree: &SeparatorTree, entries: &[ModEntry], separator: usize, color: impl Fn(usize) -> Option<Color>) -> Option<Color> {
    let colors: Vec<Color> = tree
        .descendants(separator)
        .into_iter()
        .filter(|p| entries[*p].kind != ModKind::Separator)
        .filter_map(color)
        .collect();
    if colors.is_empty() {
        return None;
    }
    let n = colors.len() as u32;
    let avg = |f: fn(&Color) -> u8| (colors.iter().map(|c| u32::from(f(c))).sum::<u32>() / n) as u8;
    Some(Color { r: avg(|c| c.r), g: avg(|c| c.g), b: avg(|c| c.b), a: avg(|c| c.a) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `"A S1 B S2:1 C S3 D"`: names, with `:n` for a separator's level (S = separator).
    fn entries(spec: &str) -> Vec<ModEntry> {
        spec.split(' ')
            .enumerate()
            .map(|(i, item)| {
                let (name, nest) = item.split_once(':').map_or((item, 0), |(n, l)| (n, l.parse().unwrap()));
                let kind = if name.starts_with('S') {
                    ModKind::Separator
                } else if name.starts_with('K') {
                    ModKind::Backup
                } else {
                    ModKind::Regular
                };
                ModEntry { name: name.into(), kind, enabled: true, priority: i, path: None, listed: true, nest }
            })
            .collect()
    }

    fn names(e: &[ModEntry], list: &[usize]) -> String {
        list.iter().map(|p| e[*p].name.as_str()).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn the_tree_nests_by_level() {
        //  A, S1 { B, S2 { C, S3 { D } }, S4 { E } }, S5 { F }, backups first, Overwrite last
        let e = entries("A S1 B S2:1 C S3:2 D S4:1 E S5 F K");
        let t = SeparatorTree::build(&e);
        assert_eq!(t.root, [Node::Mod(11), Node::Mod(0), Node::Mod(1), Node::Mod(9), Node::Overwrite]);
        assert_eq!(names(&e, &t.children[1]), "B S2 S4");
        assert_eq!(names(&e, &t.children[3]), "C S3");
        assert_eq!(names(&e, &t.descendants(1)), "B S2 C S3 D S4 E");
        assert_eq!(t.row_count(1), 3);
        assert_eq!(names(&e, &t.ancestors(6)), "S3 S2 S1");
        // a deeper level right after a separator nests; a level jump of more than one too
        let e = entries("S0 S1:5 A S2:2 B");
        let t = SeparatorTree::build(&e);
        assert_eq!(names(&e, &t.children[0]), "S1 S2");
        assert_eq!(names(&e, &t.children[1]), "A");
    }

    #[test]
    fn where_new_things_go() {
        let e = entries("A S1 B S2:1 C S3 D");
        assert_eq!(priority_after_separator(&e, 1), Some(5), "after S1 and S2 inside it");
        assert_eq!(priority_after_separator(&e, 3), Some(5));
        assert_eq!(priority_after_separator(&e, 5), None, "the last one: the end");
        // a mod in S1 goes before S2, inside S1; in S2 or S3 the same as above
        assert_eq!(priority_for_mod_in_separator(&e, 1), Some(3));
        assert_eq!(priority_for_mod_in_separator(&e, 3), Some(5));
        assert_eq!(priority_for_mod_in_separator(&e, 5), None);
        assert_eq!(enclosing_separator_nest(&e, 4), 1, "C is in S2");
        assert_eq!(enclosing_separator_nest(&e, 0), 0);
    }

    #[test]
    fn dropping_mods_between_rows_and_onto_separators() {
        let e = entries("A S1 B S2:1 C S3 D");
        let t = SeparatorTree::build(&e);
        let at = |parent, row| DropTarget { parent, row, below_expanded_separator: false };
        // between B and S2 inside S1: before S2
        assert_eq!(t.plan_drop(&e, at(Some(1), Some(1)), &[0]).unwrap(), DropPlan { priority: 3, nests: vec![] });
        // a mod onto S1: after S1's own B, before its sub-separator S2 (the flat list's end of
        // S1 would be inside S2)
        assert_eq!(t.plan_drop(&e, at(Some(1), None), &[0]).unwrap().priority, 3);
        // a separator onto S1: at its end, i.e. before S3
        assert_eq!(t.plan_drop(&e, at(Some(1), None), &[5]).unwrap().priority, 5);
        // onto S3, the last separator: before Overwrite = the end
        assert_eq!(t.plan_drop(&e, at(Some(5), None), &[0]).unwrap().priority, usize::MAX);
        // not past Overwrite, not onto a mod, not onto a dropped separator
        assert!(t.plan_drop(&e, at(None, Some(t.root.len())), &[0]).is_none());
        assert!(t.plan_drop(&e, at(Some(0), None), &[2]).is_none());
        assert!(t.plan_drop(&e, at(Some(1), None), &[1]).is_none());
        // just below expanded S1: at its start, before B
        let below = DropTarget { parent: None, row: Some(2), below_expanded_separator: true };
        assert_eq!(t.plan_drop(&e, below, &[6]).unwrap().priority, 2);
    }

    #[test]
    fn dropped_separators_nest_one_below_their_new_parent() {
        // S3 { D } and S4:1 { E } dropped into S2 (level 1): S3 -> 2, S4 stays inside S3 -> 3
        let e = entries("A S1 B S2:1 C S3 D S4:1 E");
        let t = SeparatorTree::build(&e);
        let plan = t.plan_drop(&e, DropTarget { parent: Some(3), row: None, below_expanded_separator: false }, &[5, 6, 7, 8]).unwrap();
        assert_eq!(plan.nests, [(5, 2), (7, 3)]);
        // dropped at the top level: level 0
        let plan = t.plan_drop(&e, DropTarget { parent: None, row: Some(0), below_expanded_separator: false }, &[3]).unwrap();
        assert_eq!(plan, DropPlan { priority: 0, nests: vec![(3, 0)] });
    }

    #[test]
    fn roll_ups_merge_the_inside() {
        let e = entries("S1 A S2:1 B C");
        let t = SeparatorTree::build(&e);
        assert_eq!(roll_up(&t, 0, |p| vec![e[p].name.len() % 2, p % 3]), vec![0, 1, 2]);
        let red = Color { r: 200, g: 0, b: 0, a: 255 };
        let blue = Color { r: 0, g: 0, b: 101, a: 255 };
        let color = |p: usize| match p {
            1 => Some(red),
            2 => Some(Color { r: 9, g: 9, b: 9, a: 9 }), // a separator: left out
            3 => Some(blue),
            _ => None,
        };
        assert_eq!(roll_up_color(&t, &e, 0, color), Some(Color { r: 100, g: 0, b: 50, a: 255 }));
        assert_eq!(roll_up_color(&t, &e, 2, |_| None), None);
    }
}
