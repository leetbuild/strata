//! The launch command and its environment.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::{Error, Proton, Result, Steam};

/// What to run.
#[derive(Debug, Clone)]
pub struct Launch<'a> {
    /// The Windows program, as a path on this machine (inside the mounted game folder).
    pub exe: &'a Path,
    pub args: &'a [String],
    /// The folder the program starts in.
    pub cwd: &'a Path,
    /// The folder that holds `pfx/` (Steam calls it the compatdata folder).
    pub prefix: &'a Path,
    pub proton: &'a Proton,
    pub steam: &'a Steam,
    /// The Steam app id the program runs as (`SteamAppId`).
    pub app_id: u32,
    /// The game folder (`STEAM_COMPAT_INSTALL_PATH`).
    pub game_dir: &'a Path,
    /// Folders the program must reach inside the runtime's container: the instance, tools.
    pub expose: &'a [PathBuf],
    /// The game needs the Steam client (SteamStub DRM).
    pub steam_drm: bool,
    /// The user's variables (the instance's `strata_environment`), set last: they win over
    /// Strata's own.
    pub env: &'a [(String, String)],
}

/// A command ready to run: nothing in it is decided later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Variables set on top of Strata's own environment.
    pub env: BTreeMap<String, String>,
    /// Variables removed from it.
    pub env_remove: Vec<String>,
    pub cwd: PathBuf,
}

impl Plan {
    /// The plan as text for the log: the environment, then the command, shell-quoted.
    pub fn describe(&self) -> String {
        let quote = |s: &str| {
            if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./=:,+@%".contains(&b)) {
                s.to_owned()
            } else {
                format!("'{}'", s.replace('\'', "'\\''"))
            }
        };
        let mut out = format!("cd {}\n", quote(&self.cwd.to_string_lossy()));
        for name in &self.env_remove {
            out.push_str(&format!("unset {name}\n"));
        }
        for (name, value) in &self.env {
            out.push_str(&format!("export {name}={}\n", quote(value)));
        }
        out.push_str(&quote(&self.program.to_string_lossy()));
        for arg in &self.args {
            out.push(' ');
            out.push_str(&quote(&arg.to_string_lossy()));
        }
        out.push('\n');
        out
    }

    /// The plan as a command.
    pub fn command(&self) -> std::process::Command {
        let mut command = std::process::Command::new(&self.program);
        command.args(&self.args).current_dir(&self.cwd).envs(&self.env);
        for name in &self.env_remove {
            command.env_remove(name);
        }
        command
    }
}

/// A way to run a Windows program. One backend today; the trait is where umu-launcher would
/// go for a game without Steam DRM.
pub trait Runner {
    fn plan(&self, launch: &Launch<'_>) -> Result<Plan>;
}

/// Proton inside the Steam Linux Runtime it is built for, started directly:
/// `<runtime>/_v2-entry-point --verb=waitforexitandrun -- <proton>/proton waitforexitandrun <exe>`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SlrRunner;

impl Runner for SlrRunner {
    fn plan(&self, launch: &Launch<'_>) -> Result<Plan> {
        if launch.steam_drm && !Steam::is_running() {
            return Err(Error::SteamNotRunning);
        }
        let mut args = vec![launch.exe.as_os_str().to_owned()];
        args.extend(launch.args.iter().map(OsString::from));
        let mut expose: Vec<PathBuf> = vec![launch.game_dir.to_owned()];
        expose.extend(launch.expose.iter().cloned());
        let call = ProtonCall {
            proton: launch.proton,
            steam: launch.steam,
            prefix: launch.prefix,
            verb: "waitforexitandrun",
            args,
            app_id: launch.app_id,
            expose,
        };
        let mut plan = call.plan(true)?;
        plan.env.insert("STEAM_COMPAT_INSTALL_PATH".to_owned(), launch.game_dir.to_string_lossy().into_owned());
        if launch.steam_drm {
            // Tells the Steam client library the environment is already set up; without it
            // steamclient applies its own defaults, LC_ALL=C among them, and Wine then reads
            // file names as ASCII (Fluorine's protonlauncher.cpp).
            plan.env.insert("SteamEnv".to_owned(), "1".to_owned());
            // Proton skips its Steam bridge when UMU_ID is set, and SteamStub needs the bridge.
            plan.env_remove.push("UMU_ID".to_owned());
        }
        for (name, value) in launch.env {
            plan.env_remove.retain(|n| n != name);
            plan.env.insert(name.clone(), value.clone());
        }
        plan.cwd = launch.cwd.to_owned();
        Ok(plan)
    }
}

/// One call of the `proton` script in the runtime it is built for:
/// `<runtime>/_v2-entry-point --verb=<verb> -- <proton>/proton <verb> <args...>`, with the
/// environment Steam would give it. The game launch and the prefix setup both use it.
#[derive(Debug, Clone)]
pub struct ProtonCall<'a> {
    pub proton: &'a Proton,
    pub steam: &'a Steam,
    /// The folder that holds (or will hold) `pfx/`.
    pub prefix: &'a Path,
    /// `waitforexitandrun` (through Proton's Steam bridge), `runinprefix` (plain wine).
    pub verb: &'static str,
    pub args: Vec<OsString>,
    pub app_id: u32,
    /// Folders the program must reach inside the container, besides the home folder.
    pub expose: Vec<PathBuf>,
}

