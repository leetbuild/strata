use super::*;

#[test]
fn hashes_match_known_values() {
    // The ba2 crate's test vectors (taken from Bethesda's archives).
    assert_eq!(hash_folder(normalize("textures/armor/amuletsandrings/elder council").as_bytes()), 0x04BC_422C_742C_696C);
    assert_eq!(hash_folder(normalize("sound/voice/skyrim.esm/maleuniquedbguardian").as_bytes()), 0x5940_85AC_732B_616E);
    assert_eq!(hash_folder(normalize("textures/architecture/windhelm").as_bytes()), 0xC1D9_7EBE_741E_6C6D);
    assert_eq!(hash_file("darkbrotherhood__0007469a_1.fuz".as_bytes()), 0x011F_11B0_641B_5F31);
    assert_eq!(hash_file("elder_council_amulet_n.dds".as_bytes()), 0xDC53_1E2F_6516_DFEE);
    assert_eq!(hash_file("testtoddquest_testtoddhappy_00027fa2_1.mp3".as_bytes()), 0xDE03_01EE_7426_5F31);
    assert_eq!(hash_folder(normalize("").as_bytes()), hash_folder(b"."));
    assert_eq!(normalize("/Meshes/Armor/"), "meshes\\armor");
}

/// `normalize` on raw bytes: ASCII lower case, `/` to `\`.
fn lower(b: &[u8]) -> Vec<u8> {
    b.iter().map(|c| if *c == b'/' { b'\\' } else { c.to_ascii_lowercase() }).collect()
}

/// The folder and file hashes stored in real archives (`STRATA_REAL_GAME`, read only) equal ours.
#[test]
fn hashes_match_real_archives() {
    use std::io::{Read, Seek, SeekFrom};
    let Some(game) = std::env::var_os("STRATA_REAL_GAME") else {
        return eprintln!("SKIPPED: {}: STRATA_REAL_GAME is not set (a real Skyrim SE game folder, read only)", "hashes_match_real_archives");
    };
    let data = Path::new(&game).join("Data");
    let Ok(rd) = std::fs::read_dir(&data) else { return eprintln!("SKIPPED: hashes_match_real_archives: no {} (real data, read only)", data.display()) };
    let mut checked = 0;
    for e in rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("bsa"))) {
        let mut f = std::fs::File::open(e.path()).unwrap();
        let mut h = [0u8; 36];
        f.read_exact(&mut h).unwrap();
        let u = |i: usize| u32::from_le_bytes(h[i..i + 4].try_into().unwrap()) as usize;
        let (version, folders, names_len) = (u(4), u(16), u(28));
        let rec = if version == 105 { 24 } else { 16 };
        let mut recs = vec![0u8; folders * rec];
        f.read_exact(&mut recs).unwrap();
        let mut blocks = Vec::new();
        for i in 0..folders {
            let r = &recs[i * rec..];
            let hash = u64::from_le_bytes(r[..8].try_into().unwrap());
            let count = u32::from_le_bytes(r[8..12].try_into().unwrap()) as usize;
            let mut len = [0u8; 1];
            f.read_exact(&mut len).unwrap();
            let mut name = vec![0u8; len[0] as usize];
            f.read_exact(&mut name).unwrap();
            let folder = lower(&name[..name.len() - 1]);
            assert_eq!(hash_folder(&folder), hash, "{}: folder {:?}", e.path().display(), String::from_utf8_lossy(&folder));
            let mut files = vec![0u8; count * 16];
            f.read_exact(&mut files).unwrap();
            blocks.push(files);
        }
        let pos = f.stream_position().unwrap();
        f.seek(SeekFrom::Start(pos)).unwrap();
        let mut names = vec![0u8; names_len];
        f.read_exact(&mut names).unwrap();
        let mut names = names.split(|b| *b == 0);
        for files in &blocks {
            for rec in files.as_chunks::<16>().0 {
                let hash = u64::from_le_bytes(rec[..8].try_into().unwrap());
                let name = lower(names.next().unwrap());
                assert_eq!(hash_file(&name), hash, "{}: {:?}", e.path().display(), String::from_utf8_lossy(&name));
                checked += 1;
            }
        }
    }
    eprintln!("{checked} file hashes checked");
}

fn source_tree(root: &Path, files: &[(&str, &[u8])]) {
    for (p, d) in files {
        let path = root.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, d).unwrap();
    }
}

