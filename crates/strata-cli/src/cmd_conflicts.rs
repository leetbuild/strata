//! `strata conflicts [mod]`: the conflict flags of every mod, or one mod's files with their
//! providers, as MO2's mod list and mod info dialog (Conflicts tabs, general and advanced) show
//! them. The rules are in `strata_vfs::conflicts`.

use std::io::Write;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use strata_archive::GameArchive;
use strata_vfs::conflicts::{Area, ConflictIndex, ConflictState, FileConflict, Flags, Provider};

use crate::Global;
use crate::commands::{open, print_json};

fn state_name(state: ConflictState) -> &'static str {
    match state {
        ConflictState::None => "none",
        ConflictState::Overwrite => "overwrite",
        ConflictState::Overwritten => "overwritten",
        ConflictState::Mixed => "mixed",
        ConflictState::Redundant => "redundant",
    }
}

#[derive(Serialize)]
struct ModFlags<'a> {
    name: &'a str,
    state: &'static str,
    overwrites: Vec<String>,
    overwritten_by: Vec<String>,
    archive_state: &'static str,
    archive_overwrites: Vec<String>,
    archive_overwritten_by: Vec<String>,
    archive_loose_state: &'static str,
    archive_loose_overwrites: Vec<String>,
    archive_loose_overwritten_by: Vec<String>,
    hidden_files: bool,
}

impl<'a> ModFlags<'a> {
    fn new(name: &'a str, f: Flags) -> Self {
        Self {
            name,
            state: state_name(f.state),
            overwrites: f.overwrites,
            overwritten_by: f.overwritten_by,
            archive_state: state_name(f.archive_state),
            archive_overwrites: f.archive_overwrites,
            archive_overwritten_by: f.archive_overwritten_by,
            archive_loose_state: state_name(f.archive_loose_state),
            archive_loose_overwrites: f.archive_loose_overwrites,
            archive_loose_overwritten_by: f.archive_loose_overwritten_by,
            hidden_files: f.hidden_files,
        }
    }
}

#[derive(Serialize)]
struct ProviderJson<'a> {
    name: &'a str,
    /// The archive the file is in; absent for a loose file.
    #[serde(skip_serializing_if = "Option::is_none")]
    archive: Option<&'a str>,
}

#[derive(Serialize)]
struct FileJson<'a> {
    /// As the Conflicts tabs show it: `/meshes/x.nif`, `/Root/x.dll`.
    path: String,
    area: &'static str,
    /// The general tab's list: `overwrites`, `overwritten` or `none`.
    status: &'static str,
    /// Every provider, lowest first; the last one wins (the advanced tab).
    origins: Vec<ProviderJson<'a>>,
}

/// `Mod` or `Mod (archive.bsa)`.
fn provider_text(p: &Provider<'_>) -> String {
    match p.archive {
        Some(a) => format!("{} ({a})", p.layer),
        None => p.layer.to_owned(),
    }
}

