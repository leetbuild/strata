//! Archives are written by a tiny writer here (hashes left at zero: the reader ignores them), so
//! every flag combination is covered; real archives are compared with libbsarch by
//! `tools/oracle/check-bsa.sh`.

use super::*;
use std::io::Write;

const FILES: &[(&str, &[u8])] = &[
    ("meshes/armor/iron.nif", b"a nif, a nif, a nif, a nif, a nif"),
    ("meshes/armor/steel.nif", b""),
    ("textures/x.dds", b"dds dds dds dds dds dds dds dds"),
    ("root.txt", b"at the top"),
];

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn lz4_frame(data: &[u8]) -> Vec<u8> {
    let mut e = lz4_flex::frame::FrameEncoder::new(Vec::new());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

/// A folder of the test archive and its files.
type Folder<'a> = (&'a str, Vec<(&'a str, &'a [u8])>);

/// A TES4 BSA. `toggle` lists files whose compression is the opposite of the default.
fn write_bsa(version: u32, flags: u32, toggle: &[&str]) -> Vec<u8> {
    let mut folders: Vec<Folder<'_>> = Vec::new();
    for (path, data) in FILES {
        let (folder, name) = path.rsplit_once('/').unwrap_or((".", path));
        match folders.iter_mut().find(|(f, _)| *f == folder) {
            Some((_, list)) => list.push((name, data)),
            None => folders.push((folder, vec![(name, data)])),
        }
    }
    let record = if version == 105 { 24 } else { 16 };
    let folder_names: usize = folders.iter().map(|(f, _)| f.len() + 1).sum();
    let file_names: usize = FILES.iter().map(|(p, _)| p.rsplit('/').next().unwrap().len() + 1).sum();
    let file_records_len = folders.len() + folder_names + FILES.len() * 16;
    let data_start = 36 + folders.len() * record + file_records_len + file_names;

    // Data blocks first, to know the offsets.
    let mut data = Vec::new();
    let mut entries = Vec::new(); // (size field, offset)
    for (folder, list) in &folders {
        for (name, content) in list {
            let path = if *folder == "." { (*name).to_owned() } else { format!("{folder}/{name}") };
            let compressed = (flags & BSA_COMPRESSED != 0) != toggle.contains(&path.as_str());
            let start = data.len();
            if version >= 104 && flags & BSA_EMBEDDED_NAMES != 0 {
                data.push(path.len() as u8);
                data.extend_from_slice(path.as_bytes());
            }
            if compressed {
                data.extend_from_slice(&(content.len() as u32).to_le_bytes());
                data.extend(if version == 105 { lz4_frame(content) } else { zlib(content) });
            } else {
                data.extend_from_slice(content);
            }
            let mut size = (data.len() - start) as u32;
            if toggle.contains(&path.as_str()) {
                size |= BSA_TOGGLE_COMPRESSION;
            }
            entries.push((size, (data_start + start) as u32));
        }
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"BSA\0");
    for v in [version, 36, flags, folders.len() as u32, FILES.len() as u32, folder_names as u32, file_names as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&[0u8; 4]);
    for (_, list) in &folders {
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&(list.len() as u32).to_le_bytes());
        if version == 105 {
            out.extend_from_slice(&[0u8; 12]);
        } else {
            out.extend_from_slice(&[0u8; 4]);
        }
    }
    let mut e = entries.iter();
    for (folder, list) in &folders {
        out.push((folder.len() + 1) as u8);
        out.extend_from_slice(folder.as_bytes());
        out.push(0);
        for _ in list {
            let (size, offset) = e.next().unwrap();
            out.extend_from_slice(&0u64.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
        }
    }
    for (_, list) in &folders {
        for (name, _) in list {
            out.extend_from_slice(name.as_bytes());
            out.push(0);
        }
    }
    assert_eq!(out.len(), data_start);
    out.extend(data);
    out
}

/// A Fallout 4 general BA2 (version 1), every second file zlib-compressed.
fn write_ba2() -> Vec<u8> {
    let header = 24;
    let records = FILES.len() * 36;
    let mut data = Vec::new();
    let mut recs = Vec::new();
    for (i, (_, content)) in FILES.iter().enumerate() {
        let offset = (header + records + data.len()) as u64;
        let (packed, stored) = if i % 2 == 0 { let z = zlib(content); (z.len() as u32, z) } else { (0, content.to_vec()) };
        data.extend(stored);
        recs.push((offset, packed, content.len() as u32));
    }
    let names_offset = (header + records + data.len()) as u64;
    let mut out = Vec::new();
    out.extend_from_slice(b"BTDX");
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(b"GNRL");
    out.extend_from_slice(&(FILES.len() as u32).to_le_bytes());
    out.extend_from_slice(&names_offset.to_le_bytes());
    for (offset, packed, size) in recs {
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(b"nif\0");
        out.extend_from_slice(&[0u8; 8]); // dir hash, flags
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&packed.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&0xBAAD_F00Du32.to_le_bytes());
    }
    out.extend(data);
    for (path, _) in FILES {
        let windows = path.replace('/', "\\");
        out.extend_from_slice(&(windows.len() as u16).to_le_bytes());
        out.extend_from_slice(windows.as_bytes());
    }
    out
}