impl ProtonCall<'_> {
    /// The plan. `existing_prefix`: the prefix must exist already (everything but its
    /// creation).
    pub fn plan(&self, existing_prefix: bool) -> Result<Plan> {
        let pfx = self.prefix.join("pfx");
        if existing_prefix && !pfx.join("drive_c").is_dir() {
            return Err(Error::NotAPrefix { path: self.prefix.to_owned() });
        }
        let runtime = self.proton.runtime(self.steam)?;
        let verb = self.verb;
        let (program, mut args): (PathBuf, Vec<OsString>) = match &runtime {
            Some(runtime) => (
                runtime.join("_v2-entry-point"),
                vec![format!("--verb={verb}").into(), "--".into(), self.proton.script().into_os_string()],
            ),
            None => (self.proton.script(), Vec::new()),
        };
        args.push(verb.into());
        args.extend(self.args.iter().cloned());

        let text = |path: &Path| path.to_string_lossy().into_owned();
        let app_id = self.app_id.to_string();
        let mut env = BTreeMap::new();
        // The compatdata folder, not pfx: Proton adds pfx itself (protonlauncher.cpp's lesson).
        env.insert("STEAM_COMPAT_DATA_PATH".to_owned(), text(self.prefix));
        env.insert("WINEPREFIX".to_owned(), text(&pfx));
        env.insert("STEAM_COMPAT_CLIENT_INSTALL_PATH".to_owned(), text(&self.steam.root));
        env.insert("STEAM_COMPAT_APP_ID".to_owned(), app_id.clone());
        env.insert("SteamAppId".to_owned(), app_id.clone());
        env.insert("SteamGameId".to_owned(), app_id);
        if let Some(runtime) = &runtime {
            env.insert("STEAM_COMPAT_TOOL_PATHS".to_owned(), format!("{}:{}", text(&self.proton.dir), text(runtime)));
        }
        let mut expose: Vec<String> = vec![text(self.prefix)];
        for p in &self.expose {
            let p = text(p);
            if !expose.contains(&p) {
                expose.push(p);
            }
        }
        env.insert("PRESSURE_VESSEL_FILESYSTEMS_RW".to_owned(), expose.join(":"));
        if !Path::new("/dev/ntsync").exists() {
            // Without the kernel's ntsync device a Proton that prefers it cannot start.
            for (name, value) in [("PROTON_NO_NTSYNC", "1"), ("WINENTSYNC", "0"), ("WINE_DISABLE_FAST_SYNC", "1")] {
                env.insert(name.to_owned(), value.to_owned());
            }
        }
        // PYTHONHOME would break the proton script itself.
        let env_remove = vec!["PYTHONHOME".to_owned()];
        Ok(Plan { program, args, env, env_remove, cwd: self.prefix.to_owned() })
    }
}

