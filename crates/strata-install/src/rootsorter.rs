//! Leet change 4: install-time sorting of game-folder files into a mod's `Root` folder.
//!
//! A port of `modorganizer/src/rootsorter.cpp` (branch `instances-root`), rule for rule.
//! Only a mod's top level is looked at: anything deeper (`SKSE/Plugins/*.dll`) is Data
//! content. A mod that already has `Root` is left alone, and so is a tool that runs from
//! its mod folder. The confirm dialog is the caller's (`InstallUi::confirm_root_sort`).
//!
//! Not ported: `planGameRelative` and the `Root/<exe folder>` target, which only games with
//! mod mappings use (OBRE); Strata has no such game yet.

use crate::checker::ROOT;
use crate::tree::{Dir, FileTree, Node, Policy, complete_base_name, fold, suffix};

/// Files with these extensions are never Data content.
const ROOT_EXTENSIONS: &[&str] = &["exe", "dll", "asi"];

/// Files that belong in the game folder.
const ROOT_FILES: &[&str] = &["enbseries.ini", "enblocal.ini", "reshade.ini", "reshadepreset.ini", "d3dx.ini", "dxvk.conf"];

/// ENB's shaders, textures and inis all start with "enb"; plugins and archives never match.
const ENB_PREFIX: &str = "enb";
const ENB_EXTENSIONS: &[&str] = &["ini", "fx", "fxh", "bmp", "dds", "tga", "png"];

/// Folders that belong in the game folder.
const ROOT_FOLDERS: &[&str] = &["enbseries", "enbcache", "reshade-shaders"];

/// Standalone tools that run from their mod folder: a mod with one of these executables on
/// its top level is never sorted.
const TOOL_PREFIXES: &[&str] = &[
    "pandora behaviour engine",
    "nemesis unlimited behavior engine",
    "dyndolod",
    "texgen",
    "xlodgen",
    "sseedit",
    "tes5edit",
    "fo4edit",
    "xedit",
    "wrye bash",
    "bethini",
    "loot",
    "synthesis",
    "parallaxgen",
    "pgpatcher",
    "cathedral assets optimizer",
    "zedit",
    "bodyslide",
    "outfit studio",
    "nifskope",
];

/// Files that may sit around a wrapper folder; they are dropped when it is unwrapped.
const DOC_EXTENSIONS: &[&str] =
    &["url", "txt", "md", "pdf", "htm", "html", "rtf", "zip", "7z", "rar", "jpg", "jpeg", "png"];

const PLUGIN_EXTENSIONS: &[&str] = &["esp", "esm", "esl"];

/// What would change, with paths relative to the planned tree (`RootSorter::Plan`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// The folder holding the `Data` folder when `data_split`; empty for the tree itself.
    pub base: String,
    /// The base holds a `Data` folder next to root files (SKSE, ENB): `Data`'s contents
    /// become the mod's top level.
    pub data_split: bool,
    /// Names (in `base`) of the entries that go to `Root`.
    pub to_root: Vec<String>,
    /// Paths of entries that are not installed (docs and sources around a wrapper).
    pub dropped: Vec<String>,
    /// Top-level inis left in place because they might be Data inis.
    pub unsure: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.to_root.is_empty()
    }
}

enum Kind {
    Root,
    Stay,
    Unsure,
}

fn join(base: &str, name: &str) -> String {
    if base.is_empty() { name.to_owned() } else { format!("{base}/{name}") }
}

fn has(list: &[&str], value: &str) -> bool {
    list.contains(&value)
}

/// Base names of the plugins in this folder, for the `<plugin>.ini` rule.
fn plugin_base_names(tree: &Dir) -> Vec<String> {
    tree.entries()
        .iter()
        .filter(|e| !e.is_dir() && has(PLUGIN_EXTENSIONS, &fold(suffix(e.name()))))
        .map(|e| fold(complete_base_name(e.name())))
        .collect()
}

fn classify(entry: &Node, plugins: &[String]) -> Kind {
    let name = fold(entry.name());
    if entry.is_dir() {
        return if has(ROOT_FOLDERS, &name) { Kind::Root } else { Kind::Stay };
    }
    let ext = suffix(&name);
    if has(ROOT_EXTENSIONS, ext) || has(ROOT_FILES, &name) || (name.starts_with(ENB_PREFIX) && has(ENB_EXTENSIONS, ext))
    {
        return Kind::Root;
    }
    if ext == "ini" {
        // Skyrim loads Data/<plugin>.ini next to its plugin.
        return if plugins.iter().any(|p| *p == complete_base_name(&name)) { Kind::Stay } else { Kind::Unsure };
    }
    Kind::Stay
}

