//! Tests build their archives with the installed `7z` in a temp folder (no binaries in git).

use super::*;
use std::sync::atomic::AtomicBool;

/// Makes `files` (relative path → content; a trailing `/` makes an empty folder) in a folder and
/// packs them with `7z a <extra args>`.
fn pack(dir: &Path, name: &str, files: &[(&str, &[u8])], extra: &[&str]) -> PathBuf {
    let src = dir.join(format!("{name}.src"));
    for (path, content) in files {
        let p = src.join(path);
        if path.ends_with('/') {
            std::fs::create_dir_all(&p).unwrap();
        } else {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
        }
    }
    let archive = dir.join(name);
    let status = Command::new(find_7z().expect("tests need 7z"))
        .current_dir(&src)
        .args(["a", "-bso0", "-bsp0"])
        .args(extra)
        .arg(&archive)
        .arg(".")
        .status()
        .unwrap();
    assert!(status.success());
    archive
}

fn files_under(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            if p.is_dir() {
                out.push(format!("{rel}/"));
                walk(root, &p, out);
            } else {
                out.push(rel);
            }
        }
    }
    walk(root, root, &mut out);
    out.sort();
    out
}

const TREE: &[(&str, &[u8])] = &[
    ("Data/sub/a b.txt", b"hello"),
    ("Data/ünï*?[x].esp", b"plugin"),
    ("Root/skse64_loader.exe", b"exe"),
    ("empty/", b""),
    ("readme.txt", b"read me"),
];

#[test]
fn extract_all_writes_the_tree_and_removes_staging() {
    let tmp = tempfile::tempdir().unwrap();
    for (name, extra) in [("t.7z", &[][..]), ("t.zip", &["-tzip"][..])] {
        let archive = pack(tmp.path(), name, TREE, extra);
        let a = Archive::open(&archive, None).unwrap();
        let out = tmp.path().join(format!("{name}.out"));
        let mut seen = Vec::new();
        a.extract_all(&out, &mut |p| seen.push(p.percent), &AtomicBool::new(false)).unwrap();
        assert_eq!(
            files_under(&out),
            [
                "Data/",
                "Data/sub/",
                "Data/sub/a b.txt",
                "Data/ünï*?[x].esp",
                "Root/",
                "Root/skse64_loader.exe",
                "empty/",
                "readme.txt",
            ],
            "{name}"
        );
        assert_eq!(std::fs::read(out.join("Data/sub/a b.txt")).unwrap(), b"hello");
        assert_eq!(seen.last(), Some(&100), "{name}: progress ends at 100");
    }
}

#[test]
fn listing_has_sizes_crcs_and_folders() {
    let tmp = tempfile::tempdir().unwrap();
    for (name, extra) in [("t.7z", &[][..]), ("t.zip", &["-tzip"][..])] {
        let a = Archive::open(&pack(tmp.path(), name, TREE, extra), None).unwrap();
        let get = |p: &str| a.entries().iter().find(|e| e.path == p).unwrap().clone();
        let f = get("Data/sub/a b.txt");
        assert_eq!((f.size, f.is_dir, f.encrypted), (5, false, false), "{name}");
        assert_eq!(f.crc, Some(0x3610_a686), "{name}: crc32 of 'hello'");
        assert!(get("empty").is_dir, "{name}");
        assert!(get("Data").is_dir, "{name}");
        assert_eq!(a.entries().len(), 8, "{name}");
    }
}

#[test]
fn a_plan_maps_copies_and_skips_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Archive::open(&pack(tmp.path(), "t.7z", TREE, &[]), None).unwrap();
    let idx = |p: &str| a.entries().iter().position(|e| e.path == p).unwrap();
    let out = tmp.path().join("out");
    let plan = vec![
        (idx("Data/sub/a b.txt"), "x/one.txt".to_owned()),
        (idx("Data/sub/a b.txt"), "two.txt".to_owned()),
        (idx("Root/skse64_loader.exe"), "skse64_loader.exe".to_owned()),
        (idx("empty"), "made/here".to_owned()),
    ];
    a.extract(&out, &plan, &mut |_| {}, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        files_under(&out),
        ["made/", "made/here/", "skse64_loader.exe", "two.txt", "x/", "x/one.txt"]
    );
    assert_eq!(std::fs::read(out.join("x/one.txt")).unwrap(), b"hello");
    assert_eq!(std::fs::read(out.join("two.txt")).unwrap(), b"hello");
}

