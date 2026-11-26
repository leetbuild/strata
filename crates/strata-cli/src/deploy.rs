//! `strata deploy`, `undeploy`, `status`, `resolve`, and the hidden `vfsd`.

use std::path::Path;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use strata_core::{Instance, Profile, profile_mounts, resolve_layers};
use strata_vfs::MountSpec;
use strata_vfs::daemon::{self, Request};

use crate::Global;
use crate::commands::{open, print_json};

/// The mounts for running `profile`: the game folder, then the profile's files in the prefix.
pub(crate) fn mount_specs(instance: &Instance, profile: &Profile) -> Result<Vec<MountSpec>> {
    let mut specs = vec![MountSpec {
        id: "game".into(),
        layers: resolve_layers(instance, profile)?,
        mountpoint: instance.game_path.clone(),
        whiteouts: Some(instance.root.join("strata/whiteouts.txt")),
        label: Some(profile.name.clone()),
    }];
    if let Some(prefix) = &instance.proton.prefix {
        for mount in profile_mounts(instance, profile, &prefix.join("pfx")) {
            std::fs::create_dir_all(&mount.mountpoint)
                .with_context(|| format!("cannot create {}", mount.mountpoint.display()))?;
            specs.push(MountSpec {
                id: mount.id.into(),
                layers: mount.layers,
                mountpoint: mount.mountpoint,
                whiteouts: None,
                label: Some(profile.name.clone()),
            });
        }
    }
    Ok(specs)
}

/// Fails with the daemon's own words when it refused.
pub(crate) fn checked(reply: Value) -> Result<Value> {
    if reply["ok"].as_bool() == Some(true) {
        Ok(reply)
    } else {
        bail!("{}", reply["error"].as_str().unwrap_or("the overlay daemon refused, without saying why"))
    }
}

