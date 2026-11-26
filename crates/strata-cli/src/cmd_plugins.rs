//! The `plugins` area's subcommands: the plugin list (`strata list plugins`,
//! `strata plugins ...`).

use std::io::Write;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde::Serialize;
use strata_core::plugins::{AvailablePlugin, Plugin, PluginList};
use strata_core::{Instance, Profile};

use crate::Global;
use crate::commands::{open, print_json};

/// `strata plugins <action>`.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum PluginsAction {
    /// Enable plugins (primary plugins are always on).
    Enable {
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Disable plugins.
    Disable {
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Send plugins to a priority (0 = first), keeping masters above plugins and plugins
    /// below their masters, as MO2's "Send to priority" does.
    Move {
        #[arg(required = true)]
        names: Vec<String>,
        /// The priority; "bottom" for the end of the list.
        #[arg(long)]
        to: String,
    },
    /// Lock a plugin at its load order position (lockedorder.txt).
    Lock { name: String },
    /// Unlock a plugin.
    Unlock { name: String },
}

/// Opens the instance and builds its plugin list.
pub(crate) fn open_plugins(global: &Global) -> Result<(Instance, Profile, PluginList)> {
    let (instance, profile) = open(global)?;
    let tree = strata_ui::profile_tree::merged_tree(&instance, &profile)?;
    let available = strata_ui::profile_tree::available_plugins(&instance, &tree);
    let list = instance.plugin_list(&profile, available)?;
    Ok((instance, profile, list))
}

#[derive(Serialize)]
struct PluginJson<'a> {
    priority: usize,
    name: &'a str,
    enabled: bool,
    load_order: Option<usize>,
    index: &'a str,
    origin: &'a str,
    master: bool,
    light: bool,
    master_flag: bool,
    light_flag: bool,
    forced: bool,
    locked: Option<i64>,
    form_version: Option<u16>,
    masters: &'a [String],
    missing_masters: &'a [String],
    author: Option<&'a str>,
    error: Option<&'a str>,
}

fn to_json<'a>(list: &PluginList, p: &'a Plugin) -> PluginJson<'a> {
    PluginJson {
        priority: p.priority,
        name: &p.name,
        enabled: p.enabled,
        load_order: p.load_order,
        index: &p.index,
        origin: &p.origin,
        master: p.is_master_like(),
        light: p.is_light(),
        master_flag: p.is_master_flagged,
        light_flag: p.is_light_flagged,
        forced: p.force_loaded,
        locked: list.locked_position(&p.name),
        form_version: p.form_version(),
        masters: &p.masters,
        missing_masters: &p.missing_masters,
        author: p.header.as_ref().ok().map(|h| h.author.as_str()),
        error: p.header.as_ref().err().map(String::as_str),
    }
}

/// `strata list plugins`: every plugin by priority.
pub(crate) fn list(global: &Global) -> Result<()> {
    let (_, profile, list) = open_plugins(global)?;
    if global.json {
        let plugins: Vec<_> = list.plugins().map(|p| to_json(&list, p)).collect();
        return print_json(&serde_json::json!({ "profile": profile.name, "plugins": plugins }));
    }
    let mut out = std::io::stdout().lock();
    for p in list.plugins() {
        let mut notes = Vec::new();
        if list.locked_position(&p.name).is_some() {
            notes.push("locked".to_owned());
        }
        if !p.missing_masters.is_empty() {
            notes.push(format!("missing masters: {}", p.missing_masters.join(", ")));
        }
        if let Err(e) = &p.header {
            notes.push(e.clone());
        }
        let notes = if notes.is_empty() { String::new() } else { format!("  ({})", notes.join("; ")) };
        writeln!(
            out,
            "{:>4} {} {:<6} {}{}",
            p.priority,
            if p.enabled { '+' } else { '-' },
            p.index,
            p.name,
            notes
        )?;
    }
    Ok(())
}

/// `strata plugins <action>`: change the list and write `plugins.txt`, `loadorder.txt` and
/// `lockedorder.txt` (only the files whose content changed).
pub(crate) fn run(global: &Global, action: &PluginsAction) -> Result<()> {
    let (_, profile, mut list) = open_plugins(global)?;
    let mut changed = Vec::new();
    match action {
        PluginsAction::Enable { names } => changed = report(global, list.set_enabled(names, true)?, "enabled"),
        PluginsAction::Disable { names } => changed = report(global, list.set_enabled(names, false)?, "disabled"),
        PluginsAction::Move { names, to } => {
            let priority = if to.eq_ignore_ascii_case("bottom") {
                usize::MAX
            } else {
                to.parse().with_context(|| format!("--to takes a number or \"bottom\", not \"{to}\""))?
            };
            list.send_to_priority(names, priority)?;
        }
        PluginsAction::Lock { name } => list.set_locked(name, true)?,
        PluginsAction::Unlock { name } => list.set_locked(name, false)?,
    }
    let written = list.save(&profile.dir)?;
    if global.json {
        return print_json(&serde_json::json!({ "profile": profile.name, "changed": changed, "written": written }));
    }
    for path in written {
        eprintln!("wrote {}", path.display());
    }
    Ok(())
}

