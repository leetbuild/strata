//! LOOT sorting through libloot (the Rust crate LOOT itself is built on since 0.25).
//!
//! MO2 sorts by running `lootcli` inside its virtual file system (`modorganizer/src/loot.cpp`,
//! `Loot::spawnLootcli`; `modorganizer-lootcli/src/lootthread.cpp`, `LOOTWorker::run`):
//! a game handle on the game folder with the profile as the local folder, the masterlist and
//! the user's userlist, `LoadCurrentLoadOrderState`, `LoadPlugins` of the whole load order,
//! `SortPlugins(load order)`, and the result written over the profile's `loadorder.txt`.
//! Strata does the same calls with libloot, without a mount: the mods' folders are given to
//! libloot as additional data folders, highest priority first, so it sees the merged Data
//! folder (plugins, archives and the files masterlist conditions ask about).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libloot::{Game, GameType, LogLevel};

mod report;
pub use libloot::libloot_version;
pub use report::{Dirty, FileRef, PluginReport, Report, ReportMessage};

/// What can go wrong.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("LOOT does not know the game \"{0}\"")]
    UnknownGame(String),
    #[error("libloot: {0}")]
    Loot(String),
    #[error(
        "no LOOT masterlist at {0}. Run `strata sort` while online once (it downloads the masterlist), \
         or put a masterlist.yaml there."
    )]
    NoMasterlist(PathBuf),
    #[error("cannot download {url}: {reason}")]
    Download { url: String, reason: String },
    #[error(
        "the load order libloot reads from the profile differs from Strata's (first difference at \
         position {position}: libloot has {loot:?}, Strata {strata:?}). Sorting would not match MO2's \
         lootcli; open the plugin list once to rewrite the profile's files."
    )]
    OrderMismatch { position: usize, loot: Option<String>, strata: Option<String> },
    #[error("cancelled")]
    Cancelled,
}

/// The steps of a sort, as lootcli reports them to MO2's LOOT dialog (`lootcli::Progress`,
/// same order and numbers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Progress {
    CheckingMasterlistExistence = 0,
    UpdatingMasterlist = 1,
    LoadingLists = 2,
    ReadingPlugins = 3,
    SortingPlugins = 4,
    WritingLoadorder = 5,
    ParsingLootMessages = 6,
    Done = 7,
}

/// A log line's level (MO2's `log::Levels`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    Debug = 0,
    Info = 1,
    Warning = 2,
    Error = 3,
}

/// Who watches a sort: the LOOT dialog. Called from the sorting thread and, for libloot's
/// log lines, from its worker threads.
pub trait Observer: Send + Sync {
    fn progress(&self, step: Progress);
    fn log(&self, level: Level, line: &str);
    /// Checked between the steps (libloot's own steps can't be interrupted).
    fn cancelled(&self) -> bool {
        false
    }
}

/// An observer that ignores everything (the CLI, `sort`).
pub struct Silent;

impl Observer for Silent {
    fn progress(&self, _: Progress) {}
    fn log(&self, _: Level, _: &str) {}
}

fn loot(e: impl std::fmt::Display) -> Error {
    Error::Loot(e.to_string())
}

/// The libloot game for MO2's `lootGameName` (`IPluginGame::lootGameName`, the `--game`
/// argument of lootcli; lootcli's `gameMap`).
pub fn game_type(loot_name: &str) -> Result<GameType, Error> {
    Ok(match loot_name {
        "morrowind" => GameType::Morrowind,
        "oblivion" => GameType::Oblivion,
        "fallout3" => GameType::Fallout3,
        "fallout4" => GameType::Fallout4,
        "fallout4vr" => GameType::Fallout4VR,
        "falloutnv" => GameType::FalloutNV,
        "skyrim" => GameType::Skyrim,
        "skyrimse" => GameType::SkyrimSE,
        "skyrimvr" => GameType::SkyrimVR,
        "starfield" => GameType::Starfield,
        "oblivionremastered" => GameType::OblivionRemastered,
        other => return Err(Error::UnknownGame(other.to_owned())),
    })
}

/// LOOT's masterlist branch for libloot 0.29. The repository is named like the game
/// (`loot/skyrimse`).
const BRANCH: &str = "v0.29";