fn is_tool(tree: &Dir) -> bool {
    tree.entries().iter().any(|e| {
        let name = fold(e.name());
        !e.is_dir() && suffix(&name) == "exe" && TOOL_PREFIXES.iter().any(|p| name.starts_with(p))
    })
}

fn classify_level(tree: &Dir, plan: &mut Plan) {
    let plugins = plugin_base_names(tree);
    for entry in tree.entries() {
        match classify(entry, &plugins) {
            Kind::Root => plan.to_root.push(entry.name().to_owned()),
            Kind::Unsure => plan.unsure.push(entry.name().to_owned()),
            Kind::Stay => {}
        }
    }
}

/// `planInstalled`: sorts the top level of an installer's final tree.
pub fn plan_installed(tree: &Dir) -> Plan {
    let mut plan = Plan::default();
    if tree.has_dir(ROOT) || is_tool(tree) {
        return plan;
    }
    classify_level(tree, &mut plan);
    plan
}

/// `planArchive`: plans an archive's tree before any installer sees it. It may unwrap
/// single wrapper folders and split an explicit `Data` folder.
pub fn plan_archive(tree: &Dir, looks_valid: impl Fn(&Dir) -> bool) -> Plan {
    let mut node = tree;
    let mut base = String::new();
    let mut outer_docs = Vec::new();

    loop {
        // fomod and Root layouts are left to their installers
        if node.has_dir(ROOT) || node.has_dir("fomod") {
            return Plan::default();
        }
        if node.has_dir("Data") || looks_valid(node) {
            break;
        }
        let mut only_dir = None;
        let mut docs = Vec::new();
        let mut unwrappable = true;
        for entry in node.entries() {
            match entry {
                Node::Dir(d) => {
                    if only_dir.is_some() {
                        unwrappable = false;
                        break;
                    }
                    only_dir = Some(d);
                }
                Node::File(f) if has(DOC_EXTENSIONS, &fold(suffix(&f.name))) => docs.push(join(&base, &f.name)),
                Node::File(_) => {
                    unwrappable = false;
                    break;
                }
            }
        }
        let Some(dir) = only_dir.filter(|_| unwrappable) else { break };
        outer_docs.extend(docs);
        base = join(&base, &dir.name);
        node = dir;
    }

    // only unwrap into an explicit Data layout (SKSE, ENB): a lone folder could just as
    // well be Data content itself (KiLoader/, SKSE/)
    if !base.is_empty() && !node.has_dir("Data") {
        node = tree;
        base.clear();
        outer_docs.clear();
    }
    if is_tool(node) {
        return Plan::default();
    }

    let mut plan = Plan { base: base.clone(), ..Plan::default() };
    classify_level(node, &mut plan);
    if plan.to_root.is_empty() {
        return Plan::default();
    }
    plan.data_split = node.has_dir("Data");
    if plan.data_split {
        // only Data's contents and the root files are installed; the rest is docs and
        // sources (SKSE's src/)
        for entry in node.entries() {
            if entry.is_dir() && fold(entry.name()) == "data" {
                continue;
            }
            if !plan.to_root.iter().any(|n| n == entry.name()) {
                plan.dropped.push(join(&base, entry.name()));
            }
        }
        plan.unsure.clear();
    }
    plan.dropped.extend(outer_docs);
    plan
}

