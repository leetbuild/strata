//! `strata run`: mount, launch through Proton, wait for the prefix to go quiet, unmount.
//!
//! The steps, in order (each a function below): [`find_executable`], [`resolve_target`]
//! (binary, start folder, arguments, app id), [`ensure_steam`], the launch plan, then for a
//! real run [`refuse_busy_prefix`], [`open_run_log`], [`prepare_game_files`], deploy when
//! not deployed, [`launch_and_wait`] ([`wait_until_quiet`]) and [`unmount_after_run`].
//! After the run the new crash logs are labelled (Crash Log Labeler, [`label_new_crash_logs`]).

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use strata_core::{Executable, Instance, Profile, winepath};
use strata_launch::{Launch, Plan, Proton, Runner, SlrRunner, Steam, prefix_processes, split_arguments};
use strata_ui::crashlogs;
use strata_vfs::daemon::{self, Request};

use crate::Global;
use crate::commands::open;
use crate::deploy;

/// `extra`: arguments added after the executable's own (`strata run SSEEdit64 -- -quickautoclean`).
pub(crate) fn run(
    global: &Global,
    title: Option<&str>,
    timeout: Option<u64>,
    dry_run: bool,
    extra: &[String],
) -> Result<()> {
    let (instance, profile) = open(global)?;
    #[allow(unused_mut)]
    let mut executable: Option<Executable> = None;
    let executable = match executable {
        Some(e) => e,
        None => find_executable(&instance, title)?,
    };

    let prefix = instance.proton.prefix.clone().context(
        "this instance names no prefix ([fluorine] prefix_path in ModOrganizer.ini). Give one with --prefix.",
    )?;
    let pfx = prefix.join("pfx");
    let proton_dir = instance
        .proton
        .proton
        .clone()
        .context("this instance names no Proton ([fluorine] proton_path in ModOrganizer.ini).")?;
    let steam = Steam::detect()?;
    let proton = Proton::at(&proton_dir)?;

    let target = resolve_target(&instance, &executable, extra, &pfx)?;
    let expose = [instance.root.clone()];
    let launch = Launch {
        exe: &target.exe,
        args: &target.args,
        cwd: &target.cwd,
        prefix: &prefix,
        proton: &proton,
        steam: &steam,
        app_id: target.app_id,
        game_dir: &instance.game_path,
        expose: &expose,
        steam_drm: instance.proton.steam_drm,
        env: &instance.proton.environment,
    };
    ensure_steam(&instance, dry_run)?;
    let plan = SlrRunner.plan(&launch)?;
    if dry_run {
        print!("{}", plan.describe());
        return Ok(());
    }

    refuse_busy_prefix(&pfx, &prefix)?;
    let (mut log, log_path) = open_run_log(&instance, &profile, &executable, &plan)?;
    prepare_game_files(&instance, &profile, &mut log)?;

    let socket = daemon::socket_path(&instance.root);
    let already_deployed = daemon::is_running(&socket);
    if !already_deployed {
        let quiet = Global { json: false, ..global.clone() };
        deploy::deploy(&quiet)?;
    }
    let crash_logs_before = crash_logs(&instance, &profile);
    #[allow(unused_mut)]
    let mut precached: Option<Result<()>> = None;
    let outcome = match precached {
        Some(r) => r,
        None => launch_and_wait(&plan, &target.exe, &pfx, timeout, &mut log, &executable.title, &log_path).map(|_| ()),
    };
    if !already_deployed {
        unmount_after_run(&socket);
    }
    label_new_crash_logs(&instance, &profile, crash_logs_before);
    outcome
}


/// The crash logs there are now, when the Crash Log Labeler is on for this instance.
fn crash_logs(instance: &Instance, profile: &Profile) -> Option<Vec<PathBuf>> {
    if !instance.builtin_enabled(crashlogs::LABELER) {
        return None;
    }
    let dir = crashlogs::log_dir(instance, profile).ok()??;
    Some(crashlogs::logs(&dir))
}

