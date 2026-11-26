//! Crash safety (`docs/DESIGN.md`, "The overlay"): vfsd killed with SIGKILL in the middle of a copy-up, a
//! rename, a delete or a create. After the next mount nothing is half-done: the overlay
//! shows the state before the change or the state after it, no temp file shows or stays on
//! disk, no journal is left, and every whiteout hides something a lower layer has.
//!
//! The test program plays the daemon itself (test `child`, with `STRATA_CRASH_CHILD` set):
//! it mounts, and the crash hooks of the test build (feature `crash-hooks`, `tree.rs`) stop
//! it at the chosen point, where the parent kills it.
//!
//! Needs `/dev/fuse` and `fusermount3`; without them the test passes with a note on stderr.

use std::fs;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use strata_game::{Game, Layer, LayerKind, Mapping, SkyrimSE};
use strata_vfs::{Mount, MountSpec};

const FILES: &[(&str, &str, &str)] = &[
    ("game", "SkyrimSE.exe", "the game"),
    ("game", "Data/Skyrim.esm", "esm"),
    ("low", "textures/a.dds", "low a"),
    ("low", "textures/b.dds", "low b"),
    ("low", "meshes/Ünïcödé/naïve.nif", "unicode"),
    ("low", "meshes/rock.nif", "rock"),
    ("high", "Root/enbseries.ini", "enb"),
    ("overwrite", "textures/a.dds", "overwrite a"),
    ("overwrite", "textures/c.dds", "overwrite c"),
    ("overwrite", "out/one.txt", "one"),
    ("overwrite", "out/two.txt", "two"),
    ("overwrite", "out/sub/three.txt", "three"),
];

fn spec(root: &Path) -> MountSpec {
    let rules = SkyrimSE.mod_mappings();
    let game = root.join("Game Root");
    let layer = |id: &str, kind, source: PathBuf, rules| Layer { id: id.into(), kind, source, rules };
    MountSpec {
        id: "game".into(),
        layers: vec![
            layer("<game>", LayerKind::Game, game.clone(), vec![Mapping::new("", "")]),
            layer("low", LayerKind::Mod, root.join("mods/low"), rules.clone()),
            layer("high", LayerKind::Mod, root.join("mods/high"), rules.clone()),
            layer("<overwrite>", LayerKind::Overwrite, root.join("overwrite"), rules),
        ],
        mountpoint: game,
        whiteouts: Some(root.join("strata/whiteouts.txt")),
        label: None,
    }
}

fn fixture(base: &Path, name: &str) -> PathBuf {
    let root = base.join(name);
    for dir in ["Game Root/Data", "mods/low", "mods/high", "overwrite"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (layer, path, text) in FILES {
        let dir = match *layer {
            "game" => "Game Root".to_owned(),
            "overwrite" => "overwrite".to_owned(),
            name => format!("mods/{name}"),
        };
        let path = root.join(dir).join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    root
}

type Listing = Vec<(String, Option<Vec<u8>>)>;

fn listing(game: &Path) -> Listing {
    fn walk(dir: &Path, prefix: &str, out: &mut Listing) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            if entry.path().is_dir() {
                out.push((path.clone(), None));
                walk(&entry.path(), &path, out);
            } else {
                out.push((path, Some(fs::read(entry.path()).unwrap())));
            }
        }
    }
    let mut out = Vec::new();
    walk(game, "", &mut out);
    out.sort();
    out
}

fn show(listing: &Listing) -> String {
    listing
        .iter()
        .map(|(p, d)| match d {
            Some(d) => format!("  {p} = {:?}\n", String::from_utf8_lossy(d)),
            None => format!("  {p}/\n"),
        })
        .collect()
}