#[test]
fn existing_files_are_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Archive::open(&pack(tmp.path(), "t.7z", TREE, &[]), None).unwrap();
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("readme.txt"), "old").unwrap();
    let i = a.entries().iter().position(|e| e.path == "readme.txt").unwrap();
    a.extract(&out, &[(i, "readme.txt".into())], &mut |_| {}, &AtomicBool::new(false)).unwrap();
    assert_eq!(std::fs::read(out.join("readme.txt")).unwrap(), b"read me");
}

#[test]
fn backslashes_are_separators() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = pack(tmp.path(), "bs.zip", &[("Data\\tex\\a.dds", b"dds")], &["-tzip"]);
    let a = Archive::open(&archive, None).unwrap();
    assert_eq!(a.entries()[0].path, "Data/tex/a.dds");
    let out = tmp.path().join("out");
    a.extract_all(&out, &mut |_| {}, &AtomicBool::new(false)).unwrap();
    assert_eq!(files_under(&out), ["Data/", "Data/tex/", "Data/tex/a.dds"]);
}

#[test]
fn passwords() {
    let tmp = tempfile::tempdir().unwrap();
    // Encrypted names: listing needs the password.
    let names = pack(tmp.path(), "h.7z", TREE, &["-pSECRET", "-mhe=on"]);
    assert!(matches!(Archive::open(&names, None), Err(Error::PasswordNeeded(_))));
    assert!(matches!(Archive::open(&names, Some("nope")), Err(Error::WrongPassword(_))));
    assert_eq!(Archive::open(&names, Some("SECRET")).unwrap().entries().len(), 8);

    // Encrypted data only: listing works, extraction needs the password.
    let data = pack(tmp.path(), "d.zip", TREE, &["-tzip", "-pSECRET"]);
    let a = Archive::open(&data, None).unwrap();
    assert!(a.entries().iter().any(|e| e.encrypted));
    let out = tmp.path().join("out");
    let r = a.extract_all(&out, &mut |_| {}, &AtomicBool::new(false));
    assert!(matches!(r, Err(Error::PasswordNeeded(_))), "{r:?}");
    assert_eq!(files_under(&out), Vec::<String>::new(), "staging removed after a failure");
    let a = Archive::open(&data, Some("SECRET")).unwrap();
    a.extract_all(&out, &mut |_| {}, &AtomicBool::new(false)).unwrap();
    assert_eq!(std::fs::read(out.join("readme.txt")).unwrap(), b"read me");
}

#[test]
fn bad_inputs() {
    let tmp = tempfile::tempdir().unwrap();
    let not = tmp.path().join("not.7z");
    std::fs::write(&not, "this is not an archive").unwrap();
    assert!(matches!(Archive::open(&not, None), Err(Error::CannotOpen(..))));
    assert!(matches!(Archive::open(&tmp.path().join("gone.zip"), None), Err(Error::NotFound(_))));

    let a = Archive::open(&pack(tmp.path(), "t.7z", TREE, &[]), None).unwrap();
    let out = tmp.path().join("out");
    let never = AtomicBool::new(false);
    for bad in ["../up.txt", "/abs.txt", "C:/win.txt", "a/../../b", ""] {
        let r = a.extract(&out, &[(0, bad.into())], &mut |_| {}, &never);
        assert!(matches!(r, Err(Error::UnsafeOutput(_))), "{bad}: {r:?}");
    }
    assert!(matches!(
        a.extract(&out, &[(99, "x".into())], &mut |_| {}, &never),
        Err(Error::NoSuchEntry(99))
    ));
}

#[test]
fn unsafe_names_in_the_archive_are_refused() {
    let listing = "--\nPath = x.zip\nType = zip\n\n----------\nPath = ../evil.txt\nFolder = -\nSize = 1\n\n";
    let entries = parse_listing(listing);
    assert_eq!(entries[0].path, "", "an unsafe name has no path; open() refuses it");
}