fn start_daemon(instance: &Instance, socket: &Path) -> Result<()> {
    let logs = instance.root.join("logs");
    std::fs::create_dir_all(&logs).with_context(|| format!("cannot create {}", logs.display()))?;
    let log_path = logs.join("strata-vfsd.log");
    let log = std::fs::File::options().create(true).append(true).open(&log_path)?;
    // The same program again, never fork(): the window's process has Qt running.
    let exe = strata_core::dirs::own_exe().context("cannot find Strata's own executable")?;
    let mut child = Command::new(exe)
        .arg("vfsd")
        .arg("--socket")
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        // Its own process group: Ctrl+C in the terminal must not take the mounts down.
        .process_group(0)
        .spawn()
        .context("cannot start the overlay daemon")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !daemon::is_running(socket) {
        if let Some(status) = child.try_wait()? {
            bail!("the overlay daemon stopped at once ({status}). See {}", log_path.display());
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            bail!("the overlay daemon did not answer within 10 seconds. See {}", log_path.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// Mounts the profile. Returns the daemon's status. Fails when something is mounted already.
pub(crate) fn deploy(global: &Global) -> Result<Value> {
    let (instance, profile) = open(global)?;
    let started = Instant::now();
    let status = mount_profile(&instance, &profile)?;
    if global.json {
        print_json(&status)?;
    } else {
        println!("deployed profile \"{}\" in {} ms", profile.name, started.elapsed().as_millis());
        print_mounts(&status);
    }
    Ok(status)
}

/// Mounts `profile` (starting the daemon when needed) and returns the daemon's status.
pub(crate) fn mount_profile(instance: &Instance, profile: &Profile) -> Result<Value> {
    let socket = daemon::socket_path(&instance.root);
    if daemon::is_running(&socket) {
        let status = checked(daemon::request(&socket, &Request::Status)?)?;
        if status["mounts"].as_array().is_some_and(|m| !m.is_empty()) {
            bail!("this instance is already deployed. Run `strata undeploy` first.");
        }
    } else {
        start_daemon(instance, &socket)?;
    }
    // the real game Data folder is hidden once mounted: foreign mods are read from here
    instance.snapshot_game_plugins()?;
    let specs = mount_specs(instance, profile)?;
    let reply = daemon::request(&socket, &Request::Mount { mounts: specs }).context("the overlay daemon went away");
    match reply.and_then(checked) {
        Ok(status) => Ok(status),
        Err(e) => {
            // Nothing is mounted: don't leave an idle daemon behind.
            let _ = daemon::request(&socket, &Request::Unmount { force: true });
            Err(e)
        }
    }
}

fn print_mounts(status: &Value) {
    for mount in status["mounts"].as_array().into_iter().flatten() {
        println!(
            "  {:<10} {}  ({} layers, {} entries)",
            mount["id"].as_str().unwrap_or("?"),
            mount["mountpoint"].as_str().unwrap_or("?"),
            mount["layers"],
            mount["nodes"]
        );
    }
}

pub(crate) fn undeploy(global: &Global, force: bool) -> Result<()> {
    let (instance, _) = open(global)?;
    let socket = daemon::socket_path(&instance.root);
    if !daemon::is_running(&socket) {
        // No daemon: a mount can still be there if it was killed.
        let cleaned = strata_vfs::mount::clean_stale(&instance.game_path)?;
        println!("{}", if cleaned { "removed a mount left by a crashed daemon" } else { "not deployed" });
        return Ok(());
    }
    unmount_daemon(&socket, force)?;
    if !global.json {
        println!("undeployed");
    }
    Ok(())
}

/// Asks the daemon on `socket` to unmount and waits until it has gone, so a following
/// mount starts clean.
pub(crate) fn unmount_daemon(socket: &Path, force: bool) -> Result<()> {
    checked(daemon::request(socket, &Request::Unmount { force })?)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while socket.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

pub(crate) fn status(global: &Global) -> Result<()> {
    let (instance, _) = open(global)?;
    let socket = daemon::socket_path(&instance.root);
    if !daemon::is_running(&socket) {
        if global.json {
            return print_json(&serde_json::json!({ "ok": true, "mounts": [] }));
        }
        println!("not deployed");
        return Ok(());
    }
    let status = checked(daemon::request(&socket, &Request::Status)?)?;
    if global.json {
        return print_json(&status);
    }
    println!("deployed (daemon pid {}, up {} s)", status["pid"], status["uptime_secs"]);
    print_mounts(&status);
    Ok(())
}

pub(crate) fn resolve(global: &Global, path: &str) -> Result<()> {
    let (instance, profile) = open(global)?;
    let socket = daemon::socket_path(&instance.root);
    let path = path.replace('\\', "/");
    let reply = if daemon::is_running(&socket) {
        checked(daemon::request(&socket, &Request::Resolve { path: path.clone(), mount: None })?)?
    } else {
        // Not deployed: answer from a tree built for reading.
        let tree = strata_ui::profile_tree::merged_tree(&instance, &profile)?;
        match tree.resolve(path.trim_matches('/')) {
            Some(id) => serde_json::json!({
                "ok": true, "found": true, "path": tree.path(id),
                "kind": if tree.node(id).is_some_and(|n| n.is_dir()) { "dir" } else { "file" },
                "origin": tree.layer_of(id).map(|l| l.id.clone()), "real": tree.real_path(id),
            }),
            None => serde_json::json!({ "ok": true, "found": false }),
        }
    };
    if global.json {
        return print_json(&reply);
    }
    if reply["found"].as_bool() != Some(true) {
        bail!("{path}: no such file in the merged game folder");
    }
    match reply["kind"].as_str() {
        Some("dir") => println!("{}  (a folder)", reply["path"].as_str().unwrap_or(&path)),
        _ => println!(
            "{}\n  from: {}\n  file: {}",
            reply["path"].as_str().unwrap_or(&path),
            reply["origin"].as_str().unwrap_or("?"),
            reply["real"].as_str().unwrap_or("?")
        ),
    }
    Ok(())
}

pub(crate) fn vfsd(socket: &Path) -> Result<()> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
    daemon::serve(socket, threads).context("the overlay daemon failed")
}
