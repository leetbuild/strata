//! The Nexus Mods API (v1) as MO2 calls it (`NexusInterface::nextRequest`,
//! `NXMAccessManager`): validate, mod and file info, the md5 lookup, download links,
//! endorse / abstain, track / untrack, the "updated" list for update checks.
//!
//! Requests go through a [`Transport`], so tests answer from recorded fixtures and nothing
//! here opens a socket on its own. Every request carries MO2's headers (`Protocol-Version`,
//! `Application-Name`, `Application-Version`, a user agent) and the credentials: an API key
//! (`APIKEY` header) or an OAuth access token (`Authorization: Bearer`). The rate-limit headers
//! (`x-rl-*`) are kept from the last reply, as MO2 shows them.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Error, Result};

/// MO2's API base.
pub const API_URL: &str = "https://api.nexusmods.com/v1";

/// One HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// One HTTP reply.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Response {
    pub status: u16,
    /// Header names lower case.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// How requests are sent.
pub trait Transport {
    fn send(&self, request: &Request) -> Result<Response>;
}

/// The real network, through `ureq`.
pub struct Http {
    agent: ureq::Agent,
}

impl Default for Http {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .build();
        Self { agent: config.into() }
    }
}

impl Transport for Http {
    fn send(&self, request: &Request) -> Result<Response> {
        let net = |e: ureq::Error| Error::Network(e.to_string());
        let mut reply = match (request.method, &request.body) {
            ("GET", _) => {
                let mut r = self.agent.get(&request.url);
                for (k, v) in &request.headers {
                    r = r.header(k, v);
                }
                r.call().map_err(net)?
            }
            (method, body) => {
                let mut r = match method {
                    "POST" => self.agent.post(&request.url),
                    "DELETE" => self.agent.delete(&request.url).force_send_body(),
                    other => return Err(Error::Network(format!("unsupported method {other}"))),
                };
                for (k, v) in &request.headers {
                    r = r.header(k, v);
                }
                r.send(body.clone().unwrap_or_default()).map_err(net)?
            }
        };
        let headers = reply
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_lowercase(), v.to_str().unwrap_or("").to_owned()))
            .collect();
        let body = reply.body_mut().read_to_vec().map_err(net)?;
        Ok(Response { status: reply.status().as_u16(), headers, body })
    }
}

/// The credentials.
#[derive(Clone, PartialEq, Eq)]
pub enum Auth {
    /// A personal API key (MO2's legacy login).
    ApiKey(String),
    /// An OAuth access token (MO2 2.5.3 / Fluorine's SSO). Strata never refreshes it: a
    /// refresh token rotates, and the stored one belongs to whoever logged in.
    Bearer(String),
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // never print a key
        f.write_str(match self {
            Auth::ApiKey(_) => "ApiKey(..)",
            Auth::Bearer(_) => "Bearer(..)",
        })
    }
}

/// The rate limits of the last reply (`x-rl-daily-remaining`...).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateLimits {
    pub daily_limit: Option<u32>,
    pub daily_remaining: Option<u32>,
    pub hourly_limit: Option<u32>,
    pub hourly_remaining: Option<u32>,
}

/// `users/validate`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct User {
    pub user_id: i64,
    pub name: String,
    #[serde(default)]
    pub is_premium: bool,
    #[serde(default)]
    pub is_supporter: bool,
}

/// `games/{game}/mods/{id}` (the fields MO2 uses).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModInfo {
    pub mod_id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub category_id: i64,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub updated_timestamp: i64,
    #[serde(default)]
    pub endorsement: Option<Endorsement>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Endorsement {
    /// `Endorsed`, `Abstained` or `Undecided`.
    pub endorse_status: String,
}

/// One file of a mod (`files`, `files/{id}`).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct FileInfo {
    pub file_id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub category_id: i64,
    #[serde(default)]
    pub category_name: Option<String>,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub size_kb: i64,
    #[serde(default)]
    pub uploaded_timestamp: i64,
    #[serde(default)]
    pub mod_version: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct FileUpdate {
    pub old_file_id: i64,
    pub new_file_id: i64,
}