/// Whether a lower layer has `path` (a mount path), found without regard to case.
fn lower_has(root: &Path, path: &str) -> bool {
    let find = |base: PathBuf, rest: &str| {
        rest.split('/').try_fold(base, |at, part| {
            fs::read_dir(&at).ok()?.flatten().find(|e| e.file_name().to_string_lossy().to_lowercase() == part.to_lowercase()).map(|e| e.path())
        })
    };
    let in_data = path.len() > 5 && path[..5].eq_ignore_ascii_case("data/");
    let in_mod = |name: &str| {
        let base = root.join("mods").join(name);
        if in_data { find(base, &path[5..]) } else { find(base.join("Root"), path) }
    };
    find(root.join("Game Root"), path).is_some() || in_mod("low").is_some() || in_mod("high").is_some()
}

/// Temp files and journals anywhere in overwrite and the instance's `strata` folder.
fn leftovers(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.join("overwrite"), root.join("strata")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_dir() {
                stack.push(entry.path());
            } else if name.starts_with(".strata-tmp-") || name.contains("journal") || name.ends_with(".tmp") {
                out.push(entry.path().display().to_string());
            }
        }
    }
    out
}

type Action = fn(&Path) -> std::io::Result<()>;

struct Scenario {
    name: &'static str,
    /// Done through a mount before the crash run starts (the crash is in `action`).
    setup: Option<Action>,
    action: Action,
    /// `STRATA_CRASH_AT` values: the points to stop at, one crash run each.
    points: &'static [&'static str],
}

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "a mod's file changed in place (copy-up)",
            setup: None,
            action: |g| fs::File::options().write(true).open(g.join("Data/textures/b.dds"))?.write_all_at(b"XY", 0),
            points: &["copy-up"],
        },
        Scenario {
            name: "a game file truncated (copy-up)",
            setup: None,
            action: |g| fs::write(g.join("Data/Skyrim.esm"), "new"),
            points: &["copy-up"],
        },
        Scenario {
            name: "a mod's folder renamed (copies, whiteout)",
            setup: None,
            action: |g| fs::rename(g.join("Data/meshes"), g.join("Data/Models")),
            points: &["rename-step#1", "rename-step#2", "rename-step#3", "rename-step#4", "rename-step#5"],
        },
        Scenario {
            name: "a folder in overwrite renamed (moves)",
            setup: None,
            action: |g| fs::rename(g.join("Data/out"), g.join("Data/Output")),
            points: &["rename-step#1", "rename-step#2", "rename-step#3", "rename-step#4", "rename-step#5", "rename-step#6"],
        },
        Scenario {
            name: "a mod's file renamed over a file in overwrite",
            setup: None,
            action: |g| fs::rename(g.join("Data/textures/b.dds"), g.join("Data/textures/C.dds")),
            points: &["rename-step#1", "rename-step#2"],
        },
        Scenario {
            name: "a folder renamed over an empty folder of a mod",
            setup: Some(|g| fs::remove_file(g.join("Data/meshes/rock.nif")).and(fs::remove_file(g.join("Data/meshes/Ünïcödé/naïve.nif"))).and(fs::remove_dir(g.join("Data/meshes/Ünïcödé")))),
            action: |g| fs::rename(g.join("Data/out"), g.join("Data/meshes")),
            points: &["rename-step#1", "rename-step#3", "rename-step#5", "rename-step#7"],
        },
        Scenario {
            name: "an overwrite file over a mod's file deleted",
            setup: None,
            action: |g| fs::remove_file(g.join("Data/textures/a.dds")),
            points: &["unlink"],
        },
        Scenario {
            name: "a file made again where a whiteout stands",
            setup: Some(|g| fs::remove_file(g.join("Data/textures/b.dds"))),
            action: |g| fs::File::create(g.join("Data/textures/B.DDS")).map(drop),
            points: &["create"],
        },
    ]
}

fn fuse_available() -> bool {
    let ok = Path::new("/dev/fuse").exists()
        && Command::new("fusermount3").arg("-V").output().is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("SKIPPED: /dev/fuse or fusermount3 is missing, the crash tests did not run");
    }
    ok
}

