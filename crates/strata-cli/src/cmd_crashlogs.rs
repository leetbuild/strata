//! `strata crashlogs`: the Crash Log Labeler and Crash Log Viewer (`strata_ui::crashlogs`).

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Subcommand;
use serde_json::json;
use strata_ui::crashlogs;

use crate::Global;
use crate::commands::{open, print_json};

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub(crate) enum CrashlogsAction {
    /// The crash logs, newest first, and their folder (the Crash Log Viewer)
    List,
    /// Label the crash logs' call stacks with the names database (all logs, or the given
    /// files); a changed log's old text goes to the trash
    Label { files: Vec<PathBuf> },
    /// Download the names database (meh321's AddressLibraryDatabase, skyrimae.rename)
    Update,
}

pub(crate) fn crashlogs(global: &Global, action: Option<&CrashlogsAction>) -> Result<()> {
    if let Some(CrashlogsAction::Update) = action {
        let (path, count) = crashlogs::update_database()?;
        if global.json {
            return print_json(&json!({ "database": path, "names": count }));
        }
        println!("{count} names in {}", path.display());
        return Ok(());
    }
    let (instance, profile) = open(global)?;
    if !crashlogs::supported(&instance) {
        bail!("crash logs are labelled for Skyrim Special Edition only (this instance: {})", instance.game.name());
    }
    let Some(dir) = crashlogs::log_dir(&instance, &profile)? else {
        bail!("this instance names no prefix ([fluorine] prefix_path in ModOrganizer.ini), so there is no crash log folder");
    };
    match action {
        None | Some(CrashlogsAction::List) => {
            let logs = crashlogs::logs(&dir);
            if global.json {
                return print_json(&json!({ "folder": dir, "logs": logs }));
            }
            println!("{}", dir.display());
            for log in &logs {
                println!("  {}", log.file_name().unwrap_or_default().to_string_lossy());
            }
            if logs.is_empty() {
                println!("  (no crash-*.log)");
            }
        }
        Some(CrashlogsAction::Label { files }) => {
            let logs = if files.is_empty() { crashlogs::logs(&dir) } else { files.clone() };
            let trash = strata_core::Trash::home()?;
            let (outcomes, note) = crashlogs::label_logs(&instance, &logs, &trash)?;
            if global.json {
                let rows: Vec<_> = outcomes.iter().map(|(p, o)| json!({ "log": p, "outcome": o.to_string() })).collect();
                return print_json(&json!({ "logs": rows, "note": note }));
            }
            print_outcomes(&outcomes, note.as_deref());
        }
        Some(CrashlogsAction::Update) => unreachable!(),
    }
    Ok(())
}

pub(crate) fn print_outcomes(outcomes: &[(PathBuf, crashlogs::Outcome)], note: Option<&str>) {
    if let Some(note) = note {
        eprintln!("crash logs: {note}");
    }
    for (log, outcome) in outcomes {
        println!("{}: {outcome}", log.file_name().unwrap_or_default().to_string_lossy());
    }
}