/// `strata sort`: LOOT's order for the plugin list, as MO2's "Sort" button gets it from
/// lootcli; then the list is read again from the new `loadorder.txt` (MO2's refresh after a
/// sort, which reapplies its own rules) and the three files are written.
pub(crate) fn sort(global: &Global, dry_run: bool, offline: bool) -> Result<()> {
    let (instance, profile, list) = open_plugins(global)?;
    let loot_name = instance.game.loot_name();
    let dir = strata_loot::data_dir(loot_name);
    let masterlist = dir.join("masterlist.yaml");
    if !offline {
        match strata_loot::update_masterlist(loot_name, &dir) {
            Ok(()) => eprintln!("masterlist updated in {}", dir.display()),
            Err(e) if masterlist.is_file() => eprintln!("warning: {e}; using the copy from before"),
            Err(e) => return Err(e.into()),
        }
    }
    // The user's LOOT userlist lives in the prefix, where lootcli would read it under Proton.
    let userlist = instance.proton.prefix.as_ref().map(|p| {
        p.join("pfx/drive_c/users/steamuser/AppData/Local/LOOT/games")
            .join(instance.game.appdata_dir())
            .join("userlist.yaml")
    });
    // the merged game folder through the overlay, case-insensitive as lootcli in MO2's VFS
    // (strata_ui::loot_view): the live deploy, or a private mount until `game` is dropped
    let game = strata_ui::loot_view::LootView::open(&instance, &profile)?;
    if game.how == strata_ui::loot_view::How::NoFuse {
        eprintln!("warning: {}", game.how);
    }
    let input = strata_loot::SortInput {
        loot_name: loot_name.to_owned(),
        game_path: game.game_path.clone(),
        local_path: profile.dir.clone(),
        data_paths: game.data_paths.clone(),
        plugins: list.plugins().map(|p| (p.name.clone(), p.path.clone())).collect(),
        masterlist,
        prelude: Some(dir.join("prelude.yaml")),
        userlist,
    };
    let strata_loot::Sorted { order: sorted, mut report } = strata_loot::sort_with_report(&input, std::sync::Arc::new(strata_loot::Silent))?;
    drop(game);

    // lootcli writes the sorted list over loadorder.txt; MO2 then refreshes the list from it.
    let mut loadorder = format!("{}\r\n", strata_core::MODLIST_HEADER);
    for name in &sorted {
        loadorder.push_str(name);
        loadorder.push_str("\r\n");
    }
    let rules = instance.plugin_rules();
    let files = strata_core::plugins::PluginFiles {
        plugins_txt: Some(list.plugins_txt()),
        loadorder_txt: Some(loadorder.into_bytes()),
        lockedorder_txt: Some(list.lockedorder_txt()),
    };
    let available = list.plugins().map(|p| AvailablePlugin { name: p.name.clone(), origin: p.origin.clone(), path: p.path.clone() });
    let new = PluginList::load(available.collect(), rules, &files);

    let before: Vec<&str> = list.plugins().map(|p| p.name.as_str()).collect();
    let after: Vec<&str> = new.plugins().map(|p| p.name.as_str()).collect();
    let moved: Vec<(usize, &str)> = after.iter().enumerate().filter(|(i, n)| before.get(*i) != Some(*n)).map(|(i, n)| (i, *n)).collect();
    if global.json {
        // the LOOT dialog's report: lootcli's, for the enabled plugins (MO2's Loot::reportPlugin)
        report.plugins.retain(|p| new.plugins().any(|q| q.enabled && q.name.eq_ignore_ascii_case(&p.name)));
        print_json(&serde_json::json!({
            "profile": profile.name, "sorted": after, "changed": !moved.is_empty(), "dry_run": dry_run, "report": report,
        }))?;
    } else if moved.is_empty() {
        println!("the load order is already sorted");
    } else {
        for (i, name) in &moved {
            println!("{i:>4} {name}");
        }
    }
    if !dry_run {
        for path in new.save(&profile.dir)? {
            eprintln!("wrote {}", path.display());
        }
    }
    Ok(())
}