/// Where Strata keeps LOOT's masterlist and prelude for a game:
/// `$XDG_DATA_HOME/strata/loot/<game>/` (shared by all instances, like LOOT's own folder).
pub fn data_dir(loot_name: &str) -> PathBuf {
    let base = strata_core::dirs::data_home().unwrap_or_else(|| PathBuf::from("."));
    base.join("strata/loot").join(loot_name)
}

/// Downloads the masterlist and the prelude into `dir` with `curl` (each through a temporary
/// file, so a failed download keeps the old copy).
pub fn update_masterlist(loot_name: &str, dir: &Path) -> Result<(), Error> {
    update_masterlist_with(loot_name, dir, &Silent)
}

/// [`update_masterlist`], ending curl when the observer cancels.
pub fn update_masterlist_with(loot_name: &str, dir: &Path, observer: &dyn Observer) -> Result<(), Error> {
    std::fs::create_dir_all(dir).map_err(|e| Error::Download { url: dir.display().to_string(), reason: e.to_string() })?;
    let files = [
        (format!("https://raw.githubusercontent.com/loot/{loot_name}/{BRANCH}/masterlist.yaml"), "masterlist.yaml"),
        (format!("https://raw.githubusercontent.com/loot/prelude/{BRANCH}/prelude.yaml"), "prelude.yaml"),
    ];
    for (url, name) in files {
        let temp = dir.join(format!(".{name}.download"));
        observer.log(Level::Info, &format!("downloading {url}"));
        let mut child = Command::new("curl")
            .args(["--fail", "--silent", "--show-error", "--location", "--max-time", "60", "--output"])
            .arg(&temp)
            .arg(&url)
            .stdin(std::process::Stdio::null())
            .spawn()
            .map_err(|e| Error::Download { url: url.clone(), reason: format!("cannot run curl: {e}") })?;
        let status = loop {
            if observer.cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&temp);
                return Err(Error::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => return Err(Error::Download { url, reason: e.to_string() }),
            }
        };
        if !status.success() {
            let _ = std::fs::remove_file(&temp);
            return Err(Error::Download { url, reason: format!("curl exited with {status}") });
        }
        std::fs::rename(&temp, dir.join(name)).map_err(|e| Error::Download { url, reason: e.to_string() })?;
    }
    Ok(())
}

/// The input of a sort.
#[derive(Debug, Clone)]
pub struct SortInput {
    /// MO2's `lootGameName`: `skyrimse`.
    pub loot_name: String,
    /// The real game folder (its `Data` holds the base game).
    pub game_path: PathBuf,
    /// The profile folder: libloot reads `plugins.txt` and `loadorder.txt` from there.
    pub local_path: PathBuf,
    /// The layers' Data folders, highest priority first (overwrite, then the mods).
    pub data_paths: Vec<PathBuf>,
    /// Every plugin's real file, in the current load order (primary plugins first).
    pub plugins: Vec<(String, PathBuf)>,
    pub masterlist: PathBuf,
    /// Loaded with the masterlist when present (current masterlists need it).
    pub prelude: Option<PathBuf>,
    /// The user's LOOT userlist, when present.
    pub userlist: Option<PathBuf>,
}

/// Sorts as lootcli does and returns the new load order (every plugin, primary ones first).
pub fn sort(input: &SortInput) -> Result<Vec<String>, Error> {
    Ok(sort_with_report(input, Arc::new(Silent))?.order)
}

/// What a sort gives: the new order and lootcli's report.
#[derive(Debug, Clone)]
pub struct Sorted {
    /// Every plugin, primary ones first.
    pub order: Vec<String>,
    pub report: Report,
}

/// libloot logs through one global callback: one sort at a time.
static RUNNING: Mutex<()> = Mutex::new(());

/// Points libloot's log at an observer for as long as it lives.
struct LogTo;

impl LogTo {
    fn new(observer: Arc<dyn Observer>) -> Self {
        libloot::set_log_level(LogLevel::Info);
        libloot::set_logging_callback(move |level, line| {
            let level = match level {
                LogLevel::Trace | LogLevel::Debug => Level::Debug,
                LogLevel::Info => Level::Info,
                LogLevel::Warning => Level::Warning,
                LogLevel::Error => Level::Error,
            };
            observer.log(level, line);
        });
        Self
    }
}

impl Drop for LogTo {
    fn drop(&mut self) {
        libloot::set_logging_callback(|_, _| {});
    }
}

