//! The model test: random sequences of file operations, run both through a mounted overlay
//! and against a small in-memory model of a case-insensitive, case-preserving folder (what
//! Windows programs expect), compared after every step and after every remount.
//!
//! A failing sequence is shrunk (operations dropped while it still fails) and printed with
//! its seed. `STRATA_MODEL_SEEDS=n` runs more sequences (default 24), `STRATA_MODEL_SEED=s`
//! runs only seed `s`, `STRATA_MODEL_STEPS=n` sets the length (default 60).
//!
//! Needs `/dev/fuse` and `fusermount3`; without them the test passes with a note on stderr.
//! One mount at a time (see `tests/mount.rs`, `SERIAL`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use strata_game::{Game, Layer, LayerKind, Mapping, SkyrimSE};
use strata_vfs::{Mount, MountSpec};

// ---- the model ---------------------------------------------------------------------------

/// Windows compares names one character at a time (no word-final sigma, nothing that grows).
fn fold(name: &str) -> String {
    name.chars()
        .map(|c| {
            let lower: Vec<char> = c.to_lowercase().collect();
            if lower.len() == 1 { lower[0] } else { c }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    /// `id`: which file this is, kept by renames and writes (for handles held open).
    File { name: String, data: Vec<u8>, id: u64 },
    Dir { name: String, children: BTreeMap<String, Entry> },
}

impl Entry {
    fn name(&self) -> &str {
        match self {
            Entry::File { name, .. } | Entry::Dir { name, .. } => name,
        }
    }
}

/// A folder as Windows shows it: names compare without case, the spelling a name was
/// created with is kept. A rename to the same name in another case changes nothing, as the
/// Linux kernel does before the filesystem is asked (`vfs_rename`: source and target are
/// the same file).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Model {
    root: BTreeMap<String, Entry>,
    next_id: u64,
}

type Res = Result<(), i32>;

/// Every entry as (shown path, file content or `None` for a folder).
type Listing = Vec<(String, Option<Vec<u8>>)>;

fn split(path: &str) -> (Vec<&str>, &str) {
    let mut parts: Vec<&str> = path.split('/').collect();
    let last = parts.pop().unwrap_or("");
    (parts, last)
}

impl Model {
    /// The folder at `parts`: ENOENT when a part is missing, ENOTDIR when one is a file.
    fn dir_mut(&mut self, parts: &[&str]) -> Result<&mut BTreeMap<String, Entry>, i32> {
        let mut at = &mut self.root;
        for part in parts {
            match at.get_mut(&fold(part)) {
                Some(Entry::Dir { children, .. }) => at = children,
                Some(Entry::File { .. }) => return Err(libc::ENOTDIR),
                None => return Err(libc::ENOENT),
            }
        }
        Ok(at)
    }

    fn get(&self, path: &str) -> Option<&Entry> {
        let mut at = &self.root;
        let (parts, last) = split(path);
        for part in parts {
            match at.get(&fold(part)) {
                Some(Entry::Dir { children, .. }) => at = children,
                _ => return None,
            }
        }
        at.get(&fold(last))
    }

    fn create(&mut self, path: &str, data: &[u8]) -> Res {
        let (parts, last) = split(path);
        self.next_id += 1;
        let id = self.next_id;
        let dir = self.dir_mut(&parts)?;
        match dir.get_mut(&fold(last)) {
            Some(Entry::Dir { .. }) => Err(libc::EISDIR),
            Some(Entry::File { data: old, .. }) => {
                *old = data.to_vec();
                Ok(())
            }
            None => {
                dir.insert(fold(last), Entry::File { name: last.into(), data: data.to_vec(), id });
                Ok(())
            }
        }
    }

    fn file_mut(&mut self, path: &str) -> Result<&mut Vec<u8>, i32> {
        let (parts, last) = split(path);
        match self.dir_mut(&parts)?.get_mut(&fold(last)) {
            Some(Entry::File { data, .. }) => Ok(data),
            Some(Entry::Dir { .. }) => Err(libc::EISDIR),
            None => Err(libc::ENOENT),
        }
    }

    fn write(&mut self, path: &str, offset: usize, bytes: &[u8]) -> Res {
        let data = self.file_mut(path)?;
        // writing nothing changes nothing, past the end too
        if bytes.is_empty() {
            return Ok(());
        }
        if data.len() < offset + bytes.len() {
            data.resize(offset + bytes.len(), 0);
        }
        data[offset..offset + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    fn truncate(&mut self, path: &str, len: usize) -> Res {
        self.file_mut(path)?.resize(len, 0);
        Ok(())
    }

    fn mkdir(&mut self, path: &str) -> Res {
        let (parts, last) = split(path);
        let dir = self.dir_mut(&parts)?;
        if dir.contains_key(&fold(last)) {
            return Err(libc::EEXIST);
        }
        dir.insert(fold(last), Entry::Dir { name: last.into(), children: BTreeMap::new() });
        Ok(())
    }

    fn unlink(&mut self, path: &str) -> Res {
        let (parts, last) = split(path);
        let dir = self.dir_mut(&parts)?;
        match dir.get(&fold(last)) {
            Some(Entry::Dir { .. }) => Err(libc::EISDIR),
            Some(Entry::File { .. }) => {
                dir.remove(&fold(last));
                Ok(())
            }
            None => Err(libc::ENOENT),
        }
    }

    fn rmdir(&mut self, path: &str) -> Res {
        let (parts, last) = split(path);
        let dir = self.dir_mut(&parts)?;
        match dir.get(&fold(last)) {
            Some(Entry::Dir { children, .. }) if children.is_empty() => {
                dir.remove(&fold(last));
                Ok(())
            }
            Some(Entry::Dir { .. }) => Err(libc::ENOTEMPTY),
            Some(Entry::File { .. }) => Err(libc::ENOTDIR),
            None => Err(libc::ENOENT),
        }
    }

    /// In the order the kernel checks (`do_renameat2`, `vfs_rename`), then the filesystem.
    fn rename(&mut self, from: &str, to: &str) -> Res {
        let (from_parts, from_last) = split(from);
        let (to_parts, to_last) = split(to);
        self.dir_mut(&from_parts)?;
        self.dir_mut(&to_parts)?;
        let source = self.get(from).cloned().ok_or(libc::ENOENT)?;
        let from_key: Vec<String> = from.split('/').map(fold).collect();
        let to_key: Vec<String> = to.split('/').map(fold).collect();
        if from_key == to_key {
            return Ok(()); // the same entry, any case: the kernel does nothing
        }
        if matches!(source, Entry::Dir { .. }) && to_key.starts_with(&from_key) {
            return Err(libc::EINVAL);
        }
        if from_key.starts_with(&to_key) {
            return Err(libc::ENOTEMPTY); // the target is a folder the source lies in
        }
        if let Some(target) = self.get(to) {
            match (&source, target) {
                (Entry::File { .. }, Entry::Dir { .. }) => return Err(libc::EISDIR),
                (Entry::Dir { .. }, Entry::File { .. }) => return Err(libc::ENOTDIR),
                (Entry::Dir { .. }, Entry::Dir { children, .. }) if !children.is_empty() => {
                    return Err(libc::ENOTEMPTY);
                }
                _ => {}
            }
        }
        self.dir_mut(&from_parts)?.remove(&fold(from_last));
        let moved = match source {
            Entry::File { data, id, .. } => Entry::File { name: to_last.into(), data, id },
            Entry::Dir { children, .. } => Entry::Dir { name: to_last.into(), children },
        };
        self.dir_mut(&to_parts)?.insert(fold(to_last), moved);
        Ok(())
    }

    /// The file with this id, wherever it is now.
    fn by_id(&self, wanted: u64) -> Option<&[u8]> {
        fn find(dir: &BTreeMap<String, Entry>, wanted: u64) -> Option<&[u8]> {
            dir.values().find_map(|e| match e {
                Entry::File { data, id, .. } if *id == wanted => Some(data.as_slice()),
                Entry::File { .. } => None,
                Entry::Dir { children, .. } => find(children, wanted),
            })
        }
        find(&self.root, wanted)
    }

    fn id_of(&self, path: &str) -> Option<u64> {
        match self.get(path)? {
            Entry::File { id, .. } => Some(*id),
            Entry::Dir { .. } => None,
        }
    }

    /// Every entry as (shown path, file content or `None` for a folder), sorted.
    fn listing(&self) -> Listing {
        fn walk(dir: &BTreeMap<String, Entry>, prefix: &str, out: &mut Listing) {
            for entry in dir.values() {
                let path = if prefix.is_empty() { entry.name().to_owned() } else { format!("{prefix}/{}", entry.name()) };
                match entry {
                    Entry::File { data, .. } => out.push((path, Some(data.clone()))),
                    Entry::Dir { children, .. } => {
                        out.push((path.clone(), None));
                        walk(children, &path, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, "", &mut out);
        out.sort();
        out
    }

    /// Takes over the mount's spelling of every entry (after a remount, where a lower
    /// layer's first spelling may come back for a path it has).
    fn respell(&mut self, listing: &[(String, Option<Vec<u8>>)]) {
        for (path, _) in listing {
            let (parts, last) = split(path);
            if let Ok(dir) = self.dir_mut(&parts) {
                match dir.get_mut(&fold(last)) {
                    Some(Entry::File { name, .. } | Entry::Dir { name, .. }) => *name = last.to_owned(),
                    None => {}
                }
            }
        }
    }
}

// ---- operations --------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Op {
    /// `open(O_WRONLY | O_CREAT | O_TRUNC)` and write.
    Create(String, Vec<u8>),
    /// Open an existing file for writing and write at an offset.
    Write(String, usize, Vec<u8>),
    Truncate(String, usize),
    Rename(String, String),
    Unlink(String),
    Rmdir(String),
    Mkdir(String),
    /// Open a file for reading and keep it open; every later step reads it again.
    Hold(String),
    /// `stat`: whether the path exists, and a file's size (a stale name in the kernel's cache
    /// would show a file that is gone).
    Stat(String),
    Remount,
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = |d: &[u8]| String::from_utf8_lossy(d).into_owned();
        match self {
            Op::Create(p, d) => write!(f, "create {p:?} {:?}", text(d)),
            Op::Write(p, o, d) => write!(f, "write {p:?} at {o} {:?}", text(d)),
            Op::Truncate(p, n) => write!(f, "truncate {p:?} to {n}"),
            Op::Rename(a, b) => write!(f, "rename {a:?} -> {b:?}"),
            Op::Unlink(p) => write!(f, "unlink {p:?}"),
            Op::Rmdir(p) => write!(f, "rmdir {p:?}"),
            Op::Mkdir(p) => write!(f, "mkdir {p:?}"),
            Op::Hold(p) => write!(f, "hold {p:?} open"),
            Op::Stat(p) => write!(f, "stat {p:?}"),
            Op::Remount => write!(f, "remount"),
        }
    }
}

/// xorshift64*: enough randomness for test sequences, and the same on every machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

/// Names the sequences use: some the lower layers have, unicode ones, and new ones.
const NAMES: &[&str] =
    &["a.dds", "b.dds", "c.dds", "textures", "meshes", "Ünïcödé", "naïve.nif", "x", "y", "Ärger.txt", "ΟΔΟΣ"];

/// The name in random case, letter by letter (only letters whose case round-trips).
fn recase(rng: &mut Rng, name: &str) -> String {
    name.chars()
        .map(|c| {
            let flipped: String = if rng.chance(50) { c.to_uppercase().collect() } else { c.to_lowercase().collect() };
            if flipped.chars().count() == 1 && fold(&flipped) == fold(&c.to_string()) { flipped } else { c.to_string() }
        })
        .collect()
}

/// A path for an operation: often an existing entry in another case, sometimes a new name
/// in an existing folder, sometimes anything.
fn pick(rng: &mut Rng, model: &Model) -> String {
    let listing: Vec<String> =
        model.listing().into_iter().map(|(p, _)| p).filter(|p| p != "Data" && !p.starts_with("Data/Data")).collect();
    let dirs: Vec<&String> = listing.iter().filter(|p| model.get(p).is_some_and(|e| matches!(e, Entry::Dir { .. }))).collect();
    let respell = |rng: &mut Rng, path: &str| path.split('/').map(|p| recase(rng, p)).collect::<Vec<_>>().join("/");
    let new_name = |rng: &mut Rng| {
        let name = NAMES[rng.below(NAMES.len())];
        recase(rng, name)
    };
    match rng.below(10) {
        0..=4 if !listing.is_empty() => {
            let path = listing[rng.below(listing.len())].clone();
            respell(rng, &path)
        }
        5..=7 if !dirs.is_empty() => {
            let dir = dirs[rng.below(dirs.len())].clone();
            format!("{}/{}", respell(rng, &dir), new_name(rng))
        }
        _ => {
            let base = if rng.chance(70) { "Data" } else { "" };
            let depth = 1 + rng.below(3);
            let parts: Vec<String> = (0..depth).map(|_| new_name(rng)).collect();
            if base.is_empty() { parts.join("/") } else { format!("{base}/{}", parts.join("/")) }
        }
    }
}

fn bytes(rng: &mut Rng) -> Vec<u8> {
    let len = rng.below(12);
    (0..len).map(|_| b'a' + rng.below(26) as u8).collect()
}

fn generate(rng: &mut Rng, model: &mut Model, steps: usize) -> Vec<Op> {
    let mut ops = Vec::new();
    for _ in 0..steps {
        let op = match rng.below(100) {
            0..=19 => Op::Create(pick(rng, model), bytes(rng)),
            20..=29 => Op::Write(pick(rng, model), rng.below(8), bytes(rng)),
            30..=34 => Op::Truncate(pick(rng, model), rng.below(6)),
            35..=54 => Op::Rename(pick(rng, model), pick(rng, model)),
            55..=66 => Op::Unlink(pick(rng, model)),
            67..=74 => Op::Rmdir(pick(rng, model)),
            75..=86 => Op::Mkdir(pick(rng, model)),
            87..=91 => Op::Hold(pick(rng, model)),
            92..=96 => Op::Stat(pick(rng, model)),
            _ => Op::Remount,
        };
        let _ = apply_model(model, &op);
        ops.push(op);
    }
    ops
}

fn apply_model(model: &mut Model, op: &Op) -> Res {
    match op {
        Op::Create(p, d) => model.create(p, d),
        Op::Write(p, o, d) => model.write(p, *o, d),
        Op::Truncate(p, n) => model.truncate(p, *n),
        Op::Rename(a, b) => model.rename(a, b),
        Op::Unlink(p) => model.unlink(p),
        Op::Rmdir(p) => model.rmdir(p),
        Op::Mkdir(p) => model.mkdir(p),
        Op::Hold(p) => {
            model.dir_mut(&split(p).0)?;
            match model.get(p) {
                Some(Entry::File { .. }) => Ok(()),
                Some(Entry::Dir { .. }) => Err(libc::EISDIR),
                None => Err(libc::ENOENT),
            }
        }
        Op::Stat(p) => {
            model.dir_mut(&split(p).0)?;
            match model.get(p) {
                // the size is checked too: the overlay answers this code when it differs
                Some(Entry::File { data, .. }) => Err(-(data.len() as i32) - 1000),
                Some(Entry::Dir { .. }) => Ok(()),
                None => Err(libc::ENOENT),
            }
        }
        Op::Remount => Ok(()),
    }
}

// ---- the overlay -------------------------------------------------------------------------

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// What the lower layers hold before a sequence starts.
const FILES: &[(&str, &str, &str)] = &[
    ("game", "SkyrimSE.exe", "the game"),
    ("game", "Data/Skyrim.esm", "esm"),
    ("game", "Data/textures/a.dds", "game a"),
    ("low", "textures/a.dds", "low a"),
    ("low", "textures/b.dds", "low b"),
    ("low", "Root/skse64_loader.exe", "skse"),
    ("low", "meshes/Ünïcödé/naïve.nif", "unicode"),
    ("high", "Textures/A.DDS", "high a"),
    ("high", "Textures/x/y.dds", "high y"),
    ("high", "Root/enbseries.ini", "enb"),
    ("overwrite", "textures/c.dds", "overwrite c"),
];

struct Overlay {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    game: PathBuf,
    mount: Option<Mount>,
    /// Files held open, with the model's id of the file each one opened.
    held: Vec<(u64, fs::File)>,
}

impl Overlay {
    fn new() -> Self {
        let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        let root = tmp.path().to_owned();
        for dir in ["Game Root/Data", "mods/low", "mods/high", "overwrite"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        for (layer, path, text) in FILES {
            let dir = match *layer {
                "game" => "Game Root".to_owned(),
                "overwrite" => "overwrite".to_owned(),
                name => format!("mods/{name}"),
            };
            put(&root.join(dir).join(path), text);
        }
        let game = root.join("Game Root");
        let mut overlay = Self { _tmp: tmp, root, game, mount: None, held: Vec::new() };
        overlay.mount();
        overlay
    }

    fn mount(&mut self) {
        let rules = SkyrimSE.mod_mappings();
        let layer = |id: &str, kind, source: PathBuf, rules| Layer { id: id.into(), kind, source, rules };
        let spec = MountSpec {
            id: "game".into(),
            layers: vec![
                layer("<game>", LayerKind::Game, self.game.clone(), vec![Mapping::new("", "")]),
                layer("low", LayerKind::Mod, self.root.join("mods/low"), rules.clone()),
                layer("high", LayerKind::Mod, self.root.join("mods/high"), rules.clone()),
                layer("<overwrite>", LayerKind::Overwrite, self.root.join("overwrite"), rules),
            ],
            mountpoint: self.game.clone(),
            whiteouts: Some(self.root.join("strata/whiteouts.txt")),
            label: None,
        };
        self.mount = Some(Mount::new(spec, 4).unwrap());
    }

    fn unmount(&mut self) -> Result<(), String> {
        self.held.clear();
        match self.mount.take() {
            Some(mut mount) => mount.unmount(false).map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }

    fn at(&self, path: &str) -> PathBuf {
        self.game.join(path)
    }

    fn apply(&mut self, op: &Op, model: &Model) -> Res {
        let code = |e: std::io::Error| e.raw_os_error().unwrap_or(-1);
        match op {
            Op::Create(p, d) => {
                let mut file = fs::File::create(self.at(p)).map_err(code)?;
                file.write_all(d).map_err(code)
            }
            Op::Write(p, o, d) => {
                let file = fs::File::options().write(true).open(self.at(p)).map_err(code)?;
                file.write_all_at(d, *o as u64).map_err(code)
            }
            Op::Truncate(p, n) => {
                let file = fs::File::options().write(true).open(self.at(p)).map_err(code)?;
                file.set_len(*n as u64).map_err(code)
            }
            Op::Rename(a, b) => fs::rename(self.at(a), self.at(b)).map_err(code),
            Op::Unlink(p) => fs::remove_file(self.at(p)).map_err(code),
            Op::Rmdir(p) => fs::remove_dir(self.at(p)).map_err(code),
            Op::Mkdir(p) => fs::create_dir(self.at(p)).map_err(code),
            Op::Hold(p) => {
                let file = fs::File::open(self.at(p)).map_err(code)?;
                // Opening a folder succeeds on Linux; reading it is what fails.
                if file.metadata().map_err(code)?.is_dir() {
                    return Err(libc::EISDIR);
                }
                self.held.push((model.id_of(p).unwrap_or(0), file));
                Ok(())
            }
            Op::Stat(p) => {
                let meta = fs::metadata(self.at(p)).map_err(code)?;
                if meta.is_dir() { Ok(()) } else { Err(-(meta.len() as i32) - 1000) }
            }
            Op::Remount => {
                self.unmount().map_err(|_| libc::EBUSY)?;
                self.mount();
                Ok(())
            }
        }
    }

    fn listing(&self) -> Result<Listing, String> {
        fn walk(dir: &Path, prefix: &str, out: &mut Listing) -> Result<(), String> {
            let entries = fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                let real = entry.path();
                let meta = fs::metadata(&real).map_err(|e| format!("stat {path}: {e}"))?;
                if meta.is_dir() {
                    out.push((path.clone(), None));
                    walk(&real, &path, out)?;
                } else {
                    let data = fs::read(&real).map_err(|e| format!("read {path}: {e}"))?;
                    if data.len() as u64 != meta.len() {
                        return Err(format!("{path}: stat says {} bytes, read gave {}", meta.len(), data.len()));
                    }
                    out.push((path, Some(data)));
                }
            }
            Ok(())
        }
        let mut out = Vec::new();
        walk(&self.game, "", &mut out)?;
        out.sort();
        Ok(out)
    }

    /// Things nobody may see on disk after an unmount: copy-up temp files.
    fn leftovers(&self) -> Vec<String> {
        fn walk(dir: &Path, out: &mut Vec<String>) {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if entry.file_name().to_string_lossy().starts_with(".strata-tmp-") {
                    out.push(path.display().to_string());
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root.join("overwrite"), &mut out);
        out
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = self.unmount();
    }
}

fn show(listing: &[(String, Option<Vec<u8>>)]) -> String {
    listing
        .iter()
        .map(|(p, d)| match d {
            Some(d) => format!("  {p} = {:?}\n", String::from_utf8_lossy(d)),
            None => format!("  {p}/\n"),
        })
        .collect()
}

/// Runs `ops` on a fresh overlay and model. `Err` names the first step that disagreed,
/// with the whiteouts and what overwrite holds on disk.
fn run(ops: &[Op]) -> Result<(), String> {
    let mut overlay = Overlay::new();
    run_on(&mut overlay, ops).map_err(|e| {
        let whiteouts = fs::read_to_string(overlay.root.join("strata/whiteouts.txt")).unwrap_or_default();
        let whiteouts: String = whiteouts.lines().filter(|l| !l.starts_with('#')).map(|l| format!("  {l}\n")).collect();
        let mut disk = Vec::new();
        let mut stack = vec![overlay.root.join("overwrite")];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                let shown = path.strip_prefix(&overlay.root).unwrap_or(&path).display().to_string();
                if path.is_dir() {
                    disk.push(format!("  {shown}/\n"));
                    stack.push(path);
                } else {
                    disk.push(format!("  {shown}\n"));
                }
            }
        }
        disk.sort();
        format!("{e}\nwhiteouts.txt:\n{whiteouts}on disk:\n{}", disk.concat())
    })
}

fn run_on(overlay: &mut Overlay, ops: &[Op]) -> Result<(), String> {
    let start = overlay.listing()?;
    let lower: BTreeSet<String> = start.iter().map(|(p, _)| fold(p)).collect();
    let mut model = Model { root: BTreeMap::new(), next_id: 0 };
    for (path, data) in &start {
        match data {
            Some(d) => model.create(path, d),
            None => model.mkdir(path),
        }
        .map_err(|e| format!("model setup {path}: {e}"))?;
    }
    for (step, op) in ops.iter().enumerate() {
        let want = apply_model(&mut model, op);
        let got = overlay.apply(op, &model);
        if want != got {
            return Err(format!("step {step}, {op}: model {want:?}, overlay {got:?}"));
        }
        let mut seen = overlay.listing().map_err(|e| format!("step {step}, {op}: {e}"))?;
        if matches!(op, Op::Remount) {
            if let Some(left) = overlay.leftovers().first() {
                return Err(format!("step {step}: a temp file is left: {left}"));
            }
            // A path a lower layer had may show that layer's spelling again.
            let relaxed = |l: &[(String, Option<Vec<u8>>)]| -> Listing {
                // folded component by component: a lower folder's new child keeps its case
                let relax = |path: &str| -> String {
                    let mut prefix = String::new();
                    let mut out = Vec::new();
                    for part in path.split('/') {
                        prefix = if prefix.is_empty() { fold(part) } else { format!("{prefix}/{}", fold(part)) };
                        out.push(if lower.contains(&prefix) { fold(part) } else { part.to_owned() });
                    }
                    out.join("/")
                };
                let mut l: Vec<_> = l.iter().map(|(p, d)| (relax(p), d.clone())).collect();
                l.sort();
                l
            };
            if relaxed(&seen) == relaxed(&model.listing()) {
                model.respell(&seen);
                seen = overlay.listing()?;
            }
        }
        let expected = model.listing();
        if seen != expected {
            return Err(format!("step {step}, {op}: the trees differ\nmodel:\n{}overlay:\n{}", show(&expected), show(&seen)));
        }
        for (id, file) in &mut overlay.held {
            // A file held open reads what that file holds now, wherever it was renamed
            // to (Windows: the same file). Deleted or replaced files are not checked.
            let Some(want) = model.by_id(*id) else { continue };
            let mut got = Vec::new();
            file.read_to_end(&mut got).map_err(|e| format!("step {step}, {op}: reading a held file: {e}"))?;
            std::io::Seek::rewind(file).map_err(|e| e.to_string())?;
            if got != want {
                return Err(format!(
                    "step {step}, {op}: a file held open reads {:?}, the file holds {:?}",
                    String::from_utf8_lossy(&got),
                    String::from_utf8_lossy(want)
                ));
            }
        }
    }
    overlay.unmount().map_err(|e| format!("final unmount: {e}"))?;
    if let Some(left) = overlay.leftovers().first() {
        return Err(format!("a temp file is left: {left}"));
    }
    Ok(())
}

/// Drops operations while the sequence still fails: first halves, then single steps.
fn shrink(mut ops: Vec<Op>) -> (Vec<Op>, String) {
    let mut error = run(&ops).err().unwrap_or_default();
    let mut chunk = ops.len() / 2;
    while chunk >= 1 {
        let mut i = 0;
        while i + chunk <= ops.len() {
            let mut fewer = ops.clone();
            fewer.drain(i..i + chunk);
            match run(&fewer) {
                Err(e) => {
                    ops = fewer;
                    error = e;
                }
                Ok(()) => i += chunk,
            }
        }
        chunk /= 2;
    }
    (ops, error)
}

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// One mount at a time in this test program (see `tests/mount.rs`, `SERIAL`).
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn random_operations_match_the_model() {
    let fuse = Path::new("/dev/fuse").exists()
        && std::process::Command::new("fusermount3").arg("-V").output().is_ok_and(|o| o.status.success());
    if !fuse {
        eprintln!("SKIPPED: /dev/fuse or fusermount3 is missing, the model test did not run");
        return;
    }
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let steps = env("STRATA_MODEL_STEPS", 60) as usize;
    let seeds: Vec<u64> = match std::env::var("STRATA_MODEL_SEED").ok().and_then(|v| v.parse().ok()) {
        Some(seed) => vec![seed],
        None => (1..=env("STRATA_MODEL_SEEDS", 24)).collect(),
    };
    for seed in seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        // The generator needs a model of the start to pick existing paths from.
        let start = {
            let overlay = Overlay::new();
            overlay.listing().unwrap()
        };
        let mut model = Model { root: BTreeMap::new(), next_id: 0 };
        for (path, data) in &start {
            let _ = match data {
                Some(d) => model.create(path, d),
                None => model.mkdir(path),
            };
        }
        let ops = generate(&mut rng, &mut model, steps);
        if run(&ops).is_err() {
            let (small, error) = shrink(ops);
            let steps: String = small.iter().map(|op| format!("  {op}\n")).collect();
            panic!("seed {seed} fails; shrunk to {} steps:\n{steps}{error}", small.len());
        }
    }
}

/// Sequences that once failed, kept as they were shrunk.
#[test]
fn sequences_that_once_failed() {
    let fuse = Path::new("/dev/fuse").exists()
        && std::process::Command::new("fusermount3").arg("-V").output().is_ok_and(|o| o.status.success());
    if !fuse {
        eprintln!("SKIPPED: /dev/fuse or fusermount3 is missing, the model test did not run");
        return;
    }
    let s = |t: &str| t.to_owned();
    let cases: Vec<Vec<Op>> = vec![
        // The kernel kept the name a file was created with after a rename through another
        // spelling: the second create wrote into the renamed file.
        vec![
            Op::Create(s("DaTa/meShES/b.dDS"), b"irp".to_vec()),
            Op::Rename(s("daTa/MeSHes/b.ddS"), s("dAta/tEXtUrEs/A.DDS")),
            Op::Create(s("DAta/MesheS/b.dDS"), b"hei".to_vec()),
        ],
        // The same, the other way round: the kernel kept another spelling it was asked
        // for (an alias), and a rename through the file's own name left it behind.
        vec![
            Op::Create(s("Data/meshes/b.dds"), b"1".to_vec()),
            Op::Hold(s("Data/meshes/B.DDS")),
            Op::Rename(s("Data/meshes/b.dds"), s("Data/textures/q.dds")),
            Op::Create(s("Data/meshes/B.DDS"), b"2".to_vec()),
        ],
        vec![
            Op::Create(s("Data/meshes/b.dds"), b"1".to_vec()),
            Op::Hold(s("Data/meshes/B.DDS")),
            Op::Unlink(s("Data/meshes/b.dds")),
            Op::Create(s("Data/meshes/B.DDS"), b"2".to_vec()),
        ],
        // A mod's file held open while another handle changes it (the change is a copy-up).
        vec![Op::Hold(s("Data/textures/b.dds")), Op::Write(s("Data/textures/b.dds"), 0, b"XY".to_vec())],
        vec![Op::Hold(s("Data/textures/b.dds")), Op::Truncate(s("Data/textures/b.dds"), 2)],
        vec![Op::Hold(s("Data/textures/b.dds")), Op::Create(s("Data/textures/B.DDS"), b"new".to_vec())],
    ];
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for ops in cases {
        if let Err(e) = run(&ops) {
            let steps: String = ops.iter().map(|op| format!("  {op}\n")).collect();
            panic!("this sequence fails again:\n{steps}{e}");
        }
    }
}

/// A file known to the kernel under two spellings, moved or removed through one, then used at
/// once through the other (the alias race).
/// The overlay used to tell the kernel to forget the other spelling after its reply; with
/// that invalidation held back 400 ms by a test hook, the first, second and two stat cases
/// below failed every time (a create wrote into the renamed file). Now no file name has a
/// cache lifetime (`fs.rs`, `entry_ttl`), so nothing is left to forget; were file names
/// cached again, these cases would fail every time, not only when timing is unlucky.
#[test]
fn names_used_while_the_kernel_still_holds_the_old_spelling() {
    let fuse = Path::new("/dev/fuse").exists()
        && std::process::Command::new("fusermount3").arg("-V").output().is_ok_and(|o| o.status.success());
    if !fuse {
        eprintln!("SKIPPED: /dev/fuse or fusermount3 is missing, the model test did not run");
        return;
    }
    let s = |t: &str| t.to_owned();
    let d = |t: &str| t.as_bytes().to_vec();
    let cases: Vec<(&str, Vec<Op>)> = vec![
        (
            "a file renamed through another spelling, then created under its own",
            vec![
                Op::Create(s("DaTa/meShES/b.dDS"), d("irp")),
                Op::Rename(s("daTa/MeSHes/b.ddS"), s("dAta/tEXtUrEs/A.DDS")),
                Op::Create(s("DAta/MesheS/b.dDS"), d("hei")),
            ],
        ),
        (
            "a file renamed through another spelling, then looked at under its own",
            vec![
                Op::Create(s("Data/meshes/b.dds"), d("one")),
                Op::Stat(s("Data/meshes/b.dds")),
                Op::Rename(s("Data/meshes/B.DDS"), s("Data/textures/q.dds")),
                Op::Stat(s("Data/meshes/b.dds")),
                Op::Hold(s("Data/meshes/b.dds")),
            ],
        ),
        (
            "a mod's file renamed through another spelling, then looked at under its own",
            vec![
                Op::Stat(s("Data/textures/b.dds")),
                Op::Rename(s("Data/TEXTURES/B.DDS"), s("Data/meshes/moved.dds")),
                Op::Stat(s("Data/textures/b.dds")),
                Op::Create(s("Data/textures/b.dds"), d("new")),
            ],
        ),
        (
            "the same, written to instead of created",
            vec![
                Op::Create(s("Data/meshes/b.dds"), d("one")),
                Op::Rename(s("Data/meshes/B.DDS"), s("Data/textures/q.dds")),
                Op::Write(s("Data/meshes/b.dds"), 0, d("XX")),
                Op::Truncate(s("Data/meshes/b.dds"), 1),
            ],
        ),
        (
            "a folder renamed through another spelling, then a file made in its old name",
            vec![
                Op::Mkdir(s("Data/out")),
                Op::Create(s("Data/out/x.txt"), d("one")),
                Op::Rename(s("Data/OUT"), s("Data/moved")),
                Op::Create(s("Data/out/x.txt"), d("two")),
                Op::Mkdir(s("Data/out")),
                Op::Create(s("Data/out/x.txt"), d("three")),
            ],
        ),
        (
            "a mod's folder renamed through another spelling",
            vec![
                Op::Rename(s("Data/MESHES"), s("Data/models")),
                Op::Create(s("Data/meshes/rock.nif"), d("new")),
                Op::Write(s("Data/meshes/Ünïcödé/naïve.nif"), 0, d("X")),
            ],
        ),
        (
            "a file deleted through another spelling, then created under its own",
            vec![
                Op::Create(s("Data/meshes/b.dds"), d("one")),
                Op::Unlink(s("Data/meshes/B.DDS")),
                Op::Create(s("Data/meshes/b.dds"), d("two")),
            ],
        ),
        (
            "a mod's file deleted through another spelling, then written",
            vec![
                Op::Stat(s("Data/textures/b.dds")),
                Op::Unlink(s("Data/TEXTURES/B.DDS")),
                Op::Stat(s("Data/textures/b.dds")),
                Op::Write(s("Data/textures/b.dds"), 0, d("X")),
            ],
        ),
        (
            "a folder removed through another spelling, then made again and used",
            vec![
                Op::Mkdir(s("Data/q")),
                Op::Create(s("Data/q/x.txt"), d("one")),
                Op::Unlink(s("Data/q/x.txt")),
                Op::Rmdir(s("Data/Q")),
                Op::Create(s("Data/q/x.txt"), d("two")),
                Op::Mkdir(s("Data/q")),
                Op::Create(s("Data/q/x.txt"), d("three")),
            ],
        ),
        (
            "a file replaced by a rename through another spelling of the target",
            vec![
                Op::Create(s("Data/meshes/t.dds"), d("target")),
                Op::Create(s("Data/meshes/src.dds"), d("source")),
                Op::Rename(s("Data/meshes/src.dds"), s("Data/meshes/T.DDS")),
                Op::Write(s("Data/meshes/t.dds"), 0, d("XY")),
            ],
        ),
    ];
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut failures = Vec::new();
    for (name, ops) in &cases {
        if let Err(e) = run(ops) {
            failures.push(format!("{name}:\n{e}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
