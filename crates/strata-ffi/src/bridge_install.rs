//! The install area's `cxx` bridge: the installer dialogs (`shell/src/install/`).
//!
//! The install runs on a worker thread the shell starts; every question comes back to the
//! C++ `InstallHost` as one JSON request (`strata_ui::install`, where the messages are
//! listed), which shows the dialog on the UI thread and answers. Every function catches
//! panics.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::pin::Pin;

use serde_json::{Value, json};
use crate::guarded;

#[cxx::bridge(namespace = "strata")]
mod ffi {
    /// The separators "install under" offers, and the one to preselect ("" for none).
    struct SeparatorChoices {
        separators: Vec<String>,
        last: String,
    }


    unsafe extern "C++" {
        include!("install/installhost_api.h");

        /// The window's side: shows installer dialogs.
        type InstallHost;
        /// Shows dialog `kind` with the JSON `request`; returns the JSON answer.
        fn ask(self: Pin<&mut InstallHost>, kind: &str, request: &str) -> String;
        /// Extraction progress in percent; false when the user cancelled.
        fn progress(self: Pin<&mut InstallHost>, percent: u8) -> bool;

        /// The Downloads tab's side of a download.
        type DownloadHost;
        /// The archive's name, bytes so far and in all (0 when unknown); false pauses.
        fn progress(self: Pin<&mut DownloadHost>, file: &str, done: u64, total: u64) -> bool;

        /// The settings dialog's side of a Nexus login.
        type NexusLoginHost;
        /// A log line of the login; `url` is the page to open in the browser ("" for none).
        fn state(self: Pin<&mut NexusLoginHost>, line: &str, url: &str);
    }

    extern "Rust" {
        /// Installs `archive` into the instance `root` / `profile` with the host's dialogs,
        /// under the separator `under` ("" for none), then refreshes a live deploy. Returns a
        /// JSON report: `{installed: {name, installer, merged, replaced, enabled, priority,
        /// ...}, deploy}`. A cancelled install is an error saying so.
        fn install_archive(root: &str, profile: &str, archive: &str, under: &str, host: Pin<&mut InstallHost>) -> Result<String>;

        /// The separators of the profile for "install under", and the last one chosen.
        fn separator_choices(root: &str, profile: &str) -> SeparatorChoices;

        /// A Downloads tab action on the archive `file` (a name in the downloads folder):
        /// `hide`, `unhide`, `delete` (to the trash), `installed`, `uninstalled`, `query`.
        /// Returns a line for the log ("" when nothing was done).
        fn download_action(root: &str, file: &str, action: &str) -> Result<String>;
        /// "Auto-enable installed mods" (Leet change 11, `[Settings] auto_enable_install`).
        fn auto_enable(root: &str) -> bool;
        fn set_auto_enable(root: &str, on: bool) -> Result<()>;

        /// Downloads the file of an nxm:// link into the instance's downloads folder (key
        /// from the secret service). Returns `{file, done, bytes}`: `done` false when paused.
        fn nexus_download(root: &str, link: &str, host: Pin<&mut DownloadHost>) -> Result<String>;
        /// Resumes the paused download `file` (`<archive>.unfinished` or the archive's name).
        fn resume_download(root: &str, file: &str, host: Pin<&mut DownloadHost>) -> Result<String>;

        /// Whether a Nexus key is in the secret service (no network).
        fn nexus_has_key() -> bool;
        /// The stored key's account as JSON (`strata_ui::nexus::Account`), "" when no key is
        /// stored. Asks Nexus: call it off the UI thread.
        fn nexus_account() -> Result<String>;
        /// Logs in through Nexus SSO (the host opens the browser), checks and stores the key;
        /// returns the account JSON. Blocks until done: call it off the UI thread.
        fn nexus_login_sso(host: Pin<&mut NexusLoginHost>) -> Result<String>;
        /// Stops a running [`nexus_login_sso`] (any thread).
        fn nexus_cancel_login();
        /// Checks `key` with Nexus and stores it; returns the account JSON.
        fn nexus_set_key(key: &str) -> Result<String>;
        /// Forgets the stored key.
        fn nexus_logout() -> Result<()>;
        /// Makes Strata the handler of nxm:// and modl:// links; returns a line for the log.
        fn nexus_associate() -> Result<String>;
        /// Removes the instance's cache folder (`cache_directory`, MO2's "Clear Cache");
        /// returns a line for the log.
        fn clear_web_cache(root: &str) -> Result<String>;

        /// The socket this window listens on for nxm:// links from `strata link`
        /// (`strata_ui::handover`); creates its folder.
        fn handover_socket_path() -> Result<String>;
        /// Whether the window of instance `root` takes `link`: "" when it does, else why not.
        fn handover_refusal(root: &str, link: &str) -> String;
    }
}