/// `files`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModFiles {
    pub files: Vec<FileInfo>,
    #[serde(default)]
    pub file_updates: Vec<FileUpdate>,
}

/// One hit of `md5_search`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Md5Hit {
    #[serde(rename = "mod")]
    pub mod_info: ModInfo,
    pub file_details: FileInfo,
}

/// One server of `download_link`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DownloadLink {
    pub name: String,
    pub short_name: String,
    #[serde(rename = "URI")]
    pub uri: String,
}

/// One entry of `mods/updated`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Updated {
    pub mod_id: i64,
    pub latest_file_update: i64,
    pub latest_mod_activity: i64,
}

/// The update-check period (`NexusInterface::UpdatePeriod`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day,
    Week,
    Month,
}

/// An API client.
pub struct Client<'a> {
    transport: &'a dyn Transport,
    auth: Auth,
    base: String,
    user_agent: String,
    limits: std::cell::Cell<RateLimits>,
}

fn number(r: &Response, name: &str) -> Option<u32> {
    r.header(name).and_then(|v| v.trim().parse().ok())
}

impl<'a> Client<'a> {
    pub fn new(transport: &'a dyn Transport, auth: Auth) -> Self {
        let version = env!("CARGO_PKG_VERSION");
        Self {
            transport,
            auth,
            base: API_URL.to_owned(),
            user_agent: format!("Strata/{version} (Linux; MO2 2.5.2 compatible)"),
            limits: std::cell::Cell::new(RateLimits::default()),
        }
    }

    /// Talks to another server than Nexus (`<base>/users/validate`...): a local fake in tests.
    pub fn with_base(mut self, base: &str) -> Self {
        self.base = base.trim_end_matches('/').to_owned();
        self
    }

    /// The rate limits of the last reply.
    pub fn limits(&self) -> RateLimits {
        self.limits.get()
    }

    fn request(&self, method: &'static str, path: &str, body: Option<Value>) -> Result<Value> {
        let mut headers = vec![
            ("User-Agent".to_owned(), self.user_agent.clone()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Protocol-Version".to_owned(), "1.0.0".to_owned()),
            ("Application-Name".to_owned(), "Strata".to_owned()),
            ("Application-Version".to_owned(), env!("CARGO_PKG_VERSION").to_owned()),
        ];
        match &self.auth {
            Auth::ApiKey(key) => headers.push(("APIKEY".to_owned(), key.clone())),
            Auth::Bearer(token) => headers.push(("Authorization".to_owned(), format!("Bearer {token}"))),
        }
        let request = Request { method, url: format!("{}/{path}", self.base), headers, body: body.map(|b| b.to_string()) };
        let reply = self.transport.send(&request)?;
        self.limits.set(RateLimits {
            daily_limit: number(&reply, "x-rl-daily-limit"),
            daily_remaining: number(&reply, "x-rl-daily-remaining"),
            hourly_limit: number(&reply, "x-rl-hourly-limit"),
            hourly_remaining: number(&reply, "x-rl-hourly-remaining"),
        });
        let value: Value = serde_json::from_slice(&reply.body).unwrap_or(Value::Null);
        match reply.status {
            200..=299 => Ok(value),
            status => {
                let message = value["message"].as_str().or(value["error"].as_str()).unwrap_or("").to_owned();
                Err(Error::Api { status, message })
            }
        }
    }

    fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        serde_json::from_value(self.request("GET", path, None)?).map_err(|e| Error::Format(e.to_string()))
    }

    /// `users/validate`: who the credentials belong to.
    pub fn validate(&self) -> Result<User> {
        self.get("users/validate")
    }

    pub fn mod_info(&self, game: &str, mod_id: i64) -> Result<ModInfo> {
        self.get(&format!("games/{game}/mods/{mod_id}"))
    }

    pub fn files(&self, game: &str, mod_id: i64) -> Result<ModFiles> {
        self.get(&format!("games/{game}/mods/{mod_id}/files"))
    }

