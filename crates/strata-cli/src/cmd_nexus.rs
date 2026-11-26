//! `strata nexus login | logout | status | check-updates` and `strata nxm <url>`: the Nexus
//! account (the key lives in the secret service), nxm:// downloads, update checks.
//!
//! `STRATA_NEXUS_API` and `STRATA_NEXUS_SSO` point the client at another server (the tests'
//! local fake); `STRATA_SECRET_TOOL` replaces `secret-tool` (tests again).

use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::json;
use strata_nexus::api::{Auth, Client, Http};
use strata_nexus::keystore::SecretTool;
use strata_nexus::links::{self, Link};

use crate::Global;
use crate::commands::{open, print_json};

#[derive(Debug, Clone, PartialEq, Eq, clap::Subcommand)]
pub(crate) enum NexusAction {
    /// Log in through the website (Nexus SSO: the browser opens, you allow Strata) and keep
    /// the API key in the secret service. --key-stdin stores a personal API key instead.
    Login {
        #[arg(long)]
        key_stdin: bool,
    },
    /// Forget the stored API key.
    Logout,
    /// Who the stored key belongs to, and the API's rate limits (the key is never shown).
    Status,
    /// Ask Nexus which of the instance's mods have newer versions; writes `newestVersion`.
    CheckUpdates,
}

fn api_base() -> String {
    std::env::var("STRATA_NEXUS_API").unwrap_or_else(|_| strata_nexus::api::API_URL.to_owned())
}

fn stored_key() -> Result<String> {
    SecretTool::default()
        .lookup()?
        .ok_or_else(|| anyhow!("not logged in to Nexus: run `strata nexus login` first"))
}

