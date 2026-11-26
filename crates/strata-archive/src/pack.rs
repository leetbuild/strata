//! Writing BSAs (TES4 format: v103 Oblivion, v104 Skyrim LE, v105 Skyrim SE), and the two mod
//! tools built on archives: MO2's BSA packer (`bsapacker`: a mod's loose files into archives
//! named after its plugin) and BSA extractor (`bsa_extractor`: a freshly installed mod's
//! archives unpacked into it).
//!
//! The layout is the one `bsa::GameArchive` reads (UESP's "BSA File Format"): header, folder
//! records sorted by hash, each folder's name and file records sorted by hash, the file names,
//! then the data. The hashes are the game's (it looks files up by them); the function follows
//! the `ba2` crate's `tes4::hash_file` / `hash_directory`, and the tests check it against the
//! hashes stored in real archives.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::bsa::{Error, GameArchive, Result};

const DIR_NAMES: u32 = 0x1;
const FILE_NAMES: u32 = 0x2;
const COMPRESSED: u32 = 0x4;

/// The game's spelling of a path inside an archive: lower case ASCII, `\` separators, no
/// leading or trailing separator; empty or too long becomes `.` (`ba2::hashing::normalize_path`).
pub fn normalize(path: &str) -> String {
    let mut s: String = path
        .chars()
        .map(|c| match c {
            '/' => '\\',
            c => c.to_ascii_lowercase(),
        })
        .collect();
    while s.ends_with('\\') {
        s.pop();
    }
    while s.starts_with('\\') {
        s.remove(0);
    }
    if s.is_empty() || s.len() >= 260 {
        s = ".".to_owned();
    }
    s
}

fn crc(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |crc, b| u32::from(*b).wrapping_add(crc.wrapping_mul(0x1003F)))
}

/// The hash parts: last byte, second to last, length, first byte, then the "crc" of the middle.
fn hash_parts(path: &[u8]) -> [u32; 5] {
    let len = path.len();
    let last2 = if len >= 3 { path[len - 2] } else { 0 };
    let (last, first) = if len >= 1 { (path[len - 1], path[0]) } else { (0, 0) };
    let length = len as u8;
    let middle = if length > 3 { crc(&path[1..len - 2]) } else { 0 };
    [u32::from(last), u32::from(last2), u32::from(length), u32::from(first), middle]
}

fn numeric(p: [u32; 5]) -> u64 {
    u64::from(p[0] & 0xff) | u64::from(p[1] & 0xff) << 8 | u64::from(p[2] & 0xff) << 16 | u64::from(p[3] & 0xff) << 24 | u64::from(p[4]) << 32
}

/// A folder's hash (`hash_directory`), of the bytes `normalize` gives (names that are not
/// ASCII hash as the bytes the archive stores).
pub fn hash_folder(path: &[u8]) -> u64 {
    numeric(hash_parts(path))
}

/// A file name's hash (`hash_file`), of the bytes `normalize` gives.
pub fn hash_file(name: &[u8]) -> u64 {
    let name = name.rsplit(|b| *b == b'\\').next().unwrap_or(name);
    let (stem, ext) = match name.iter().rposition(|b| *b == b'.') {
        Some(at) => (&name[..at], &name[at..]),
        None => (name, &b""[..]),
    };
    if stem.is_empty() || stem.len() >= 260 || ext.len() >= 16 {
        return 0;
    }
    let mut p = hash_parts(stem);
    p[4] = p[4].wrapping_add(crc(ext));
    // The archive tool compares the first four bytes of the extension.
    let four = |e: &[u8]| {
        let mut b = [0u8; 4];
        for (i, c) in e.iter().take(4).enumerate() {
            b[i] = *c;
        }
        b
    };
    let lut: [&[u8]; 6] = [b"", b".nif", b".kf", b".dds", b".wav", b".adp"];
    if let Some(i) = lut.iter().position(|e| four(e) == four(ext)) {
        let i = i as u32;
        p[3] = p[3].wrapping_add(32 * (i & 0xFC)) & 0xff;
        p[0] = p[0].wrapping_add((i & 0xFE) << 6) & 0xff;
        p[1] = p[1].wrapping_add(i.wrapping_shl(7)) & 0xff;
    }
    numeric(p)
}

