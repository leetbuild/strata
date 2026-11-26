//! Oracle check 6 (`spec/07-oracle.md`): the mounted overlay, used through ordinary file calls.
//!
//! Needs `/dev/fuse` and `fusermount3`. Without them every test passes with a note on stderr.

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use strata_game::{Game, Layer, LayerKind, Mapping, SkyrimSE};
use strata_vfs::{Mount, MountSpec};

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// The mount tests run one at a time. They share one process: while one test holds a file
/// of its mount open, another test's `fusermount3` child (spawned to mount or unmount) gets a
/// copy of the descriptor table until it execs, and the first test's unmount then fails with
/// "Device or resource busy". vfsd never opens files on its own mounts, so only tests meet
/// this. (This was the flaky `a_refresh_swaps_the_layers_of_a_live_mount`: any test could
/// fail that way, that one most often.)
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    /// The Game Root, which is also the mountpoint.
    game: PathBuf,
    mount: Option<Mount>,
    /// Last: fields drop in order, and the mount must be gone before the next test starts.
    _serial: std::sync::MutexGuard<'static, ()>,
}

fn fuse_available() -> bool {
    // Asked once, before any test mounts (see `SERIAL` for why a child process matters).
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ok = *AVAILABLE.get_or_init(|| {
        Path::new("/dev/fuse").exists()
            && std::process::Command::new("fusermount3").arg("-V").output().is_ok_and(|o| o.status.success())
    });
    if !ok {
        eprintln!("SKIPPED: /dev/fuse or fusermount3 is missing, the mount tests did not run");
    }
    ok
}