fn open_browser(url: &str) {
    eprintln!("open this page to allow Strata (it should open by itself): {url}");
    let _ = std::process::Command::new("xdg-open")
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub(crate) fn nexus(global: &Global, action: &NexusAction) -> Result<()> {
    let http = Http::default();
    match action {
        NexusAction::Login { key_stdin } => {
            let key = if *key_stdin {
                let mut text = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
                text.trim().to_owned()
            } else {
                let server = std::env::var("STRATA_NEXUS_SSO").unwrap_or_else(|_| strata_nexus::sso::SSO_URL.to_owned());
                let mut on_state = |s: &strata_nexus::sso::State| {
                    if let strata_nexus::sso::State::WaitingForBrowser(url) = s {
                        open_browser(url);
                    }
                };
                strata_nexus::sso::login(&server, strata_nexus::sso::SSO_PAGE, &mut on_state, &AtomicBool::new(false))?
            };
            if key.is_empty() {
                bail!("no key given");
            }
            let user = Client::new(&http, Auth::ApiKey(key.clone()))
                .with_base(&api_base())
                .validate()
                .context("Nexus did not accept the key; nothing was stored")?;
            SecretTool::default().store(&key)?;
            if global.json {
                return print_json(&json!({ "user": user.name, "premium": user.is_premium }));
            }
            println!("logged in as {}{}; the key is in the secret service", user.name, if user.is_premium { " (premium)" } else { "" });
        }
        NexusAction::Logout => {
            SecretTool::default().clear()?;
            println!("the Nexus key was removed from the secret service");
        }
        NexusAction::Status => {
            let client = Client::new(&http, Auth::ApiKey(stored_key()?)).with_base(&api_base());
            let user = client.validate()?;
            let l = client.limits();
            if global.json {
                return print_json(&json!({
                    "user": user.name, "premium": user.is_premium,
                    "daily_remaining": l.daily_remaining, "hourly_remaining": l.hourly_remaining,
                }));
            }
            println!("logged in as {}{}", user.name, if user.is_premium { " (premium)" } else { "" });
            if let (Some(d), Some(h)) = (l.daily_remaining, l.hourly_remaining) {
                println!("requests left: {d} today, {h} this hour");
            }
        }
        NexusAction::CheckUpdates => check_updates(global, &http)?,
    }
    Ok(())
}

/// `strata nxm <url>`: the link's file is downloaded into the instance's downloads folder
/// (with MO2's `.meta`), then installed with `--install`.
pub(crate) fn nxm(global: &Global, url: &str, install: bool) -> Result<()> {
    let (instance, profile) = open(global)?;
    let Link::NexusFile { game, mod_id, file_id, key, expires, .. } = links::parse(url)? else {
        bail!("only nxm:// links to a file can be downloaded (collections and other sites are not supported yet)");
    };
    let http = Http::default();
    let client = Client::new(&http, Auth::ApiKey(stored_key()?)).with_base(&api_base());
    let nxm_key = key.as_deref().zip(expires);
    let (mod_id, file_id) = (i64::try_from(mod_id)?, i64::try_from(file_id)?);
    let p = strata_nexus::nxm::prepare(&client, &game, mod_id, file_id, nxm_key)?;
    let mut queue = strata_nexus::queue::Queue::new(instance.paths.downloads.clone());
    let url = p.urls.first().cloned().unwrap_or_default();
    queue.push(&url, &p.file_name, p.info.clone());
    let mut last = 0;
    let (_, result) = queue
        .run_next(
            &mut |_, done, total| {
                if let Some(t) = total.filter(|t| *t > 0) {
                    let pct = done * 100 / t;
                    if pct >= last + 10 {
                        last = pct;
                        eprintln!("{}: {pct}%", p.file_name);
                    }
                }
            },
            &AtomicBool::new(false),
        )
        .ok_or_else(|| anyhow!("nothing to download"))?;
    let archive = match result? {
        strata_nexus::queue::Finished::Done(path) => path,
        strata_nexus::queue::Finished::Paused(n) => bail!("the download stopped after {n} bytes; run the link again to resume"),
    };
    let mut report = json!({ "downloaded": archive, "mod_name": p.info.mod_name, "version": p.info.version });
    if !global.json {
        println!("downloaded {}", archive.display());
    }
    if install {
        let args = crate::cmd_install::InstallArgs { archive: Some(archive.clone()), ..Default::default() };
        let done = crate::cmd_install::install_downloaded(&instance, &profile, &args)?;
        if !global.json {
            println!("installed as '{}' ({:?} installer)", done.outcome.name, done.outcome.installer);
        }
        report["installed"] = serde_json::to_value(&done)?;
        if let Some(line) = crate::cmd_modlist::refresh_deploy(global, &profile.name)? {
            println!("{line}");
        }
    }
    if global.json {
        return print_json(&report);
    }
    Ok(())
}

/// `ModInfo::checkAllForUpdate` + `nxmDescriptionAvailable`, for the instance's Nexus mods
/// of the managed game: the "updated" list of the period that covers the oldest check picks
/// the mods to ask about (all of them when a check is older than a month); each answer sets
/// `newestVersion`, `lastNexusQuery` and `nexusLastModified`.
fn check_updates(global: &Global, http: &Http) -> Result<()> {
    let (instance, _) = open(global)?;
    let client = Client::new(http, Auth::ApiKey(stored_key()?)).with_base(&api_base());
    let game = instance.game.nexus_name();
    // the gameName MO2 writes into meta.ini (`gameShortName`)
    let short = match instance.game.name() {
        "Skyrim Special Edition" => "SkyrimSE",
        other => other,
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64;
    let mut mods = Vec::new();
    for entry in std::fs::read_dir(&instance.paths.mods)?.flatten() {
        let dir = entry.path();
        let Ok(meta) = strata_core::ModMeta::open(&dir) else { continue };
        if meta.mod_id() <= 0 || !meta.game_name().eq_ignore_ascii_case(short) || meta.repository() != "Nexus" {
            continue;
        }
        let last = iso_to_unix(&meta.document().string("", "lastNexusUpdate").unwrap_or_default()).unwrap_or(0);
        mods.push((dir, meta.mod_id(), last));
    }
    let oldest = mods.iter().map(|m| m.2).min().unwrap_or(now);
    let ask: Vec<i64> = match strata_nexus::api::period_for(now - oldest) {
        Some(period) => {
            let updated = client.updated(game, period)?;
            let list: Vec<(i64, i64)> = mods.iter().map(|m| (m.1, m.2)).collect();
            strata_nexus::api::needs_update_query(&updated, &list)
        }
        None => mods.iter().map(|m| m.1).collect(),
    };
    let mut changed = Vec::new();
    let stamp = unix_to_iso(now);
    for id in ask.iter().copied().collect::<std::collections::BTreeSet<_>>() {
        let info = client.mod_info(game, id)?;
        for (dir, _, _) in mods.iter().filter(|m| m.1 == id) {
            let mut meta = strata_core::ModMeta::open(dir)?;
            let old = meta.newest_version();
            meta.set_newest_version(&info.version);
            meta.set_text("lastNexusQuery", &stamp);
            meta.set_text("lastNexusUpdate", &stamp);
            meta.set_text("nexusLastModified", &unix_to_iso(info.updated_timestamp));
            meta.save()?;
            let name = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if old != info.version {
                changed.push(json!({ "mod": name, "newest": info.version, "version": meta.version() }));
            }
        }
    }
    if global.json {
        return print_json(&json!({ "asked": ask.len(), "changed": changed }));
    }
    println!("asked Nexus about {} of {} mods", ask.len(), mods.len());
    for c in &changed {
        println!("{}: newest {} (installed {})", c["mod"].as_str().unwrap_or(""), c["newest"].as_str().unwrap_or(""), c["version"].as_str().unwrap_or(""));
    }
    Ok(())
}

/// `QDateTime::toString(Qt::ISODate)` of a UTC Unix time: `2026-11-25T08:00:00Z`.
fn unix_to_iso(t: i64) -> String {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    // civil from days (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// The Unix time of `yyyy-MM-ddThh:mm:ss[Z]`, `None` for anything else (an empty key).
fn iso_to_unix(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once('T')?;
    let d: Vec<i64> = date.split('-').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let t: Vec<i64> = time.split(':').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let [y, m, day] = d[..] else { return None };
    let [hh, mm, ss] = t[..] else { return None };
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_times_both_ways() {
        assert_eq!(unix_to_iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_to_iso(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso_to_unix("2023-11-14T22:13:20Z"), Some(1_700_000_000));
        assert_eq!(iso_to_unix("2024-02-29T00:00:00"), Some(1_709_164_800));
        assert_eq!(iso_to_unix(""), None);
    }
}