/// Sorts as lootcli does (`LOOTWorker::run`), telling the observer each step, and builds
/// lootcli's report (`LOOTWorker::createJsonReport`): the general messages, and for every
/// loaded plugin its evaluated messages, dirty and clean info, incompatibilities present,
/// missing masters and requirements.
pub fn sort_with_report(input: &SortInput, observer: Arc<dyn Observer>) -> Result<Sorted, Error> {
    let _one = RUNNING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let _log = LogTo::new(observer.clone());
    let check = || if observer.cancelled() { Err(Error::Cancelled) } else { Ok(()) };
    if !input.masterlist.is_file() {
        return Err(Error::NoMasterlist(input.masterlist.clone()));
    }
    check()?;
    observer.progress(Progress::LoadingLists);
    // libloadorder looks for `Plugins.txt` with a capital P (Windows doesn't care); MO2's
    // profiles have `plugins.txt`. Give it a folder with the profile's lists under the names
    // it wants.
    let local = LocalCopy::new(&input.local_path)?;
    let mut game = Game::with_local_path(game_type(&input.loot_name)?, &input.game_path, &local.0).map_err(loot)?;
    game.set_additional_data_paths(input.data_paths.clone()).map_err(loot)?;
    {
        let db = game.database();
        let mut db = db.write().map_err(loot)?;
        observer.log(Level::Info, &format!("loading the masterlist {}", input.masterlist.display()));
        match &input.prelude {
            Some(prelude) if prelude.is_file() => db.load_masterlist_with_prelude(&input.masterlist, prelude).map_err(loot)?,
            _ => db.load_masterlist(&input.masterlist).map_err(loot)?,
        }
        if let Some(userlist) = input.userlist.as_ref().filter(|u| u.is_file()) {
            observer.log(Level::Info, &format!("loading the userlist {}", userlist.display()));
            db.load_userlist(userlist).map_err(loot)?;
        }
    }
    game.load_current_load_order_state().map_err(loot)?;

    // lootcli sorts libloadorder's view of the profile; Strata's must be the same, or the
    // result could differ from MO2's.
    let loot_order: Vec<String> = game.load_order().iter().map(|s| (*s).to_owned()).collect();
    let mine: Vec<&str> = input.plugins.iter().map(|(n, _)| n.as_str()).collect();
    if let Some(position) = (0..loot_order.len().max(mine.len()))
        .find(|&i| !loot_order.get(i).zip(mine.get(i)).is_some_and(|(a, b)| a.eq_ignore_ascii_case(b)))
    {
        return Err(Error::OrderMismatch {
            position,
            loot: loot_order.get(position).cloned(),
            strata: mine.get(position).map(|s| (*s).to_owned()),
        });
    }

    check()?;
    observer.progress(Progress::ReadingPlugins);
    observer.log(Level::Info, &format!("reading {} plugins", input.plugins.len()));
    let paths: Vec<&Path> = input.plugins.iter().map(|(_, p)| p.as_path()).collect();
    game.load_plugins(&paths).map_err(loot)?;

    check()?;
    observer.progress(Progress::SortingPlugins);
    let started = Instant::now();
    let order = game.sort_plugins(&mine).map_err(loot)?;
    observer.log(Level::Info, &format!("sorted in {:.2} s", started.elapsed().as_secs_f64()));

    check()?;
    observer.progress(Progress::ParsingLootMessages);
    let report = report::build(&game, input, &order)?;
    Ok(Sorted { order, report })
}

/// A temporary folder holding the profile's `plugins.txt` as `Plugins.txt` and its
/// `loadorder.txt`; removed when dropped.
struct LocalCopy(PathBuf);

impl LocalCopy {
    fn new(profile: &Path) -> Result<Self, Error> {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("strata-loot-{}-{n}", std::process::id()));
        let io = |e: std::io::Error| Error::Loot(format!("cannot prepare {}: {e}", dir.display()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(io)?;
        let copy = Self(dir.clone());
        for (from, to) in [("plugins.txt", "Plugins.txt"), ("loadorder.txt", "loadorder.txt")] {
            match std::fs::copy(profile.join(from), dir.join(to)) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io(e)),
            }
        }
        Ok(copy)
    }
}

