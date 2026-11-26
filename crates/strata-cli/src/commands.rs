//! The subcommands. Each prints text for people or, with `--json`, one JSON document.

use std::io::Write;

use anyhow::{Context, Result};
use serde::Serialize;
use strata_core::{Instance, ModKind, Profile};

use crate::{Command, Global, ListWhat, deploy};

pub(crate) fn run(global: &Global, command: &Command) -> Result<()> {
    match command {
        Command::List { what: ListWhat::Mods } => list_mods(global),
        Command::List { what: ListWhat::Profiles } => list_profiles(global),
        Command::Info => info(global),
        Command::Tree { origin } => tree(global, *origin),
        Command::List { what: ListWhat::Executables } => list_executables(global),
        Command::Run {
            title,
            timeout,
            dry_run,
            extra,
        } => crate::run::run(
            global,
            title.as_deref(),
            *timeout,
            *dry_run,
            extra,
        ),
        Command::Deploy => deploy::deploy(global).map(|_| ()),
        Command::Undeploy { force } => deploy::undeploy(global, *force),
        Command::Status => deploy::status(global),
        Command::Resolve { path } => deploy::resolve(global, path),
        Command::Oracle { check: crate::OracleCheck::Roundtrip } => crate::oracle::roundtrip(global),
        Command::Oracle { check: crate::OracleCheck::Conflicts } => crate::cmd_conflicts::oracle(global),
        Command::Vfsd { socket } => deploy::vfsd(socket),
        Command::Enable { .. }
        | Command::Disable { .. }
        | Command::Move { .. }
        | Command::Mod { .. }
        | Command::Separator { .. }
        | Command::Profile { .. }
        | Command::Backup { .. }
        | Command::Overwrite { .. }
        | Command::Category { .. }
        | Command::Executable { .. }
        | Command::Link { .. }
        | Command::RegisterLinks { .. } => crate::cmd_modlist::run(global, command),
        Command::List { what: ListWhat::Plugins } => crate::cmd_plugins::list(global),
        Command::List { what: ListWhat::Saves } => crate::cmd_plugins::list_saves(global),
        Command::Plugins { action } => crate::cmd_plugins::run(global, action),
        Command::Sort { dry_run, offline } => crate::cmd_plugins::sort(global, *dry_run, *offline),
        Command::Problems { fix } => crate::cmd_plugins::problems(global, fix.as_deref()),
        Command::Conflicts { name } => crate::cmd_conflicts::conflicts(global, name.as_deref()),
        Command::PackBsa { name, plugin, no_compress, hide_loose } => {
            crate::cmd_conflicts::pack_bsa(global, name, plugin.as_deref(), !*no_compress, *hide_loose)
        }
        Command::ExtractBsa { name, remove } => crate::cmd_conflicts::extract_bsa(global, name, *remove),
        Command::Preview { file, out } => crate::cmd_conflicts::preview(file, out),
        Command::Archive { file, extract } => crate::cmd_conflicts::archive(global, file, extract.as_deref()),
        Command::Install(args) => crate::cmd_install::install_cmd(global, args),
        Command::Prefix { action } => crate::cmd_prefix::prefix(global, action),
        Command::Proton { action } => crate::cmd_prefix::proton(global, action),
        Command::CreatePortable(args) => crate::cmd_prefix::create_portable(args),
        Command::Downloads(args) => crate::cmd_install::downloads_cmd(global, args),
        Command::Ini { action } => crate::cmd_builtins::ini(global, action),
        Command::Origins { paths } => crate::cmd_builtins::origins(global, paths),
        Command::Ls { path, filter, recursive, dirs } => crate::cmd_builtins::ls(global, path, filter.as_deref(), *recursive, *dirs),
        Command::VfsExport { file } => crate::cmd_builtins::vfs_export(global, file),
        Command::Analyze { name, limit } => crate::cmd_builtins::analyze(global, name, *limit),
        Command::Refresh => crate::cmd_builtins::refresh(global),
        Command::Crashlogs { action } => crate::cmd_crashlogs::crashlogs(global, action.as_ref()),
        Command::Nexus { action } => crate::cmd_nexus::nexus(global, action),
        Command::Nxm { url, install } => crate::cmd_nexus::nxm(global, url, *install),
        Command::CloneInstance(args) => crate::cmd_clone::clone_instance(global, args),
    }
}

