use super::*;
use std::path::Path;
use strata_game::Mapping;

/// Skyrim's rules for a mod (as `SkyrimSE::mod_mappings`, without the ini list).
fn mod_rules() -> Vec<Mapping> {
    vec![
        Mapping { exclude: vec!["Root".into(), "meta.ini".into()], ..Mapping::new("", "Data") },
        Mapping { exclude: vec!["Data".into()], write_back: true, ..Mapping::new("Root", "") },
    ]
}

fn write(root: &Path, files: &[&str]) {
    for f in files {
        let p = root.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, f).unwrap();
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_owned();
        Self { _tmp: tmp, root }
    }

    fn game(&self, files: &[&str]) -> Layer {
        let dir = self.root.join("game");
        write(&dir, files);
        std::fs::create_dir_all(&dir).unwrap();
        Layer { id: "<game>".into(), kind: LayerKind::Game, source: dir, rules: vec![Mapping::new("", "")] }
    }

    fn layer(&self, id: &str, kind: LayerKind, files: &[&str]) -> Layer {
        let dir = self.root.join("mods").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        write(&dir, files);
        Layer { id: id.into(), kind, source: dir, rules: mod_rules() }
    }

    fn mod_(&self, id: &str, files: &[&str]) -> Layer {
        self.layer(id, LayerKind::Mod, files)
    }
}

/// The layer ids of a provider list.
fn names(list: Option<Vec<Provider<'_>>>) -> Vec<String> {
    list.unwrap().iter().map(|p| match p.archive {
        Some(a) => format!("{}[{a}]", p.layer),
        None => p.layer.to_owned(),
    }).collect()
}

fn state(index: &ConflictIndex, id: &str) -> ConflictState {
    index.flags(id).unwrap().state
}

#[test]
fn flags_follow_mo2() {
    let fx = Fixture::new();
    let layers = vec![
        fx.game(&["Data/Skyrim.esm", "Data/shared.txt", "SkyrimSE.exe"]),
        fx.mod_("R", &["a1.txt"]),
        fx.mod_("A", &["shared.txt", "a1.txt", "ab.txt"]),
        fx.mod_("B", &["AB.TXT", "b1.txt"]),
        fx.mod_("Empty", &[]),
        fx.layer("<overwrite>", LayerKind::Overwrite, &[]),
    ];
    let index = ConflictIndex::build(&layers, "Data", None);

    let r = index.flags("R").unwrap();
    assert_eq!(r.state, ConflictState::Redundant);
    assert_eq!(r.overwritten_by, ["A"]);

    let a = index.flags("A").unwrap();
    assert_eq!(a.state, ConflictState::Mixed);
    assert_eq!(a.overwrites, ["R"], "shared.txt over the game alone is no conflict");
    assert_eq!(a.overwritten_by, ["B"]);

    let b = index.flags("B").unwrap();
    assert_eq!((b.state, b.overwrites.as_slice()), (ConflictState::Overwrite, &["A".to_owned()][..]));
    assert_eq!(state(&index, "Empty"), ConflictState::None);
    assert_eq!(state(&index, "<overwrite>"), ConflictState::None);
    assert_eq!(index.flags("<game>"), None, "the game is not a mod");

    assert_eq!(names(index.origins(Area::Data, "ab.txt")), ["A", "B"]);
    assert_eq!(names(index.origins(Area::Data, "/SHARED.txt")), ["<game>", "A"]);
    assert_eq!(index.origins(Area::Root, "SkyrimSE.exe"), None, "the game folder is not indexed");
}

#[test]
fn losing_a_file_the_game_also_has_is_mixed_as_in_mo2() {
    let fx = Fixture::new();
    let layers = vec![fx.game(&["Data/x.txt"]), fx.mod_("M", &["x.txt", "m.txt"]), fx.mod_("N", &["x.txt"])];
    let index = ConflictIndex::build(&layers, "Data", None);
    let m = index.flags("M").unwrap();
    assert_eq!(m.state, ConflictState::Mixed);
    assert_eq!(m.overwrites, ["<game>"]);
    assert_eq!(m.overwritten_by, ["N"]);
    let n = index.flags("N").unwrap();
    assert_eq!((n.state, n.overwrites.as_slice()), (ConflictState::Overwrite, &["<game>".to_owned(), "M".to_owned()][..]));
}