#[test]
fn cancel_stops_and_cleans_up() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Archive::open(&pack(tmp.path(), "t.7z", TREE, &[]), None).unwrap();
    let out = tmp.path().join("out");
    let r = a.extract_all(&out, &mut |_| {}, &AtomicBool::new(true));
    assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    assert_eq!(files_under(&out), Vec::<String>::new());
}

#[test]
fn rar_listing_as_7z_prints_it() {
    // From `7z l -slt` of a real rar5 download (a Nexus mod).
    let listing = "\n----------\nPath = JK's Whiterun V1.4b SSE/JKs Whiterun.esp\nFolder = -\n\
        Size = 168654\nPacked Size = 79367\nModified = 2017-02-11 05:13:46.0000000\nCreated = \n\
        Attributes = A\nEncrypted = -\nCRC = EA2E8F3A\nHost OS = Win32\n\n\
        Path = JK's Whiterun V1.4b SSE\nFolder = +\nSize = 0\nAttributes = D\nEncrypted = -\nCRC = \n";
    let e = parse_listing(listing);
    assert_eq!(e.len(), 2);
    assert_eq!(e[0].path, "JK's Whiterun V1.4b SSE/JKs Whiterun.esp");
    assert_eq!((e[0].size, e[0].crc, e[0].is_dir), (168_654, Some(0xEA2E_8F3A), false));
    assert!(e[1].is_dir);
}

#[test]
fn progress_numbers() {
    assert_eq!(parse_percent(b" 45% 3 - Data/x.esp\x08"), Some(45));
    assert_eq!(parse_percent(b"  0M Scan\x08"), None);
    assert_eq!(parse_percent(b"100%\r"), Some(100));
}

#[test]
fn safe_relative_matches_fluorine() {
    assert_eq!(safe_relative("a\\b/./c//d").as_deref(), Some("a/b/c/d"));
    assert_eq!(safe_relative("./x"), Some("x".into()));
    assert_eq!(safe_relative("c:x"), None);
    assert_eq!(safe_relative("a/c:x"), Some("a/c:x".into()), "a drive letter only counts first");
    assert_eq!(safe_relative("\\x"), None);
    assert_eq!(safe_relative("."), None);
}

/// The first few small 7z, zip and rar archives of real download folders (the instances of
/// `STRATA_REAL_INSTANCES`, read only): extract to a temp folder, every file of the listing
/// lands with its listed size. Skipped when it isn't set.
#[test]
fn real_archives_extract_as_listed() {
    let mut found: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let instances = std::env::var("STRATA_REAL_INSTANCES").unwrap_or_default();
    for inst in instances.split(':').filter(|s| !s.is_empty()) {
        let dir = Path::new(inst).join("downloads");
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase());
            if let Some(ext @ ("7z" | "zip" | "rar")) = ext.as_deref()
                && p.metadata().is_ok_and(|m| m.len() < 16 << 20)
            {
                found.entry(ext.to_owned()).or_default().push(p);
            }
        }
    }
    if found.is_empty() {
        eprintln!("SKIPPED: real_archives_extract_as_listed: no small archives in the downloads of STRATA_REAL_INSTANCES (real data, read only)");
        return;
    }
    for rar in found.values().flat_map(|v| v.iter().take(3)) {
        let a = Archive::open(rar, None).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        a.extract_all(tmp.path(), &mut |_| {}, &AtomicBool::new(false)).unwrap();
        for e in a.entries().iter().filter(|e| !e.is_dir) {
            let len = std::fs::metadata(tmp.path().join(&e.path)).unwrap().len();
            assert_eq!(len, e.size, "{}: {}", rar.display(), e.path);
        }
    }
}

#[test]
fn a_truncated_archive_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let noise: Vec<u8> = (0..200_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let archive = pack(dir.path(), "t.zip", &[("a.bin", &noise), ("b.bin", &noise)], &["-mx0"]);
    let bytes = std::fs::read(&archive).unwrap();
    std::fs::write(&archive, &bytes[..bytes.len() - 1000]).unwrap();
    let err = Archive::open(&archive, None).unwrap_err().to_string();
    assert!(err.contains("Unexpected end of archive"), "{err}");
}
