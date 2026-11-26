//! The Downloads tab's queue: an nxm:// link downloaded into the downloads folder, a paused
//! download resumed (from its `<file>.unfinished.meta`), with progress and pause. The window
//! runs one at a time on a worker (`shell/src/install/downloadqueue.cpp`).
//!
//! The API key comes from the secret service (`strata nexus login`); `STRATA_NEXUS_API`
//! points at another server (tests).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use strata_nexus::api::{Auth, Client, Http};
use strata_nexus::links::{self, Link};
use strata_nexus::queue::{self, Finished, Job, State};

use crate::{Error, Result};

/// How a download ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ended {
    /// The archive's name in the downloads folder.
    pub file: String,
    /// Complete (`<file>` exists) or paused (`<file>.unfinished` stays).
    pub done: bool,
    pub bytes: u64,
    pub path: PathBuf,
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::Invalid(e.to_string())
}

/// What a download reports: `(archive name, bytes so far, bytes in all or 0)`; false pauses
/// it after the current chunk.
pub type Progress<'a> = dyn FnMut(&str, u64, u64) -> bool + 'a;

/// The account the settings dialog's Nexus tab shows (`NexusSettingsTab::updateNexusData`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Account {
    pub id: i64,
    pub name: String,
    /// `Premium`, `Supporter` or `Regular` (`localizedUserAccountType`).
    pub kind: &'static str,
    pub daily_remaining: Option<u32>,
    pub daily_limit: Option<u32>,
    pub hourly_remaining: Option<u32>,
    pub hourly_limit: Option<u32>,
}

fn api_base() -> String {
    std::env::var("STRATA_NEXUS_API").unwrap_or_else(|_| strata_nexus::api::API_URL.to_owned())
}

/// Whether a key is stored (no network).
pub fn has_key() -> bool {
    strata_nexus::keystore::SecretTool::default().lookup().ok().flatten().is_some()
}

/// Who `key` belongs to, with the rate limits of the reply.
pub fn account_for(key: &str) -> Result<Account> {
    let http = Http::default();
    let client = Client::new(&http, Auth::ApiKey(key.to_owned())).with_base(&api_base());
    let user = client.validate().map_err(err)?;
    let l = client.limits();
    Ok(Account {
        id: user.user_id,
        name: user.name,
        kind: if user.is_premium { "Premium" } else if user.is_supporter { "Supporter" } else { "Regular" },
        daily_remaining: l.daily_remaining,
        daily_limit: l.daily_limit,
        hourly_remaining: l.hourly_remaining,
        hourly_limit: l.hourly_limit,
    })
}

/// The stored key's account; `None` when no key is stored.
pub fn account() -> Result<Option<Account>> {
    match strata_nexus::keystore::SecretTool::default().lookup().map_err(err)? {
        Some(key) => account_for(&key).map(Some),
        None => Ok(None),
    }
}

/// Checks `key` with Nexus and stores it in the secret service (MO2's `NexusKeyValidator`,
/// then `setNexusApiKey`). A key Nexus refuses is not stored.
pub fn set_key(key: &str) -> Result<Account> {
    let key = key.trim();
    if key.is_empty() {
        return Err(Error::Invalid("no key given".to_owned()));
    }
    let account = account_for(key).map_err(|e| Error::Invalid(format!("Nexus did not accept the key: {e}")))?;
    strata_nexus::keystore::SecretTool::default().store(key).map_err(err)?;
    Ok(account)
}

/// Logs in through Nexus SSO (`strata_nexus::sso`), then [`set_key`]. `state` hears each step
/// as MO2's log lines (`NexusSSOLogin::stateToString`) and, on the browser step, the page to
/// open; `cancel` stops the wait.
pub fn login_sso(state: &mut dyn FnMut(&str, &str), cancel: &AtomicBool) -> Result<Account> {
    use strata_nexus::sso::{self, State as S};
    let server = std::env::var("STRATA_NEXUS_SSO").unwrap_or_else(|_| sso::SSO_URL.to_owned());
    let key = sso::login(
        &server,
        sso::SSO_PAGE,
        &mut |s| match s {
            S::ConnectingToSso => state("Connecting to Nexus...", ""),
            S::WaitingForToken => state("Waiting for Nexus...", ""),
            S::WaitingForBrowser(url) => {
                state("Opened Nexus in browser.\nSwitch to your browser and accept the request.", url)
            }
            S::Finished => state("Received API key.", ""),
        },
        cancel,
    )
    .map_err(|e| match e {
        strata_nexus::Error::Cancelled => Error::Invalid("Cancelled.".to_owned()),
        e => err(e),
    })?;
    state("Checking API key...", "");
    set_key(&key)
}

/// Forgets the stored key (`clearNexusApiKey`).
pub fn logout() -> Result<()> {
    strata_nexus::keystore::SecretTool::default().clear().map_err(err)
}

