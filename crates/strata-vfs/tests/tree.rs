//! The merged tree on real folders: who wins, what is hidden, where writes land.

use std::fs;
use std::path::{Path, PathBuf};

use strata_game::{Game, Layer, LayerKind, Mapping, SkyrimSE};
use strata_vfs::{Options, Tree, tree::ROOT};

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

impl Fixture {
    /// game + mods "low" and "high" + overwrite, with the files given as (layer, path, text).
    fn new(files: &[(&str, &str, &str)]) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_owned();
        for dir in ["game", "low", "high", "overwrite"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        for (layer, path, text) in files {
            put(&root.join(layer).join(path), text);
        }
        Self { _tmp: tmp, root }
    }

    fn layers(&self) -> Vec<Layer> {
        let rules = SkyrimSE.mod_mappings();
        let layer = |id: &str, kind, rules| Layer { id: id.into(), kind, source: self.root.join(id), rules };
        vec![
            layer(
                "game",
                LayerKind::Game,
                vec![Mapping::new("", "")],
            ),
            layer("low", LayerKind::Mod, rules.clone()),
            layer("high", LayerKind::Mod, rules.clone()),
            layer("overwrite", LayerKind::Overwrite, rules),
        ]
    }

    fn tree(&self) -> Tree {
        Tree::build(self.layers(), Options::default(), Some(self.root.join("strata/whiteouts.txt"))).unwrap()
    }

    fn read(&self, tree: &Tree, path: &str) -> Option<String> {
        let id = tree.resolve(path)?;
        fs::read_to_string(tree.real_path(id)?).ok()
    }

    fn origin(&self, tree: &Tree, path: &str) -> String {
        let id = tree.resolve(path).unwrap_or_else(|| panic!("{path} not in the tree"));
        tree.layer_of(id).unwrap().id.clone()
    }

    fn disk(&self, path: &str) -> Option<String> {
        fs::read_to_string(self.root.join(path)).ok()
    }
}

fn listing(tree: &Tree) -> Vec<String> {
    tree.files().into_iter().map(|(path, _)| path).collect()
}

#[test]
fn the_highest_layer_wins_and_folders_merge() {
    let f = Fixture::new(&[
        ("game", "SkyrimSE.exe", "exe"),
        ("game", "Data/Skyrim.esm", "esm"),
        ("game", "Data/textures/a.dds", "game"),
        ("low", "textures/a.dds", "low"),
        ("low", "textures/b.dds", "low"),
        ("high", "Textures/A.DDS", "high"),
        ("high", "meshes/m.nif", "high"),
        ("overwrite", "textures/b.dds", "overwrite"),
    ]);
    let tree = f.tree();
    assert_eq!(
        listing(&tree),
        ["Data/meshes/m.nif", "Data/Skyrim.esm", "Data/textures/a.dds", "Data/textures/b.dds", "SkyrimSE.exe"]
    );
    assert_eq!(f.origin(&tree, "Data/textures/a.dds"), "high");
    assert_eq!(f.read(&tree, "data/TEXTURES/a.DDS").as_deref(), Some("high"));
    assert_eq!(f.origin(&tree, "Data/textures/b.dds"), "overwrite");
    assert_eq!(f.origin(&tree, "Data/Skyrim.esm"), "game");
    assert!(tree.resolve("Data/nope").is_none());
    assert!(tree.node(tree.resolve("Data/Textures").unwrap()).unwrap().is_dir());
}

#[test]
fn root_folders_land_in_the_game_folder_and_stay_out_of_data() {
    let f = Fixture::new(&[
        ("game", "SkyrimSE.exe", "exe"),
        ("game", "Data/Skyrim.esm", "esm"),
        ("low", "root/skse64_loader.exe", "skse"),
        ("low", "root/Data/never.txt", "x"),
        ("low", "root/Skyrim.ini", "x"),
        ("low", "SKSE/Plugins/p.dll", "dll"),
        ("low", "meta.ini", "[General]"),
        ("high", "Root/enbseries.ini", "enb"),
        ("high", "Root/enbseries/effect.fx", "fx"),
        ("high", "meshes/root/kept.nif", "nested root is a normal folder"),
        ("high", "hidden.esp.mohidden", "x"),
        ("high", ".git/config", "x"),
    ]);
    let tree = f.tree();
    assert_eq!(
        listing(&tree),
        [
            "Data/meshes/root/kept.nif",
            "Data/SKSE/Plugins/p.dll",
            "Data/Skyrim.esm",
            "enbseries.ini",
            "enbseries/effect.fx",
            "skse64_loader.exe",
            "SkyrimSE.exe"
        ]
    );
    assert_eq!(f.origin(&tree, "skse64_loader.exe"), "low");
}