/// One file to put in an archive.
#[derive(Debug, Clone)]
pub struct PackFile {
    /// Its path inside the archive (any case, `/` or `\`).
    pub path: String,
    pub source: PathBuf,
}

/// What to write.
#[derive(Debug, Clone, Copy)]
pub struct PackOptions {
    /// 103 (Oblivion), 104 (Skyrim LE), 105 (Skyrim SE).
    pub version: u32,
    pub compress: bool,
}

/// The archive's content types (`file_flags`), from the top folders: meshes 1, textures 2,
/// interface 4, sound (not voice) 8, voice 0x10, shaders 0x20, trees 0x40, fonts 0x80, misc 0x100.
fn content_flags(folders: &[&str]) -> u16 {
    let mut flags = 0;
    for f in folders {
        let top = f.split('\\').next().unwrap_or("");
        flags |= match top {
            "meshes" => 1,
            "textures" => 2,
            "interface" => 4,
            "sound" if f.starts_with("sound\\voice") => 0x10,
            "sound" | "music" => 8,
            "shaders" => 0x20,
            "trees" => 0x40,
            "fonts" => 0x80,
            _ => 0x100,
        };
    }
    flags
}

fn compress(data: &[u8], version: u32) -> std::io::Result<Vec<u8>> {
    let mut out = (data.len() as u32).to_le_bytes().to_vec();
    if version == 105 {
        let mut e = lz4_flex::frame::FrameEncoder::new(out);
        e.write_all(data)?;
        out = e.finish().map_err(std::io::Error::other)?;
    } else {
        let mut e = flate2::write::ZlibEncoder::new(out, flate2::Compression::default());
        e.write_all(data)?;
        out = e.finish()?;
    }
    Ok(out)
}

/// A folder of the archive being written: its name, and file hash -> (name, source).
type Folder<'a> = (String, BTreeMap<u64, (String, &'a Path)>);

