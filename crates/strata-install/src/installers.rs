//! The installers that work on the archive's tree (MO2's `IPluginInstallerSimple`s): quick,
//! BAIN, bundle detection, FOMOD's tree building and the manual installer's model. Each
//! function names the plugin function it ports (Fluorine `libs/installer_*`, which are
//! MO2's).

use crate::checker::DataChecker;
use crate::fomod::FileInstall;
use crate::tree::{Dir, FileTree, Node, Policy, fold, suffix};

/// The installers in MO2's priority order (`IPluginInstaller::priority`, highest first).
/// FOMOD is 110 because its `prefer` setting is on by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Installer {
    /// "Fomod Installer", 110.
    Fomod,
    /// "Simple Installer", 50.
    Quick,
    /// "BAIN Installer", 40.
    Bain,
    /// "Bundle Installer", 10.
    Bundle,
    /// "Manual Installer", 0; the only manual installer.
    Manual,
}

impl Installer {
    pub const BY_PRIORITY: [Installer; 5] =
        [Installer::Fomod, Installer::Quick, Installer::Bain, Installer::Bundle, Installer::Manual];

    /// The plugin's name, used for its `[Plugins]` settings in `meta.ini`.
    pub fn plugin_name(self) -> &'static str {
        match self {
            Installer::Fomod => "Fomod Installer",
            Installer::Quick => "Simple Installer",
            Installer::Bain => "BAIN Installer",
            Installer::Bundle => "Bundle Installer",
            Installer::Manual => "Manual Installer",
        }
    }
}

fn path_join(base: &str, name: &str) -> String {
    if base.is_empty() { name.to_owned() } else { format!("{base}/{name}") }
}

// --- quick ----------------------------------------------------------------------------

/// `InstallerQuick::isDataTextArchiveTopLayer`: exactly one folder, named like the data
/// folder, and one or more "useless" files (text, pdf, pictures).
pub fn is_data_text(tree: &Dir, data_dir: &str) -> bool {
    const TEXT: &[&str] = &["txt", "pdf", "md", "jpg", "jpeg", "png", "bmp"];
    let mut data = false;
    let mut text = false;
    for entry in tree.entries() {
        if entry.is_dir() {
            if data || fold(entry.name()) != fold(data_dir) {
                return false;
            }
            data = true;
        } else if TEXT.contains(&fold(suffix(entry.name())).as_str()) {
            text = true;
        } else {
            return false;
        }
    }
    data && text
}

/// `InstallerQuick::getSimpleArchiveBase`: the folder (path) where valid content starts,
/// following single folders.
pub fn quick_base(tree: &Dir, checker: &DataChecker) -> Option<String> {
    let mut dir = tree;
    let mut path = String::new();
    loop {
        if checker.looks_valid(dir) || is_data_text(dir, checker.data_dir) {
            return Some(path);
        }
        match dir.entries() {
            [Node::Dir(only)] => {
                path = path_join(&path, &only.name);
                dir = only;
            }
            _ => return None,
        }
    }
}

/// `InstallerQuick::install` after the dialog: the base becomes the mod; a "Data + docs"
/// layer puts the docs into Data and Data's content at the top.
pub fn quick_install(tree: &mut FileTree, base: &str, checker: &DataChecker) {
    let mut new = tree.find_dir(base).cloned().unwrap_or_default();
    new.name.clear();
    if is_data_text(&new, checker.data_dir) {
        let Some(Node::Dir(mut data)) = new.remove(checker.data_dir) else { unreachable!("checked by is_data_text") };
        data.merge(new);
        data.name.clear();
        new = data;
    }
    *tree = new;
}

// --- BAIN -----------------------------------------------------------------------------

/// `InstallerBAIN::findSubpackages`: top-level folders that look like Data, and how many
/// other folders there are (`fomod`, `docs`, ... and `--` folders don't count).
pub fn bain_packages(tree: &Dir, checker: &DataChecker) -> (Vec<String>, usize) {
    const IGNORED: &[&str] = &["fomod", "omod conversion data", "images", "screenshots", "docs"];
    let mut packages = Vec::new();
    let mut invalid = 0;
    for entry in tree.entries() {
        let Node::Dir(d) = entry else { continue };
        if IGNORED.contains(&fold(&d.name).as_str()) || d.name.starts_with("--") {
            continue;
        }
        if checker.looks_valid(d) {
            packages.push(d.name.clone());
        } else {
            invalid += 1;
        }
    }
    (packages, invalid)
}