#[test]
fn new_files_go_to_overwrite_under_the_right_rule() {
    let f = Fixture::new(&[("game", "Data/Skyrim.esm", "esm"), ("low", "SKSE/Plugins/p.dll", "dll")]);
    let mut tree = f.tree();

    // Data/SKSE/Plugins/new.log -> overwrite/SKSE/Plugins/new.log
    let plugins = tree.resolve("Data/skse/plugins").unwrap();
    let (id, real) = tree.create_file(plugins, "new.log", 0o644).unwrap();
    fs::write(&real, "log").unwrap();
    assert_eq!(real, f.root.join("overwrite/SKSE/Plugins/new.log"));
    assert_eq!(tree.path(id), "Data/SKSE/Plugins/new.log");

    // <game folder>/enbcache/x -> overwrite/Root/enbcache/x
    let cache = tree.create_dir(ROOT, "enbcache").unwrap();
    let (_, real) = tree.create_file(cache, "x.bin", 0o644).unwrap();
    assert_eq!(real, f.root.join("overwrite/Root/enbcache/x.bin"));

    // names the rules leave out cannot be made
    let data = tree.resolve("Data").unwrap();
    assert_eq!(tree.create_dir(data, "Root").unwrap_err().raw_os_error(), Some(libc::EACCES));
    assert_eq!(tree.create_file(data, "meta.ini", 0o644).unwrap_err().raw_os_error(), Some(libc::EACCES));
    assert_eq!(tree.create_file(plugins, "P.DLL", 0o644).unwrap_err().raw_os_error(), Some(libc::EEXIST));

    // and a rebuilt tree sees what was made
    fs::write(f.root.join("overwrite/Root/enbcache/x.bin"), "bin").unwrap();
    let again = f.tree();
    assert_eq!(f.read(&again, "enbcache/x.bin").as_deref(), Some("bin"));
    assert_eq!(f.read(&again, "Data/SKSE/Plugins/new.log").as_deref(), Some("log"));
}

#[test]
fn changing_a_lower_file_copies_it_to_overwrite_first() {
    let f = Fixture::new(&[
        ("game", "Data/Skyrim.esm", "esm"),
        ("game", "steam_api64.cfg", "game file"),
        ("low", "scripts/s.pex", "low"),
        ("high", "Root/enbseries.ini", "enb"),
    ]);
    let mut tree = f.tree();

    let id = tree.resolve("Data/Scripts/S.PEX").unwrap();
    let real = tree.make_writable(id).unwrap();
    assert_eq!(real, f.root.join("overwrite/scripts/s.pex"));
    fs::write(&real, "changed").unwrap();
    assert_eq!(f.disk("low/scripts/s.pex").as_deref(), Some("low"), "the mod's file is untouched");
    assert_eq!(f.origin(&tree, "Data/scripts/s.pex"), "overwrite");
    assert_eq!(tree.make_writable(id).unwrap(), real, "a second time changes nothing");

    // a game file: copied up under Root
    let id = tree.resolve("steam_api64.cfg").unwrap();
    assert_eq!(tree.make_writable(id).unwrap(), f.root.join("overwrite/Root/steam_api64.cfg"));
    assert_eq!(f.disk("game/steam_api64.cfg").as_deref(), Some("game file"));

    // a mod's Root file is edited in place (Leet change 6)
    let id = tree.resolve("enbseries.ini").unwrap();
    assert_eq!(tree.make_writable(id).unwrap(), f.root.join("high/Root/enbseries.ini"));
}