#[test]
fn root_files_are_their_own_structure() {
    let fx = Fixture::new();
    let layers = vec![
        fx.game(&["skse64_loader.exe", "Data/Skyrim.esm"]),
        fx.mod_("SKSE", &["Root/skse64_loader.exe", "Root/skse64_1_5_97.dll", "skse64_loader.exe"]),
        fx.mod_("SKSE copy", &["root/SKSE64_loader.exe"]),
        fx.layer("<overwrite>", LayerKind::Overwrite, &["Root/skse64_1_5_97.dll"]),
    ];
    let index = ConflictIndex::build(&layers, "Data", None);
    assert_eq!(names(index.origins(Area::Root, "skse64_loader.exe")), ["SKSE", "SKSE copy"]);
    assert_eq!(names(index.origins(Area::Root, "skse64_1_5_97.dll")), ["SKSE"], "overwrite has no Root structure");
    assert_eq!(names(index.origins(Area::Data, "skse64_loader.exe")), ["SKSE"], "a Data file of the same name");
    assert_eq!(state(&index, "SKSE"), ConflictState::Overwritten);
    assert_eq!(state(&index, "SKSE copy"), ConflictState::Overwrite);
    let files = index.files_of("SKSE").unwrap();
    let shown: Vec<String> = files.iter().map(FileConflict::shown).collect();
    assert_eq!(shown, ["/skse64_loader.exe", "/Root/skse64_1_5_97.dll", "/Root/skse64_loader.exe"]);
    assert_eq!(files[2].winner(), "SKSE copy");
}

#[test]
fn clean_structure_and_hidden_files() {
    let fx = Fixture::new();
    let layers = vec![
        fx.game(&["Data/readme.txt"]),
        fx.mod_("A", &["meta.ini", "Readme.txt", "fomod/ModuleConfig.xml", "docs/readme.txt", "x.dds.mohidden"]),
        fx.mod_("B", &["readme.txt", "docs/readme.txt", "x.dds.mohidden", "fomod/info.xml", "Root/fomod/x"]),
    ];
    let index = ConflictIndex::build(&layers, "Data", None);
    let a: Vec<String> = index.files_of("A").unwrap().iter().map(FileConflict::shown).collect();
    assert_eq!(a, ["/docs/readme.txt", "/x.dds.mohidden"]);
    assert_eq!(index.origins(Area::Data, "readme.txt"), None);
    assert_eq!(names(index.origins(Area::Data, "x.dds.mohidden")), ["A", "B"]);
    assert!(index.flags("A").unwrap().hidden_files);
    assert_eq!(index.origins(Area::Root, "fomod/x"), None, "Root's top-level fomod goes too");
}

#[test]
fn update_reports_who_changed_and_rescans_nothing_known() {
    let fx = Fixture::new();
    let game = fx.game(&["Data/Skyrim.esm"]);
    let (r, a, b) = (fx.mod_("R", &["a1.txt"]), fx.mod_("A", &["a1.txt", "ab.txt"]), fx.mod_("B", &["ab.txt"]));
    let mut index = ConflictIndex::build(&[game.clone(), r.clone(), a.clone(), b.clone()], "Data", None);
    assert_eq!(state(&index, "A"), ConflictState::Mixed);

    // B off: A and B change, R does not.
    assert_eq!(index.update(&[game.clone(), r.clone(), a.clone()]), ["A", "B"]);
    assert_eq!(state(&index, "A"), ConflictState::Overwrite);
    assert!(index.flags("B").is_none());

    // B's folder disappears from disk: enabling it again uses what was scanned.
    std::fs::remove_dir_all(&b.source).unwrap();
    assert_eq!(index.update(&[game.clone(), r.clone(), a.clone(), b.clone()]), ["A", "B"]);
    assert_eq!(state(&index, "A"), ConflictState::Mixed);

    // B below A: A wins everything.
    let changed = index.update(&[game.clone(), r.clone(), b.clone(), a.clone()]);
    assert_eq!(changed, ["A", "B"]);
    assert_eq!(state(&index, "A"), ConflictState::Overwrite);
    assert_eq!(state(&index, "B"), ConflictState::Redundant);
    assert!(index.update(&[game, r, b, a]).is_empty(), "the same list changes nothing");
}