/// Splits a Windows-style argument string as MO2 stores it (`-D:"Z:\a b" -x`): spaces
/// separate, double quotes group, and `\"` is a quote character.
pub fn split_arguments(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
                has_token = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    args.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(current);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_split_like_a_windows_command_line() {
        assert_eq!(split_arguments(""), Vec::<String>::new());
        assert_eq!(split_arguments("-a  -b"), ["-a", "-b"]);
        assert_eq!(
            split_arguments(r#"-D:"Z:\home\x y\Data" -IKnowWhatImDoing"#),
            [r"-D:Z:\home\x y\Data", "-IKnowWhatImDoing"]
        );
        assert_eq!(split_arguments(r#"--game="Skyrim Special Edition" """#), ["--game=Skyrim Special Edition", ""]);
        assert_eq!(split_arguments(r#"say \"hi\""#), ["say", "\"hi\""]);
    }

    #[test]
    fn the_plan_is_the_runtime_then_proton_then_the_program() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let make = |path: &str| std::fs::create_dir_all(root.join(path)).unwrap();
        make("Steam/steamapps/common/SteamLinuxRuntime_sniper");
        make("Prefix/pfx/drive_c");
        make("Game");
        std::fs::write(
            root.join("Steam/steamapps/appmanifest_1628350.acf"),
            "\"AppState\"\n{\n\t\"installdir\"\t\t\"SteamLinuxRuntime_sniper\"\n}\n",
        )
        .unwrap();
        make("Proton");
        std::fs::write(root.join("Proton/proton"), "").unwrap();
        std::fs::write(root.join("Proton/toolmanifest.vdf"), "\"require_tool_appid\" \"1628350\"\n").unwrap();

        let steam = Steam::at(&root.join("Steam"));
        let proton = Proton::at(&root.join("Proton")).unwrap();
        let exe = root.join("Game/skse64_loader.exe");
        let args = ["-x".to_owned()];
        let expose = [root.join("Instance")];
        let launch = Launch {
            exe: &exe,
            args: &args,
            cwd: &root.join("Game"),
            prefix: &root.join("Prefix"),
            proton: &proton,
            steam: &steam,
            app_id: 489_830,
            game_dir: &root.join("Game"),
            expose: &expose,
            steam_drm: false,
            env: &[],
        };
        let plan = SlrRunner.plan(&launch).unwrap();
        assert!(plan.program.ends_with("SteamLinuxRuntime_sniper/_v2-entry-point"));
        let args: Vec<String> = plan.args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args[0], "--verb=waitforexitandrun");
        assert_eq!(args[1], "--");
        assert!(args[2].ends_with("Proton/proton"));
        assert_eq!(args[3], "waitforexitandrun");
        assert!(args[4].ends_with("Game/skse64_loader.exe"));
        assert_eq!(args[5], "-x");
        assert!(plan.env["STEAM_COMPAT_DATA_PATH"].ends_with("/Prefix"));
        assert!(plan.env["WINEPREFIX"].ends_with("/Prefix/pfx"));
        assert_eq!(plan.env["SteamAppId"], "489830");
        assert!(plan.env["PRESSURE_VESSEL_FILESYSTEMS_RW"].contains("/Instance"));
        assert!(!plan.env.contains_key("SteamEnv"));
        assert!(plan.describe().contains("export SteamGameId=489830\n"));

        // the user's variables come last and win
        let user = [("PROTON_NO_NTSYNC".to_owned(), "1".to_owned()), ("SteamAppId".to_owned(), "1".to_owned())];
        let with_env = SlrRunner.plan(&Launch { env: &user, ..launch.clone() }).unwrap();
        assert_eq!(with_env.env["PROTON_NO_NTSYNC"], "1");
        assert_eq!(with_env.env["SteamAppId"], "1");
        assert!(with_env.describe().contains("export PROTON_NO_NTSYNC=1\n"));

        // a folder without pfx/ is not a prefix
        let bad = Launch { prefix: root, ..launch };
        assert!(matches!(SlrRunner.plan(&bad), Err(Error::NotAPrefix { .. })));
    }
}
