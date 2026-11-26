//! Finding Steam, its libraries, a Proton and the runtime that Proton asks for.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// The value of `"key" "value"` in Valve's text format, first match. Enough for the three
/// files read here (`libraryfolders.vdf`, `appmanifest_*.acf`, `toolmanifest.vdf`).
fn vdf_values<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
    let needle = format!("\"{key}\"");
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix(needle.as_str())?;
            let rest = rest.trim().strip_prefix('"')?;
            rest.strip_suffix('"')
        })
        .collect()
}

/// A Steam installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Steam {
    /// The folder with `steamapps/` and `compatibilitytools.d/`.
    pub root: PathBuf,
    /// Every library folder (each has its own `steamapps/`), the root's first.
    pub libraries: Vec<PathBuf>,
}

impl Steam {
    /// Finds Steam: the native install first, then the Flatpak.
    pub fn detect() -> Result<Self> {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let candidates = [
            home.join(".local/share/Steam"),
            home.join(".steam/steam"),
            home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        ];
        candidates.iter().find(|c| c.join("steamapps").is_dir()).map(|root| Self::at(root)).ok_or(Error::NoSteam)
    }

    /// The Steam installation in `root`.
    pub fn at(root: &Path) -> Self {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_owned());
        let mut libraries = vec![root.clone()];
        if let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
            for path in vdf_values(&text, "path") {
                let path = PathBuf::from(path.replace("\\\\", "\\"));
                let path = path.canonicalize().unwrap_or(path);
                if !libraries.contains(&path) {
                    libraries.push(path);
                }
            }
        }
        Self { root, libraries }
    }

    /// The install folder of an app, if any library has it.
    pub fn app_dir(&self, app_id: u32) -> Option<PathBuf> {
        self.libraries.iter().find_map(|library| {
            let steamapps = library.join("steamapps");
            let manifest = std::fs::read_to_string(steamapps.join(format!("appmanifest_{app_id}.acf"))).ok()?;
            let dir = steamapps.join("common").join(vdf_values(&manifest, "installdir").first()?);
            dir.is_dir().then_some(dir)
        })
    }

    /// Whether the Steam client is running (a process named `steam`).
    pub fn is_running() -> bool {
        !processes_named(Path::new("/proc"), "steam").is_empty()
    }

    /// Whether the client is up far enough for a game: the `steam` process and its
    /// `steamwebhelper` (started once the client has logged in and shows its UI, even with
    /// `-silent`).
    pub fn is_ready() -> bool {
        ready_in(Path::new("/proc"))
    }

    /// Makes sure Steam runs, as Fluorine's `ProtonLauncher::ensureSteamRunning` does
    /// (`steam -silent`), and waits until it is ready. Never starts a second client: when a
    /// `steam` process exists, Strata only waits for it. Returns whether Strata started it.
    pub fn ensure_running(timeout: std::time::Duration) -> Result<bool> {
        let started = if Self::is_running() {
            false
        } else {
            use std::os::unix::process::CommandExt;
            let mut command = std::process::Command::new("steam");
            command
                .arg("-silent")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            // Its own session: Steam outlives Strata and gets none of its signals.
            // SAFETY: setsid is async-signal-safe and touches no memory of this process.
            unsafe {
                command.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
            command.spawn().map_err(|_| Error::SteamStartFailed)?;
            true
        };
        let begin = std::time::Instant::now();
        while !Self::is_ready() {
            if begin.elapsed() > timeout {
                return Err(Error::SteamNotReady { seconds: timeout.as_secs() });
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        if started {
            // The web helper comes up a moment before the client takes DRM requests.
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        Ok(started)
    }
}

/// The pids of processes named `name` (their `comm`) under a `/proc`-like folder.
fn processes_named(proc_root: &Path, name: &str) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir(proc_root) else { return Vec::new() };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let pid = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            let comm = std::fs::read_to_string(entry.path().join("comm")).ok()?;
            (comm.trim() == name).then_some(pid)
        })
        .collect()
}

fn ready_in(proc_root: &Path) -> bool {
    !processes_named(proc_root, "steam").is_empty() && !processes_named(proc_root, "steamwebhelper").is_empty()
}

/// A Proton build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proton {
    pub dir: PathBuf,
    /// `require_tool_appid` of its `toolmanifest.vdf`: the Steam Linux Runtime it is built for.
    pub required_tool: Option<u32>,
}

impl Proton {
    /// Reads the Proton in `dir`.
    pub fn at(dir: &Path) -> Result<Self> {
        if !dir.join("proton").is_file() {
            return Err(Error::NotAProton { path: dir.to_owned() });
        }
        let manifest = dir.join("toolmanifest.vdf");
        let required_tool = match std::fs::read_to_string(&manifest) {
            Ok(text) => vdf_values(&text, "require_tool_appid").first().and_then(|v| v.parse().ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => return Err(Error::Read { path: manifest, source }),
        };
        Ok(Self { dir: dir.to_owned(), required_tool })
    }

    /// The `proton` script.
    pub fn script(&self) -> PathBuf {
        self.dir.join("proton")
    }

    /// The runtime this Proton must run in, found in Steam's libraries. `None` for a Proton
    /// that asks for none. A runtime that is asked for and not installed is an error: another
    /// runtime is never used in its place (GE-Proton 10 in SLR 4 lacks `libvpx.so.6`: silent voices).
    pub fn runtime(&self, steam: &Steam) -> Result<Option<PathBuf>> {
        let Some(app_id) = self.required_tool else { return Ok(None) };
        match steam.app_dir(app_id) {
            Some(dir) => Ok(Some(dir)),
            None => Err(Error::RuntimeMissing {
                proton: self.dir.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                app_id,
                name: match app_id {
                    1_628_350 => "Steam Linux Runtime 3.0 (sniper)",
                    1_391_110 => "Steam Linux Runtime 2.0 (soldier)",
                    _ => "a Steam Linux Runtime",
                },
            }),
        }
    }
}

/// One Proton build found on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundProton {
    pub name: String,
    pub path: PathBuf,
    /// The runtime app id its `toolmanifest.vdf` asks for.
    pub runtime: Option<u32>,
    /// Whether that runtime is installed.
    pub runtime_installed: bool,
}

