//! The program's entry point: parse the command line, run a subcommand, or open the window.
//!
//! See `docs/ARCHITECTURE.md`, "Entry point" and "CLI".

mod cmd_builtins;
mod cmd_crashlogs;
mod cmd_modlist;
pub use cmd_modlist::{ProfileSwitch, register_link_handler, select_profile};
mod cmd_plugins;
mod cmd_prefix;
mod cmd_conflicts;
mod cmd_install;
mod cmd_nexus;
mod cmd_clone;
// install area: the window refreshes a live deploy after an install
pub use cmd_install::refresh_live_deploy;
mod commands;
mod deploy;
mod oracle;
mod run;

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Opens the window and returns the process exit code. Supplied by the C++ shell; `None` in
/// `strata-headless`.
pub type Shell<'a> = &'a dyn Fn(&WindowArgs) -> i32;

/// What the window takes from the command line: `strata -i <instance> -p <profile>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowArgs {
    pub instance: Option<PathBuf>,
    pub profile: Option<String>,
}

#[derive(Debug, Parser)]
#[command(name = "strata", version, about = "Linux-native mod manager (MO2 2.5.2 + Leet Edition)")]
struct Cli {
    #[command(flatten)]
    global: Global,

    #[command(subcommand)]
    command: Option<Command>,
}

/// Options every subcommand takes.
#[derive(Debug, Args, Clone, Default, PartialEq, Eq)]
pub(crate) struct Global {
    /// The instance folder (the one with ModOrganizer.ini). Default: the current folder.
    #[arg(long, short, global = true, value_name = "PATH")]
    pub instance: Option<PathBuf>,

    /// The profile. Default: the one selected in the instance.
    #[arg(long, short, global = true, value_name = "NAME")]
    pub profile: Option<String>,

    /// Print JSON instead of text.
    #[arg(long, global = true)]
    pub json: bool,