pub(crate) fn open(global: &Global) -> Result<(Instance, Profile)> {
    let root = match &global.instance {
        Some(path) => path.clone(),
        None => std::env::current_dir().context("cannot read the current folder")?,
    };
    // Mount paths must not depend on how the folder was named on the command line.
    let root = root.canonicalize().with_context(|| format!("cannot find {}", root.display()))?;
    let mut instance = Instance::open(&root)?;
    if let Some(prefix) = &global.prefix {
        instance.proton.prefix = Some(prefix.canonicalize().with_context(|| format!("cannot find {}", prefix.display()))?);
    }
    let profile = instance.profile(global.profile.as_deref())?;
    Ok((instance, profile))
}

pub(crate) fn print_json(value: &impl Serialize) -> Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, value)?;
    writeln!(out)?;
    Ok(())
}

#[derive(Serialize)]
struct ModJson<'a> {
    priority: usize,
    name: &'a str,
    kind: &'static str,
    enabled: bool,
    listed: bool,
}

fn list_mods(global: &Global) -> Result<()> {
    let (instance, profile) = open(global)?;
    let mods = instance.mod_list(&profile)?;
    let kind = |k| match k {
        ModKind::Regular => "mod",
        ModKind::Separator => "separator",
        ModKind::Foreign => "foreign",
        ModKind::Backup => "backup",
    };
    if global.json {
        let list: Vec<_> = mods
            .iter()
            .map(|m| ModJson { priority: m.priority, name: &m.name, kind: kind(m.kind), enabled: m.enabled, listed: m.listed })
            .collect();
        return print_json(&serde_json::json!({ "profile": profile.name, "mods": list }));
    }
    let mut out = std::io::stdout().lock();
    for m in &mods {
        let mark = match (m.kind, m.enabled) {
            (ModKind::Foreign, _) => '*',
            (_, true) => '+',
            (_, false) => '-',
        };
        writeln!(out, "{:>5} {mark} {}{}", m.priority, m.name, if m.listed { "" } else { "  (not in modlist.txt)" })?;
    }
    Ok(())
}

fn list_profiles(global: &Global) -> Result<()> {
    let root = match &global.instance {
        Some(path) => path.clone(),
        None => std::env::current_dir()?,
    };
    let instance = Instance::open(&root)?;
    let names = instance.profile_names()?;
    if global.json {
        return print_json(&serde_json::json!({ "selected": instance.selected_profile, "profiles": names }));
    }
    for name in names {
        println!("{} {name}", if name == instance.selected_profile { '*' } else { ' ' });
    }
    Ok(())
}

fn tree(global: &Global, origin: bool) -> Result<()> {
    let (instance, profile) = open(global)?;
    let tree = strata_ui::profile_tree::merged_tree(&instance, &profile)?;
    let files = tree.files();
    if global.json {
        let list: Vec<_> = files
            .iter()
            .map(|(path, id)| serde_json::json!({ "path": path, "origin": tree.layer_of(*id).map(|l| l.id.as_str()) }))
            .collect();
        return print_json(&serde_json::json!({ "profile": profile.name, "files": list }));
    }
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for (path, id) in &files {
        if origin {
            writeln!(out, "{path}\t{}", tree.layer_of(*id).map_or("?", |l| l.id.as_str()))?;
        } else {
            writeln!(out, "{path}")?;
        }
    }
    out.flush()?;
    Ok(())
}

fn list_executables(global: &Global) -> Result<()> {
    let (instance, _) = open(global)?;
    let executables = instance.executables();
    if global.json {
        let list: Vec<_> = executables
            .iter()
            .map(|e| {
                serde_json::json!({
                    "title": e.title, "binary": e.binary, "arguments": e.arguments,
                    "workingDirectory": e.working_directory, "steamAppID": e.steam_app_id,
                })
            })
            .collect();
        return print_json(&serde_json::json!({ "executables": list }));
    }
    for e in executables {
        println!("{}\n    {} {}", e.title, e.binary, e.arguments);
    }
    Ok(())
}

pub(crate) fn info(global: &Global) -> Result<()> {
    let (instance, profile) = open(global)?;
    let value = serde_json::json!({
        "instance": instance.root,
        "game": instance.game.name(),
        "game_path": instance.game_path,
        "profile": profile.name,
        "profile_dir": profile.dir,
        "local_saves": profile.local_saves,
        "local_settings": profile.local_settings,
        "mods": instance.paths.mods,
        "overwrite": instance.paths.overwrite,
        "downloads": instance.paths.downloads,
        "prefix": instance.proton.prefix,
        "proton": instance.proton.proton,
        "steam_drm": instance.proton.steam_drm,
        "primary_plugins": instance.primary_plugins(),
    });
    if global.json {
        return print_json(&value);
    }
    for (key, entry) in value.as_object().into_iter().flatten() {
        let text = match entry {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Null => "(not set)".to_owned(),
            other => other.to_string(),
        };
        println!("{key:<15} {text}");
    }
    Ok(())
}