/// After any sequence of toggles and moves, the incrementally updated index equals a fresh build.
#[test]
fn incremental_equals_a_fresh_build() {
    let fx = Fixture::new();
    let game = fx.game(&["Data/f0.txt", "Data/f3.txt"]);
    let mut seed = 0x2545_f491_u64;
    let mut rand = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let all: Vec<Layer> = (0..12)
        .map(|i| {
            let files: Vec<String> = (0..6)
                .map(|_| {
                    let n = rand(10);
                    if rand(4) == 0 { format!("Root/r{n}.dll") } else { format!("f{n}.txt") }
                })
                .collect();
            let refs: Vec<&str> = files.iter().map(String::as_str).collect();
            fx.mod_(&format!("m{i}"), &refs)
        })
        .collect();
    let overwrite = fx.layer("<overwrite>", LayerKind::Overwrite, &["f1.txt"]);
    let mut on: Vec<Layer> = all[..8].to_vec();
    let list = |on: &[Layer]| {
        let mut l = vec![game.clone()];
        l.extend_from_slice(on);
        l.push(overwrite.clone());
        l
    };
    let mut index = ConflictIndex::build(&list(&on), "Data", None);
    for step in 0..200 {
        match rand(3) {
            0 if on.len() > 1 => {
                let m = on.remove(rand(on.len()));
                let at = rand(on.len() + 1);
                on.insert(at, m);
            }
            1 if !on.is_empty() => {
                on.remove(rand(on.len()));
            }
            _ => {
                let candidate = &all[rand(all.len())];
                if !on.iter().any(|l| l.id == candidate.id) {
                    let at = rand(on.len() + 1);
                    on.insert(at, candidate.clone());
                }
            }
        }
        let layers = list(&on);
        index.update(&layers);
        let fresh = ConflictIndex::build(&layers, "Data", None);
        for l in &layers {
            assert_eq!(index.flags(&l.id), fresh.flags(&l.id), "step {step}, {}", l.id);
            assert_eq!(index.files_of(&l.id), fresh.files_of(&l.id), "step {step}, {}", l.id);
        }
        assert_eq!(index.file_count(), fresh.file_count(), "step {step}");
    }
}

#[test]
fn longest_increasing_run() {
    let pick = |v: &[u32]| {
        let mut out: Vec<usize> = longest_increasing(v).into_iter().collect();
        out.sort_unstable();
        out
    };
    assert_eq!(pick(&[0, 1, 2, 3]), [0, 1, 2, 3]);
    assert_eq!(pick(&[0, 3, 1, 2]), [0, 2, 3], "one layer moved up");
    assert_eq!(pick(&[]), Vec::<usize>::new());
    assert_eq!(pick(&[2, 1, 0]).len(), 1);
}

/// A minimal uncompressed Skyrim SE BSA (hashes left at zero: readers that matter ignore them).
fn write_bsa(path: &Path, files: &[&str]) {
    let mut folders: Vec<(String, Vec<String>)> = Vec::new();
    for f in files {
        let (dir, name) = f.rsplit_once('/').unwrap_or(("", f));
        match folders.iter_mut().find(|(d, _)| d == dir) {
            Some((_, list)) => list.push(name.into()),
            None => folders.push((dir.into(), vec![name.into()])),
        }
    }
    let folder_names: usize = folders.iter().map(|(d, _)| d.len() + 1).sum();
    let file_names: usize = files.iter().map(|f| f.rsplit('/').next().unwrap().len() + 1).sum();
    let data_start = 36 + folders.len() * 24 + folders.len() + folder_names + files.len() * 16 + file_names;
    let mut out = b"BSA\0".to_vec();
    for v in [105u32, 36, 3, folders.len() as u32, files.len() as u32, folder_names as u32, file_names as u32, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for (_, list) in &folders {
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&(list.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 12]);
    }
    let mut offset = data_start;
    for (dir, list) in &folders {
        out.push(dir.len() as u8 + 1);
        out.extend_from_slice(dir.as_bytes());
        out.push(0);
        for name in list {
            out.extend_from_slice(&[0; 8]);
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += name.len();
        }
    }
    for (_, list) in &folders {
        for name in list {
            out.extend_from_slice(name.as_bytes());
            out.push(0);
        }
    }
    for (_, list) in &folders {
        for name in list {
            out.extend_from_slice(name.as_bytes());
        }
    }
    std::fs::write(path, out).unwrap();
}