/// The Protons of `compatibilitytools.d` (Steam's own and the system's) and the `Proton*`
/// apps of every Steam library, sorted by name.
pub fn find_protons(steam: &Steam) -> Vec<FoundProton> {
    let mut dirs: Vec<PathBuf> = vec![
        steam.root.join("compatibilitytools.d"),
        PathBuf::from("/usr/share/steam/compatibilitytools.d"),
        PathBuf::from("/usr/local/share/steam/compatibilitytools.d"),
    ];
    let mut candidates: Vec<PathBuf> = Vec::new();
    for library in &steam.libraries {
        dirs.push(library.join("steamapps/common"));
    }
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.filter_map(|e| e.ok()) {
            let path = e.path();
            if path.join("proton").is_file() && !candidates.contains(&path) {
                candidates.push(path);
            }
        }
    }
    let mut out: Vec<FoundProton> = candidates
        .into_iter()
        .filter_map(|path| {
            let proton = Proton::at(&path).ok()?;
            Some(FoundProton {
                name: path.file_name()?.to_string_lossy().into_owned(),
                runtime: proton.required_tool,
                runtime_installed: proton.runtime(steam).is_ok(),
                path,
            })
        })
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protons_are_found_in_compatibilitytools_and_the_libraries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Steam");
        let put = |p: PathBuf, text: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        put(root.join("compatibilitytools.d/GE-Proton10-14/proton"), "");
        put(root.join("compatibilitytools.d/GE-Proton10-14/toolmanifest.vdf"), "\"require_tool_appid\" \"1628350\"\n");
        put(root.join("compatibilitytools.d/not-a-proton/readme"), "");
        put(root.join("steamapps/common/Proton 9.0 (Beta)/proton"), "");
        let found = find_protons(&Steam::at(&root));
        let names: Vec<_> = found.iter().map(|p| (p.name.as_str(), p.runtime, p.runtime_installed)).collect();
        assert_eq!(names, [("GE-Proton10-14", Some(1_628_350), false), ("Proton 9.0 (Beta)", None, true)]);
    }



    #[test]
    fn steam_is_ready_with_its_web_helper() {
        let tmp = tempfile::tempdir().unwrap();
        let proc_ = tmp.path();
        let process = |pid: u32, comm: &str| put(&proc_.join(pid.to_string()).join("comm"), &format!("{comm}\n"));
        process(10, "bash");
        assert!(!ready_in(proc_));
        process(11, "steam");
        assert_eq!(processes_named(proc_, "steam"), [11]);
        assert!(!ready_in(proc_), "the client alone is still starting");
        process(12, "steamwebhelper");
        assert!(ready_in(proc_));
    }

    fn put(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_proton_finds_the_runtime_it_asks_for() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Steam");
        let other = tmp.path().join("Library 2");
        put(
            &root.join("steamapps/libraryfolders.vdf"),
            &format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                root.display(),
                other.display()
            ),
        );
        put(
            &other.join("steamapps/appmanifest_1628350.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"1628350\"\n\t\"installdir\"\t\t\"SteamLinuxRuntime_sniper\"\n}\n",
        );
        std::fs::create_dir_all(other.join("steamapps/common/SteamLinuxRuntime_sniper")).unwrap();
        let proton_dir = root.join("compatibilitytools.d/GE-Proton10-14");
        put(&proton_dir.join("proton"), "#!/usr/bin/env python3\n");
        put(
            &proton_dir.join("toolmanifest.vdf"),
            "\"manifest\"\n{\n  \"version\" \"2\"\n  \"commandline\" \"/proton %verb%\"\n  \"require_tool_appid\" \"1628350\"\n}\n",
        );

        let steam = Steam::at(&root);
        assert_eq!(steam.libraries.len(), 2);
        let proton = Proton::at(&proton_dir).unwrap();
        assert_eq!(proton.required_tool, Some(1_628_350));
        assert!(proton.runtime(&steam).unwrap().unwrap().ends_with("Library 2/steamapps/common/SteamLinuxRuntime_sniper"));

        // not installed: an error that names the app, never another runtime
        std::fs::remove_file(other.join("steamapps/appmanifest_1628350.acf")).unwrap();
        let message = proton.runtime(&steam).unwrap_err().to_string();
        assert!(message.contains("steam://install/1628350") && message.contains("sniper"), "{message}");

        assert!(matches!(Proton::at(tmp.path()), Err(Error::NotAProton { .. })));
    }
}