#[test]
fn deleting_a_lower_file_leaves_a_whiteout_that_survives_a_rebuild() {
    let f = Fixture::new(&[
        ("game", "Data/Skyrim.esm", "esm"),
        ("low", "textures/a.dds", "low"),
        ("low", "textures/keep.dds", "low"),
        ("overwrite", "textures/a.dds", "overwrite"),
        ("overwrite", "only.txt", "overwrite"),
    ]);
    let mut tree = f.tree();
    let textures = tree.resolve("Data/textures").unwrap();
    let data = tree.resolve("Data").unwrap();

    // only in overwrite: really deleted, no whiteout
    tree.unlink(data, "ONLY.txt").unwrap();
    assert!(f.disk("overwrite/only.txt").is_none());
    assert!(f.disk("strata/whiteouts.txt").is_none());

    // in overwrite over a mod's file: deleted from overwrite, the mod's file stays hidden
    tree.unlink(textures, "a.dds").unwrap();
    assert!(tree.resolve("Data/textures/a.dds").is_none());
    assert!(f.disk("overwrite/textures/a.dds").is_none());
    assert_eq!(f.disk("low/textures/a.dds").as_deref(), Some("low"));
    assert!(f.disk("strata/whiteouts.txt").unwrap().contains("Data/textures/a.dds\n"));
    assert_eq!(tree.unlink(textures, "a.dds").unwrap_err().raw_os_error(), Some(libc::ENOENT));

    let again = f.tree();
    assert_eq!(listing(&again), ["Data/Skyrim.esm", "Data/textures/keep.dds"]);

    // making the file again ends the whiteout
    let mut again = again;
    let textures = again.resolve("Data/textures").unwrap();
    let (_, real) = again.create_file(textures, "A.dds", 0o644).unwrap();
    fs::write(real, "new").unwrap();
    assert!(!f.disk("strata/whiteouts.txt").unwrap().contains("a.dds"));
    assert_eq!(f.read(&f.tree(), "Data/textures/a.dds").as_deref(), Some("new"));
}

#[test]
fn folders_are_removed_only_when_empty_and_stay_hidden() {
    let f = Fixture::new(&[("game", "Data/Skyrim.esm", "esm"), ("low", "interface/x.swf", "low")]);
    let mut tree = f.tree();
    let data = tree.resolve("Data").unwrap();
    assert_eq!(tree.rmdir(data, "interface").unwrap_err().raw_os_error(), Some(libc::ENOTEMPTY));
    let interface = tree.resolve("Data/interface").unwrap();
    tree.unlink(interface, "x.swf").unwrap();
    tree.rmdir(data, "Interface").unwrap();
    assert!(tree.resolve("Data/interface").is_none());
    assert_eq!(listing(&f.tree()), ["Data/Skyrim.esm"]);

    // a folder made and removed in overwrite leaves nothing behind
    let made = tree.create_dir(data, "temp").unwrap();
    assert!(f.root.join("overwrite/temp").is_dir());
    assert!(tree.node(made).is_some());
    tree.rmdir(data, "temp").unwrap();
    assert!(!f.root.join("overwrite/temp").exists());
    assert!(!f.disk("strata/whiteouts.txt").unwrap().contains("temp"));
}

#[test]
fn renames_end_up_in_overwrite() {
    let f = Fixture::new(&[
        ("game", "Data/Skyrim.esm", "esm"),
        ("low", "a/one.txt", "one"),
        ("low", "a/sub/two.txt", "two"),
        ("low", "target.txt", "old target"),
        ("overwrite", "save.tmp", "new save"),
    ]);
    let mut tree = f.tree();
    let data = tree.resolve("Data").unwrap();

    // the "write a temp file, rename it over the real one" pattern
    tree.rename(data, "save.tmp", data, "Target.txt").unwrap();
    assert_eq!(f.read(&tree, "Data/target.txt").as_deref(), Some("new save"));
    assert!(tree.resolve("Data/save.tmp").is_none());
    assert_eq!(f.disk("low/target.txt").as_deref(), Some("old target"));
    assert!(!f.disk("strata/whiteouts.txt").unwrap_or_default().contains("target"));

    // a folder of a mod: copied to overwrite under the new name, the old name whited out
    tree.rename(data, "a", data, "b").unwrap();
    assert_eq!(f.read(&tree, "Data/b/one.txt").as_deref(), Some("one"));
    assert_eq!(f.read(&tree, "Data/B/Sub/two.txt").as_deref(), Some("two"));
    assert!(tree.resolve("Data/a").is_none());
    assert_eq!(f.disk("overwrite/b/sub/two.txt").as_deref(), Some("two"));
    assert_eq!(f.disk("low/a/sub/two.txt").as_deref(), Some("two"));

    let again = f.tree();
    assert_eq!(listing(&again), ["Data/b/one.txt", "Data/b/sub/two.txt", "Data/Skyrim.esm", "Data/target.txt"]);

    // a folder cannot move into itself
    let b = tree.resolve("Data/b").unwrap();
    let sub = tree.resolve("Data/b/sub").unwrap();
    assert_eq!(tree.rename(data, "b", sub, "b").unwrap_err().raw_os_error(), Some(libc::EINVAL));
    assert!(tree.node(b).is_some());
}

