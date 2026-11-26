//! The built-ins: the ini editor's files, and the
//! automation surface that replaces the MO2 ApI Bridge (`origins`, `ls`, `vfs-export`,
//! `analyze`, `refresh`).

use std::io::{BufRead, Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::{Value, json};
use strata_core::inis::ReadOnly;
use strata_core::ModList;
use strata_vfs::Tree;
use strata_vfs::conflicts::{Area, Provider};

use crate::Global;
use crate::commands::{open, print_json};

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum IniAction {
    /// The game's inis of the profile and where each one is.
    List,
    /// Print one ini (`Skyrim.ini`, any case).
    Show { name: String },
    /// Replace one ini with the text read from standard input (written with CRLF, as MO2's
    /// ini editor writes; nothing written when unchanged).
    Save {
        name: String,
        /// When the file is read-only: write it and clear the flag, or write it once and
        /// keep it read-only.
        #[arg(long, value_parser = ["clear", "once"])]
        read_only: Option<String>,
    },
}

pub(crate) fn ini(global: &Global, action: &IniAction) -> Result<()> {
    let (instance, profile) = open(global)?;
    let files = instance.ini_files(&profile);
    let find = |name: &str| {
        files.iter().find(|f| f.name.eq_ignore_ascii_case(name)).with_context(|| {
            let names: Vec<_> = files.iter().map(|f| f.name.as_str()).collect();
            format!("\"{name}\" is not one of the game's inis ({})", names.join(", "))
        })
    };
    match action {
        IniAction::List => {
            if global.json {
                let list: Vec<Value> = files
                    .iter()
                    .map(|f| json!({ "name": f.name, "path": f.path, "size": f.size, "read_only": f.read_only }))
                    .collect();
                return print_json(&json!({ "profile": profile.name, "local": profile.local_settings, "inis": list }));
            }
            for f in &files {
                let place = f.path.as_ref().map_or("(not found: no prefix)".to_owned(), |p| p.display().to_string());
                let state = match (f.size, f.read_only) {
                    (None, _) => " (missing)",
                    (Some(_), true) => " (read-only)",
                    _ => "",
                };
                println!("{}\t{place}{state}", f.name);
            }
            Ok(())
        }
        IniAction::Show { name } => {
            let file = find(name)?;
            let path = file.path.as_ref().filter(|_| file.size.is_some()).with_context(|| format!("{name} does not exist"))?;
            std::io::stdout().write_all(&std::fs::read(path)?)?;
            Ok(())
        }
        IniAction::Save { name, read_only } => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text).context("cannot read the new text from standard input")?;
            let choice = match read_only.as_deref() {
                Some("clear") => ReadOnly::Clear,
                Some(_) => ReadOnly::Once,
                None => ReadOnly::Refuse,
            };
            let written = instance.save_ini(&profile, &find(name)?.name, &text, choice)?;
            if global.json {
                return print_json(&json!({ "name": name, "written": written }));
            }
            println!("{}", if written { "saved" } else { "unchanged" });
            Ok(())
        }
    }
}

/// The paths of the arguments, or one per line from standard input for `-`.
fn paths_from(args: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for a in args {
        if a == "-" {
            for line in std::io::stdin().lock().lines() {
                let line = line?;
                if !line.trim().is_empty() {
                    out.push(line.trim().to_owned());
                }
            }
        } else {
            out.push(a.clone());
        }
    }
    Ok(out)
}

/// A bridge-style path (`meshes\x.nif`, `/Root/x.dll`, `Data/meshes/x.nif`) as an area and
/// a path inside it.
fn area_path(path: &str, data_dir: &str) -> (Area, String) {
    let p = path.replace('\\', "/");
    let p = p.trim_matches('/');
    let strip = |prefix: &str| {
        (p.len() > prefix.len() && p[..prefix.len()].eq_ignore_ascii_case(prefix) && p.as_bytes()[prefix.len()] == b'/')
            .then(|| p[prefix.len() + 1..].to_owned())
    };
    if let Some(rest) = strip("Root") {
        (Area::Root, rest)
    } else if let Some(rest) = strip(data_dir) {
        (Area::Data, rest)
    } else {
        (Area::Data, p.to_owned())
    }
}

fn provider_json(p: &Provider<'_>) -> Value {
    match p.archive {
        Some(archive) => json!({ "mod": p.layer, "archive": archive }),
        None => json!({ "mod": p.layer }),
    }
}