    pub fn file_info(&self, game: &str, mod_id: i64, file_id: i64) -> Result<FileInfo> {
        self.get(&format!("games/{game}/mods/{mod_id}/files/{file_id}"))
    }

    /// `md5_search`: the mod and file a downloaded archive is (`queryInfoMd5`).
    pub fn md5_search(&self, game: &str, md5_hex: &str) -> Result<Vec<Md5Hit>> {
        self.get(&format!("games/{game}/mods/md5_search/{md5_hex}"))
    }

    /// `download_link`: premium accounts ask directly; others need the `key` and `expires`
    /// of the nxm:// link the website gave.
    pub fn download_links(&self, game: &str, mod_id: i64, file_id: i64, nxm: Option<(&str, i64)>) -> Result<Vec<DownloadLink>> {
        let mut path = format!("games/{game}/mods/{mod_id}/files/{file_id}/download_link");
        if let Some((key, expires)) = nxm {
            path.push_str(&format!("?key={key}&expires={expires}"));
        }
        self.get(&path)
    }

    /// Endorse or abstain (`TYPE_TOGGLEENDORSEMENT`, with the mod's version).
    pub fn endorse(&self, game: &str, mod_id: i64, version: &str, endorse: bool) -> Result<Value> {
        let what = if endorse { "endorse" } else { "abstain" };
        self.request("POST", &format!("games/{game}/mods/{mod_id}/{what}"), Some(json!({ "Version": version })))
    }

    /// Track or untrack (`TYPE_TOGGLETRACKING`: POST to track, DELETE with the same body to
    /// untrack).
    pub fn track(&self, game: &str, mod_id: i64, track: bool) -> Result<Value> {
        let method = if track { "POST" } else { "DELETE" };
        self.request(method, &format!("user/tracked_mods?domain_name={game}"), Some(json!({ "mod_id": mod_id })))
    }

    /// `mods/updated?period=`: the mods changed in the period (`TYPE_CHECKUPDATES`).
    pub fn updated(&self, game: &str, period: Period) -> Result<Vec<Updated>> {
        let p = match period {
            Period::Day => "1d",
            Period::Week => "1w",
            Period::Month => "1m",
        };
        self.get(&format!("games/{game}/mods/updated?period={p}"))
    }
}