/// Writes a BSA holding `files` to `out` (replaced if it exists, through a temporary file).
/// Two files with the same path are an error.
pub fn write_bsa(out: &Path, files: &[PackFile], options: PackOptions) -> Result<()> {
    if !matches!(options.version, 103..=105) {
        return Err(Error::Unsupported(out.to_owned(), format!("BSA {} (103, 104 or 105)", options.version)));
    }
    let damaged = |why: String| Error::Damaged(out.to_owned(), why);
    let mut tree: BTreeMap<u64, Folder<'_>> = BTreeMap::new();
    for f in files {
        let path = normalize(&f.path);
        let (folder, name) = path.rsplit_once('\\').unwrap_or(("", &path));
        let folder = normalize(folder);
        let entry = tree.entry(hash_folder(folder.as_bytes())).or_insert_with(|| (folder.clone(), BTreeMap::new()));
        if entry.0 != folder {
            return Err(damaged(format!("folders {} and {folder} have the same hash", entry.0)));
        }
        if entry.1.insert(hash_file(name.as_bytes()), (name.to_owned(), &f.source)).is_some() {
            return Err(damaged(format!("{path} is given twice (or two names share a hash)")));
        }
    }

    let v105 = options.version == 105;
    let record = if v105 { 24 } else { 16 };
    let folder_names: usize = tree.values().map(|(f, _)| f.len() + 1).sum();
    let file_names: usize = tree.values().flat_map(|(_, fs)| fs.values()).map(|(n, _)| n.len() + 1).sum();
    let file_count: usize = tree.values().map(|(_, fs)| fs.len()).sum();
    let records_start = 36 + tree.len() * record;
    let data_start = records_start + tree.len() + folder_names + file_count * 16 + file_names;

    // Data first: sizes and offsets go into the records.
    let mut data = Vec::new();
    let mut placed = Vec::with_capacity(file_count);
    for (_, fs) in tree.values() {
        for (name, source) in fs.values() {
            let raw = std::fs::read(source).map_err(|e| Error::Io(source.to_path_buf(), e))?;
            let stored = if options.compress { compress(&raw, options.version).map_err(|e| Error::Io(out.to_owned(), e))? } else { raw };
            let offset = data_start + data.len();
            if offset + stored.len() > u32::MAX as usize {
                return Err(damaged(format!("{name}: an archive is limited to 4 GB; split the files")));
            }
            placed.push((stored.len() as u32, offset as u32));
            data.extend_from_slice(&stored);
        }
    }

    let flags = DIR_NAMES | FILE_NAMES | if options.compress { COMPRESSED } else { 0 };
    let folders: Vec<&str> = tree.values().map(|(f, _)| f.as_str()).collect();
    let mut buf = Vec::with_capacity(data_start + data.len());
    buf.extend_from_slice(b"BSA\0");
    for v in [options.version, 36, flags, tree.len() as u32, file_count as u32, folder_names as u32, file_names as u32] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf.extend_from_slice(&content_flags(&folders).to_le_bytes());
    buf.extend_from_slice(&[0, 0]);
    // Folder records: the offset is where the folder's block starts, plus the length of the
    // file name block (a quirk of the format the readers expect).
    let mut block = records_start;
    for (hash, (folder, fs)) in &tree {
        buf.extend_from_slice(&hash.to_le_bytes());
        buf.extend_from_slice(&(fs.len() as u32).to_le_bytes());
        let offset = (block + file_names) as u64;
        if v105 {
            buf.extend_from_slice(&[0; 4]);
            buf.extend_from_slice(&offset.to_le_bytes());
        } else {
            buf.extend_from_slice(&(offset as u32).to_le_bytes());
        }
        block += 1 + folder.len() + 1 + fs.len() * 16;
    }
    let mut at = placed.iter();
    for (folder, fs) in tree.values() {
        buf.push((folder.len() + 1) as u8);
        buf.extend_from_slice(folder.as_bytes());
        buf.push(0);
        for hash in fs.keys() {
            let (size, offset) = at.next().copied().unwrap_or_default();
            buf.extend_from_slice(&hash.to_le_bytes());
            buf.extend_from_slice(&size.to_le_bytes());
            buf.extend_from_slice(&offset.to_le_bytes());
        }
    }
    for (_, fs) in tree.values() {
        for (name, _) in fs.values() {
            buf.extend_from_slice(name.as_bytes());
            buf.push(0);
        }
    }
    debug_assert_eq!(buf.len(), data_start);
    buf.extend_from_slice(&data);
    let dir = out.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| Error::Io(dir.to_owned(), e))?;
    tmp.write_all(&buf).map_err(|e| Error::Io(out.to_owned(), e))?;
    tmp.persist(out).map_err(|e| Error::Io(out.to_owned(), e.error))?;
    Ok(())
}

/// MO2's packer settings (`bsapacker`'s `SettingsService`).
#[derive(Debug, Clone)]
pub struct PackModOptions {
    pub version: u32,
    /// `compress_archives` (default on): an archive is compressed unless it holds an
    /// incompressible file (wav, ogg, mp3: `ArchiveBuilderHelper::INCOMPRESSIBLE_TYPES`).
    pub compress: bool,
    /// `blacklisted_files`: extensions never packed. Default `.txt;.hkx;.xml;.ini;.bk2`.
    pub blacklist: Vec<String>,
    /// `hide_loose_assets`: packed loose files get `.mohidden` appended.
    pub hide_loose: bool,
}

impl Default for PackModOptions {
    fn default() -> Self {
        Self {
            version: 105,
            compress: true,
            blacklist: [".txt", ".hkx", ".xml", ".ini", ".bk2"].map(String::from).to_vec(),
            hide_loose: false,
        }
    }
}

/// What packing a mod made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packed {
    pub archives: Vec<PathBuf>,
    pub files: usize,
}