/// The general tab's list for a file of `name` (`GeneralConflictsTab::addFiles`).
fn status(file: &FileConflict<'_>, name: &str) -> &'static str {
    if file.winner() != name {
        "overwritten"
    } else if file.origins.len() > 1 {
        "overwrites"
    } else {
        "none"
    }
}

fn build_index(global: &Global) -> Result<(ConflictIndex, String)> {
    let (instance, profile) = open(global)?;
    Ok((strata_ui::conflicts::build_index(&instance, &profile)?, profile.name))
}

pub(crate) fn conflicts(global: &Global, name: Option<&str>) -> Result<()> {
    let (index, profile) = build_index(global)?;
    match name {
        None => all_mods(global, &index, &profile),
        Some(name) => one_mod(global, &index, name),
    }
}

fn all_mods(global: &Global, index: &ConflictIndex, profile: &str) -> Result<()> {
    let mods: Vec<ModFlags<'_>> =
        index.layer_ids().filter_map(|id| Some(ModFlags::new(id, index.flags(id)?))).collect();
    if global.json {
        return print_json(&serde_json::json!({ "profile": profile, "files": index.file_count(), "mods": mods }));
    }
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for m in &mods {
        let mut extra = String::new();
        if m.archive_state != "none" {
            extra += &format!("  archives: {}", m.archive_state);
        }
        if m.archive_loose_state != "none" {
            extra += &format!("  loose/archives: {}", m.archive_loose_state);
        }
        if m.hidden_files {
            extra += "  (hidden files)";
        }
        writeln!(
            out,
            "{:<12} {:>4} over {:>4} under  {}{extra}",
            m.state,
            m.overwrites.len(),
            m.overwritten_by.len(),
            m.name
        )?;
    }
    out.flush()?;
    Ok(())
}

fn one_mod(global: &Global, index: &ConflictIndex, wanted: &str) -> Result<()> {
    let Some(name) = index
        .layer_ids()
        .find(|id| *id == wanted)
        .or_else(|| index.layer_ids().find(|id| id.eq_ignore_ascii_case(wanted)))
    else {
        bail!("no enabled mod named {wanted:?} in this profile (see `strata list`)");
    };
    let flags = ModFlags::new(name, index.flags(name).unwrap_or_default());
    let files = index.files_of(name).unwrap_or_default();
    if global.json {
        let list: Vec<FileJson<'_>> = files
            .iter()
            .map(|f| FileJson {
                path: f.shown(),
                area: match f.area {
                    Area::Data => "data",
                    Area::Root => "root",
                },
                status: status(f, name),
                origins: f.origins.iter().map(|p| ProviderJson { name: p.layer, archive: p.archive }).collect(),
            })
            .collect();
        return print_json(&serde_json::json!({ "flags": flags, "files": list }));
    }
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    writeln!(out, "{name}: {} (archives: {}, loose/archives: {})", flags.state, flags.archive_state, flags.archive_loose_state)?;
    for (title, wanted) in [("Overwrites", "overwrites"), ("Overwritten", "overwritten"), ("No conflict", "none")] {
        let picked: Vec<&FileConflict<'_>> = files.iter().filter(|f| status(f, name) == wanted).collect();
        writeln!(out, "\n{title} ({}):", picked.len())?;
        for f in picked {
            match wanted {
                // The general tab lists every loser for a won file, the winner for a lost one.
                "overwrites" => {
                    let losers: Vec<String> = f.origins[..f.origins.len() - 1].iter().map(provider_text).collect();
                    writeln!(out, "  {}  <- {}", f.shown(), losers.join(", "))?
                }
                "overwritten" => writeln!(out, "  {}  -> {}", f.shown(), f.origins.last().map(provider_text).unwrap_or_default())?,
                _ => writeln!(out, "  {}", f.shown())?,
            }
        }
    }
    out.flush()?;
    Ok(())
}

/// `strata archive <file> [--extract <folder>]`: the files of a game archive or an install
/// archive, in archive order, or all of them extracted.
pub(crate) fn archive(global: &Global, file: &std::path::Path, extract: Option<&std::path::Path>) -> Result<()> {
    if let Some(out) = extract {
        return match GameArchive::open(file) {
            Ok(a) => Ok(a.extract_all(out)?),
            Err(strata_archive::bsa::Error::NotAnArchive(_)) => {
                let never = std::sync::atomic::AtomicBool::new(false);
                Ok(strata_archive::Archive::open(file, None)?.extract_all(out, &mut |_| {}, &never)?)
            }
            Err(e) => Err(e.into()),
        };
    }
    let (format, files): (String, Vec<(String, u64, bool)>) = match GameArchive::open(file) {
        Ok(a) => (
            format!("{:?}", a.format()),
            a.files().iter().map(|f| (f.path.clone(), f.stored_size, f.compressed)).collect(),
        ),
        Err(strata_archive::bsa::Error::NotAnArchive(_)) => {
            let a = strata_archive::Archive::open(file, None)?;
            let files = a.entries().iter().filter(|e| !e.is_dir).map(|e| (e.path.clone(), e.size, false)).collect();
            ("7-Zip".to_owned(), files)
        }
        Err(e) => return Err(e.into()),
    };
    if global.json {
        let list: Vec<_> = files
            .iter()
            .map(|(path, size, compressed)| serde_json::json!({ "path": path, "size": size, "compressed": compressed }))
            .collect();
        return print_json(&serde_json::json!({ "format": format, "files": list }));
    }
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for (path, _, _) in &files {
        writeln!(out, "{path}")?;
    }
    out.flush()?;
    Ok(())
}

/// `strata oracle conflicts`: one line per file with providers (`F area path origins`, the path
/// folded, an archived provider as `Mod[archive]`) and per mod (`M name state overwrites
/// overwritten_by archive_state archive_overwrites archive_overwritten_by archive_loose_state
/// archive_loose_overwrites archive_loose_overwritten_by hidden`), tab-separated, lists joined by
/// `|`. tools/oracle/conflicts.py prints the same from MO2's rules.
pub(crate) fn oracle(global: &Global) -> Result<()> {
    let (index, _) = build_index(global)?;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for f in index.all_files() {
        let area = if f.area == Area::Data { "data" } else { "root" };
        let origins: Vec<String> = f
            .origins
            .iter()
            .map(|p| match p.archive {
                Some(a) => format!("{}[{a}]", p.layer),
                None => p.layer.to_owned(),
            })
            .collect();
        writeln!(out, "F\t{area}\t{}\t{}", f.path.to_lowercase(), origins.join("|"))?;
    }
    for id in index.layer_ids() {
        if let Some(f) = index.flags(id) {
            let j = |l: &[String]| l.join("|");
            writeln!(
                out,
                "M\t{id}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                state_name(f.state),
                j(&f.overwrites),
                j(&f.overwritten_by),
                state_name(f.archive_state),
                j(&f.archive_overwrites),
                j(&f.archive_overwritten_by),
                state_name(f.archive_loose_state),
                j(&f.archive_loose_overwrites),
                j(&f.archive_loose_overwritten_by),
                f.hidden_files
            )?;
        }
    }
    out.flush()?;
    Ok(())
}

/// The folder of an instance mod.
fn mod_folder(global: &Global, name: &str) -> Result<std::path::PathBuf> {
    let (instance, _) = open(global)?;
    let dir = instance.paths.mods.join(name);
    if !dir.is_dir() {
        bail!("no mod folder {} (see `strata list`)", dir.display());
    }
    Ok(dir)
}

/// `strata pack-bsa <mod>`: MO2's BSA packer (`strata_archive::pack::pack_mod`), Skyrim SE
/// archives (v105; the only game Strata runs yet).
pub(crate) fn pack_bsa(global: &Global, name: &str, plugin: Option<&str>, compress: bool, hide_loose: bool) -> Result<()> {
    let dir = mod_folder(global, name)?;
    let archive_name = match plugin {
        Some(p) => p.trim_end_matches(".esp").trim_end_matches(".esm").trim_end_matches(".esl").to_owned(),
        None => {
            let plugins: Vec<String> = std::fs::read_dir(&dir)?
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| {
                    let l = n.to_lowercase();
                    l.ends_with(".esp") || l.ends_with(".esm") || l.ends_with(".esl")
                })
                .collect();
            match plugins.as_slice() {
                [one] => one[..one.len() - 4].to_owned(),
                [] => name.to_owned(),
                _ => bail!("{name} has several plugins ({}): choose one with --plugin", plugins.join(", ")),
            }
        }
    };
    let options = strata_archive::pack::PackModOptions { compress, hide_loose, ..Default::default() };
    let packed = strata_archive::pack::pack_mod(&dir, &archive_name, &options)?;
    if packed.archives.is_empty() {
        println!("nothing to pack in {name}");
    }
    for a in &packed.archives {
        println!("{}", a.display());
    }
    println!("{} files packed", packed.files);
    Ok(())
}