/// The update check's choice (MO2 `ModInfo::checkAllForUpdate`): of the mods the instance
/// has `(mod id, last Nexus update as Unix time)`, those the "updated" list shows changed
/// later, to ask for their files. The period is the smallest that covers the oldest check.
pub fn needs_update_query(updated: &[Updated], mods: &[(i64, i64)]) -> Vec<i64> {
    let mut out: Vec<i64> = mods
        .iter()
        .filter(|(id, last)| updated.iter().any(|u| u.mod_id == *id && u.latest_file_update > *last))
        .map(|(id, _)| *id)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The period that covers a check made `age_secs` ago, or `None` when it is older than a
/// month (MO2 then asks each mod's info one by one).
pub fn period_for(age_secs: i64) -> Option<Period> {
    const DAY: i64 = 86_400;
    match age_secs {
        a if a < DAY => Some(Period::Day),
        a if a < 7 * DAY => Some(Period::Week),
        a if a < 28 * DAY => Some(Period::Month),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Answers from `tests/fixtures/<path with / as _>.json` and records the requests.
    struct Fixtures {
        sent: RefCell<Vec<Request>>,
    }

    impl Transport for Fixtures {
        fn send(&self, request: &Request) -> Result<Response> {
            self.sent.borrow_mut().push(request.clone());
            let path = request.url.strip_prefix(API_URL).unwrap_or(&request.url).trim_start_matches('/');
            let path = path.split('?').next().unwrap_or("");
            let file = format!("{}/tests/fixtures/{}.json", env!("CARGO_MANIFEST_DIR"), path.replace('/', "_"));
            match std::fs::read(&file) {
                Ok(body) => Ok(Response {
                    status: 200,
                    headers: vec![("x-rl-daily-remaining".into(), "19999".into()), ("x-rl-hourly-remaining".into(), "499".into())],
                    body,
                }),
                Err(_) => Ok(Response { status: 404, headers: vec![], body: br#"{"message":"No Mod Found"}"#.to_vec() }),
            }
        }
    }

    fn fixtures() -> Fixtures {
        Fixtures { sent: RefCell::new(Vec::new()) }
    }

    #[test]
    fn validate_sends_mo2s_headers_and_reads_limits() {
        let t = fixtures();
        let c = Client::new(&t, Auth::ApiKey("secret".into()));
        let user = c.validate().unwrap();
        assert_eq!(user.name, "TestUser");
        assert!(user.is_premium);
        let sent = &t.sent.borrow()[0];
        assert_eq!(sent.url, "https://api.nexusmods.com/v1/users/validate");
        let h = |n: &str| sent.headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
        assert_eq!(h("APIKEY").as_deref(), Some("secret"));
        assert_eq!(h("Protocol-Version").as_deref(), Some("1.0.0"));
        assert_eq!(c.limits().daily_remaining, Some(19999));
        assert_eq!(format!("{:?}", Auth::ApiKey("secret".into())), "ApiKey(..)", "keys never print");
    }

    #[test]
    fn mod_files_md5_and_links() {
        let t = fixtures();
        let c = Client::new(&t, Auth::Bearer("tok".into()));
        let m = c.mod_info("skyrimspecialedition", 12604).unwrap();
        assert_eq!((m.name.as_str(), m.version.as_str()), ("SkyUI", "5.2SE"));
        assert_eq!(m.endorsement.unwrap().endorse_status, "Endorsed");
        let files = c.files("skyrimspecialedition", 12604).unwrap();
        assert_eq!(files.files[0].file_id, 35407);
        assert_eq!(files.file_updates[0].new_file_id, 35407);
        let hits = c.md5_search("skyrimspecialedition", "0123abcd").unwrap();
        assert_eq!(hits[0].file_details.file_name, "SkyUI_5_2_SE-12604-5-2SE.7z");
        let links = c.download_links("skyrimspecialedition", 12604, 35407, Some(("k", 99))).unwrap();
        assert_eq!(links[0].short_name, "Nexus CDN");
        let sent = t.sent.borrow();
        assert!(sent.iter().any(|r| r.headers.iter().any(|(k, v)| k == "Authorization" && v == "Bearer tok")));
        assert!(sent.last().unwrap().url.ends_with("/download_link?key=k&expires=99"));
    }

    #[test]
    fn endorse_track_and_errors() {
        let t = fixtures();
        let c = Client::new(&t, Auth::ApiKey("k".into()));
        c.endorse("skyrimspecialedition", 12604, "5.2SE", true).unwrap();
        c.track("skyrimspecialedition", 12604, false).unwrap();
        let sent = t.sent.borrow();
        assert_eq!(sent[0].method, "POST");
        assert_eq!(sent[0].body.as_deref(), Some(r#"{"Version":"5.2SE"}"#));
        assert_eq!(sent[1].method, "DELETE");
        assert!(sent[1].url.ends_with("user/tracked_mods?domain_name=skyrimspecialedition"));
        drop(sent);
        match c.mod_info("skyrimspecialedition", 1) {
            Err(Error::Api { status: 404, message }) => assert_eq!(message, "No Mod Found"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn update_check_choice() {
        let t = fixtures();
        let c = Client::new(&t, Auth::ApiKey("k".into()));
        let updated = c.updated("skyrimspecialedition", Period::Week).unwrap();
        assert!(t.sent.borrow()[0].url.ends_with("mods/updated?period=1w"));
        let mods = [(12604, 1_700_000_000), (266, 1_800_000_000), (5, 0)];
        assert_eq!(needs_update_query(&updated, &mods), [12604]);
        assert_eq!(period_for(3600), Some(Period::Day));
        assert_eq!(period_for(3 * 86_400), Some(Period::Week));
        assert_eq!(period_for(40 * 86_400), None);
    }
}