/// `bsapacker` for a TES4-format game: every file below the mod's top folder (files directly
/// in it, the plugins and `meta.ini`, stay loose; blacklisted extensions too) into
/// `<archive name>.bsa` in the mod folder, or `<name>1.bsa`... when that exists
/// (`ArchiveNameService::Suffix`), a new archive every 4 GB (`GeneralArchiveBuilder`).
/// `archive_name` is the plugin's name without extension (MO2 asks which plugin).
pub fn pack_mod(mod_dir: &Path, archive_name: &str, options: &PackModOptions) -> Result<Packed> {
    const LIMIT: u64 = 4 * 1024 * 1024 * 1024;
    let incompressible = |p: &str| [".wav", ".ogg", ".mp3"].iter().any(|e| p.to_lowercase().ends_with(e));
    let blacklisted = |p: &str| options.blacklist.iter().any(|e| !e.is_empty() && p.to_lowercase().ends_with(&e.to_lowercase()));
    let mut files: Vec<(String, PathBuf, u64)> = Vec::new();
    let mut stack = vec![(mod_dir.to_owned(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let rd = std::fs::read_dir(&dir).map_err(|e| Error::Io(dir.clone(), e))?;
        for e in rd.flatten() {
            let Ok(name) = e.file_name().into_string() else { continue };
            let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let Ok(meta) = std::fs::metadata(e.path()) else { continue };
            if meta.is_dir() {
                stack.push((e.path(), path));
            } else if !rel.is_empty() && !blacklisted(&path) && !path.to_lowercase().ends_with(".mohidden") {
                files.push((path, e.path(), meta.len()));
            }
        }
    }
    files.sort_by_cached_key(|f| f.0.to_lowercase());
    if files.is_empty() {
        return Ok(Packed { archives: Vec::new(), files: 0 });
    }
    // Split at the size limit, in path order.
    let mut groups: Vec<Vec<&(String, PathBuf, u64)>> = vec![Vec::new()];
    let mut size = 0;
    for f in &files {
        if size + f.2 > LIMIT && !groups.last().is_some_and(Vec::is_empty) {
            groups.push(Vec::new());
            size = 0;
        }
        size += f.2;
        if let Some(g) = groups.last_mut() {
            g.push(f);
        }
    }
    let mut archives = Vec::new();
    for group in groups {
        let mut out = mod_dir.join(format!("{archive_name}.bsa"));
        let mut n = 0;
        while out.exists() {
            n += 1;
            out = mod_dir.join(format!("{archive_name}{n}.bsa"));
        }
        let compress = options.compress && !group.iter().any(|f| incompressible(&f.0));
        let list: Vec<PackFile> = group.iter().map(|f| PackFile { path: f.0.clone(), source: f.1.clone() }).collect();
        write_bsa(&out, &list, PackOptions { version: options.version, compress })?;
        archives.push(out);
    }
    if options.hide_loose {
        for (_, source, _) in &files {
            let mut hidden = source.as_os_str().to_owned();
            hidden.push(".mohidden");
            std::fs::rename(source, &hidden).map_err(|e| Error::Io(source.clone(), e))?;
        }
    }
    Ok(Packed { archives, files: files.len() })
}

/// `bsa_extractor`: unpacks every `*.bsa` / `*.ba2` in the mod's top folder into the mod
/// folder (existing files are replaced), removing each archive afterwards when `remove`.
/// Returns the archives extracted.
pub fn extract_mod_archives(mod_dir: &Path, remove: bool) -> Result<Vec<PathBuf>> {
    let rd = std::fs::read_dir(mod_dir).map_err(|e| Error::Io(mod_dir.to_owned(), e))?;
    let mut archives: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bsa") || x.eq_ignore_ascii_case("ba2"))
        })
        .collect();
    archives.sort();
    for archive in &archives {
        GameArchive::open(archive)?.extract_all(mod_dir)?;
        if remove {
            std::fs::remove_file(archive).map_err(|e| Error::Io(archive.clone(), e))?;
        }
    }
    Ok(archives)
}

#[cfg(test)]
mod tests;