/// `strata extract-bsa <mod>`: MO2's BSA extractor (`strata_archive::pack::extract_mod_archives`).
pub(crate) fn extract_bsa(global: &Global, name: &str, remove: bool) -> Result<()> {
    let dir = mod_folder(global, name)?;
    let done = strata_archive::pack::extract_mod_archives(&dir, remove)?;
    for a in &done {
        println!("{}{}", a.display(), if remove { " (removed)" } else { "" });
    }
    if done.is_empty() {
        println!("no archives in {name}");
    }
    Ok(())
}

/// `strata preview <file> --out <file.pam>` (hidden): the preview's pixels of a DDS texture as
/// a PAM image, for tools/oracle/check-dds.sh.
pub(crate) fn preview(file: &std::path::Path, out: &std::path::Path) -> Result<()> {
    let data = std::fs::read(file).with_context(|| format!("cannot read {}", file.display()))?;
    let img = strata_ui::conflicts::preview::decode_dds(&data)?;
    let mut pam = format!("P7\nWIDTH {}\nHEIGHT {}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n", img.width, img.height)
        .into_bytes();
    for px in img.bgra.as_chunks::<4>().0 {
        pam.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    std::fs::write(out, pam).with_context(|| format!("cannot write {}", out.display()))?;
    println!("{} {}x{}", img.format, img.width, img.height);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Timings on a real instance (read only):
    /// `STRATA_BENCH_INSTANCE=<instance> cargo test --release -p strata-cli measure -- --nocapture`
    #[test]
    fn measure_build_toggle_move() {
        let Some(path) = std::env::var_os("STRATA_BENCH_INSTANCE") else {
            return eprintln!("SKIPPED: measure_build_toggle_move: a benchmark, runs only with STRATA_BENCH_INSTANCE=<instance>");
        };
        let instance = strata_core::Instance::open(std::path::Path::new(&path)).unwrap();
        let profile = instance.profile(None).unwrap();
        let mut layers = strata_core::resolve_layers(&instance, &profile).unwrap();
        let t = Instant::now();
        let mut index = ConflictIndex::build(&layers, instance.game.data_dir(), strata_ui::conflicts::archive_rules(&instance, &profile).unwrap());
        println!("build: {:?}, {} layers, {} files", t.elapsed(), layers.len(), index.file_count());
        // The mod with the most files: the worst case for a toggle.
        let (big, _) = layers
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, l)| (i, index.files_of(&l.id).map_or(0, |f| f.len())))
            .max_by_key(|(_, n)| *n)
            .unwrap();
        let removed = layers.remove(big);
        let t = Instant::now();
        let changed = index.update(&layers);
        let took = t.elapsed();
        println!("disable {:?} ({} files): {took:?}, {} mods changed", removed.id, index_len(&removed), changed.len());
        layers.insert(big, removed.clone());
        let t = Instant::now();
        let changed = index.update(&layers);
        println!("enable again: {:?}, {} mods changed", t.elapsed(), changed.len());
        let again = layers.remove(big);
        let t = Instant::now();
        index.update(&layers);
        println!("disable again: {:?}", t.elapsed());
        layers.insert(big, again);
        index.update(&layers);
        let moved = layers.remove(big);
        layers.insert(1, moved);
        let t = Instant::now();
        let changed = index.update(&layers);
        println!("move it to the bottom: {:?}, {} mods changed", t.elapsed(), changed.len());
        let fresh = ConflictIndex::build(&layers, instance.game.data_dir(), strata_ui::conflicts::archive_rules(&instance, &profile).unwrap());
        for l in &layers {
            assert_eq!(index.flags(&l.id), fresh.flags(&l.id), "{}", l.id);
        }
        println!("incremental result equals a fresh build");
    }

    fn index_len(layer: &strata_game::Layer) -> usize {
        walk(&layer.source)
    }

    fn walk(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| {
            let p = e.path();
            if p.is_dir() { walk(&p) } else { 1 }
        }).sum()
    }
}