#[test]
fn archives_join_the_data_structure() {
    let fx = Fixture::new();
    let b = fx.mod_("B", &[]);
    write_bsa(&b.source.join("B.bsa"), &["textures/a.dds", "textures/b.dds"]);
    let a = fx.mod_("A", &["textures/a.dds"]);
    let c = fx.mod_("C", &[]);
    write_bsa(&c.source.join("C - Textures.bsa"), &["textures/b.dds", "meshes/c.nif"]);
    let d = fx.mod_("D", &[]);
    write_bsa(&d.source.join("D.bsa"), &["meshes/c.nif"]);
    let e = fx.mod_("E", &[]);
    write_bsa(&e.source.join("E.bsa"), &["meshes/c.nif"]);
    let f = fx.mod_("F", &["x.txt"]);
    write_bsa(&f.source.join("F.bsa"), &["x.txt", "readme.txt"]);
    let rules = ArchiveRules {
        enabled: ["B.bsa", "C - Textures.bsa", "E.bsa", "F.bsa"].map(String::from).into(),
        load_order: ["Skyrim.esm", "B.esp", "c.esp"].map(String::from).into(),
    };
    let layers = vec![fx.game(&["Data/Skyrim.esm"]), b, a, c, d, e, f];
    let index = ConflictIndex::build(&layers, "Data", Some(rules.clone()));

    assert_eq!(names(index.origins(Area::Data, "textures/a.dds")), ["B[B.bsa]", "A"], "loose beats archived");
    assert_eq!(names(index.origins(Area::Data, "textures/b.dds")), ["B[B.bsa]", "C[C - Textures.bsa]"], "by plugin order");
    assert_eq!(
        names(index.origins(Area::Data, "meshes/c.nif")),
        ["C[C - Textures.bsa]", "E[E.bsa]"],
        "an archive without a plugin sorts last; D.bsa is not loaded"
    );
    assert_eq!(names(index.origins(Area::Data, "x.txt")), ["F"], "the mod's loose file hides its archived one");
    assert_eq!(index.origins(Area::Data, "readme.txt"), None, "cleanStructure applies to archives too");

    let fb = index.flags("B").unwrap();
    assert_eq!(fb.state, ConflictState::None, "B.bsa itself is a loose file only B has");
    assert_eq!((fb.archive_loose_state, fb.archive_loose_overwritten_by.as_slice()), (ConflictState::Overwritten, &["A".to_owned()][..]));
    assert_eq!((fb.archive_state, fb.archive_overwritten_by.as_slice()), (ConflictState::Overwritten, &["C".to_owned()][..]));
    let fa = index.flags("A").unwrap();
    assert_eq!((fa.state, fa.archive_loose_state), (ConflictState::None, ConflictState::Overwrite));
    assert_eq!(fa.archive_loose_overwrites, ["B"]);
    let fc = index.flags("C").unwrap();
    assert_eq!(fc.archive_state, ConflictState::Mixed);
    assert_eq!((fc.archive_overwrites.as_slice(), fc.archive_overwritten_by.as_slice()), (&["B".to_owned()][..], &["E".to_owned()][..]));
    assert_eq!(index.flags("D").unwrap(), Flags::default());

    // Without archive parsing the archives are just files.
    let plain = ConflictIndex::build(&layers, "Data", None);
    assert_eq!(names(plain.origins(Area::Data, "textures/a.dds")), ["A"]);
    assert_eq!(names(plain.origins(Area::Data, "B.bsa")), ["B"]);
}

#[test]
fn archive_order_follows_the_plugin_name() {
    let rules = ArchiveRules {
        enabled: HashSet::new(),
        load_order: ["Skyrim.esm", "Foo.esp", "Foo - Bar.esp", "x.y.esm"].map(String::from).into(),
    };
    assert_eq!(rules.order_of("Skyrim - Textures0.bsa"), 0);
    assert_eq!(rules.order_of("foo.bsa"), 1);
    assert_eq!(rules.order_of("Foo - Bar.bsa"), 2, "the last plugin that matches wins");
    assert_eq!(rules.order_of("Foobar.bsa"), -1);
    assert_eq!(rules.order_of("x.y - Textures.ba2"), 3, "the stem keeps inner dots");
}