/// Labels the crash logs that appeared during the run (the plugin's `onFinishedRun`).
fn label_new_crash_logs(instance: &Instance, profile: &Profile, before: Option<Vec<PathBuf>>) {
    let (Some(before), Some(now)) = (before, crash_logs(instance, profile)) else { return };
    let new: Vec<PathBuf> = now.into_iter().filter(|p| !before.contains(p)).collect();
    if new.is_empty() {
        return;
    }
    let labelled = strata_core::Trash::home()
        .map_err(anyhow::Error::from)
        .and_then(|trash| crashlogs::label_logs(instance, &new, &trash).map_err(anyhow::Error::from));
    match labelled {
        Ok((outcomes, note)) => crate::cmd_crashlogs::print_outcomes(&outcomes, note.as_deref()),
        Err(e) => eprintln!("strata: crash logs: {e:#}"),
    }
}

/// The executable titled `title` (any case); the error lists the instance's titles.
fn find_executable(instance: &Instance, title: Option<&str>) -> Result<Executable> {
    let executables = instance.executables();
    let titles = || executables.iter().map(|e| e.title.as_str()).collect::<Vec<_>>().join(", ");
    let Some(title) = title else { bail!("which executable? This instance has: {}", titles()) };
    let Some(executable) = executables.iter().find(|e| e.title.eq_ignore_ascii_case(title)) else {
        bail!("no executable titled \"{title}\". This instance has: {}", titles())
    };
    Ok(executable.clone())
}

/// What runs: the binary and start folder on this machine, the arguments, the Steam app id.
struct Target {
    exe: PathBuf,
    cwd: PathBuf,
    args: Vec<String>,
    app_id: u32,
}

fn resolve_target(instance: &Instance, executable: &Executable, extra: &[String], pfx: &Path) -> Result<Target> {
    // Leet backlog: a binary in a mod's Root/ runs from the game folder, where the overlay
    // shows it (`strata problems --fix executables-in-root` stores that for good).
    let in_game = |stored: &str| match instance.root_path_in_game(stored) {
        Some(game) => {
            eprintln!("\"{stored}\" is in a mod's Root folder: using {game}");
            game
        }
        None => stored.to_owned(),
    };
    let exe = winepath::to_host(&in_game(&executable.binary), Some(pfx))?;
    let cwd = if executable.working_directory.is_empty() {
        exe.parent().map(ToOwned::to_owned).unwrap_or_else(|| instance.game_path.clone())
    } else {
        winepath::to_host(&in_game(&executable.working_directory), Some(pfx))?
    };
    let mut args = split_arguments(&executable.arguments);
    args.extend(extra.iter().cloned());
    let app_id = executable.steam_app_id.parse().unwrap_or_else(|_| instance.game.steam_app_id());
    Ok(Target { exe, cwd, args, app_id })
}

/// A game with SteamStub DRM needs the Steam client: start it, or wait for the one starting
/// (not for a dry run).
fn ensure_steam(instance: &Instance, dry_run: bool) -> Result<()> {
    if instance.proton.steam_drm && !dry_run && !Steam::is_ready() {
        eprintln!("waiting for Steam (starting it if needed)...");
        if Steam::ensure_running(Duration::from_secs(180))? {
            eprintln!("started Steam");
        }
    }
    Ok(())
}

/// Refuses to start while programs of an earlier run still live in the prefix.
fn refuse_busy_prefix(pfx: &Path, prefix: &Path) -> Result<()> {
    let busy = prefix_processes(pfx);
    if !busy.is_empty() {
        let names: Vec<_> = busy.iter().take(5).map(|(pid, name)| format!("{name} ({pid})")).collect();
        bail!("programs are still running in the prefix {}: {}. Close them first.", prefix.display(), names.join(", "));
    }
    Ok(())
}

/// `<instance>/logs/strata-run-<time>.log`, starting with the exact command and environment
/// (`spec/04-proton.md`, rule 8).
fn open_run_log(instance: &Instance, profile: &Profile, executable: &Executable, plan: &Plan) -> Result<(File, PathBuf)> {
    let logs = instance.root.join("logs");
    std::fs::create_dir_all(&logs)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let log_path = logs.join(format!("strata-run-{stamp}.log"));
    let mut log = File::create(&log_path).with_context(|| format!("cannot write {}", log_path.display()))?;
    writeln!(log, "# strata run \"{}\" (profile \"{}\")\n{}", executable.title, profile.name, plan.describe())?;
    Ok((log, log_path))
}