/// The daemon's part: mount, say so, wait to be killed. Does nothing in a normal run.
#[test]
fn child() {
    let Some(root) = std::env::var_os("STRATA_CRASH_CHILD") else { return };
    let root = PathBuf::from(root);
    let _mount = Mount::new(spec(&root), 2).unwrap();
    fs::write(root.join("ready"), "").unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn wait_for(path: &Path, what: &str) -> Result<(), String> {
    let start = Instant::now();
    while !path.exists() {
        if start.elapsed() > Duration::from_secs(20) {
            return Err(format!("timed out waiting for {what}"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

fn with_mount<T>(root: &Path, f: impl FnOnce(&Path) -> T) -> T {
    let mut mount = Mount::new(spec(root), 2).unwrap();
    let result = f(&root.join("Game Root"));
    mount.unmount(false).unwrap();
    result
}

/// One crash run: `Err` says what was wrong after the crash.
fn crash_run(base: &Path, scenario: &Scenario, point: &str, before: &Listing, after: &Listing) -> Result<(), String> {
    let root = fixture(base, &format!("crash-{}", point.replace('#', "-")));
    if let Some(setup) = scenario.setup {
        with_mount(&root, setup).map_err(|e| format!("setup: {e}"))?;
    }
    let mark = root.join("mark");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child", "--nocapture", "--test-threads=1"])
        .env("STRATA_CRASH_CHILD", &root)
        .env("STRATA_CRASH_AT", point)
        .env("STRATA_CRASH_MARK", &mark)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let result = (|| {
        wait_for(&root.join("ready"), "the daemon to mount")?;
        let game = root.join("Game Root");
        let action = scenario.action;
        let worker = std::thread::spawn(move || action(&game));
        let reached = wait_for(&mark, &format!("the crash point {point}"));
        let _ = child.kill(); // SIGKILL
        let _ = child.wait();
        let _ = worker.join();
        reached
    })();
    let _ = child.kill();
    let _ = child.wait();
    if result.is_err() {
        // the dead mount stays otherwise
        let _ = strata_vfs::mount::clean_stale(&root.join("Game Root"));
    }
    result?;

    // The next mount cleans the dead one up and finishes what the journal holds.
    let (seen, whiteouts) = with_mount(&root, |g| {
        let whiteouts = fs::read_to_string(root.join("strata/whiteouts.txt")).unwrap_or_default();
        (listing(g), whiteouts)
    });
    // Names compare without case: after a remount a lower layer's spelling comes back (the
    // model test checks spellings).
    let folded = |l: &Listing| -> Listing {
        let mut l: Listing = l.iter().map(|(p, d)| (p.to_lowercase(), d.clone())).collect();
        l.sort();
        l
    };
    if folded(&seen) != folded(before) && folded(&seen) != folded(after) {
        return Err(format!("half done:\n{}before:\n{}after:\n{}", show(&seen), show(before), show(after)));
    }
    let left = leftovers(&root);
    if !left.is_empty() {
        return Err(format!("left on disk: {left:?}"));
    }
    for line in whiteouts.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        let shown = seen.iter().find(|(p, _)| p.to_lowercase() == line.to_lowercase());
        // a whited-out folder may stand again, made in overwrite
        if matches!(shown, Some((_, Some(_)))) {
            return Err(format!("whiteout {line} stands over a file that shows"));
        }
        if !lower_has(&root, line) {
            return Err(format!("whiteout {line} hides nothing"));
        }
    }
    Ok(())
}

#[test]
fn a_killed_daemon_leaves_the_state_before_or_after() {
    if !fuse_available() || std::env::var_os("STRATA_CRASH_CHILD").is_some() {
        return;
    }
    let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut failures = Vec::new();
    for (index, scenario) in scenarios().iter().enumerate() {
        let base = tmp.path().join(index.to_string());
        let root = fixture(&base, "reference");
        let (before, after) = with_mount(&root, |g| {
            if let Some(setup) = scenario.setup {
                setup(g).unwrap();
            }
            let before = listing(g);
            (scenario.action)(g).unwrap();
            (before, listing(g))
        });
        assert_ne!(before, after, "{}: the action changes nothing", scenario.name);
        for point in scenario.points {
            if let Err(e) = crash_run(&base, scenario, point, &before, &after) {
                failures.push(format!("{} (stopped at {point}): {e}", scenario.name));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
