//! The bridge of the Crash Log Viewer / Labeler built in (`strata_ui::crashlogs`). Each
//! function opens the instance in `root` itself and catches panics.

use std::path::Path;

use strata_core::{Instance, Profile};
use strata_ui::crashlogs;

use crate::guarded;

#[cxx::bridge(namespace = "strata::crashlogs")]
mod ffi {
    struct CrashLogFolder {
        /// The folder of the crash logs; empty: none (another game, or no prefix).
        folder: String,
        /// The logs (full paths), newest first.
        logs: Vec<String>,
    }

    extern "Rust" {
        /// The crash log folder and its logs for `profile` ("" = the selected one).
        fn crash_log_folder(root: &str, profile: &str) -> Result<CrashLogFolder>;
        /// Labels every log when the Crash Log Labeler is on (as the plugin does when the
        /// window opens); returns what to show in the status line ("" = nothing done).
        fn crash_logs_label(root: &str, profile: &str) -> Result<String>;
        /// Downloads the names database; returns "<n> names".
        fn crash_logs_update_database() -> Result<String>;
        /// Moves one log to the trash (the viewer's Delete).
        fn crash_log_trash(path: &str) -> Result<()>;

    }
}

fn text<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn open(root: &str, profile: &str) -> Result<(Instance, Profile), String> {
    let instance = Instance::open(Path::new(root)).map_err(text)?;
    let profile = instance.profile((!profile.is_empty()).then_some(profile)).map_err(text)?;
    Ok((instance, profile))
}

fn crash_log_folder(root: &str, profile: &str) -> Result<ffi::CrashLogFolder, String> {
    guarded(|| {
        let (instance, profile) = open(root, profile)?;
        let Some(dir) = crashlogs::log_dir(&instance, &profile).map_err(text)? else {
            return Ok(ffi::CrashLogFolder { folder: String::new(), logs: Vec::new() });
        };
        let logs = crashlogs::logs(&dir).iter().map(|p| p.to_string_lossy().into_owned()).collect();
        Ok(ffi::CrashLogFolder { folder: dir.to_string_lossy().into_owned(), logs })
    })
}

fn crash_logs_label(root: &str, profile: &str) -> Result<String, String> {
    guarded(|| {
        let (instance, profile) = open(root, profile)?;
        if !instance.builtin_enabled(crashlogs::LABELER) {
            return Ok(String::new());
        }
        let Some(dir) = crashlogs::log_dir(&instance, &profile).map_err(text)? else { return Ok(String::new()) };
        let trash = strata_core::Trash::home().map_err(text)?;
        let (outcomes, note) = crashlogs::label_logs(&instance, &crashlogs::logs(&dir), &trash).map_err(text)?;
        let labelled = outcomes.iter().filter(|(_, o)| matches!(o, crashlogs::Outcome::Labelled(_))).count();
        let other = outcomes.iter().find_map(|(_, o)| match o {
            crashlogs::Outcome::OtherVersion(_) => Some(o.to_string()),
            _ => None,
        });
        Ok(match (labelled, note, other) {
            (0, Some(note), _) => note,
            (0, None, Some(other)) => other,
            (0, None, None) => String::new(),
            (n, _, _) => format!("labelled {n} crash log(s)"),
        })
    })
}

fn crash_logs_update_database() -> Result<String, String> {
    guarded(|| {
        let (_, count) = crashlogs::update_database().map_err(text)?;
        Ok(format!("{count} names"))
    })
}

fn crash_log_trash(path: &str) -> Result<(), String> {
    guarded(|| {
        strata_core::Trash::home().and_then(|t| t.put(Path::new(path))).map_err(text)?;
        Ok(())
    })
}