fn check(bytes: &[u8], format: Format) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("t.bsa");
    std::fs::write(&path, bytes).unwrap();
    let a = GameArchive::open(&path).unwrap();
    assert_eq!(a.format(), format);
    let mut listed: Vec<&str> = a.files().iter().map(|f| f.path.as_str()).collect();
    listed.sort_unstable();
    let mut wanted: Vec<&str> = FILES.iter().map(|(p, _)| *p).collect();
    wanted.sort_unstable();
    assert_eq!(listed, wanted, "{format:?}");
    for (i, f) in a.files().iter().enumerate() {
        let content = FILES.iter().find(|(p, _)| *p == f.path).unwrap().1;
        assert_eq!(a.read(i).unwrap(), content, "{format:?} {}", f.path);
    }
    let out = tmp.path().join("out");
    a.extract_all(&out).unwrap();
    assert_eq!(std::fs::read(out.join("textures/x.dds")).unwrap(), FILES[2].1);
}

#[test]
fn bsa_versions_and_flags() {
    let names = BSA_DIR_NAMES | BSA_FILE_NAMES;
    for version in [103, 104, 105] {
        check(&write_bsa(version, names, &[]), Format::Bsa(version));
        check(&write_bsa(version, names | BSA_COMPRESSED, &[]), Format::Bsa(version));
        check(&write_bsa(version, names | BSA_COMPRESSED, &["textures/x.dds"]), Format::Bsa(version));
        check(&write_bsa(version, names, &["meshes/armor/iron.nif"]), Format::Bsa(version));
    }
    for version in [104, 105] {
        check(&write_bsa(version, names | BSA_EMBEDDED_NAMES | BSA_COMPRESSED, &[]), Format::Bsa(version));
        check(&write_bsa(version, names | BSA_EMBEDDED_NAMES, &["root.txt"]), Format::Bsa(version));
    }
}

#[test]
fn compression_flags_per_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("t.bsa");
    std::fs::write(&path, write_bsa(105, BSA_DIR_NAMES | BSA_FILE_NAMES | BSA_COMPRESSED, &["root.txt"])).unwrap();
    let a = GameArchive::open(&path).unwrap();
    for f in a.files() {
        assert_eq!(f.compressed, f.path != "root.txt", "{}", f.path);
    }
}

#[test]
fn ba2_general() {
    check(&write_ba2(), Format::Ba2 { version: 1, textures: false });
}

#[test]
fn not_archives_and_damage() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("x.bsa");
    std::fs::write(&path, b"PK\x03\x04 zip").unwrap();
    assert!(matches!(GameArchive::open(&path), Err(Error::NotAnArchive(_))));
    std::fs::write(&path, b"BS").unwrap();
    assert!(matches!(GameArchive::open(&path), Err(Error::NotAnArchive(_))));
    let mut v100 = write_bsa(104, BSA_DIR_NAMES | BSA_FILE_NAMES, &[]);
    v100[4] = 100;
    std::fs::write(&path, &v100).unwrap();
    assert!(matches!(GameArchive::open(&path), Err(Error::Unsupported(..))));
    let whole = write_bsa(105, BSA_DIR_NAMES | BSA_FILE_NAMES | BSA_COMPRESSED, &[]);
    std::fs::write(&path, &whole[..80]).unwrap();
    assert!(matches!(GameArchive::open(&path), Err(Error::Damaged(..))), "cut short");
    // Data cut off: the list opens, reading fails clearly.
    std::fs::write(&path, &whole[..whole.len() - 3]).unwrap();
    let a = GameArchive::open(&path).unwrap();
    let last = (0..a.files().len()).max_by_key(|i| a.files()[*i].offset).unwrap();
    assert!(matches!(a.read(last), Err(Error::Damaged(..))));
}

/// The SSE archives of a real game folder (`STRATA_REAL_GAME`, read only): every file reads
/// and decompresses to a plausible size. Skipped when it isn't set.
#[test]
fn real_game_archives_read() {
    let Some(game) = std::env::var_os("STRATA_REAL_GAME") else {
        return eprintln!("SKIPPED: {}: STRATA_REAL_GAME is not set (a real Skyrim SE game folder, read only)", "real_game_archives_read");
    };
    let data = Path::new(&game).join("Data");
    let Ok(rd) = std::fs::read_dir(&data) else {
        eprintln!("SKIPPED: real_game_archives_read: no {} (real data, read only)", data.display());
        return;
    };
    let mut seen = 0;
    for e in rd.flatten() {
        let p = e.path();
        if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bsa")) || p.metadata().map_or(0, |m| m.len()) > 400 << 20 {
            continue;
        }
        let a = GameArchive::open(&p).unwrap();
        assert_eq!(a.format(), Format::Bsa(105), "{}", p.display());
        // A sample: the first and last files of each archive.
        for i in [0, a.files().len().saturating_sub(1)] {
            if a.files().is_empty() {
                break;
            }
            a.read(i).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        }
        seen += 1;
    }
    assert!(seen > 0, "no BSA under 400 MB in {}", data.display());
}

#[test]
fn zlib_cut_short_after_the_data_still_reads() {
    // Skyrim LE's textures/clutter/placeholder.txt: "Temp", with no end block or checksum.
    let mut out = Vec::new();
    inflate(b"x\x9c\x0bI\xcd-\x00\x00", &mut out).unwrap();
    assert_eq!(out, b"Temp");
    let mut whole = Vec::new();
    inflate(&zlib(FILES[0].1), &mut whole).unwrap();
    assert_eq!(whole, FILES[0].1);
}