/// [`strata_ui::install::Host`] over the C++ host. An answer that is not JSON cancels.
struct CppHost<'a>(Pin<&'a mut ffi::InstallHost>);

impl strata_ui::install::Host for CppHost<'_> {
    fn ask(&mut self, kind: &str, request: &Value) -> Value {
        let answer = self.0.as_mut().ask(kind, &request.to_string());
        serde_json::from_str(&answer).unwrap_or_else(|_| json!({ "action": "cancel" }))
    }

    fn progress(&mut self, percent: u8) -> bool {
        self.0.as_mut().progress(percent)
    }
}

fn install_archive(root: &str, profile: &str, archive: &str, under: &str, host: Pin<&mut ffi::InstallHost>) -> Result<String, String> {
    guarded(|| {
        let session = strata_ui::Session::open(Path::new(root), Some(profile)).map_err(|e| e.to_string())?;
        let mut host = CppHost(host);
        let under = (!under.is_empty()).then_some(under);
        let done = strata_ui::install::install(session.instance(), session.profile(), &mut host, Path::new(archive), under)
            .map_err(|e| e.to_string())?;
        let deploy = strata_cli::refresh_live_deploy(Path::new(root), profile).unwrap_or_else(|e| Some(format!("{e:#}")));
        Ok(json!({ "installed": done, "deploy": deploy }).to_string())
    })
}

fn separator_choices(root: &str, profile: &str) -> ffi::SeparatorChoices {
    catch_unwind(AssertUnwindSafe(|| {
        let session = strata_ui::Session::open(Path::new(root), Some(profile)).ok()?;
        let (separators, last) = strata_ui::install::separator_choices(session.instance(), session.profile());
        Some(ffi::SeparatorChoices { separators, last: last.unwrap_or_default() })
    }))
    .ok()
    .flatten()
    .unwrap_or(ffi::SeparatorChoices { separators: Vec::new(), last: String::new() })
}

fn download_action(root: &str, file: &str, action: &str) -> Result<String, String> {
    use strata_install::downloads as dl;
    guarded(|| {
        let instance = strata_core::Instance::open(Path::new(root)).map_err(|e| e.to_string())?;
        let archive = instance.paths.downloads.join(file);
        let err = |e: strata_install::Error| e.to_string();
        Ok(match action {
            "hide" => dl::set_hidden(&archive, true).map(|()| format!("{file}: hidden")).map_err(err)?,
            "unhide" => dl::set_hidden(&archive, false).map(|()| format!("{file}: shown")).map_err(err)?,
            "installed" => strata_install::meta::mark_download_installed(&archive).map(|()| format!("{file}: marked installed")).map_err(err)?,
            "uninstalled" => dl::mark_uninstalled(&archive).map(|()| format!("{file}: marked uninstalled")).map_err(err)?,
            "delete" => {
                let trash = strata_core::Trash::home().map_err(|e| e.to_string())?;
                dl::delete(&archive, Some(&trash)).map_err(err)?;
                format!("{file}: moved to the trash")
            }
            "query" => {
                let q = dl::query_info_offline(&archive, "SkyrimSE").map_err(err)?;
                match q.mod_id {
                    Some(id) => format!("{file}: mod id {id}, name '{}'", q.mod_name),
                    None => format!("{file}: the file name has no mod id"),
                }
            }
            other => return Err(format!("unknown download action '{other}'")),
        })
    })
}


fn auto_enable(root: &str) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        strata_core::Instance::open(Path::new(root))
            .map(|i| strata_install::settings::InstallSettings::read(&i.ini).auto_enable)
            .unwrap_or(true)
    }))
    .unwrap_or(true)
}

