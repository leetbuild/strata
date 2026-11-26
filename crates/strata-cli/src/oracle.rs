//! Oracle check 1 (`docs/DESIGN.md`, "Oracle checks"): the format round trip.
//!
//! For every MO2 file of an instance:
//! - ini files (`ModOrganizer.ini`, `meta.ini`, `*.meta`, `settings.ini`): parsing and writing
//!   gives the same bytes, and every key and value, decoded and encoded again, is what the
//!   file holds. The second part shows that what Strata writes looks like what Qt writes.
//! - list files (`modlist.txt`, `plugins.txt`, `loadorder.txt`, `lockedorder.txt`): reading
//!   and writing gives the same bytes, for files that carry MO2's header. A file without it
//!   was written by something else (the game, LOOT); Strata only rewrites a list when its
//!   content changes, so such a file is counted, not failed.
//!
//! Reads only.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use strata_core::{Instance, MODLIST_HEADER, ModListFile, PluginListFile};
use strata_ini::{Document, codec};

use crate::Global;
use crate::commands::print_json;

#[derive(Default, Serialize)]
struct Report {
    ini_files: u64,
    ini_entries: u64,
    list_files: u64,
    /// List files without MO2's header: not written by MO2, not compared.
    foreign_lists: u64,
    /// Values whose bytes differ from what Qt would write for the same value.
    /// Harmless (Strata never rewrites an unchanged value) but each needs a reason.
    noncanonical_values: Vec<String>,
    /// Real failures.
    failures: Vec<String>,
}

fn files_in(dir: &Path, wanted: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(&wanted))
        .collect();
    files.sort();
    files
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs
}

fn check_ini(path: &Path, report: &mut Report) {
    let Ok(data) = std::fs::read(path) else {
        report.failures.push(format!("{}: cannot read", path.display()));
        return;
    };
    report.ini_files += 1;
    let doc = Document::parse(&data);
    if doc.to_bytes() != data {
        report.failures.push(format!("{}: parse + write changed the bytes", path.display()));
    }
    for entry in doc.entries() {
        report.ini_entries += 1;
        if codec::escape_key(entry.key) != entry.raw_key {
            report.failures.push(format!("{}: key {:?} does not encode back", path.display(), entry.key));
        }
        if codec::encode_value(&entry.value()) != entry.raw_value {
            let shown = String::from_utf8_lossy(&entry.raw_value[..entry.raw_value.len().min(120)]).into_owned();
            report.noncanonical_values.push(format!("{}: {}={shown}", path.display(), entry.key));
        }
    }
}

fn check_list(path: &Path, report: &mut Report) {
    let Ok(data) = std::fs::read(path) else {
        report.failures.push(format!("{}: cannot read", path.display()));
        return;
    };
    report.list_files += 1;
    if !data.starts_with(MODLIST_HEADER.as_bytes()) {
        report.foreign_lists += 1;
        return;
    }
    let is_modlist = path.file_name().is_some_and(|n| n == "modlist.txt");
    let written = if is_modlist { ModListFile::parse(&data).to_bytes() } else { PluginListFile::parse(&data).to_bytes() };
    if written != data {
        report.failures.push(format!("{}: read + write changed the bytes", path.display()));
    }
}

pub(crate) fn roundtrip(global: &Global) -> Result<()> {
    let root = global.instance.clone().context("give the instance with --instance")?;
    let instance = Instance::open(&root)?;
    let mut report = Report::default();

    check_ini(&root.join("ModOrganizer.ini"), &mut report);
    for mod_dir in subdirs(&instance.paths.mods) {
        let meta = mod_dir.join("meta.ini");
        if meta.is_file() {
            check_ini(&meta, &mut report);
        }
    }
    for meta in files_in(&instance.paths.downloads, |name| name.ends_with(".meta")) {
        check_ini(&meta, &mut report);
    }
    for profile in subdirs(&instance.paths.profiles) {
        let settings = profile.join("settings.ini");
        if settings.is_file() {
            check_ini(&settings, &mut report);
        }
        for list in files_in(&profile, |n| matches!(n, "modlist.txt" | "plugins.txt" | "loadorder.txt" | "lockedorder.txt")) {
            check_list(&list, &mut report);
        }
    }

    if global.json {
        print_json(&report)?;
    } else {
        println!(
            "ini files {} (entries {}), list files {} ({} not written by MO2), values not as Qt writes them {}, failures {}",
            report.ini_files,
            report.ini_entries,
            report.list_files,
            report.foreign_lists,
            report.noncanonical_values.len(),
            report.failures.len()
        );
        for line in report.noncanonical_values.iter().take(20) {
            println!("  not as Qt writes it: {line}");
        }
        for line in report.failures.iter().take(50) {
            println!("  FAILED {line}");
        }
    }
    if !report.failures.is_empty() {
        bail!("check 1 (format round trip) failed for {}", root.display());
    }
    Ok(())
}