/// Runs `job`.
fn run(downloads: &Path, job: &Job, progress: &mut Progress) -> Result<Ended> {
    let pause = AtomicBool::new(false);
    let mut report = |done: u64, total: Option<u64>| {
        if !progress(&job.file_name, done, total.unwrap_or(0)) {
            pause.store(true, Ordering::Relaxed);
        }
    };
    let result = queue::download(&queue::agent(), job, downloads, &mut report, &pause).map_err(err)?;
    Ok(match result {
        Finished::Done(path) => Ended { file: job.file_name.clone(), done: true, bytes: file_len(&path), path },
        Finished::Paused(bytes) => {
            Ended { file: job.file_name.clone(), done: false, bytes, path: downloads.join(&job.file_name) }
        }
    })
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// The archive name and the queue job of an nxm:// link (asks Nexus with the stored key).
pub fn job_for_link(link: &str) -> Result<Job> {
    let key_store = strata_nexus::keystore::SecretTool::default();
    let api_key = key_store
        .lookup()
        .map_err(err)?
        .ok_or_else(|| Error::Invalid("not logged in to Nexus: log in first (strata nexus login)".to_owned()))?;
    let http = Http::default();
    let base = std::env::var("STRATA_NEXUS_API").unwrap_or_else(|_| strata_nexus::api::API_URL.to_owned());
    job_for(&Client::new(&http, Auth::ApiKey(api_key)).with_base(&base), link)
}

/// [`job_for_link`] with a given client: the link's `key` and `expires` go to the
/// download-link request (free accounts need them).
pub fn job_for(client: &Client, link: &str) -> Result<Job> {
    let Link::NexusFile { game, mod_id, file_id, key, expires, .. } = links::parse(link).map_err(err)? else {
        return Err(Error::Invalid("only nxm:// links to a file can be downloaded".to_owned()));
    };
    let mod_id = i64::try_from(mod_id).map_err(err)?;
    let file_id = i64::try_from(file_id).map_err(err)?;
    let p = strata_nexus::nxm::prepare(client, &game, mod_id, file_id, key.as_deref().zip(expires)).map_err(err)?;
    let url = p.urls.into_iter().next().unwrap_or_default();
    Ok(Job { url, file_name: p.file_name, info: p.info, state: State::Pending })
}

/// Downloads the file of an nxm:// link into `downloads`.
pub fn download_link(downloads: &Path, link: &str, progress: &mut Progress) -> Result<Ended> {
    let job = job_for_link(link)?;
    run(downloads, &job, progress)
}

/// Resumes the paused download `file` (with or without its `.unfinished` suffix).
pub fn resume(downloads: &Path, file: &str, progress: &mut Progress) -> Result<Ended> {
    let name = file.strip_suffix(crate::downloads::UNFINISHED).unwrap_or(file);
    let job = Job::from_unfinished(downloads, name).map_err(err)?;
    run(downloads, &job, progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_and_resume_a_queued_file() {
        let server = strata_nexus::fake::serve();
        let data: Vec<u8> = (0..500_000u32).map(|i| (i % 253) as u8).collect();
        server.add(std::collections::HashMap::from([("/cdn/Big File.7z".to_owned(), data.clone())]));
        let dir = tempfile::tempdir().unwrap();
        let job = Job {
            url: format!("{}/cdn/Big File.7z", server.base),
            file_name: "Big File.7z".into(),
            info: strata_nexus::queue::DownloadInfo { name: "Big".into(), mod_id: 9, ..Default::default() },
            state: State::Pending,
        };
        let ended = run(dir.path(), &job, &mut |name, done, _| name == "Big File.7z" && done < 100_000).unwrap();
        assert!(!ended.done && ended.bytes >= 100_000);
        let rows = crate::downloads::rows(dir.path());
        assert_eq!((rows[0].file_name.as_str(), rows[0].status), ("Big File.7z.unfinished", "Paused"));

        let mut seen = Vec::new();
        let ended = resume(dir.path(), "Big File.7z.unfinished", &mut |_, d, t| {
            seen.push((d, t));
            true
        })
        .unwrap();
        assert!(ended.done);
        assert_eq!(std::fs::read(dir.path().join("Big File.7z")).unwrap(), data);
        assert_eq!(seen.last().unwrap().1, 500_000, "the total counts the part from before");
        let rows = crate::downloads::rows(dir.path());
        assert_eq!((rows[0].file_name.as_str(), rows[0].status), ("Big File.7z", "Downloaded"));
    }

    /// The window's path: the nxm:// link's `key` and `expires` reach the download-link
    /// request, and a link without them asks without (a premium account).
    #[test]
    fn link_key_and_expires_reach_the_download_link_request() {
        let server = strata_nexus::fake::serve();
        server.add(strata_nexus::fake::nexus_file_routes(
            &server.base,
            "skyrimspecialedition",
            5,
            77,
            "Cool Mod",
            "Cool.7z",
            "1",
            vec![1, 2, 3],
        ));
        let http = Http::default();
        let client = Client::new(&http, Auth::ApiKey("K".into())).with_base(&server.api());
        let job =
            job_for(&client, "nxm://skyrimspecialedition/mods/5/files/77?key=s3cr3t&expires=1900000123&user_id=9").unwrap();
        assert_eq!((job.file_name.as_str(), job.info.mod_id, job.info.file_id), ("Cool.7z", 5, 77));
        job_for(&client, "nxm://skyrimspecialedition/mods/5/files/77").unwrap();
        let links: Vec<String> =
            server.requests.lock().unwrap().iter().filter(|r| r.contains("download_link")).cloned().collect();
        assert_eq!(
            links,
            [
                "GET /v1/games/skyrimspecialedition/mods/5/files/77/download_link?key=s3cr3t&expires=1900000123 apikey=K",
                "GET /v1/games/skyrimspecialedition/mods/5/files/77/download_link apikey=K",
            ]
        );
    }
}