const FILES: &[(&str, &[u8])] = &[
    ("meshes/Armor/Iron.nif", b"nif nif nif nif nif nif"),
    ("textures/armor/iron.dds", b"dds dds dds dds"),
    ("textures/armor/iron_n.dds", b""),
    ("sound/voice/skyrim.esm/hi.fuz", b"fuz"),
    ("scripts/x.pex", b"pex"),
];

#[test]
fn written_archives_read_back() {
    let tmp = tempfile::tempdir().unwrap();
    source_tree(tmp.path(), FILES);
    let list: Vec<PackFile> = FILES.iter().map(|(p, _)| PackFile { path: (*p).into(), source: tmp.path().join(p) }).collect();
    for version in [103, 104, 105] {
        for compress in [false, true] {
            let out = tmp.path().join(format!("t{version}{compress}.bsa"));
            write_bsa(&out, &list, PackOptions { version, compress }).unwrap();
            let a = GameArchive::open(&out).unwrap();
            assert_eq!(a.format(), crate::Format::Bsa(version));
            assert_eq!(a.files().len(), FILES.len());
            for (i, f) in a.files().iter().enumerate() {
                let want = FILES.iter().find(|(p, _)| p.to_lowercase() == f.path).unwrap().1;
                assert_eq!(a.read(i).unwrap(), want, "{version} {compress} {}", f.path);
                assert_eq!(f.compressed, compress);
            }
        }
    }
    let twice = vec![list[0].clone(), PackFile { path: "MESHES/armor/iron.nif".into(), source: list[0].source.clone() }];
    assert!(write_bsa(&tmp.path().join("x.bsa"), &twice, PackOptions { version: 105, compress: false }).is_err());
    assert!(write_bsa(&tmp.path().join("x.bsa"), &list, PackOptions { version: 100, compress: false }).is_err());
}

#[test]
fn pack_a_mod_like_bsapacker() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path().join("My Mod");
    source_tree(&m, FILES);
    source_tree(&m, &[("My Mod.esp", b"TES4"), ("meta.ini", b""), ("docs/readme.txt", b"skip"), ("sound/fx/a.wav", b"w")]);
    std::fs::write(m.join("My Mod.bsa"), b"an older archive").unwrap();
    let packed = pack_mod(&m, "My Mod", &PackModOptions { hide_loose: true, ..PackModOptions::default() }).unwrap();
    assert_eq!(packed.archives, [m.join("My Mod1.bsa")], "the existing name is kept");
    assert_eq!(packed.files, FILES.len() + 1);
    let a = GameArchive::open(&packed.archives[0]).unwrap();
    let mut names: Vec<&str> = a.files().iter().map(|f| f.path.as_str()).collect();
    names.sort_unstable();
    assert!(names.contains(&"sound/fx/a.wav") && !names.iter().any(|n| n.ends_with(".txt") || n.ends_with(".esp")));
    assert!(a.files().iter().all(|f| !f.compressed), "a wav keeps the archive uncompressed");
    assert!(m.join("meshes/Armor/Iron.nif.mohidden").is_file() && !m.join("meshes/Armor/Iron.nif").exists());
    assert!(m.join("docs/readme.txt").is_file(), "blacklisted files stay");
    assert!(m.join("My Mod.esp").is_file());

    let empty = tmp.path().join("Empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert_eq!(pack_mod(&empty, "Empty", &PackModOptions::default()).unwrap().archives, Vec::<PathBuf>::new());
}

#[test]
fn extract_like_bsa_extractor() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    source_tree(&src, FILES);
    let m = tmp.path().join("mod");
    std::fs::create_dir_all(&m).unwrap();
    let list: Vec<PackFile> = FILES.iter().map(|(p, _)| PackFile { path: (*p).into(), source: src.join(p) }).collect();
    write_bsa(&m.join("Mod.bsa"), &list, PackOptions { version: 105, compress: true }).unwrap();
    assert_eq!(extract_mod_archives(&m, true).unwrap(), [m.join("Mod.bsa")]);
    assert!(!m.join("Mod.bsa").exists());
    assert_eq!(std::fs::read(m.join("meshes/armor/iron.nif")).unwrap(), FILES[0].1);
    assert!(extract_mod_archives(&m, false).unwrap().is_empty());
}