/// `strata origins <path>...` (the bridge's `GET /origins`): every mod that provides each
/// file, the winner first, loose files and files in the loaded archives (the bridge saw
/// loose files only). Paths are relative to Data (`Root/...` for the Root folder), any
/// case, `/` or `\`.
pub(crate) fn origins(global: &Global, args: &[String]) -> Result<()> {
    let (instance, profile) = open(global)?;
    let index = strata_ui::conflicts::build_index(&instance, &profile)?;
    let data_dir = instance.game.data_dir();
    let mut results = Vec::new();
    for path in paths_from(args)? {
        let (area, inner) = area_path(&path, data_dir);
        let found = index.origins(area, &inner).unwrap_or_default();
        let winner_first: Vec<Value> = found.iter().rev().map(provider_json).collect();
        results.push(json!({ "path": path, "found": !found.is_empty(), "origins": winner_first }));
    }
    if global.json {
        return print_json(&json!({ "profile": profile.name, "results": results }));
    }
    let mut out = std::io::stdout().lock();
    for r in &results {
        let names: Vec<String> = r["origins"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|o| match o["archive"].as_str() {
                Some(a) => format!("{} ({a})", o["mod"].as_str().unwrap_or("?")),
                None => o["mod"].as_str().unwrap_or("?").to_owned(),
            })
            .collect();
        writeln!(out, "{}\t{}", r["path"].as_str().unwrap_or(""), if names.is_empty() { "(nobody)".into() } else { names.join(", ") })?;
    }
    Ok(())
}