fn set_auto_enable(root: &str, on: bool) -> Result<(), String> {
    guarded(|| strata_install::settings::store_auto_enable(Path::new(root), on).map_err(|e| e.to_string()))
}

fn downloads_dir(root: &str) -> Result<std::path::PathBuf, String> {
    Ok(strata_core::Instance::open(Path::new(root)).map_err(|e| e.to_string())?.paths.downloads)
}

fn ended_json(e: strata_ui::nexus::Ended) -> String {
    json!({ "file": e.file, "done": e.done, "bytes": e.bytes }).to_string()
}

fn nexus_download(root: &str, link: &str, mut host: Pin<&mut ffi::DownloadHost>) -> Result<String, String> {
    guarded(|| {
        let dir = downloads_dir(root)?;
        let ended = strata_ui::nexus::download_link(&dir, link, &mut |f, d, t| host.as_mut().progress(f, d, t))
            .map_err(|e| e.to_string())?;
        Ok(ended_json(ended))
    })
}

fn resume_download(root: &str, file: &str, mut host: Pin<&mut ffi::DownloadHost>) -> Result<String, String> {
    guarded(|| {
        let dir = downloads_dir(root)?;
        let ended =
            strata_ui::nexus::resume(&dir, file, &mut |f, d, t| host.as_mut().progress(f, d, t)).map_err(|e| e.to_string())?;
        Ok(ended_json(ended))
    })
}

/// Set to stop the running SSO login; cleared when one starts.
static LOGIN_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn account_json(a: &strata_ui::nexus::Account) -> String {
    serde_json::to_string(a).unwrap_or_default()
}

fn nexus_has_key() -> bool {
    crate::quiet(strata_ui::nexus::has_key)
}

fn nexus_account() -> Result<String, String> {
    guarded(|| Ok(strata_ui::nexus::account().map_err(|e| e.to_string())?.map(|a| account_json(&a)).unwrap_or_default()))
}

fn nexus_login_sso(mut host: Pin<&mut ffi::NexusLoginHost>) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    guarded(|| {
        LOGIN_CANCEL.store(false, Ordering::Relaxed);
        let a = strata_ui::nexus::login_sso(&mut |line, url| host.as_mut().state(line, url), &LOGIN_CANCEL)
            .map_err(|e| e.to_string())?;
        Ok(account_json(&a))
    })
}

fn nexus_cancel_login() {
    LOGIN_CANCEL.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn nexus_set_key(key: &str) -> Result<String, String> {
    guarded(|| strata_ui::nexus::set_key(key).map(|a| account_json(&a)).map_err(|e| e.to_string()))
}

fn nexus_logout() -> Result<(), String> {
    guarded(|| strata_ui::nexus::logout().map_err(|e| e.to_string()))
}

fn nexus_associate() -> Result<String, String> {
    guarded(|| {
        let file = strata_cli::register_link_handler(true).map_err(|e| format!("{e:#}"))?;
        Ok(format!("Strata now opens nxm:// links ({})", file.display()))
    })
}

fn clear_web_cache(root: &str) -> Result<String, String> {
    guarded(|| {
        let instance = strata_core::Instance::open(Path::new(root)).map_err(|e| e.to_string())?;
        let cache = &instance.paths.cache;
        // never the instance itself or a folder above it (a cache_directory set to "." or "..")
        if instance.root.starts_with(cache) {
            return Err(format!("{} holds the instance; not removed", cache.display()));
        }
        if !cache.exists() {
            return Ok("the cache is already empty".to_owned());
        }
        std::fs::remove_dir_all(cache).map_err(|e| format!("cannot remove {}: {e}", cache.display()))?;
        Ok(format!("cleared {}", cache.display()))
    })
}

fn handover_socket_path() -> Result<String, String> {
    guarded(|| {
        strata_ui::handover::socket_path(std::process::id())
            .map(|p| p.to_string_lossy().into_owned())
            .map_err(|e| format!("cannot make the link socket's folder: {e}"))
    })
}

fn handover_refusal(root: &str, link: &str) -> String {
    crate::quiet(|| match strata_core::Instance::open(Path::new(root)) {
        Ok(instance) => strata_ui::handover::accepts(instance.game.nexus_name(), link).err().unwrap_or_default(),
        Err(e) => e.to_string(),
    })
}