/// The BAIN dialog's ticks before the user changes anything: names starting with "00",
/// and the options of the previous install (`BainComplexInstallerDialog`).
pub fn bain_defaults(packages: &[String], previous: &[String]) -> Vec<bool> {
    packages.iter().map(|p| p.starts_with("00") || previous.iter().any(|o| fold(o) == fold(p))).collect()
}

/// `BainComplexInstallerDialog::updateTree`: the selected folders merged into a new tree
/// in tree order (a later package's file replaces an earlier one's).
pub fn bain_install(tree: &mut FileTree, selected: &[String]) {
    let mut new = Dir::default();
    for entry in tree.entries() {
        if let Node::Dir(d) = entry
            && selected.iter().any(|s| fold(s) == fold(&d.name))
        {
            new.merge(d.clone());
        }
    }
    *tree = new;
}

// --- bundle ---------------------------------------------------------------------------

/// The archive types the installation manager opens (`getSupportedExtensions` without
/// custom installers).
pub const ARCHIVE_EXTENSIONS: &[&str] = &["zip", "rar", "7z", "fomod", "001"];

/// `InstallerBundle::findObjects`: archives at the first level that has any, following a
/// single folder. Paths are from the top of the tree.
pub fn bundle_archives(tree: &Dir) -> Vec<String> {
    let mut dir = tree;
    let mut base = String::new();
    loop {
        let found: Vec<String> = dir
            .entries()
            .iter()
            .filter(|e| !e.is_dir() && ARCHIVE_EXTENSIONS.contains(&fold(suffix(e.name())).as_str()))
            .map(|e| path_join(&base, e.name()))
            .collect();
        if !found.is_empty() {
            return found;
        }
        let dirs: Vec<&Dir> = dir.entries().iter().filter_map(Node::as_dir).collect();
        match dirs.as_slice() {
            [only] => {
                base = path_join(&base, &only.name);
                dir = only;
            }
            _ => return Vec::new(),
        }
    }
}

// --- FOMOD ----------------------------------------------------------------------------

/// `InstallerFomod::findFomodDirectory`: the path of the `fomod` folder, following single
/// folders.
pub fn fomod_dir(tree: &Dir) -> Option<String> {
    let mut dir = tree;
    let mut base = String::new();
    loop {
        if let Some(Node::Dir(f)) = dir.child("fomod") {
            return Some(path_join(&base, &f.name));
        }
        match dir.entries() {
            [Node::Dir(only)] => {
                base = path_join(&base, &only.name);
                dir = only;
            }
            _ => return None,
        }
    }
}

/// `InstallerFomod::isArchiveSupported`: a `fomod` folder with `ModuleConfig.xml`.
pub fn is_fomod(tree: &Dir) -> bool {
    fomod_dir(tree)
        .and_then(|p| tree.find_dir(&p))
        .is_some_and(|d| d.entries().iter().any(|e| !e.is_dir() && fold(e.name()) == "moduleconfig.xml"))
}

/// The files the FOMOD dialog needs before anything else: `info.xml`, `ModuleConfig.xml`
/// and every picture in the archive (`InstallerFomod::buildFomodTree`).
pub fn fomod_files_to_read(tree: &Dir) -> Vec<String> {
    let Some(fomod) = fomod_dir(tree) else { return Vec::new() };
    let mut out = Vec::new();
    if let Some(dir) = tree.find_dir(&fomod) {
        for e in dir.entries() {
            if let Node::File(f) = e
                && matches!(fold(&f.name).as_str(), "info.xml" | "moduleconfig.xml")
            {
                out.push(f.source.clone());
            }
        }
    }
    for (_, source) in tree.files() {
        if matches!(fold(suffix(source)).as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp") {
            out.push(source.to_owned());
        }
    }
    out
}

/// `FomodInstallerDialog::updateTree`'s copying: builds the mod from the chosen file list.
/// `base` is the folder holding `fomod/` (sources are relative to it). Returns the new tree
/// and the sources that were not found (MO2 ignores the source "no folder").
pub fn fomod_install(tree: &FileTree, base: &str, files: &[&FileInstall]) -> (FileTree, Vec<String>) {
    let mut new = Dir::default();
    let mut missing = Vec::new();
    for f in files {
        let source = path_join(base, &f.source);
        if f.is_folder {
            let Some(src) = tree.find_dir(&source) else {
                missing.push(f.source.clone());
                continue;
            };
            let target = new.add_dir(&f.destination);
            for e in src.entries() {
                target.put_with(e.clone(), Policy::Merge);
            }
        } else {
            let Some(node) = tree.find(&source).cloned() else {
                missing.push(f.source.clone());
                continue;
            };
            new.insert(&f.destination, node, Policy::Replace);
        }
    }
    missing.retain(|m| fold(m) != "no folder");
    (new, missing)
}

