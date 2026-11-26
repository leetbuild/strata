//! Keeping a live deploy in step with the window's changes: after a mod is switched, moved,
//! renamed or removed, the deployed profile's layers go to the overlay daemon (`refresh`),
//! as the CLI's mod list commands do (`strata-cli/src/cmd_modlist.rs`, `refresh_deploy`).

use serde_json::Value;
use strata_core::{Instance, Profile, profile_mounts, resolve_layers};
use strata_vfs::daemon::{self, Request};

use crate::{Error, Result};

/// What a refresh did, in words for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing is deployed.
    NotDeployed,
    /// Another profile is deployed; its mount was left alone.
    OtherProfile(String),
    /// The profile now needs other mounts (local saves or inis switched): redeploy.
    NeedsRedeploy,
    /// The mounts were refreshed; this many cached entries were replaced.
    Refreshed(u64),
}

fn checked(reply: Value) -> Result<Value> {
    if reply["ok"].as_bool() == Some(true) {
        Ok(reply)
    } else {
        Err(Error::Daemon(reply["error"].as_str().unwrap_or("the overlay daemon refused, without saying why").to_owned()))
    }
}

fn ask(socket: &std::path::Path, request: &Request) -> Result<Value> {
    checked(daemon::request(socket, request).map_err(|e| Error::Daemon(e.to_string()))?)
}

/// The profile shown by a live deploy of this instance, if one runs (the game mount's label).
pub fn deployed_profile(instance: &Instance) -> Result<Option<String>> {
    let socket = daemon::socket_path(&instance.root);
    if !daemon::is_running(&socket) {
        return Ok(None);
    }
    let status = ask(&socket, &Request::Status)?;
    let mounts = status["mounts"].as_array().cloned().unwrap_or_default();
    Ok(mounts.iter().find(|m| m["id"] == "game").map(|m| m["label"].as_str().unwrap_or("").to_owned()))
}

/// Sends `profile`'s layers to the daemon when it is the deployed profile.
pub fn refresh(instance: &Instance, profile: &Profile) -> Result<Outcome> {
    let Some(deployed) = deployed_profile(instance)? else { return Ok(Outcome::NotDeployed) };
    if deployed != profile.name {
        return Ok(Outcome::OtherProfile(deployed));
    }
    let socket = daemon::socket_path(&instance.root);
    let status = ask(&socket, &Request::Status)?;
    let live: Vec<String> =
        status["mounts"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(str::to_owned)).collect();

    // the same mounts as `strata deploy` (strata-cli deploy.rs, mount_specs)
    let mut wanted: Vec<(String, Vec<strata_game::Layer>)> = vec![("game".into(), resolve_layers(instance, profile)?)];
    if let Some(prefix) = &instance.proton.prefix {
        for mount in profile_mounts(instance, profile, &prefix.join("pfx")) {
            wanted.push((mount.id.into(), mount.layers));
        }
    }
    if wanted.iter().map(|(id, _)| id.as_str()).ne(live.iter().map(String::as_str)) {
        return Ok(Outcome::NeedsRedeploy);
    }
    let mut changed = 0;
    for (id, layers) in wanted {
        let reply = ask(&socket, &Request::Refresh { id, layers })?;
        changed += reply["changed"].as_u64().unwrap_or(0) + reply["entries"].as_u64().unwrap_or(0);
    }
    Ok(Outcome::Refreshed(changed))
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::NotDeployed => write!(f, "not deployed"),
            Outcome::OtherProfile(p) => write!(f, "profile \"{p}\" is deployed: its mount was left as it is"),
            Outcome::NeedsRedeploy => write!(f, "the profile now needs other mounts: undeploy and deploy to apply it"),
            Outcome::Refreshed(n) => write!(f, "refreshed the live deploy ({n} cached entries replaced)"),
        }
    }
}