/// Applies a plan to the tree it was made from (`RootSorter::apply`).
pub fn apply(tree: &mut FileTree, plan: &Plan) {
    if tree.find_dir(&plan.base).is_none() {
        return;
    }
    tree.add_dir(ROOT);
    for name in &plan.to_root {
        tree.move_entry(&join(&plan.base, name), &format!("{ROOT}/"), Policy::Merge);
    }
    for path in &plan.dropped {
        tree.remove(path);
    }
    if plan.data_split
        && let Some(Node::Dir(data)) = tree.remove(&join(&plan.base, "Data"))
    {
        for entry in data.entries().to_vec() {
            tree.insert("", entry, Policy::Merge);
        }
    }
    // the wrapper chain is empty now
    if let Some(first) = plan.base.split('/').next().filter(|s| !s.is_empty()) {
        tree.remove(first);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checker::DataChecker;
    use crate::tree::listing;

    fn sse() -> impl Fn(&Dir) -> bool {
        let checker = DataChecker::for_game("Skyrim Special Edition").unwrap();
        move |d| checker.looks_valid(d)
    }

    fn sorted(paths: &[&str]) -> (Plan, Vec<String>) {
        let mut tree = listing(paths);
        let plan = plan_archive(&tree, sse());
        apply(&mut tree, &plan);
        (plan, tree.paths())
    }

    /// SKSE 2.0.20's archive: loader and dlls next to a Data folder, inside a wrapper, with
    /// docs and the sources (the case from the Leet Edition notes, change 4).
    #[test]
    fn skse_unwraps_and_splits_data() {
        let (plan, paths) = sorted(&[
            "skse64_2_00_20/skse64_loader.exe",
            "skse64_2_00_20/skse64_1_5_97.dll",
            "skse64_2_00_20/skse64_steam_loader.dll",
            "skse64_2_00_20/skse64_readme.txt",
            "skse64_2_00_20/skse64_whatsnew.txt",
            "skse64_2_00_20/Data/Scripts/Actor.pex",
            "skse64_2_00_20/src/skse64/main.cpp",
        ]);
        assert_eq!(plan.base, "skse64_2_00_20");
        assert!(plan.data_split);
        assert_eq!(
            paths,
            [
                "Root/",
                "Root/skse64_1_5_97.dll",
                "Root/skse64_loader.exe",
                "Root/skse64_steam_loader.dll",
                "Scripts/",
                "Scripts/Actor.pex"
            ]
        );
        assert!(plan.dropped.contains(&"skse64_2_00_20/src".to_owned()));
    }

    /// ENB 0.503: d3d11.dll, enblocal.ini, enbseries/ at the top, no Data.
    #[test]
    fn enb_binaries_and_files_go_to_root() {
        let (plan, paths) = sorted(&[
            "d3d11.dll",
            "d3dcompiler_46e.dll",
            "enblocal.ini",
            "enbseries.ini",
            "enbeffect.fx",
            "enbseries/effect.txt",
            "readme.txt",
        ]);
        assert!(!plan.data_split);
        assert_eq!(
            paths,
            [
                "Root/",
                "Root/d3d11.dll",
                "Root/d3dcompiler_46e.dll",
                "Root/enbeffect.fx",
                "Root/enblocal.ini",
                "Root/enbseries.ini",
                "Root/enbseries/",
                "Root/enbseries/effect.txt",
                "readme.txt"
            ]
        );
    }

    /// Address Library and KiLoader: Data content only, untouched.
    #[test]
    fn data_only_mods_are_left_alone() {
        assert!(plan_archive(&listing(&["SKSE/Plugins/versionlib-1-5-97-0.bin"]), sse()).is_empty());
        assert!(plan_archive(&listing(&["KiLoader/x.dll", "readme.txt"]), sse()).is_empty());
    }

    /// A lone wrapper folder is not unwrapped unless it holds a Data folder.
    #[test]
    fn a_wrapper_without_data_is_not_unwrapped() {
        let tree = listing(&["Wrapper/d3d11.dll", "Wrapper/readme.txt"]);
        assert!(plan_archive(&tree, sse()).is_empty(), "Wrapper/ is a folder: it stays");
    }

    #[test]
    fn tools_and_fomods_and_root_mods_are_never_sorted() {
        assert!(plan_archive(&listing(&["Pandora Behaviour Engine+.exe", "x.dll"]), sse()).is_empty());
        assert!(plan_archive(&listing(&["fomod/ModuleConfig.xml", "a.dll"]), sse()).is_empty());
        assert!(plan_archive(&listing(&["Root/a.dll", "b.dll"]), sse()).is_empty());
        assert!(plan_installed(&listing(&["SSEEdit.exe", "x.dll"])).is_empty());
    }

    /// `<plugin>.ini` stays next to its plugin; other top-level inis stay and are unsure.
    #[test]
    fn plugin_inis_stay_and_other_inis_are_unsure() {
        let tree = listing(&["My Mod.esp", "My Mod.ini", "Other.ini", "patch.dll"]);
        let plan = plan_installed(&tree);
        assert_eq!(plan.to_root, ["patch.dll"]);
        assert_eq!(plan.unsure, ["Other.ini"]);
    }

    #[test]
    fn planned_installed_tree_moves_only_the_top_level() {
        let mut tree = listing(&["SKSE/Plugins/a.dll", "b.asi", "enbseries.ini"]);
        let plan = plan_installed(&tree);
        apply(&mut tree, &plan);
        assert_eq!(tree.paths(), ["Root/", "Root/b.asi", "Root/enbseries.ini", "SKSE/", "SKSE/Plugins/", "SKSE/Plugins/a.dll"]);
    }
}
