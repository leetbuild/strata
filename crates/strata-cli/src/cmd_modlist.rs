//! The mod list and profile commands (area "modlist"): `enable`, `disable`, `move`, `mod`,
//! `separator`, `profile`, `backup`, `overwrite`, `category`. Each one that changes what the
//! game sees refreshes a live deploy of the same profile.

use std::io::Write;

use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use serde_json::{Value, json};
use strata_core::backups::{self, PLUGIN_FILES};
use strata_core::separators::SeparatorTree;
use strata_core::write::Transaction;
use strata_core::{Categories, Color, Instance, ModKind, ModList, ModListEvent, ModMeta, Profile, ProfileEvent, Trash};
use strata_vfs::daemon::{self, Request};

use crate::commands::{open, print_json};
use crate::deploy::{checked, mount_specs};
use crate::{Command, Global};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum OnOff {
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum BackupWhat {
    /// modlist.txt (the left pane's backup buttons)
    Modlist,
    /// plugins.txt, loadorder.txt and lockedorder.txt (the right pane's)
    Plugins,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum ModAction {
    /// Create an empty mod (on top, or just above --at; at the end of --at when it is a separator).
    Create {
        name: String,
        #[arg(long, value_name = "MOD")]
        at: Option<String>,
    },
    /// Rename a mod: its folder and its line in every profile's modlist.txt.
    Rename { old: String, new: String },
    /// Remove mods: their folders go to the trash (or with --permanent are deleted).
    Remove {
        #[arg(required = true)]
        names: Vec<String>,
        #[arg(long)]
        permanent: bool,
    },
    /// Show a mod's meta.ini as MO2 reads it.
    Info { name: String },
    /// Show or set a mod's notes.
    Notes { name: String, text: Option<String> },
    /// Show or set a separator's colour: #rrggbb, #aarrggbb, or "none".
    Color { name: String, color: Option<String> },
    /// Show or set a mod's categories: ids, comma-separated, primary first (`none` clears).
    Categories { name: String, ids: Option<String> },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum SeparatorAction {
    /// Add a separator (name without "_separator"). --at a separator: at its end, one level
    /// deeper; --at a mod: just above it, at its separator's level.
    Add {
        name: String,
        #[arg(long, value_name = "MOD")]
        at: Option<String>,
    },
    /// Set a separator's Sub-Separator level.
    Nest { name: String, level: u32 },
    /// Print the separator tree as the window shows it in ascending priority.
    Tree,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum ProfileAction {
    /// Create a profile (inis and plugins.txt from the prefix).
    Create {
        name: String,
        /// Take the game's default ini instead of the prefix's.
        #[arg(long)]
        defaults: bool,
    },
    /// Copy a profile, saves and all.
    Copy { from: String, name: String },
    /// Make a profile the instance's selected one (a live deploy is unmounted and mounted again).
    Select { name: String },
    /// Rename a profile (not the selected one).
    Rename { old: String, new: String },
    /// Delete a profile with its local saves (not the selected one). To the trash by default.
    Delete {
        name: String,
        #[arg(long)]
        permanent: bool,
    },
    /// Profile-specific saves on or off (--profile or the selected one).
    LocalSaves {
        state: OnOff,
        /// When switching off: delete the profile's saves (to the trash).
        #[arg(long)]
        delete: bool,
    },
    /// Profile-specific game inis on or off.
    LocalInis {
        state: OnOff,
        /// When switching off: delete the profile's inis (to the trash).
        #[arg(long)]
        delete: bool,
    },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum BackupAction {
    /// Back up the mod list or the load order, as MO2's backup buttons do.
    Create {
        #[arg(value_enum, default_value_t = BackupWhat::Modlist)]
        what: BackupWhat,
    },
    /// List the backups, newest first.
    List {
        #[arg(value_enum, default_value_t = BackupWhat::Modlist)]
        what: BackupWhat,
    },
    /// Restore a backup by its suffix (as `backup list` shows it).
    Restore {
        suffix: String,
        #[arg(value_enum, default_value_t = BackupWhat::Modlist)]
        what: BackupWhat,
    },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum OverwriteAction {
    /// Delete everything in overwrite (to the trash by default), and Strata's whiteouts.
    Clear {
        #[arg(long)]
        permanent: bool,
    },
    /// Move overwrite's content into a mod (merging; overwrite's files win).
    Move {
        name: String,
        /// Create the mod first.
        #[arg(long)]
        create: bool,
    },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum ExecutableAction {
    /// List the executables with all their fields.
    List,
    /// Add an executable, or change the one with this title.
    Set {
        title: String,
        #[arg(long)]
        binary: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        arguments: Option<String>,
        #[arg(long)]
        working_dir: Option<String>,
        #[arg(long)]
        steam_app_id: Option<String>,
        #[arg(long)]
        toolbar: Option<bool>,
        #[arg(long)]
        hide: Option<bool>,
    },
    /// Remove an executable.
    Remove { title: String },
    /// Move an executable to a place in the list (0 = first).
    Move { title: String, to: usize },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum CategoryAction {
    /// List the categories (categories.dat).
    List,
    /// Add a category; prints its id.
    Add {
        name: String,
        #[arg(long, default_value_t = 0)]
        parent: i32,
    },
    /// Rename a category.
    Rename { id: i32, name: String },
    /// Remove a category from categories.dat.
    Remove { id: i32 },
}

pub(crate) fn run(global: &Global, command: &Command) -> Result<()> {
    match command {
        Command::Enable { mods } => switch(global, mods, true),
        Command::Disable { mods } => switch(global, mods, false),
        Command::Move { mods, to, by } => move_mods(global, mods, *to, *by),
        Command::Mod { action } => mod_action(global, action),
        Command::Separator { action } => separator_action(global, action),
        Command::Profile { action } => profile_action(global, action),
        Command::Backup { action } => backup_action(global, action),
        Command::Overwrite { action } => overwrite_action(global, action),
        Command::Category { action } => category_action(global, action),
        Command::Executable { action } => executable_action(global, action),
        Command::Link { url } => link(global, url),
        Command::RegisterLinks { make_default } => register_links(global, *make_default),
        _ => unreachable!("not a modlist command"),
    }
}

fn mod_event(event: &ModListEvent) -> Value {
    match event {
        ModListEvent::StateChanged { names, enabled } => json!({ "event": "state", "mods": names, "enabled": enabled }),
        ModListEvent::Moved { name, from, to } => json!({ "event": "moved", "mod": name, "from": from, "to": to }),
        ModListEvent::NestChanged { name, nest } => json!({ "event": "nest", "mod": name, "nest": nest }),
        ModListEvent::Renamed { from, to } => json!({ "event": "renamed", "from": from, "to": to }),
        ModListEvent::Removed { name, priority } => json!({ "event": "removed-mod", "mod": name, "priority": priority }),
        ModListEvent::Added { name, priority } => json!({ "event": "added", "mod": name, "priority": priority }),
        ModListEvent::MetaChanged { name } => json!({ "event": "meta", "mod": name }),
        ModListEvent::OverwriteChanged => json!({ "event": "overwrite" }),
    }
}

fn profile_event(event: &ProfileEvent) -> Value {
    match event {
        ProfileEvent::Created { name } => json!({ "event": "created", "profile": name }),
        ProfileEvent::Renamed { from, to } => json!({ "event": "renamed", "from": from, "to": to }),
        ProfileEvent::Removed { name } => json!({ "event": "removed", "profile": name }),
        ProfileEvent::Selected { name } => json!({ "event": "selected", "profile": name }),
        ProfileEvent::SettingsChanged { name } => json!({ "event": "settings", "profile": name }),
        ProfileEvent::BackupChanged { name } => json!({ "event": "backup", "profile": name }),
    }
}

/// The profile shown by a live deploy of this instance, if one runs.
fn deployed_profile(instance: &Instance) -> Result<Option<String>> {
    Ok(strata_ui::live::deployed_profile(instance)?)
}

/// Refuses while `profile` (or, with `None`, anything) is deployed.
fn refuse_if_deployed(instance: &Instance, profile: Option<&str>, what: &str) -> Result<()> {
    if let Some(deployed) = deployed_profile(instance)?
        && profile.is_none_or(|p| p.eq_ignore_ascii_case(&deployed))
    {
        bail!("{what} while the instance is deployed (profile \"{deployed}\"). Run `strata undeploy` first.");
    }
    Ok(())
}

/// After a change to `profile`: if it is the deployed profile, sends its new layers to the
/// daemon. Returns a line for the user, if something was (or could not be) refreshed.
pub(crate) fn refresh_deploy(global: &Global, profile: &str) -> Result<Option<String>> {
    let (instance, _) = open(global)?;
    let Some(deployed) = deployed_profile(&instance)? else { return Ok(None) };
    if deployed != profile {
        return Ok(Some(format!("profile \"{deployed}\" is deployed, not \"{profile}\": the mount was left as it is")));
    }
    let profile = instance.profile(Some(profile))?;
    let socket = daemon::socket_path(&instance.root);
    let status = checked(daemon::request(&socket, &Request::Status)?)?;
    let live: Vec<String> =
        status["mounts"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(str::to_owned)).collect();
    let specs = mount_specs(&instance, &profile)?;
    let wanted: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
    if wanted != live.iter().map(String::as_str).collect::<Vec<_>>() {
        return Ok(Some("the profile now needs other mounts: run `strata undeploy` and `strata deploy` to apply it".into()));
    }
    let mut changed = 0;
    for spec in specs {
        let reply = checked(daemon::request(&socket, &Request::Refresh { id: spec.id, layers: spec.layers })?)?;
        changed += reply["changed"].as_u64().unwrap_or(0) + reply["entries"].as_u64().unwrap_or(0);
    }
    Ok(Some(format!("refreshed the live deploy ({changed} cached entries replaced)")))
}

/// Prints what happened: the events as JSON, or one line each.
fn report(global: &Global, events: Vec<Value>, refreshed: Option<String>) -> Result<()> {
    if global.json {
        return print_json(&json!({ "events": events, "deploy": refreshed }));
    }
    let mut out = std::io::stdout().lock();
    if events.is_empty() {
        writeln!(out, "nothing changed")?;
    }
    for e in &events {
        let line = match e["event"].as_str().unwrap_or("") {
            "state" => format!(
                "{} {}",
                if e["enabled"] == true { "enabled" } else { "disabled" },
                e["mods"].as_array().map(|m| m.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default()
            ),
            "moved" => format!("moved {} from priority {} to {}", e["mod"].as_str().unwrap_or("?"), e["from"], e["to"]),
            "nest" => format!("{} is now at level {}", e["mod"].as_str().unwrap_or("?"), e["nest"]),
            "renamed" => format!("renamed {} to {}", e["from"].as_str().unwrap_or("?"), e["to"].as_str().unwrap_or("?")),
            "added" => format!("added {} at priority {}", e["mod"].as_str().unwrap_or("?"), e["priority"]),
            "created" => format!("created profile {}", e["profile"].as_str().unwrap_or("?")),
            "removed" => format!("removed profile {}", e["profile"].as_str().unwrap_or("?")),
            "selected" => format!("selected profile {}", e["profile"].as_str().unwrap_or("?")),
            "redeployed" => format!("unmounted and mounted again with profile {}", e["profile"].as_str().unwrap_or("?")),
            "removed-mod" => format!("removed {}", e["mod"].as_str().unwrap_or("?")),
            "settings" => format!("updated settings.ini of {}", e["profile"].as_str().unwrap_or("?")),
            "overwrite" => "overwrite changed".into(),
            _ => e.to_string(),
        };
        writeln!(out, "{line}")?;
    }
    if let Some(line) = refreshed {
        writeln!(out, "{line}")?;
    }
    Ok(())
}

fn load(global: &Global) -> Result<(Instance, Profile, ModList)> {
    let (instance, profile) = open(global)?;
    strata_core::write::recover(&instance.strata_dir())?;
    let list = ModList::load(&instance, &profile)?;
    Ok((instance, profile, list))
}

/// Saves the list and refreshes a live deploy when something changed.
fn finish(global: &Global, profile: &Profile, list: &mut ModList, events: Vec<ModListEvent>) -> Result<()> {
    let mut refreshed = None;
    if !events.is_empty() {
        list.save()?;
        refreshed = refresh_deploy(global, &profile.name)?;
    }
    report(global, events.iter().map(mod_event).collect(), refreshed)
}

fn switch(global: &Global, mods: &[String], enabled: bool) -> Result<()> {
    let (_, profile, mut list) = load(global)?;
    let events = list.set_enabled(mods, enabled)?;
    finish(global, &profile, &mut list, events)
}

fn move_mods(global: &Global, mods: &[String], to: Option<usize>, by: Option<isize>) -> Result<()> {
    let (_, profile, mut list) = load(global)?;
    let events = match (to, by) {
        (Some(to), None) => list.move_mods(mods, to)?,
        (None, Some(by)) => list.shift_mods(mods, by)?,
        _ => bail!("give either --to <priority> or --by <offset>"),
    };
    finish(global, &profile, &mut list, events)
}

fn mod_action(global: &Global, action: &ModAction) -> Result<()> {
    let (instance, profile, mut list) = load(global)?;
    let meta_of = |list: &ModList, name: &str| -> Result<(String, ModMeta)> {
        let entry = list.get(name).ok_or_else(|| strata_core::Error::NoSuchMod { name: name.to_owned() })?;
        let path = entry.path.clone().with_context(|| format!("\"{}\" has no folder (a foreign mod)", entry.name))?;
        Ok((entry.name.clone(), ModMeta::open(&path)?))
    };
    match action {
        ModAction::Create { name, at } => {
            let events = list.create_mod(&instance.paths.mods, name, at.as_deref())?;
            finish(global, &profile, &mut list, events)
        }
        ModAction::Remove { names, permanent } => {
            let events = list.remove_mods(names, trash(*permanent)?.as_ref())?;
            finish(global, &profile, &mut list, events)
        }
        ModAction::Rename { old, new } => {
            let events = list.rename_mod(&instance, old, new)?;
            let refreshed = refresh_deploy(global, &profile.name)?;
            report(global, events.iter().map(mod_event).collect(), refreshed)
        }
        ModAction::Info { name } => {
            let (name, meta) = meta_of(&list, name)?;
            let entry = list.get(&name).cloned();
            let value = json!({
                "name": name,
                "priority": entry.as_ref().map(|e| e.priority),
                "enabled": entry.as_ref().map(|e| e.enabled),
                "nest": entry.as_ref().map(|e| e.nest),
                "gameName": meta.game_name(), "modid": meta.mod_id(), "version": meta.version(),
                "newestVersion": meta.newest_version(), "installationFile": meta.installation_file(),
                "repository": meta.repository(), "url": meta.url(), "hasCustomURL": meta.has_custom_url(),
                "categories": meta.categories(), "primaryCategory": meta.primary_category(), "comments": meta.comments(), "notes": meta.notes(),
                "color": meta.color().map(|c| c.name()), "endorsed": format!("{:?}", meta.endorsed()),
                "tracked": meta.tracked(), "installedFiles": meta.installed_files(),
            });
            if global.json {
                return print_json(&value);
            }
            for (key, v) in value.as_object().into_iter().flatten() {
                println!("{key:<17} {}", if let Value::String(s) = v { s.clone() } else { v.to_string() });
            }
            Ok(())
        }
        ModAction::Notes { name, text } => edit_meta(global, meta_of(&list, name)?, text.is_some(), |m| match text {
            Some(t) => Ok(m.set_notes(t)),
            None => Ok(false),
        }, |m| m.notes()),
        ModAction::Color { name, color } => {
            let (real, meta) = meta_of(&list, name)?;
            if list.get(&real).is_some_and(|e| e.kind != ModKind::Separator) {
                bail!("\"{real}\" is not a separator; only separators have a colour");
            }
            edit_meta(global, (real, meta), color.is_some(), |m| match color.as_deref() {
                Some("none") => Ok(m.set_color(None)),
                Some(text) => Ok(m.set_color(Some(Color::parse(text).with_context(|| format!("\"{text}\" is not #rrggbb or #aarrggbb"))?))),
                None => Ok(false),
            }, |m| m.color().map_or_else(|| "none".into(), |c| c.name()))
        }
        ModAction::Categories { name, ids } => edit_meta(global, meta_of(&list, name)?, ids.is_some(), |m| match ids.as_deref() {
            Some("none") => Ok(m.set_categories(None, &[])),
            Some(text) => {
                let ids: Vec<i32> = text.split(',').map(|s| s.trim().parse()).collect::<std::result::Result<_, _>>()
                    .with_context(|| format!("\"{text}\" is not a comma-separated list of category ids"))?;
                Ok(m.set_categories(ids.first().copied(), ids.get(1..).unwrap_or_default()))
            }
            None => Ok(false),
        }, |m| m.categories().iter().map(i32::to_string).collect::<Vec<_>>().join(",")),
    }
}

/// Shows a `meta.ini` value, or sets it and saves.
fn edit_meta(
    global: &Global,
    (name, mut meta): (String, ModMeta),
    setting: bool,
    set: impl FnOnce(&mut ModMeta) -> Result<bool>,
    show: impl Fn(&ModMeta) -> String,
) -> Result<()> {
    if !setting {
        if global.json {
            return print_json(&json!({ "mod": name, "value": show(&meta) }));
        }
        println!("{}", show(&meta));
        return Ok(());
    }
    let events = if set(&mut meta)? && meta.save()? { vec![mod_event(&ModListEvent::MetaChanged { name })] } else { Vec::new() };
    report(global, events, None)
}

fn separator_action(global: &Global, action: &SeparatorAction) -> Result<()> {
    let (instance, profile, mut list) = load(global)?;
    match action {
        SeparatorAction::Add { name, at } => {
            let events = list.create_separator(&instance.paths.mods, name, at.as_deref())?;
            finish(global, &profile, &mut list, events)
        }
        SeparatorAction::Nest { name, level } => {
            let mut txn = Transaction::new(&instance.strata_dir())?;
            let events = list.set_nest(name, *level, &mut txn)?;
            txn.commit()?;
            report(global, events.iter().map(mod_event).collect(), None)
        }
        SeparatorAction::Tree => print_tree(global, &list),
    }
}

fn print_tree(global: &Global, list: &ModList) -> Result<()> {
    let entries = list.entries();
    let tree = SeparatorTree::build(entries);
    let mut rows = Vec::new();
    fn walk(tree: &SeparatorTree, p: usize, depth: usize, rows: &mut Vec<(usize, usize)>) {
        rows.push((p, depth));
        for c in &tree.children[p] {
            walk(tree, *c, depth + 1, rows);
        }
    }
    for node in &tree.root {
        if let strata_core::separators::Node::Mod(p) = node {
            walk(&tree, *p, 0, &mut rows);
        }
    }
    if global.json {
        let list: Vec<Value> = rows
            .iter()
            .map(|(p, depth)| {
                let e = &entries[*p];
                json!({ "priority": p, "name": e.name, "depth": depth, "parent": tree.parent[*p].map(|s| &entries[s].name),
                        "separator": e.kind == ModKind::Separator, "nest": e.nest, "rows": tree.row_count(*p) })
            })
            .collect();
        return print_json(&json!({ "tree": list }));
    }
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for (p, depth) in rows {
        let e = &entries[p];
        let indent = "  ".repeat(depth);
        if e.kind == ModKind::Separator {
            let level = "#".repeat(e.nest as usize);
            writeln!(out, "{p:>5} {indent}{} {level} ({})", e.name.trim_end_matches("_separator"), tree.row_count(p))?;
        } else {
            writeln!(out, "{p:>5} {indent}{} {}", if e.enabled { '+' } else { '-' }, e.name)?;
        }
    }
    out.flush()?;
    Ok(())
}

/// What [`select_profile`] did.
pub struct ProfileSwitch {
    pub events: Vec<ProfileEvent>,
    /// The instance was deployed and is now mounted with the new profile.
    pub redeployed: bool,
}

/// Makes `name` the selected profile of the instance in `root`. When the instance is
/// deployed, it is unmounted first (not forced: a running game makes this an error and
/// nothing changes) and mounted again with the new profile. `prefix` is the one-run
/// override of `--prefix`.
pub fn select_profile(root: &std::path::Path, name: &str, prefix: Option<&std::path::Path>) -> Result<ProfileSwitch> {
    let mut instance = Instance::open(root)?;
    if let Some(prefix) = prefix {
        instance.proton.prefix = Some(prefix.canonicalize()?);
    }
    let target = instance.profile(Some(name))?;
    if target.name == instance.selected_profile {
        return Ok(ProfileSwitch { events: Vec::new(), redeployed: false });
    }
    let deployed = deployed_profile(&instance)?.is_some();
    if deployed {
        crate::deploy::unmount_daemon(&daemon::socket_path(&instance.root), false)
            .context("cannot switch profiles while the deploy is busy (is the game running?)")?;
    }
    let events = instance.select_profile(&target.name)?;
    if deployed {
        let profile = instance.profile(None)?;
        crate::deploy::mount_profile(&instance, &profile).context("the profile was selected, but mounting it failed")?;
    }
    Ok(ProfileSwitch { events, redeployed: deployed })
}

fn trash(permanent: bool) -> Result<Option<Trash>> {
    Ok(if permanent { None } else { Some(Trash::home()?) })
}

fn profile_action(global: &Global, action: &ProfileAction) -> Result<()> {
    let root = global.instance.clone().map_or_else(std::env::current_dir, Ok).context("cannot read the current folder")?;
    let root = root.canonicalize().with_context(|| format!("cannot find {}", root.display()))?;
    let mut instance = Instance::open(&root)?;
    if let Some(prefix) = &global.prefix {
        instance.proton.prefix = Some(prefix.canonicalize()?);
    }
    strata_core::write::recover(&instance.strata_dir())?;
    let events = match action {
        ProfileAction::Create { name, defaults } => instance.create_profile(name, *defaults)?.1,
        ProfileAction::Copy { from, name } => instance.copy_profile(from, name)?.1,
        ProfileAction::Select { name } => {
            let switched = select_profile(&instance.root, name, global.prefix.as_deref())?;
            let mut events: Vec<Value> = switched.events.iter().map(profile_event).collect();
            if switched.redeployed {
                events.push(json!({ "event": "redeployed", "profile": name }));
            }
            return report(global, events, None);
        }
        ProfileAction::Rename { old, new } => {
            refuse_if_deployed(&instance, Some(old), "a deployed profile can't be renamed")?;
            instance.rename_profile(old, new)?
        }
        ProfileAction::Delete { name, permanent } => {
            refuse_if_deployed(&instance, Some(name), "a deployed profile can't be deleted")?;
            instance.delete_profile(name, trash(*permanent)?.as_ref())?
        }
        ProfileAction::LocalSaves { state, delete } | ProfileAction::LocalInis { state, delete } => {
            let profile = instance.profile(global.profile.as_deref())?;
            let on = *state == OnOff::On;
            let bin = if *delete { trash(false)? } else { None };
            let events = if matches!(action, ProfileAction::LocalSaves { .. }) {
                instance.set_local_saves(&profile, on, *delete, bin.as_ref())?
            } else {
                instance.set_local_settings(&profile, on, *delete, bin.as_ref())?
            };
            let refreshed = if events.is_empty() { None } else { refresh_deploy(global, &profile.name)? };
            return report(global, events.iter().map(profile_event).collect(), refreshed);
        }
    };
    report(global, events.iter().map(profile_event).collect(), None)
}

fn backup_action(global: &Global, action: &BackupAction) -> Result<()> {
    let (_, profile) = open(global)?;
    let files = |what: BackupWhat| -> Vec<std::path::PathBuf> {
        match what {
            BackupWhat::Modlist => vec![profile.dir.join("modlist.txt")],
            BackupWhat::Plugins => PLUGIN_FILES.iter().map(|f| profile.dir.join(f)).collect(),
        }
    };
    match action {
        BackupAction::Create { what } => {
            let suffix = backups::now_suffix();
            // like MO2: stop at the first file that can't be copied
            let made = files(*what).iter().map(|f| backups::create_backup(f, &suffix)).collect::<strata_core::Result<Vec<_>>>()?;
            if global.json {
                return print_json(&json!({ "suffix": suffix, "files": made }));
            }
            println!("backup {suffix} made ({} files)", made.len());
            Ok(())
        }
        BackupAction::List { what } => {
            let list = backups::list_backups(&files(*what)[0])?;
            if global.json {
                let items: Vec<Value> = list.iter().map(|b| json!({ "suffix": b.suffix, "dated": b.dated })).collect();
                return print_json(&json!({ "backups": items }));
            }
            for b in list {
                println!("{}{}", b.suffix, if b.dated { "" } else { "  (not made by the backup button)" });
            }
            Ok(())
        }
        BackupAction::Restore { suffix, what } => {
            for file in files(*what) {
                backups::restore_backup(&file, suffix)?;
            }
            let refreshed = if *what == BackupWhat::Modlist { refresh_deploy(global, &profile.name)? } else { None };
            report(global, vec![profile_event(&ProfileEvent::BackupChanged { name: profile.name.clone() })], refreshed)
        }
    }
}

fn overwrite_action(global: &Global, action: &OverwriteAction) -> Result<()> {
    let (instance, _, mut list) = load(global)?;
    refuse_if_deployed(&instance, None, "overwrite can't change")?;
    match action {
        OverwriteAction::Clear { permanent } => {
            let events = instance.clear_overwrite(trash(*permanent)?.as_ref())?;
            report(global, events.iter().map(mod_event).collect(), None)
        }
        OverwriteAction::Move { name, create } => {
            let mut events = Vec::new();
            if *create {
                events.extend(list.create_mod(&instance.paths.mods, name, None)?);
                list.save()?;
            }
            let entry = list.get(name).ok_or_else(|| strata_core::Error::NoSuchMod { name: name.to_owned() })?;
            if entry.kind != ModKind::Regular {
                bail!("\"{}\" is not a regular mod", entry.name);
            }
            let path = entry.path.clone().context("the mod has no folder")?;
            events.extend(instance.move_overwrite_to(&path)?);
            report(global, events.iter().map(mod_event).collect(), None)
        }
    }
}

fn category_action(global: &Global, action: &CategoryAction) -> Result<()> {
    let root = global.instance.clone().map_or_else(std::env::current_dir, Ok).context("cannot read the current folder")?;
    let mut cats = Categories::open(&root)?;
    let changed = match action {
        CategoryAction::List => {
            if global.json {
                let list: Vec<Value> = cats.list.iter().map(|c| json!({ "id": c.id, "name": c.name, "parent": c.parent })).collect();
                return print_json(&json!({ "categories": list }));
            }
            for c in &cats.list {
                println!("{:>4} {}{}", c.id, c.name, if c.parent != 0 { format!("  (in {})", c.parent) } else { String::new() });
            }
            return Ok(());
        }
        CategoryAction::Add { name, parent } => {
            let id = cats.add(name, *parent);
            println!("{id}");
            true
        }
        CategoryAction::Rename { id, name } => cats.rename(*id, name),
        CategoryAction::Remove { id } => cats.remove(*id),
    };
    if !changed {
        bail!("no such category; `strata category list` shows them");
    }
    cats.save()?;
    Ok(())
}

fn executable_action(global: &Global, action: &ExecutableAction) -> Result<()> {
    let (mut instance, _) = open(global)?;
    let mut list = instance.executables();
    let find = |list: &[strata_core::Executable], title: &str| list.iter().position(|e| e.title.eq_ignore_ascii_case(title));
    match action {
        ExecutableAction::List => {
            if global.json {
                let items: Vec<Value> = list
                    .iter()
                    .map(|e| json!({ "title": e.title, "binary": e.binary, "arguments": e.arguments,
                        "workingDirectory": e.working_directory, "steamAppID": e.steam_app_id,
                        "toolbar": e.toolbar, "ownicon": e.own_icon, "hide": e.hide }))
                    .collect();
                return print_json(&json!({ "executables": items }));
            }
            for e in &list {
                println!("{}{}\n    {} {}", e.title, if e.hide { "  (hidden)" } else { "" }, e.binary, e.arguments);
            }
            return Ok(());
        }
        ExecutableAction::Set { title, binary, arguments, working_dir, steam_app_id, toolbar, hide } => {
            let i = match find(&list, title) {
                Some(i) => i,
                None => {
                    let binary = binary.clone().context("a new executable needs --binary")?;
                    list.push(strata_core::Executable { title: title.clone(), binary, ..Default::default() });
                    list.len() - 1
                }
            };
            let e = &mut list[i];
            if let Some(v) = binary {
                e.binary.clone_from(v);
            }
            if let Some(v) = arguments {
                e.arguments.clone_from(v);
            }
            if let Some(v) = working_dir {
                e.working_directory.clone_from(v);
            }
            if let Some(v) = steam_app_id {
                e.steam_app_id.clone_from(v);
            }
            if let Some(v) = toolbar {
                e.toolbar = *v;
            }
            if let Some(v) = hide {
                e.hide = *v;
            }
            // Leet backlog: a binary in a mod's Root folder runs from the game folder
            if let Some(fixed) = instance.root_path_in_game(&e.binary) {
                eprintln!("{} is in a mod's Root folder; stored as {fixed}, where the overlay shows it", e.binary);
                e.binary = fixed;
            }
            if let Some(fixed) = instance.root_path_in_game(&e.working_directory) {
                e.working_directory = fixed;
            }
        }
        ExecutableAction::Remove { title } => {
            let i = find(&list, title).with_context(|| format!("no executable \"{title}\""))?;
            list.remove(i);
        }
        ExecutableAction::Move { title, to } => {
            let i = find(&list, title).with_context(|| format!("no executable \"{title}\""))?;
            let e = list.remove(i);
            list.insert((*to).min(list.len()), e);
        }
    }
    let changed = instance.store_executables(&list)?;
    if global.json {
        return print_json(&json!({ "changed": changed }));
    }
    println!("{}", if changed { "executables saved" } else { "nothing changed" });
    Ok(())
}

/// `strata link <url>`: what a link asks for. Downloading comes with M4.
fn link(global: &Global, url: &str) -> Result<()> {
    use strata_nexus::links::{self, Link};
    let parsed = links::parse(url)?;
    let value = match &parsed {
        Link::NexusFile { game, mod_id, file_id, expires, user_id, key } => json!({
            "kind": "nexus-file", "game": game, "mod_id": mod_id, "file_id": file_id,
            "expires": expires, "user_id": user_id, "has_key": key.is_some(),
        }),
        Link::NexusCollection { game, slug, revision } => {
            json!({ "kind": "nexus-collection", "game": game, "collection": slug, "revision": revision })
        }
        Link::SiteDownload { game, url, site } => json!({ "kind": "site-download", "game": game, "url": url, "site": site }),
    };
    if global.json {
        return print_json(&value);
    }
    for (key, v) in value.as_object().into_iter().flatten() {
        println!("{key:<10} {}", if let Value::String(s) = v { s.clone() } else { v.to_string() });
    }
    if !matches!(parsed, Link::NexusFile { .. }) {
        return tell("Strata can only download nxm:// links to a file (collections and other sites are not supported yet)");
    }
    hand_over(global, url).or_else(|e| tell(&format!("{e:#}")))
}

/// An error of `strata link`, which the browser runs out of sight: on stderr and, without a
/// terminal, as a desktop notification.
fn tell(message: &str) -> Result<()> {
    use std::io::IsTerminal;
    if !std::io::stderr().is_terminal() {
        let _ = std::process::Command::new("notify-send")
            .args(["--app-name=Strata", "Strata: the Nexus link was not downloaded", message])
            .status();
    }
    bail!("{message}")
}

/// Hands an nxm:// link to a running window (`strata_ui::handover`); when none takes it,
/// starts the window on the instance (`-i`, else the last one opened) and hands it over
/// once the window listens.
fn hand_over(global: &Global, url: &str) -> Result<()> {
    use strata_ui::handover::{Offered, link_game, offer};
    let reasons = match offer(url) {
        Offered::Taken(_) => {
            println!("handed to the open Strata window");
            return Ok(());
        }
        Offered::NotTaken(reasons) => reasons,
    };
    let config = strata_ui::config::config_file();
    let cwd = std::env::current_dir().unwrap_or_default();
    let Ok(root) = strata_ui::config::choose_instance(global.instance.as_deref(), &cwd, config.as_deref()) else {
        bail!("no Strata window took the link ({}) and no instance was opened before", reasons.join("; "));
    };
    let instance = strata_core::Instance::open(&root)?;
    let game = link_game(url).unwrap_or_default();
    if !instance.game.nexus_name().eq_ignore_ascii_case(&game) {
        bail!(
            "the link is for {game}; the last instance ({}) manages {}{}",
            root.display(),
            instance.game.nexus_name(),
            if reasons.is_empty() { String::new() } else { format!(" (open windows: {})", reasons.join("; ")) }
        );
    }
    let exe = strata_core::dirs::own_exe().context("cannot find Strata's own executable")?;
    let mut window = std::process::Command::new(&exe);
    window.arg("-i").arg(&root).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    std::os::unix::process::CommandExt::process_group(&mut window, 0);
    window.spawn().with_context(|| format!("cannot start {}", exe.display()))?;
    println!("started Strata on {}", root.display());
    // the window opens the instance first; then it listens
    for _ in 0..240 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if let Offered::Taken(_) = offer(url) {
            println!("handed to the new window");
            return Ok(());
        }
    }
    bail!("the Strata window started but did not take the link within a minute")
}

/// `strata register-links`: writes Strata's desktop entry for nxm:// and modl:// links,
/// and with --make-default makes it the handler (`xdg-mime default`). Never done without
/// being asked: another manager (Fluorine) may be the handler today.
fn register_links(global: &Global, make_default: bool) -> Result<()> {
    if let Ok(exe) = strata_core::dirs::own_exe()
        && exe.file_name().is_some_and(|n| n != "strata")
    {
        eprintln!("note: {} is not the installed `strata` binary; the entry points at it anyway", exe.display());
    }
    let file = register_link_handler(make_default)?;
    let made_default = make_default;
    if global.json {
        return print_json(&json!({ "desktop_file": file, "default": made_default }));
    }
    println!("wrote {}", file.display());
    if !made_default {
        println!("not made the default handler; run `strata register-links --make-default` for that");
    }
    Ok(())
}

/// Writes Strata's desktop entry for nxm:// and modl:// links (pointing at this executable)
/// and, with `make_default`, makes it the handler (`xdg-mime default`). Returns the entry's
/// path. `strata register-links` and the settings dialog's "Associate" button.
pub fn register_link_handler(make_default: bool) -> Result<std::path::PathBuf> {
    use strata_nexus::links::{DESKTOP_FILE, desktop_entry};
    let exe = strata_core::dirs::own_exe().context("cannot find Strata's own executable")?;
    let data = strata_core::dirs::data_home().context("neither XDG_DATA_HOME nor HOME is set")?;
    // xdg-mime checks a handler by the first word of its Exec line, quotes ignored: a path with
    // a space (this repository's) fails and it falls back to another handler. A link without
    // spaces to the program goes in Exec instead.
    let exe = if exe.to_string_lossy().contains(char::is_whitespace) {
        let link = data.join("strata/bin/strata");
        std::fs::create_dir_all(link.parent().unwrap_or(&data))?;
        if std::fs::read_link(&link).ok().as_deref() != Some(exe.as_path()) {
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(&exe, &link).with_context(|| format!("cannot link {}", link.display()))?;
        }
        link
    } else {
        exe
    };
    let file = data.join("applications").join(DESKTOP_FILE);
    std::fs::create_dir_all(file.parent().unwrap_or(&data))?;
    strata_core::write::write_if_changed(&file, desktop_entry(&exe.to_string_lossy()).as_bytes())?;
    if make_default {
        for scheme in ["x-scheme-handler/nxm", "x-scheme-handler/modl"] {
            let status = std::process::Command::new("xdg-mime").args(["default", DESKTOP_FILE, scheme]).status()
                .context("cannot run xdg-mime (package xdg-utils)")?;
            if !status.success() {
                bail!("xdg-mime default {DESKTOP_FILE} {scheme} failed");
            }
        }
    }
    Ok(file)
}