/// `strata list saves`: the profile's saves, newest first, with the plugins each one misses.
pub(crate) fn list_saves(global: &Global) -> Result<()> {
    let (instance, profile, list) = open_plugins(global)?;
    let saves = instance.saves(&profile, true)?;
    let mut rows = Vec::new();
    for entry in &saves {
        let file = entry.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let row = match &entry.save {
            Ok(save) => {
                let missing = instance.missing_plugins(&profile, &list, save)?;
                serde_json::json!({
                    "file": file,
                    "name": save.display_name(),
                    "player": save.player_name,
                    "level": save.player_level,
                    "location": save.location,
                    "save_number": save.save_number,
                    "created": utc(save.created),
                    "plugins": save.plugins,
                    "light_plugins": save.light_plugins,
                    "screenshot": [save.screenshot.width, save.screenshot.height],
                    "cosave": entry.has_cosave,
                    "missing": missing.iter().map(|m| serde_json::json!({"plugin": m.name, "providers": m.providers})).collect::<Vec<_>>(),
                })
            }
            Err(e) => serde_json::json!({ "file": file, "error": e }),
        };
        rows.push(row);
    }
    if global.json {
        let dir = instance.saves_dir(&profile);
        return print_json(&serde_json::json!({ "profile": profile.name, "folder": dir, "saves": rows }));
    }
    let mut out = std::io::stdout().lock();
    if saves.is_empty() {
        let dir = instance.saves_dir(&profile).map_or("(no prefix set)".into(), |d| d.display().to_string());
        writeln!(out, "no saves in {dir}")?;
    }
    for row in rows {
        let text = |k: &str| row[k].as_str().unwrap_or_default().to_owned();
        if let Some(e) = row["error"].as_str() {
            writeln!(out, "{}  (unreadable: {e})", text("file"))?;
            continue;
        }
        writeln!(out, "{}  {}  {}", text("created"), text("name"), text("file"))?;
        for m in row["missing"].as_array().into_iter().flatten() {
            let providers: Vec<&str> = m["providers"].as_array().into_iter().flatten().filter_map(|p| p.as_str()).collect();
            let from = if providers.is_empty() { "in no mod".to_owned() } else { format!("in {}", providers.join(", ")) };
            writeln!(out, "    missing {} ({from})", m["plugin"].as_str().unwrap_or_default())?;
        }
    }
    Ok(())
}

/// Unix seconds as `YYYY-MM-DD HH:MM:SS UTC` (days to civil date: Howard Hinnant's
/// algorithm).
fn utc(secs: i64) -> String {
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC", rest / 3600, rest / 60 % 60, rest % 60)
}

/// `strata problems`: MO2's Problems dialog (the built-in diagnose plugins).
pub(crate) fn problems(global: &Global, fix: Option<&str>) -> Result<()> {
    let (instance, profile) = open(global)?;
    if let Some(id) = fix {
        let trash = strata_core::Trash::home()?;
        let done = strata_ui::modlist::problems::fix(&instance, &profile, id, &trash)?;
        if global.json {
            return print_json(&serde_json::json!({ "profile": profile.name, "fixed": id, "done": done }));
        }
        println!("{done}");
        return Ok(());
    }
    let problems = strata_ui::modlist::problems::collect(&instance, &profile)?;
    if global.json {
        let list: Vec<_> = problems
            .iter()
            .map(|r| {
                let p = &r.problem;
                serde_json::json!({ "id": p.id, "summary": p.summary, "details": p.details, "fix": r.fix })
            })
            .collect();
        return print_json(&serde_json::json!({ "profile": profile.name, "problems": list }));
    }
    let mut out = std::io::stdout().lock();
    if problems.is_empty() {
        writeln!(out, "no problems")?;
    }
    for r in &problems {
        writeln!(out, "{}", r.problem.summary)?;
        for d in &r.problem.details {
            writeln!(out, "    {d}")?;
        }
        if let Some(fix) = r.fix {
            writeln!(out, "    fix: {fix} (strata problems --fix {})", r.problem.id)?;
        }
    }
    Ok(())
}

/// Says which plugins changed (unless `--json`, which prints them with the result).
fn report(global: &Global, changed: Vec<String>, what: &str) -> Vec<String> {
    if !global.json {
        if changed.is_empty() {
            eprintln!("nothing {what}: already so, or a primary plugin");
        }
        for name in &changed {
            eprintln!("{what} {name}");
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates_print_in_utc() {
        assert_eq!(super::utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(super::utc(1_793_583_024), "2026-11-02 01:30:24 UTC");
        assert_eq!(super::utc(951_782_400), "2000-02-29 00:00:00 UTC");
    }
}