/// Archive invalidation and ENB's `dxvk.conf`, written before the game starts.
fn prepare_game_files(instance: &Instance, profile: &Profile, log: &mut File) -> Result<()> {
    let invalidation = crate::cmd_prefix::prepare_invalidation(instance, profile)?;
    for path in invalidation.ini.iter().chain(&invalidation.dummy) {
        writeln!(log, "# archive invalidation: wrote {}", path.display())?;
        println!("archive invalidation: wrote {}", path.display());
    }
    if let Some(conf) = crate::cmd_prefix::ensure_enb_dxvk_conf(instance, profile)? {
        writeln!(log, "# ENB found: wrote {}", conf.display())?;
        println!("ENB found: wrote {} (shows next to the game's exe)", conf.display());
    }
    Ok(())
}

/// Starts the plan with its output in the log and waits for the run to be over.
fn launch_and_wait(
    plan: &Plan,
    exe: &Path,
    pfx: &Path,
    timeout: Option<u64>,
    log: &mut File,
    title: &str,
    log_path: &Path,
) -> Result<ExitStatus> {
    if !exe.is_file() {
        bail!("{} does not exist in the mounted game folder", exe.display());
    }
    println!("running \"{title}\" (log: {})", log_path.display());
    let started = Instant::now();
    let mut child = plan
        .command()
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log.try_clone()?)
        .spawn()
        .with_context(|| format!("cannot start {}", plan.program.display()))?;
    let deadline = timeout.map(|secs| started + Duration::from_secs(secs));
    let status = wait_until_quiet(&mut child, pfx, deadline, timeout.unwrap_or(0), log)?;
    writeln!(log, "# ended after {} s: {status}", started.elapsed().as_secs())?;
    println!("ended after {} s ({status})", started.elapsed().as_secs());
    Ok(status)
}

/// The first program may only be a loader: the run is over when it has exited and the
/// prefix has had no programs for two seconds. At `deadline` everything in the prefix is
/// killed.
fn wait_until_quiet(child: &mut Child, pfx: &Path, deadline: Option<Instant>, timeout_secs: u64, log: &mut File) -> Result<ExitStatus> {
    let mut status = None;
    let mut quiet_since: Option<Instant> = None;
    loop {
        if status.is_none() {
            status = child.try_wait()?;
        }
        if let Some(done) = status {
            if prefix_processes(pfx).is_empty() {
                let since = *quiet_since.get_or_insert_with(Instant::now);
                if since.elapsed() > Duration::from_secs(2) {
                    return Ok(done);
                }
            } else {
                quiet_since = None;
            }
        }
        if deadline.is_some_and(|d| Instant::now() > d) {
            writeln!(log, "# timeout after {timeout_secs} s: stopping everything in the prefix")?;
            eprintln!("strata: timeout, stopping everything in the prefix");
            let _ = child.kill();
            for (pid, _) in prefix_processes(pfx) {
                // SAFETY: kill only sends a signal; a pid that is gone gives ESRCH.
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            }
            let done = child.wait()?;
            std::thread::sleep(Duration::from_secs(1));
            return Ok(done);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Unmounts what this run mounted, also after a failed launch. Retries while the last
/// handles close, forcing on the last attempt.
fn unmount_after_run(socket: &Path) {
    let mut result = Ok(serde_json::Value::Null);
    for attempt in 0..20 {
        result = daemon::request(socket, &Request::Unmount { force: attempt == 19 });
        match &result {
            Ok(reply) if reply["ok"].as_bool() == Some(true) => break,
            Err(_) => break,
            _ => std::thread::sleep(Duration::from_millis(500)),
        }
    }
    if let Ok(reply) = &result
        && reply["ok"].as_bool() != Some(true)
    {
        eprintln!("strata: {}", reply["error"].as_str().unwrap_or("could not unmount"));
    }
}