    /// Use this prefix (the folder holding pfx/) for this run only. It is never stored.
    /// For tests on a copy of a prefix.
    #[arg(long, global = true, value_name = "PATH")]
    pub prefix: Option<PathBuf>,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum Command {
    /// List the instance's mods (in priority order, lowest first) or profiles.
    List {
        #[arg(value_enum, default_value_t = ListWhat::Mods)]
        what: ListWhat,
    },
    /// Show the instance's game, folders, profile, prefix and Proton.
    Info,
    /// Print the merged game folder: every file the game would see.
    Tree {
        /// Also print the layer each file comes from (tab-separated).
        #[arg(long)]
        origin: bool,
    },
    /// Mount the profile's layers over the game folder (and its files into the prefix).
    Deploy,
    /// Mount, run one of the instance's executables through Proton, wait, unmount.
    Run {
        /// The executable's title in the instance's list, any case: SKSE
        title: Option<String>,
        /// Stop everything in the prefix after this many seconds (for unattended runs).
        #[arg(long, value_name = "SECONDS")]
        timeout: Option<u64>,
        /// Print the command and environment, run nothing.
        #[arg(long)]
        dry_run: bool,
        /// More arguments for the program, after the executable's own: `-- -quickautoclean`.
        #[arg(last = true, value_name = "ARGS")]
        extra: Vec<String>,
    },
    /// Unmount what `deploy` mounted.
    Undeploy {
        /// Detach the mounts even while programs still use them.
        #[arg(long)]
        force: bool,
    },
    /// Show what is mounted for this instance.
    Status,
    /// Show which mod a path of the mounted game folder comes from, and its real file.
    Resolve {
        /// Relative to the game folder, any case: Data/SKSE/Plugins/x.dll
        path: String,
    },
    /// Oracle checks that need Strata's own code (`docs/DESIGN.md`, "Oracle checks").
    #[command(hide = true)]
    Oracle {
        #[arg(value_enum)]
        check: OracleCheck,
    },
    /// The overlay daemon. Started by Strata itself, not by hand.
    #[command(hide = true)]
    Vfsd {
        #[arg(long)]
        socket: PathBuf,
    },

    // Area "modlist": the mod list and profiles (cmd_modlist.rs).
    /// Enable mods (names in any case). Refreshes a live deploy of the profile.
    Enable {
        #[arg(required = true)]
        mods: Vec<String>,
    },
    /// Disable mods. Refreshes a live deploy of the profile.
    Disable {
        #[arg(required = true)]
        mods: Vec<String>,
    },
    /// Move mods to a priority (as a drop on that row in MO2), or by an offset.
    Move {
        #[arg(required = true)]
        mods: Vec<String>,
        /// The priority to drop them at (`strata list` numbers).
        #[arg(long, conflicts_with = "by")]
        to: Option<usize>,
        /// Move each by this many places (negative = lower priority).
        #[arg(long, allow_hyphen_values = true)]
        by: Option<isize>,
    },
    /// Create, rename and edit mods (meta.ini).
    Mod {
        #[command(subcommand)]
        action: cmd_modlist::ModAction,
    },
    /// Separators and Sub-Separator levels.
    Separator {
        #[command(subcommand)]
        action: cmd_modlist::SeparatorAction,
    },
    /// Create, copy, rename and delete profiles; local saves and inis.
    Profile {
        #[command(subcommand)]
        action: cmd_modlist::ProfileAction,
    },
    /// Backups of the mod list and the load order.
    Backup {
        #[command(subcommand)]
        action: cmd_modlist::BackupAction,
    },
    /// Clear overwrite or move its content into a mod.
    Overwrite {
        #[command(subcommand)]
        action: cmd_modlist::OverwriteAction,
    },
    /// The instance's mod categories.
    Category {
        #[command(subcommand)]
        action: cmd_modlist::CategoryAction,
    },
    /// The executables list: add, change, remove, reorder.
    Executable {
        #[command(subcommand)]
        action: cmd_modlist::ExecutableAction,
    },
    /// The browser's handler of nxm:// links: hands the link to the open window, which
    /// downloads it (starts the window on the last instance when none is open).
    Link { url: String },
    /// Write Strata's desktop entry for nxm:// and modl:// links (asked for, never silent).
    RegisterLinks {
        /// Also make Strata the default handler (`xdg-mime default`).
        #[arg(long)]
        make_default: bool,
    },
    // --- plugins area: the plugin list, LOOT, saves, diagnostics (cmd_plugins.rs) ---
    /// Change the plugin list: enable, disable, move, lock, unlock.
    Plugins {
        #[command(subcommand)]
        action: cmd_plugins::PluginsAction,
    },
    /// Sort the plugins with LOOT (libloot), as MO2's Sort button does.
    Sort {
        /// Show the new order, write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Don't download the masterlist; use the copy from before.
        #[arg(long)]
        offline: bool,
    },
    /// List problems, as MO2's Problems dialog: missing masters, form 43 plugins, script
    /// extender plugins that failed to load, files in overwrite (when enabled), ...
    Problems {
        /// Run the fix of this problem (its id: `profile-tweaks`, `missing-masters`).
        #[arg(long, value_name = "ID")]
        fix: Option<String>,
    },
    // ── conflicts area ──
    /// Conflicts: every mod's flag, or one mod's files with who wins them (the mod info
    /// dialog's Conflicts tabs).
    Conflicts {
        /// The mod (its name in the list, any case; `<overwrite>` for overwrite).
        #[arg(value_name = "MOD")]
        name: Option<String>,
    },
    /// List the files of an archive: BSA / BA2, or 7z / zip / rar (through 7-Zip).
    Archive {
        /// The archive file.
        file: PathBuf,
        /// Extract every file into this folder instead of listing.
        #[arg(long, value_name = "FOLDER")]
        extract: Option<PathBuf>,
    },
    /// Pack a mod's loose files into a BSA named after its plugin (MO2's BSA packer).
    PackBsa {
        /// The mod (its folder name).
        #[arg(value_name = "MOD")]
        name: String,
        /// The archive's name without extension: the plugin it loads with. Default: the mod's
        /// only plugin, else the mod's name.
        #[arg(long, value_name = "NAME")]
        plugin: Option<String>,
        /// Store the files uncompressed.
        #[arg(long)]
        no_compress: bool,
        /// Rename the packed loose files to *.mohidden afterwards.
        #[arg(long)]
        hide_loose: bool,
    },
    /// Unpack a mod's BSA/BA2 archives into the mod folder (MO2's BSA extractor).
    ExtractBsa {
        #[arg(value_name = "MOD")]
        name: String,
        /// Delete each archive after it is unpacked.
        #[arg(long)]
        remove: bool,
    },
    /// The preview's pixels of a DDS texture, as a PAM image (for the oracle).
    #[command(hide = true)]
    Preview {
        file: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    // --- install area ---
    /// Install a mod from an archive, answering the installer's questions from options.
    Install(cmd_install::InstallArgs),
    // --- prefix area (M5): prefix setup, Protons, new instances (cmd_prefix.rs) ---
    /// The instance's Wine prefix: `strata prefix setup [--check]`.
    Prefix {
        #[command(subcommand)]
        action: cmd_prefix::PrefixAction,
    },
    /// The Protons on this machine: `strata proton list`.
    Proton {
        #[command(subcommand)]
        action: cmd_prefix::ProtonAction,
    },
    /// Create a new portable instance from scratch (Fluorine's CLI of the same name).
    CreatePortable(cmd_prefix::CreatePortable),
    /// Act on downloaded archives (the Downloads tab's actions).
    Downloads(cmd_install::DownloadsArgs),
    // Area "modlist", round 3: the built-ins and the automation surface (cmd_builtins.rs).
    /// The profile's game inis (MO2's ini editor): list, show, save from standard input.
    Ini {
        #[command(subcommand)]
        action: cmd_builtins::IniAction,
    },
    /// Every mod that provides a file, the winner first (loose files and loaded archives).
    /// Paths relative to Data (`Root/...` for the Root folder); `-` reads paths from
    /// standard input, one per line.
    Origins {
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// The entries of a folder of the merged game folder, with their origin.
    Ls {
        /// Relative to the game folder, any case (`Data/meshes`); empty: the game folder.
        #[arg(default_value = "")]
        path: String,
        /// Only names matching this mask (`*.nif`; `*` and `?`, any case).
        #[arg(long)]
        filter: Option<String>,
        /// Also everything below.
        #[arg(long, short)]
        recursive: bool,
        /// Only folders.
        #[arg(long)]
        dirs: bool,
    },
    /// Write every file of the merged game folder as CSV (path,origin,real); `-` = stdout.
    VfsExport { file: PathBuf },
    /// One mod: its place, plugins, conflicts and files per conflict status.
    Analyze {
        #[arg(value_name = "MOD")]
        name: String,
        /// At most this many files per status.
        #[arg(long, default_value_t = 80)]
        limit: usize,
    },
    /// Tell a live deploy that mods changed on disk (a mod extracted by hand).
    Refresh,
    /// Crash Logger's logs: list them, label their call stacks with function names, update
    /// the names database (Crash Log Labeler / Viewer)
    Crashlogs {
        #[command(subcommand)]
        action: Option<cmd_crashlogs::CrashlogsAction>,
    },
    // --- install area, round 3: Nexus (cmd_nexus.rs) ---
    /// The Nexus account: login (SSO), logout, status, check-updates.
    Nexus {
        #[command(subcommand)]
        action: cmd_nexus::NexusAction,
    },
    /// Download the file of an nxm:// link into the downloads folder (and install it).
    Nxm {
        url: String,
        /// Install the archive once it is downloaded.
        #[arg(long)]
        install: bool,
    },
    /// Make a safe test copy of a portable instance: reflink copy, every path in its
    /// ModOrganizer.ini and its symlinks repointed into the copy (cmd_clone.rs).
    CloneInstance(cmd_clone::CloneArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum OracleCheck {
    /// Check 1: every MO2 file of the instance survives reading and writing byte for byte.
    Roundtrip,
    /// Check 3 (conflicts area): every file's providers and every mod's flags, as lines for
    /// tools/oracle/check-conflicts.sh.
    Conflicts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ListWhat {
    Mods,
    Profiles,
    Executables,
    /// The plugin list, in load order (plugins area).
    Plugins,
    /// The profile's saves, newest first, with missing plugins (plugins area).
    Saves,
}

/// What the command line asks for, separated from doing it so it can be tested.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    /// No subcommand: open the window.
    Window(WindowArgs),
    Run(Global, Box<Command>),
    /// `--help`, `--version` or a usage error: print `text` and exit with `code`.
    Print { text: String, code: i32, to_stderr: bool },
}

fn parse(args: Vec<OsString>) -> Action {
    match Cli::try_parse_from(args) {
        Ok(Cli { command: None, global }) => {
            Action::Window(WindowArgs { instance: global.instance, profile: global.profile })
        }
        Ok(Cli { command: Some(command), global }) => Action::Run(global, Box::new(command)),
        Err(e) => Action::Print { text: e.render().to_string(), code: e.exit_code(), to_stderr: e.use_stderr() },
    }
}

fn is_broken_pipe(error: &anyhow::Error) -> bool {
    use std::io::ErrorKind::BrokenPipe;
    error.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == BrokenPipe)
            // `--json` output: serde_json wraps the write error in its own type.
            || cause.downcast_ref::<serde_json::Error>().is_some_and(|e| e.io_error_kind() == Some(BrokenPipe))
    })
}

/// Runs Strata and returns the process exit code. `args` includes the program name.
pub fn run(args: Vec<OsString>, shell: Option<Shell<'_>>) -> i32 {
    match parse(args) {
        Action::Window(window) => match shell {
            Some(open_window) => open_window(&window),
            None => {
                eprintln!(
                    "strata-headless was built without the window. Give a subcommand (see --help), \
                     or build the full program with tools/build.sh and run build/strata."
                );
                2
            }
        },
        Action::Run(global, command) => match commands::run(&global, &command) {
            Ok(()) => 0,
            // The reader went away (`strata tree | head`): not an error.
            Err(error) if is_broken_pipe(&error) => 0,
            Err(error) => {
                eprintln!("strata: {error:#}");
                1
            }
        },
        Action::Print { text, code, to_stderr } => {
            if to_stderr {
                eprint!("{text}");
            } else {
                print!("{text}");
            }
            code
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_subcommand_opens_the_window() {
        assert_eq!(parse(args(&["strata"])), Action::Window(WindowArgs::default()));
        assert_eq!(
            parse(args(&["strata", "-i", "/x", "-p", "red"])),
            Action::Window(WindowArgs { instance: Some("/x".into()), profile: Some("red".into()) })
        );
    }

    #[test]
    fn subcommands_and_global_options_parse() {
        assert_eq!(
            parse(args(&["strata", "vfsd", "--socket", "/s"])),
            Action::Run(Global::default(), Box::new(Command::Vfsd { socket: "/s".into() }))
        );
        let Action::Run(global, command) = parse(args(&["strata", "tree", "--origin", "-i", "/x", "--json"])) else {
            panic!("not a command")
        };
        assert_eq!(*command, Command::Tree { origin: true });
        assert_eq!(global, Global { instance: Some("/x".into()), json: true, ..Global::default() });
        assert_eq!(
            parse(args(&["strata", "--profile", "red", "list"])),
            Action::Run(Global { profile: Some("red".into()), ..Global::default() }, Box::new(Command::List { what: ListWhat::Mods }))
        );
    }

    #[test]
    fn run_takes_extra_arguments_after_a_double_dash() {
        let Action::Run(_, command) = parse(args(&["strata", "run", "SSEEdit64", "--", "-quickautoclean", "-autoexit"])) else {
            panic!("not a command")
        };
        assert_eq!(
            *command,
            Command::Run {
                title: Some("SSEEdit64".into()),
                timeout: None,
                dry_run: false,
                extra: vec!["-quickautoclean".into(), "-autoexit".into()]
            }
        );
    }

    #[test]
    fn version_prints_and_succeeds() {
        match parse(args(&["strata", "--version"])) {
            Action::Print { text, code, to_stderr } => {
                assert!(text.starts_with("strata "), "{text}");
                assert_eq!(code, 0);
                assert!(!to_stderr);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn unknown_subcommand_is_a_usage_error() {
        match parse(args(&["strata", "no-such-command"])) {
            Action::Print { code, to_stderr, .. } => {
                assert_eq!(code, 2);
                assert!(to_stderr);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn the_window_is_only_opened_without_a_subcommand() {
        let opened = std::cell::Cell::new(0);
        let shell = |window: &WindowArgs| {
            assert_eq!(window.instance.as_deref(), Some(std::path::Path::new("/i")));
            opened.set(opened.get() + 1);
            7
        };
        assert_eq!(run(args(&["strata", "--version"]), Some(&shell)), 0);
        assert_eq!(opened.get(), 0);
        assert_eq!(run(args(&["strata", "-i", "/i"]), Some(&shell)), 7);
        assert_eq!(opened.get(), 1);
    }

    #[test]
    fn headless_without_a_subcommand_fails_clearly() {
        assert_eq!(run(args(&["strata"]), None), 2);
    }
}