impl Fixture {
    /// An instance with a game folder, two mods and overwrite, mounted in place over the game.
    fn new(files: &[(&str, &str, &str)]) -> Self {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Not under /tmp when that is a tmpfs without user mounts; the target dir always works.
        let base = Path::new(env!("CARGO_TARGET_TMPDIR"));
        let tmp = tempfile::tempdir_in(base).unwrap();
        let root = tmp.path().to_owned();
        for dir in ["Game Root/Data", "mods/low", "mods/high", "overwrite"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        for (layer, path, text) in files {
            let dir = match *layer {
                "game" => "Game Root".to_owned(),
                "overwrite" => "overwrite".to_owned(),
                name => format!("mods/{name}"),
            };
            put(&root.join(dir).join(path), text);
        }
        let game = root.join("Game Root");
        let mut fixture = Self { _serial: serial, _tmp: tmp, root, game, mount: None };
        fixture.mount();
        fixture
    }

    fn spec(&self) -> MountSpec {
        let rules = SkyrimSE.mod_mappings();
        let layer = |id: &str, kind, source: PathBuf, rules| Layer { id: id.into(), kind, source, rules };
        MountSpec {
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
        }
    }

    fn mount(&mut self) {
        self.mount = Some(Mount::new(self.spec(), 4).unwrap());
    }

    fn unmount(&mut self) {
        if let Some(mut mount) = self.mount.take() {
            mount.unmount(false).unwrap();
        }
    }

    /// A path inside the mounted game folder.
    fn at(&self, path: &str) -> PathBuf {
        self.game.join(path)
    }

    /// A file on disk, relative to the instance (only meaningful for `Game Root` once unmounted).
    fn disk(&self, path: &str) -> Option<String> {
        fs::read_to_string(self.root.join(path)).ok()
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

const BASE: &[(&str, &str, &str)] = &[
    ("game", "SkyrimSE.exe", "the game"),
    ("game", "Data/Skyrim.esm", "esm"),
    ("game", "Data/textures/a.dds", "game a"),
    ("low", "textures/a.dds", "low a"),
    ("low", "textures/b.dds", "low b"),
    ("low", "Root/skse64_loader.exe", "skse"),
    ("high", "Textures/A.DDS", "high a"),
    ("high", "Root/enbseries.ini", "enb"),
    ("high", "meshes/Ünïcödé/naïve — file.nif", "unicode"),
];

#[test]
fn reads_are_case_insensitive_and_the_highest_layer_wins() {
    if !fuse_available() {
        return;
    }
    let mut f = Fixture::new(BASE);
    assert_eq!(fs::read_to_string(f.at("Data/textures/a.dds")).unwrap(), "high a");
    assert_eq!(fs::read_to_string(f.at("DATA/TEXTURES/A.DDS")).unwrap(), "high a");
    assert_eq!(fs::read_to_string(f.at("data/Textures/B.dds")).unwrap(), "low b");
    assert_eq!(fs::read_to_string(f.at("skse64_LOADER.exe")).unwrap(), "skse");
    assert_eq!(fs::read_to_string(f.at("Data/meshes/ünïcödé/NAÏVE — FILE.nif")).unwrap(), "unicode");
    assert!(fs::metadata(f.at("data")).unwrap().is_dir());
    assert_eq!(fs::metadata(f.at("Data/Skyrim.ESM")).unwrap().len(), 3);
    assert!(fs::metadata(f.at("Data/nope.esp")).is_err());
    assert!(!f.at("Data/Root").exists(), "Root never shows under Data");
    assert!(!f.at("Data/meta.ini").exists());

    // folders merge, each name once, in the first spelling seen
    assert_eq!(names(&f.game), ["Data", "SkyrimSE.exe", "enbseries.ini", "skse64_loader.exe"]);
    assert_eq!(names(&f.at("Data/TEXTURES")), ["a.dds", "b.dds"]);
    f.unmount();
}

#[test]
fn writes_go_to_overwrite_and_the_real_folders_stay_as_they_were() {
    if !fuse_available() {
        return;
    }
    let mut f = Fixture::new(BASE);

    // a new file in Data, and one in the game folder itself
    fs::create_dir_all(f.at("Data/SKSE/Plugins")).unwrap();
    fs::write(f.at("Data/SKSE/Plugins/probe.json"), "{}").unwrap();
    fs::write(f.at("d3dx9_42.log"), "log").unwrap();
    assert_eq!(fs::read_to_string(f.at("data/skse/plugins/PROBE.JSON")).unwrap(), "{}");

    // changing a mod's file: copy-on-write
    let mut file = fs::File::options().read(true).write(true).open(f.at("Data/textures/b.dds")).unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    file.write_all(b" + edit").unwrap();
    drop(file);
    assert_eq!(fs::read_to_string(f.at("Data/textures/b.dds")).unwrap(), "low b + edit");

    // truncating a game file
    fs::write(f.at("Data/Skyrim.esm"), "new").unwrap();
    assert_eq!(fs::read_to_string(f.at("Data/Skyrim.esm")).unwrap(), "new");

    // a mod's Root file is edited in place (Leet change 6)
    fs::write(f.at("enbseries.ini"), "enb edited").unwrap();

    f.unmount();
    assert_eq!(f.disk("overwrite/SKSE/Plugins/probe.json").as_deref(), Some("{}"));
    assert_eq!(f.disk("overwrite/Root/d3dx9_42.log").as_deref(), Some("log"));
    assert_eq!(f.disk("overwrite/textures/b.dds").as_deref(), Some("low b + edit"));
    assert_eq!(f.disk("mods/low/textures/b.dds").as_deref(), Some("low b"));
    assert_eq!(f.disk("overwrite/Skyrim.esm").as_deref(), Some("new"));
    assert_eq!(f.disk("Game Root/Data/Skyrim.esm").as_deref(), Some("esm"), "the real game folder is untouched");
    assert_eq!(f.disk("mods/high/Root/enbseries.ini").as_deref(), Some("enb edited"));
    assert_eq!(names(&f.game), ["Data", "SkyrimSE.exe"]);
    assert_eq!(names(&f.game.join("Data")), ["Skyrim.esm", "textures"]);
}

#[test]
fn delete_and_rename_behave_and_survive_a_remount() {
    if !fuse_available() {
        return;
    }
    let mut f = Fixture::new(BASE);

    fs::remove_file(f.at("Data/Textures/a.dds")).unwrap();
    assert!(!f.at("Data/textures/a.dds").exists(), "no lower layer's copy shows through");
    assert_eq!(fs::remove_file(f.at("Data/textures/a.dds")).unwrap_err().kind(), std::io::ErrorKind::NotFound);

    // the save pattern: write a temp file, rename it over the real one
    fs::write(f.at("Data/textures/b.dds.tmp"), "saved").unwrap();
    fs::rename(f.at("Data/textures/b.dds.tmp"), f.at("Data/textures/b.dds")).unwrap();
    assert_eq!(fs::read_to_string(f.at("Data/textures/b.dds")).unwrap(), "saved");
    assert_eq!(names(&f.at("Data/textures")), ["b.dds"]);

    // a folder: made, filled, renamed, emptied, removed
    fs::create_dir(f.at("Data/out")).unwrap();
    fs::write(f.at("Data/out/x.txt"), "x").unwrap();
    fs::rename(f.at("Data/out"), f.at("Data/Output")).unwrap();
    assert_eq!(fs::read_to_string(f.at("Data/output/X.TXT")).unwrap(), "x");
    assert!(fs::remove_dir(f.at("Data/Output")).is_err(), "not empty");
    fs::remove_file(f.at("Data/Output/x.txt")).unwrap();
    fs::remove_dir(f.at("Data/Output")).unwrap();

    // a mod's folder, renamed
    fs::rename(f.at("Data/meshes"), f.at("Data/meshes2")).unwrap();
    assert!(!f.at("Data/meshes").exists());

    f.unmount();
    assert_eq!(f.disk("mods/low/textures/a.dds").as_deref(), Some("low a"), "mods keep their files");
    assert_eq!(f.disk("mods/high/Textures/A.DDS").as_deref(), Some("high a"));
    assert!(f.disk("strata/whiteouts.txt").unwrap().contains("Data/textures/a.dds"));
    assert!(!f.root.join("overwrite/Output").exists());

    f.mount();
    assert!(!f.at("Data/textures/a.dds").exists(), "the whiteout holds after a remount");
    assert_eq!(fs::read_to_string(f.at("Data/textures/b.dds")).unwrap(), "saved");
    assert!(!f.at("Data/meshes").exists());
    assert_eq!(fs::read_to_string(f.at("Data/meshes2/ünïcödé/naïve — file.nif")).unwrap(), "unicode");
    f.unmount();
}

#[test]
fn folders_say_they_fold_case() {
    if !fuse_available() {
        return;
    }
    const FS_IOC_GETFLAGS: libc::c_ulong = 0x8008_6601;
    const FS_CASEFOLD_FL: libc::c_long = 0x4000_0000;
    let mut f = Fixture::new(BASE);
    let ask = |path: &Path| -> Result<libc::c_long, std::io::Error> {
        let file = fs::File::open(path)?;
        let mut flags: libc::c_long = 0;
        // SAFETY: the descriptor is open and `flags` is a valid place for the answer.
        let result = unsafe { libc::ioctl(file.as_raw_fd(), FS_IOC_GETFLAGS, &mut flags) };
        if result == 0 { Ok(flags) } else { Err(std::io::Error::last_os_error()) }
    };
    assert_ne!(ask(&f.at("Data")).unwrap() & FS_CASEFOLD_FL, 0);
    assert_ne!(ask(&f.game).unwrap() & FS_CASEFOLD_FL, 0);
    assert_eq!(ask(&f.at("SkyrimSE.exe")).unwrap_err().raw_os_error(), Some(libc::ENOTTY));
    f.unmount();
}

#[test]
fn many_readers_at_once_see_whole_files() {
    if !fuse_available() {
        return;
    }
    let big: String = (0..200_000).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    let mut files: Vec<(&str, &str, &str)> = BASE.to_vec();
    files.push(("low", "big.bsa", &big));
    let mut f = Fixture::new(&files);
    let path = f.at("Data/BIG.bsa");
    std::thread::scope(|scope| {
        for reader in 0..16 {
            let (path, big) = (&path, &big);
            scope.spawn(move || {
                for round in 0..20 {
                    let mut file = fs::File::open(path).unwrap();
                    let offset = (reader * 7919 + round * 104_729) % 150_000;
                    file.seek(SeekFrom::Start(offset as u64)).unwrap();
                    let mut buffer = vec![0u8; 40_000];
                    file.read_exact(&mut buffer).unwrap();
                    assert_eq!(buffer, big.as_bytes()[offset..offset + 40_000]);
                }
            });
        }
    });
    f.unmount();
}

#[test]
fn a_mount_left_behind_is_cleaned_by_the_next_one() {
    if !fuse_available() {
        return;
    }
    let mut f = Fixture::new(BASE);
    // Forget the mount without unmounting, as a killed daemon would leave it.
    std::mem::forget(f.mount.take());
    assert_eq!(strata_vfs::mount::mounted_at(&f.game).unwrap().as_deref(), Some("fuse.strata"));
    f.mount();
    assert_eq!(fs::read_to_string(f.at("Data/textures/a.dds")).unwrap(), "high a");
    f.unmount();
    assert_eq!(strata_vfs::mount::mounted_at(&f.game).unwrap(), None);
}

#[test]
fn a_refresh_swaps_the_layers_of_a_live_mount() {
    if !fuse_available() {
        return;
    }
    let mut f = Fixture::new(BASE);
    // a file kept open across the refresh, and one held by path
    let mut open = fs::File::open(f.at("Data/textures/b.dds")).unwrap();
    assert_eq!(fs::read_to_string(f.at("Data/textures/a.dds")).unwrap(), "high a");
    let inode = |p: &Path| std::os::unix::fs::MetadataExt::ino(&fs::metadata(p).unwrap());
    let textures = inode(&f.at("Data/textures"));
    assert!(names(&f.game).contains(&"enbseries.ini".to_owned()), "the listing is cached now");

    // "high" switched off, "low" moved above where it was: only the game and low remain
    let mut spec = f.spec();
    spec.layers.retain(|l| l.id != "high");
    let report = f.mount.as_ref().unwrap().refresh(spec.layers).unwrap();
    assert!(report.changed >= 1 && report.entries >= 1, "{report:?}");

    assert_eq!(fs::read_to_string(f.at("Data/textures/a.dds")).unwrap(), "low a", "the lower layer shows at once");
    assert!(!f.at("enbseries.ini").exists(), "a file of the removed layer is gone");
    assert!(!names(&f.game).contains(&"enbseries.ini".to_owned()), "and the cached listing forgot it");
    assert!(!f.at("Data/meshes").exists());
    assert_eq!(inode(&f.at("Data/textures")), textures, "unchanged folders keep their inode");
    let mut text = String::new();
    open.read_to_string(&mut text).unwrap();
    assert_eq!(text, "low b", "an open file reads on");

    // and back: the layer's files return, writes still go to overwrite
    let report = f.mount.as_ref().unwrap().refresh(f.spec().layers).unwrap();
    assert!(report.changed >= 1, "{report:?}");
    assert_eq!(fs::read_to_string(f.at("Data/textures/a.dds")).unwrap(), "high a");
    assert_eq!(fs::read_to_string(f.at("ENBSERIES.ini")).unwrap(), "enb");
    assert!(names(&f.game).contains(&"enbseries.ini".to_owned()));
    fs::write(f.at("Data/new.txt"), "new").unwrap();
    drop(open);
    f.unmount();
    assert_eq!(f.disk("overwrite/Data/new.txt").or(f.disk("overwrite/new.txt")).as_deref(), Some("new"));
}