/// `*` and `?` matching without case (the bridge's `filter=` masks, `QDir` name filters).
fn glob(mask: &str, name: &str) -> bool {
    let (m, n): (Vec<char>, Vec<char>) = (mask.to_lowercase().chars().collect(), name.to_lowercase().chars().collect());
    let (mut mi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if mi < m.len() && (m[mi] == '?' || m[mi] == n[ni]) {
            mi += 1;
            ni += 1;
        } else if mi < m.len() && m[mi] == '*' {
            star = Some(mi);
            mark = ni;
            mi += 1;
        } else if let Some(s) = star {
            mi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    m[mi..].iter().all(|c| *c == '*')
}

/// `strata ls [path]` (the bridge's `GET /vfs` and `GET /dirs`): the merged game folder's
/// entries under `path` (relative to the game folder, any case), with their origin and
/// real file.
pub(crate) fn ls(global: &Global, path: &str, filter: Option<&str>, recursive: bool, dirs: bool) -> Result<()> {
    let (instance, profile) = open(global)?;
    let tree = strata_ui::profile_tree::merged_tree(&instance, &profile)?;
    let start = path.replace('\\', "/");
    let start = start.trim_matches('/');
    let Some(id) = tree.resolve(start) else {
        bail!("{start}: no such folder in the merged game folder");
    };
    let mut entries = Vec::new();
    collect(&tree, id, recursive, &mut entries);
    let rows: Vec<Value> = entries
        .into_iter()
        .filter(|(_, dir)| !dirs || *dir)
        .filter(|(id, _)| filter.is_none_or(|m| tree.node(*id).is_some_and(|n| glob(m, &n.name))))
        .map(|(id, dir)| {
            json!({
                "path": tree.path(id),
                "kind": if dir { "dir" } else { "file" },
                "origin": tree.layer_of(id).map(|l| l.id.clone()),
                "real": if dir { None } else { tree.real_path(id) },
            })
        })
        .collect();
    if global.json {
        return print_json(&json!({ "profile": profile.name, "entries": rows }));
    }
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for r in &rows {
        let slash = if r["kind"] == "dir" { "/" } else { "" };
        writeln!(out, "{}{slash}\t{}", r["path"].as_str().unwrap_or(""), r["origin"].as_str().unwrap_or(""))?;
    }
    Ok(())
}

fn collect(tree: &Tree, id: strata_vfs::NodeId, recursive: bool, out: &mut Vec<(strata_vfs::NodeId, bool)>) {
    let mut children: Vec<_> = tree.children(id).map(|(c, n)| (c, n.is_dir(), n.name.to_lowercase())).collect();
    children.sort_by(|a, b| a.2.cmp(&b.2));
    for (child, dir, _) in children {
        out.push((child, dir));
        if dir && recursive {
            collect(tree, child, true, out);
        }
    }
}

/// A CSV field, quoted when needed (RFC 4180).
fn csv(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) { format!("\"{}\"", field.replace('"', "\"\"")) } else { field.to_owned() }
}

/// `strata vfs-export <file>` (the bridge's `POST /vfsexport`): every file of the merged
/// game folder as CSV, `path,origin,real`, sorted by path. `-` writes to standard output.
pub(crate) fn vfs_export(global: &Global, file: &Path) -> Result<()> {
    let (instance, profile) = open(global)?;
    let tree = strata_ui::profile_tree::merged_tree(&instance, &profile)?;
    let mut files = tree.files();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut text = String::from("path,origin,real\n");
    for (path, id) in &files {
        let origin = tree.layer_of(*id).map_or("", |l| l.id.as_str());
        let real = tree.real_path(*id).map(|p| p.display().to_string()).unwrap_or_default();
        text += &format!("{},{},{}\n", csv(path), csv(origin), csv(&real));
    }
    if file == Path::new("-") {
        std::io::stdout().write_all(text.as_bytes())?;
        return Ok(());
    }
    std::fs::write(file, &text).with_context(|| format!("cannot write {}", file.display()))?;
    if global.json {
        return print_json(&json!({ "profile": profile.name, "file": file, "files": files.len() }));
    }
    println!("{} files written to {}", files.len(), file.display());
    Ok(())
}

/// `strata analyze <mod>` (the bridge's `GET /analyze name= conflicts=1 limit=`): one mod's
/// place in the list, its plugins, its conflicts with other mods and up to `limit` of its
/// files per conflict status.
pub(crate) fn analyze(global: &Global, wanted: &str, limit: usize) -> Result<()> {
    let (instance, profile) = open(global)?;
    let list = ModList::load(&instance, &profile)?;
    let entry = list
        .entries()
        .iter()
        .find(|e| e.name == wanted)
        .or_else(|| list.entries().iter().find(|e| e.name.eq_ignore_ascii_case(wanted)))
        .with_context(|| format!("no mod named {wanted:?} in this profile (see `strata list`)"))?;
    let name = entry.name.clone();
    let index = strata_ui::conflicts::build_index(&instance, &profile)?;
    let flags = index.flags(&name).unwrap_or_default();
    let files = index.files_of(&name).unwrap_or_default();
    let (_, _, plugin_list) = crate::cmd_plugins::open_plugins(global)?;
    let plugins: Vec<Value> = plugin_list
        .plugins()
        .filter(|p| p.origin == name)
        .map(|p| json!({ "name": p.name, "enabled": p.enabled, "priority": p.priority }))
        .collect();

    let mut groups = serde_json::Map::new();
    for status in ["overwrites", "overwritten", "none"] {
        let picked: Vec<_> = files
            .iter()
            .filter(|f| {
                let winner = f.origins.last().is_some_and(|p| p.layer == name);
                match status {
                    "overwrites" => f.origins.len() > 1 && winner,
                    "overwritten" => f.origins.len() > 1 && !winner,
                    _ => f.origins.len() == 1,
                }
            })
            .collect();
        let shown: Vec<Value> = picked
            .iter()
            .take(limit)
            .map(|f| json!({ "path": f.shown(), "origins": f.origins.iter().rev().map(provider_json).collect::<Vec<_>>() }))
            .collect();
        groups.insert(status.to_owned(), json!({ "count": picked.len(), "files": shown }));
    }
    let report = json!({
        "profile": profile.name,
        "name": name,
        "priority": entry.priority,
        "enabled": entry.enabled,
        "path": entry.path,
        "files": files.len(),
        "plugins": plugins,
        "overwrites": flags.overwrites,
        "overwritten_by": flags.overwritten_by,
        "archive_overwrites": flags.archive_overwrites,
        "archive_overwritten_by": flags.archive_overwritten_by,
        "conflicts": groups,
    });
    if global.json {
        return print_json(&report);
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "{name}: priority {}, {}", entry.priority, if entry.enabled { "enabled" } else { "disabled" })?;
    writeln!(out, "{} files; plugins: {}", files.len(), plugins.iter().filter_map(|p| p["name"].as_str()).collect::<Vec<_>>().join(", "))?;
    writeln!(out, "overwrites: {}", flags.overwrites.join(", "))?;
    writeln!(out, "overwritten by: {}", flags.overwritten_by.join(", "))?;
    if !flags.archive_overwrites.is_empty() || !flags.archive_overwritten_by.is_empty() {
        writeln!(out, "archives overwrite: {}", flags.archive_overwrites.join(", "))?;
        writeln!(out, "archives overwritten by: {}", flags.archive_overwritten_by.join(", "))?;
    }
    for (status, group) in &groups {
        writeln!(out, "{status}: {}", group["count"])?;
        for f in group["files"].as_array().into_iter().flatten() {
            writeln!(out, "  {}", f["path"].as_str().unwrap_or(""))?;
        }
    }
    Ok(())
}

/// `strata refresh` (the bridge's `POST /refresh`): Strata reads the disk at every command,
/// so only a live deploy needs telling that `mods/` changed (a mod extracted by hand).
pub(crate) fn refresh(global: &Global) -> Result<()> {
    let (_, profile) = open(global)?;
    let line = crate::cmd_modlist::refresh_deploy(global, &profile.name)?;
    if global.json {
        return print_json(&json!({ "profile": profile.name, "deploy": line }));
    }
    println!("{}", line.unwrap_or_else(|| "not deployed: nothing to refresh (every command reads the disk)".into()));
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_match_as_qdir_name_filters() {
        assert!(glob("*.nif", "Body.NIF"));
        assert!(glob("a?c*", "abcdef"));
        assert!(!glob("*.nif", "body.dds"));
        assert!(glob("*", ""));
        assert!(glob("*a*b", "xxaxxb"));
    }

    #[test]
    fn bridge_paths_name_an_area() {
        assert_eq!(area_path("meshes\\x.nif", "Data"), (Area::Data, "meshes/x.nif".into()));
        assert_eq!(area_path("/Root/skse64_loader.exe", "Data"), (Area::Root, "skse64_loader.exe".into()));
        assert_eq!(area_path("data/meshes/x.nif", "Data"), (Area::Data, "meshes/x.nif".into()));
        assert_eq!(area_path("Database/x", "Data"), (Area::Data, "Database/x".into()));
    }


    #[test]
    fn csv_fields_are_quoted_when_needed() {
        assert_eq!(csv("a,b"), "\"a,b\"");
        assert_eq!(csv("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv("plain"), "plain");
    }
}