#[test]
fn without_an_overwrite_layer_the_tree_is_read_only() {
    let f = Fixture::new(&[("game", "Data/Skyrim.esm", "esm")]);
    let mut layers = f.layers();
    layers.pop();
    let mut tree = Tree::build(layers, Options::default(), None).unwrap();
    let data = tree.resolve("Data").unwrap();
    assert_eq!(tree.create_file(data, "x", 0o644).unwrap_err().raw_os_error(), Some(libc::EROFS));
    assert_eq!(tree.unlink(data, "Skyrim.esm").unwrap_err().raw_os_error(), Some(libc::EROFS));
}

/// A profile mount (`docs/DESIGN.md`, "The overlay"): the prefix's folder underneath, written straight
/// through, with a few files of the profile folder laid over it.
#[test]
fn a_profile_mount_shows_the_profiles_files_and_writes_the_rest_through() {
    let tmp = tempfile::tempdir().unwrap();
    let (prefix, profile) = (tmp.path().join("prefix"), tmp.path().join("profile"));
    put(&prefix.join("Skyrim.ini"), "prefix ini");
    put(&prefix.join("SKSE/skse64.log"), "old log");
    put(&prefix.join("Saves/global.ess"), "global save");
    put(&profile.join("skyrim.ini"), "profile ini");
    put(&profile.join("modlist.txt"), "never shown");
    put(&profile.join("saves/local.ess"), "local save");
    let layers = vec![
        Layer {
            id: "<prefix>".into(),
            kind: LayerKind::Passthrough,
            source: prefix.clone(),
            rules: vec![Mapping { exclude: vec!["Saves".into()], ..Mapping::new("", "") }],
        },
        Layer {
            id: "<profile>".into(),
            kind: LayerKind::Overwrite,
            source: profile.clone(),
            rules: vec![
                Mapping { include: vec!["Skyrim.ini".into(), "SkyrimPrefs.ini".into()], ..Mapping::new("", "") },
                Mapping::new("saves", "Saves"),
            ],
        },
    ];
    let mut tree = Tree::build(layers, Options::default(), None).unwrap();
    assert_eq!(listing(&tree), ["Saves/local.ess", "SKSE/skse64.log", "Skyrim.ini"]);
    let read = |tree: &Tree, path: &str| fs::read_to_string(tree.real_path(tree.resolve(path).unwrap()).unwrap()).unwrap();
    assert_eq!(read(&tree, "Skyrim.ini"), "profile ini");

    // a profile file that does not exist yet is created in the profile
    let (_, real) = tree.create_file(ROOT, "SkyrimPrefs.ini", 0o644).unwrap();
    assert_eq!(real, profile.join("SkyrimPrefs.ini"));
    // anything else is created in the prefix, as without Strata
    let (_, real) = tree.create_file(ROOT, "RendererInfo.txt", 0o644).unwrap();
    assert_eq!(real, prefix.join("RendererInfo.txt"));
    let skse = tree.resolve("skse").unwrap();
    let (_, real) = tree.create_file(skse, "new.log", 0o644).unwrap();
    assert_eq!(real, prefix.join("SKSE/new.log"));
    // existing files are written where they are
    assert_eq!(tree.make_writable(tree.resolve("SKSE/skse64.log").unwrap()).unwrap(), prefix.join("SKSE/skse64.log"));
    assert_eq!(tree.make_writable(tree.resolve("Skyrim.ini").unwrap()).unwrap(), profile.join("skyrim.ini"));
    // saves go to the profile
    let saves = tree.resolve("Saves").unwrap();
    let (_, real) = tree.create_file(saves, "quick.ess", 0o644).unwrap();
    assert_eq!(real, profile.join("saves/quick.ess"));
    // a prefix file is really deleted, without a whiteout
    put(&prefix.join("SKSE/new.log"), "x");
    tree.unlink(skse, "new.log").unwrap();
    assert!(!prefix.join("SKSE/new.log").exists());
}

/// Names compare character by character, as Windows compares them.
#[test]
fn case_folding_has_no_context() {
    use strata_vfs::tree::fold;
    assert_eq!(fold("Data/Textures/A.DDS"), "data/textures/a.dds");
    assert_eq!(fold("ÜNÏCÖDÉ"), fold("ünïcödé"));
    // a word-final capital sigma is the same letter as any other sigma
    assert_eq!(fold("ΟΔΟΣ.txt"), fold("οδοσ.txt"));
    assert_ne!(fold("ΟΔΟΣ"), fold("οδος"));
    // a capital whose lower case takes two characters stays itself
    assert_ne!(fold("İ"), fold("i\u{307}"));
    assert_ne!(fold("İ"), fold("i"));
}