impl Drop for LocalCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mo2_game_names_map_to_libloot() {
        assert_eq!(game_type("skyrimse").unwrap(), GameType::SkyrimSE);
        assert!(matches!(game_type("tes5se"), Err(Error::UnknownGame(_))));
    }

    #[test]
    fn a_missing_masterlist_says_how_to_get_one() {
        let dir = tempfile::tempdir().unwrap();
        let input = SortInput {
            loot_name: "skyrimse".into(),
            game_path: dir.path().into(),
            local_path: dir.path().into(),
            data_paths: Vec::new(),
            plugins: Vec::new(),
            masterlist: dir.path().join("masterlist.yaml"),
            prelude: None,
            userlist: None,
        };
        let e = sort(&input).unwrap_err();
        assert!(e.to_string().contains("strata sort"), "{e}");
    }

    /// A masterlist rule moves a plugin: `a.esp` must load after `b.esp`.
    #[test]
    fn sorts_by_the_masterlist() {
        use strata_game::plugin::{FLAG_MASTER, build_plugin};
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let profile = dir.path().join("profile");
        let mod_dir = dir.path().join("mod");
        for d in [game.join("Data"), profile.clone(), mod_dir.clone()] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(game.join("Data/Skyrim.esm"), build_plugin(FLAG_MASTER, 44, &[], "", 1)).unwrap();
        for name in ["a.esp", "b.esp"] {
            std::fs::write(mod_dir.join(name), build_plugin(0, 44, &["Skyrim.esm"], "", 1)).unwrap();
        }
        let header = "# This file was automatically generated by Mod Organizer.\r\n";
        std::fs::write(profile.join("plugins.txt"), format!("{header}*a.esp\r\n*b.esp\r\n")).unwrap();
        std::fs::write(profile.join("loadorder.txt"), format!("{header}Skyrim.esm\r\na.esp\r\nb.esp\r\n")).unwrap();
        let masterlist = dir.path().join("masterlist.yaml");
        std::fs::write(&masterlist, "plugins:\n  - name: a.esp\n    after: [b.esp]\n").unwrap();
        let input = SortInput {
            loot_name: "skyrimse".into(),
            game_path: game.clone(),
            local_path: profile,
            data_paths: vec![mod_dir.clone()],
            plugins: vec![
                ("Skyrim.esm".into(), game.join("Data/Skyrim.esm")),
                ("a.esp".into(), mod_dir.join("a.esp")),
                ("b.esp".into(), mod_dir.join("b.esp")),
            ],
            masterlist,
            prelude: None,
            userlist: None,
        };
        assert_eq!(sort(&input).unwrap(), ["Skyrim.esm", "b.esp", "a.esp"]);

        // An order libloot reads differently from Strata's is refused.
        let mut wrong = input.clone();
        wrong.plugins.swap(1, 2);
        assert!(matches!(sort(&wrong), Err(Error::OrderMismatch { position: 1, .. })));
    }

    /// CRC-32 as libloot computes a plugin's (IEEE, reflected).
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>, std::sync::atomic::AtomicBool);

    impl Observer for Recorder {
        fn progress(&self, step: Progress) {
            self.0.lock().unwrap().push(format!("{step:?}"));
        }
        fn log(&self, level: Level, line: &str) {
            self.0.lock().unwrap().push(format!("[{level:?}] {line}"));
        }
        fn cancelled(&self) -> bool {
            self.1.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    /// The report: general messages and plugin messages whose conditions hold, dirty info
    /// whose CRC matches, clean info, incompatibilities present, missing masters and
    /// requirements.
    #[test]
    fn reports_messages_dirty_info_and_requirements() {
        use strata_game::plugin::{FLAG_MASTER, build_plugin};
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let profile = dir.path().join("profile");
        let mod_dir = dir.path().join("mod");
        for d in [game.join("Data"), profile.clone(), mod_dir.join("skse/plugins")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(game.join("Data/Skyrim.esm"), build_plugin(FLAG_MASTER, 44, &[], "", 1)).unwrap();
        let a = build_plugin(0, 44, &["Skyrim.esm", "Missing.esm"], "", 1);
        let b = build_plugin(0, 44, &["Skyrim.esm"], "b", 1);
        std::fs::write(mod_dir.join("a.esp"), &a).unwrap();
        std::fs::write(mod_dir.join("b.esp"), &b).unwrap();
        std::fs::write(mod_dir.join("skse/plugins/Present.dll"), "x").unwrap();
        let header = "# This file was automatically generated by Mod Organizer.\r\n";
        std::fs::write(profile.join("plugins.txt"), format!("{header}*a.esp\r\n*b.esp\r\n")).unwrap();
        std::fs::write(profile.join("loadorder.txt"), format!("{header}Skyrim.esm\r\na.esp\r\nb.esp\r\n")).unwrap();
        let masterlist = dir.path().join("masterlist.yaml");
        std::fs::write(
            &masterlist,
            format!(
                r#"globals:
  - type: warn
    content: 'A general warning.'
  - type: say
    content: 'Not shown: Nope.esp is absent.'
    condition: 'file("Nope.esp")'
plugins:
  - name: a.esp
    after: [b.esp]
    msg:
      - type: say
        content: 'A note about **a**.'
      - type: error
        content: 'Not shown.'
        condition: 'file("Absent.esp")'
    inc: [b.esp, Absent.esp]
    req:
      - 'SKSE/Plugins/present.dll'
      - name: 'SKSE/Plugins/Needed.dll'
        display: 'Needed Framework'
    dirty:
      - crc: 0x{:08X}
        util: 'SSEEdit v4.0.4'
        itm: 3
        udr: 2
        nav: 1
        detail: 'Clean it with SSEEdit.'
      - crc: 0xDEADBEEF
        util: 'not this version'
        itm: 99
  - name: b.esp
    clean:
      - crc: 0x{:08X}
        util: 'SSEEdit v4.0.4'
"#,
                crc32(&a),
                crc32(&b)
            ),
        )
        .unwrap();
        let input = SortInput {
            loot_name: "skyrimse".into(),
            game_path: game.clone(),
            local_path: profile,
            data_paths: vec![mod_dir.clone()],
            plugins: vec![
                ("Skyrim.esm".into(), game.join("Data/Skyrim.esm")),
                ("a.esp".into(), mod_dir.join("a.esp")),
                ("b.esp".into(), mod_dir.join("b.esp")),
            ],
            masterlist,
            prelude: None,
            userlist: None,
        };
        let recorder = Arc::new(Recorder::default());
        let sorted = sort_with_report(&input, recorder.clone()).unwrap();
        assert_eq!(sorted.order, ["Skyrim.esm", "b.esp", "a.esp"]);
        let r = &sorted.report;
        assert_eq!(r.messages, [ReportMessage { kind: "warn".into(), text: "A general warning.".into() }]);
        let names: Vec<&str> = r.plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["b.esp", "a.esp"], "new order, plugins with nothing to say left out");
        let pa = &r.plugins[1];
        assert_eq!(pa.messages, [ReportMessage { kind: "info".into(), text: "A note about **a**.".into() }]);
        assert_eq!(
            pa.dirty,
            [Dirty {
                crc: crc32(&a),
                itm: 3,
                deleted_references: 2,
                deleted_navmesh: 1,
                cleaning_utility: "SSEEdit v4.0.4".into(),
                info: "Clean it with SSEEdit.".into(),
            }]
        );
        assert_eq!(pa.incompatibilities, [FileRef { name: "b.esp".into(), display_name: String::new() }]);
        assert_eq!(
            pa.missing_requirements,
            [FileRef { name: "SKSE/Plugins/Needed.dll".into(), display_name: "Needed Framework".into() }]
        );
        assert_eq!(pa.missing_masters, ["Missing.esm"]);
        assert!(!pa.is_master && !pa.loads_archive);
        let pb = &r.plugins[0];
        assert!(pb.dirty.is_empty() && pb.messages.is_empty());
        assert_eq!(pb.clean.len(), 1);
        assert_eq!(pb.clean[0].cleaning_utility, "SSEEdit v4.0.4");

        // lootcli's JSON names
        let json = serde_json::to_value(r).unwrap();
        assert_eq!(json["plugins"][1]["dirty"][0]["deletedNavmesh"], 1);
        assert_eq!(json["plugins"][1]["missingMasters"][0], "Missing.esm");
        assert_eq!(json["messages"][0]["type"], "warn");

        // the steps, in lootcli's order, with log lines between them
        let seen = recorder.0.lock().unwrap().clone();
        let steps: Vec<&str> = seen.iter().filter(|l| !l.starts_with('[')).map(String::as_str).collect();
        assert_eq!(steps, ["LoadingLists", "ReadingPlugins", "SortingPlugins", "ParsingLootMessages"]);
        assert!(seen.iter().any(|l| l.starts_with("[Info] reading 3 plugins")), "{seen:?}");

        // a cancelled sort stops at the next step
        let cancelled = Arc::new(Recorder::default());
        cancelled.1.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(sort_with_report(&input, cancelled), Err(Error::Cancelled)));
    }
}