// --- manual ---------------------------------------------------------------------------

/// The manual installer's result: the tree as the user left it and the folder chosen as
/// Data (`InstallDialog::getModifiedTree`: the "data root" item's subtree).
pub fn manual_install(tree: &mut FileTree, data_root: &str) -> bool {
    let Some(mut root) = tree.find_dir(data_root).cloned() else { return false };
    root.name.clear();
    *tree = root;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::listing;

    fn sse() -> DataChecker {
        DataChecker::for_game("Skyrim Special Edition").unwrap()
    }

    #[test]
    fn quick_follows_single_folders_and_merges_data_text() {
        let tree = listing(&["Wrap/Inner/meshes/a.nif", "Wrap/Inner/x.esp"]);
        assert_eq!(quick_base(&tree, &sse()).as_deref(), Some("Wrap/Inner"));
        let mut tree = listing(&["Data/textures/a.dds", "readme.txt"]);
        let base = quick_base(&tree, &sse()).unwrap();
        quick_install(&mut tree, &base, &sse());
        assert_eq!(tree.paths(), ["readme.txt", "textures/", "textures/a.dds"]);
        assert!(quick_base(&listing(&["a/x.esp", "b/y.esp"]), &sse()).is_none());
    }

    #[test]
    fn bain_packages_and_install() {
        let tree = listing(&[
            "00 Core/x.esp",
            "10 Option/textures/a.dds",
            "20 Other/textures/a.dds",
            "fomod/info.xml",
            "--notes/a.txt",
            "package.txt",
        ]);
        let (packages, invalid) = bain_packages(&tree, &sse());
        assert_eq!(packages, ["00 Core", "10 Option", "20 Other"]);
        assert_eq!(invalid, 0);
        assert_eq!(bain_defaults(&packages, &["20 OTHER".into()]), [true, false, true]);
        let mut t = tree.clone();
        bain_install(&mut t, &["00 Core".into(), "20 Other".into(), "10 Option".into()]);
        assert_eq!(t.paths(), ["textures/", "textures/a.dds", "x.esp"]);
        let Some(Node::File(f)) = t.find("textures/a.dds") else { panic!() };
        assert_eq!(f.source, "20 Other/textures/a.dds", "later folder wins");
    }

    #[test]
    fn bundle_and_fomod_detection() {
        assert_eq!(bundle_archives(&listing(&["Wrap/inner.7z", "Wrap/readme.txt"])), ["Wrap/inner.7z"]);
        assert!(bundle_archives(&listing(&["a/x.7z", "b/y.7z"])).is_empty());
        let tree = listing(&["Wrap/fomod/ModuleConfig.XML", "Wrap/fomod/pic.png", "Wrap/core/a.esp"]);
        assert_eq!(fomod_dir(&tree).as_deref(), Some("Wrap/fomod"));
        assert!(is_fomod(&tree));
        assert_eq!(fomod_files_to_read(&tree), ["Wrap/fomod/ModuleConfig.XML", "Wrap/fomod/pic.png"]);
        assert!(!is_fomod(&listing(&["fomod/info.xml"])));
    }

    #[test]
    fn fomod_copies_by_priority_and_reports_missing() {
        let tree = listing(&["W/fomod/ModuleConfig.xml", "W/core/a.esp", "W/core/meshes/m.nif", "W/opt/a.esp", "W/f.txt"]);
        let f = |source: &str, destination: &str, is_folder| FileInstall {
            source: source.into(),
            destination: destination.into(),
            is_folder,
            priority: 0,
            sequence: 0,
        };
        let files = [f("core", "", true), f("opt", "", true), f("f.txt", "docs\\", false), f("f.txt", "renamed.txt", false), f("gone", "", true), f("no folder", "", true)];
        let refs: Vec<&FileInstall> = files.iter().collect();
        let (new, missing) = fomod_install(&tree, "W", &refs);
        assert_eq!(new.paths(), ["a.esp", "docs/", "docs/f.txt", "meshes/", "meshes/m.nif", "renamed.txt"]);
        let Some(Node::File(a)) = new.find("a.esp") else { panic!() };
        assert_eq!(a.source, "W/opt/a.esp");
        assert_eq!(missing, ["gone"]);
    }

    #[test]
    fn manual_data_root() {
        let mut tree = listing(&["Wrap/meshes/a.nif", "readme.txt"]);
        assert!(manual_install(&mut tree, "Wrap"));
        assert_eq!(tree.paths(), ["meshes/", "meshes/a.nif"]);
    }
}
