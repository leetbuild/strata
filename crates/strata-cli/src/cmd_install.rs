//! `strata install <archive>`: the install pipeline of `strata-install`, with the dialogs'
//! answers taken from the command line (`--choices`, `--merge`/`--replace`/`--rename`), then
//! the mod added to the profile's `modlist.txt` as MO2 does (top of the list, enabled when
//! `[Settings] auto_enable_install` is on, Leet change 11).

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use strata_install::archive::SevenZip;
use strata_install::checker::DataChecker;
use strata_install::fomod::Info;
use strata_install::fomod::session::Session;
use strata_install::install::{Answer, Clash, ClashAction, InstallUi};
use strata_install::instance::{Options, install_into, preview_into, reinstall_archive};
use strata_install::name::GuessedName;
use strata_install::rootsorter::Plan;
use strata_install::tree::FileTree;

use crate::Global;
use crate::commands::{open, print_json};

/// What `--choices` may hold. Every part is optional; what is left out takes the dialog's
/// defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Choices {
    /// FOMOD: step name → group name → the plugin names to tick (all others unticked).
    #[serde(default)]
    fomod: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// BAIN: the package folders to install.
    bain: Option<Vec<String>>,
    /// Bundle: the inner archive to install (its path in the archive).
    bundle: Option<String>,
    /// Manual installer: the folder of the archive to install as Data (`""` = the top).
    manual: Option<String>,
}

/// The CLI's options for one install.
#[derive(Debug, Clone, PartialEq, Eq, Default, clap::Args)]
pub(crate) struct InstallArgs {
    /// The archive (.7z, .zip, .rar, .fomod, .001). Not needed with --reinstall.
    #[arg(required_unless_present = "reinstall")]
    pub archive: Option<PathBuf>,
    /// Reinstall this mod from its installation file (MO2's "Reinstall Mod"): the installers
    /// offer its previous options (BAIN packages, the bundle's archive, FOMOD choices). Add
    /// --merge or --replace for the existing files.
    #[arg(long, value_name = "MOD")]
    pub reinstall: Option<String>,
    /// The archive's password, when it has one.
    #[arg(long, value_name = "PASSWORD")]
    pub password: Option<String>,
    /// The mod's name (otherwise guessed from the download's metadata and the file name).
    #[arg(long)]
    pub name: Option<String>,
    /// Dialog answers as JSON, or @file: {"fomod": {"Step": {"Group": ["Plugin"]}},
    /// "bain": ["00 Core"], "bundle": "inner.7z", "manual": "Data folder"}. Unset parts take
    /// the defaults.
    #[arg(long, value_name = "JSON")]
    pub choices: Option<String>,
    /// A mod of that name exists: install over its files.
    #[arg(long, conflicts_with_all = ["replace", "rename"])]
    pub merge: bool,
    /// A mod of that name exists: move its files to the trash first (its meta.ini is kept).
    #[arg(long, conflicts_with = "rename")]
    pub replace: bool,
    /// A mod of that name exists: install under this name instead.
    #[arg(long, value_name = "NAME")]
    pub rename: Option<String>,
    /// With --merge or --replace: copy the existing mod to <name>_backup first.
    #[arg(long)]
    pub backup: bool,
    /// Don't move game-folder files (dll, exe, ENB files) into the mod's Root folder.
    #[arg(long)]
    pub no_root_sort: bool,
    /// Leave the new mod disabled (default: the instance's "auto-enable installed mods").
    #[arg(long)]
    pub no_enable: bool,
    /// BSA Extractor: unpack the mod's BSAs when it asks (MO2's default answer is No).
    #[arg(long)]
    pub extract_bsa: bool,
    /// BSA Extractor: remove the BSAs after unpacking.
    #[arg(long)]
    pub remove_bsa: bool,
    /// Put the new mod at the end of this separator ("Install to Separator").
    #[arg(long, value_name = "SEPARATOR")]
    pub under: Option<String>,
    /// Change nothing: show the installer, every FOMOD step with its options (ticked ones
    /// marked [x]) and the files the mod would get.
    #[arg(long)]
    pub plan: bool,
}

fn parse_choices(text: Option<&str>) -> Result<Choices> {
    let Some(text) = text else { return Ok(Choices::default()) };
    let json = match text.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path).with_context(|| format!("cannot read {path}"))?,
        None => text.to_owned(),
    };
    serde_json::from_str(&json).context("--choices is not valid: see `strata install --help` for its shape")
}

/// Answers from the command line. Questions it cannot answer fail with a message saying
/// which option answers them.
struct CliUi {
    choices: Choices,
    args: InstallArgs,
    used_steps: HashSet<String>,
    asked_clash: bool,
    error: Option<anyhow::Error>,
    log: Vec<String>,
}

impl CliUi {
    fn fail<T>(&mut self, error: anyhow::Error, value: T) -> T {
        self.error.get_or_insert(error);
        value
    }

    fn apply_step(&mut self, session: &mut Session) -> Result<()> {
        let view = session.current().ok_or_else(|| anyhow!("no FOMOD step"))?;
        let Some(groups) = self.choices.fomod.get(&view.name) else { return Ok(()) };
        self.used_steps.insert(view.name.clone());
        for (group_name, wanted) in groups {
            let (g, group) = view
                .groups
                .iter()
                .enumerate()
                .find(|(_, x)| &x.name == group_name)
                .ok_or_else(|| anyhow!("FOMOD step '{}' has no group '{group_name}'", view.name))?;
            for w in wanted {
                if !group.plugins.iter().any(|p| &p.name == w) {
                    let names: Vec<_> = group.plugins.iter().map(|p| p.name.as_str()).collect();
                    bail!("group '{group_name}' has no plugin '{w}' (it has: {})", names.join(", "));
                }
            }
            if group.kind.is_radio() && wanted.len() > 1 {
                bail!("group '{group_name}' takes one plugin at most");
            }
            // tick first, then untick: radio buttons only allow switching
            for (p, plugin) in group.plugins.iter().enumerate() {
                if wanted.contains(&plugin.name) && !plugin.checked {
                    session.set(g, p, true)?;
                }
            }
            let now = session.current().ok_or_else(|| anyhow!("no FOMOD step"))?;
            for (p, plugin) in now.groups[g].plugins.iter().enumerate() {
                if !wanted.contains(&plugin.name) && plugin.checked {
                    session.set(g, p, false)?;
                }
            }
        }
        Ok(())
    }
}

impl InstallUi for CliUi {
    fn quick(&mut self, _name: &mut GuessedName) -> Answer<()> {
        Answer::Accept(())
    }

    fn bain_unsure(&mut self) -> bool {
        // MO2 asks; the CLI says yes only when BAIN choices were given
        self.choices.bain.is_some()
    }

    fn bain(&mut self, _name: &mut GuessedName, packages: &[String], defaults: &[bool]) -> Answer<Vec<String>> {
        let picked = match &self.choices.bain {
            Some(list) => list.clone(),
            None => packages.iter().zip(defaults).filter(|(_, d)| **d).map(|(p, _)| p.clone()).collect(),
        };
        if let Some(bad) = picked.iter().find(|p| !packages.iter().any(|x| x.eq_ignore_ascii_case(p))) {
            let e = anyhow!("BAIN package '{bad}' is not in the archive (it has: {})", packages.join(", "));
            return self.fail(e, Answer::Cancel);
        }
        self.log.push(format!("BAIN packages: {}", picked.join(", ")));
        Answer::Accept(picked)
    }

    fn bundle(&mut self, archives: &[String]) -> Answer<String> {
        match &self.choices.bundle {
            Some(pick) if archives.contains(pick) => Answer::Accept(pick.clone()),
            _ => {
                let e = anyhow!(
                    "the archive holds several archives; pick one with --choices '{{\"bundle\": \"...\"}}': {}",
                    archives.join(", ")
                );
                self.fail(e, Answer::Cancel)
            }
        }
    }

    fn fomod(&mut self, _name: &mut GuessedName, _info: &Info, session: &mut Session, _images: &std::path::Path) -> Answer<()> {
        loop {
            if session.current().is_none() {
                break;
            }
            if let Err(e) = self.apply_step(session) {
                return self.fail(e, Answer::Cancel);
            }
            if let Some(view) = session.current().filter(|_| self.args.plan) {
                self.log.push(format!("FOMOD step '{}':", view.name));
                for g in &view.groups {
                    self.log.push(format!("  group '{}' ({:?})", g.name, g.kind));
                    for p in &g.plugins {
                        let mark = if p.checked { "[x]" } else { "[ ]" };
                        let lock = if p.enabled { "" } else { " (fixed)" };
                        self.log.push(format!("    {mark} {} ({:?}){lock}", p.name, p.kind));
                    }
                }
            } else if let Some(view) = session.current() {
                let picked: Vec<String> = view
                    .groups
                    .iter()
                    .flat_map(|g| g.plugins.iter().filter(|p| p.checked).map(move |p| format!("{}/{}", g.name, p.name)))
                    .collect();
                self.log.push(format!("FOMOD step '{}': {}", view.name, picked.join(", ")));
            }
            match session.next_step() {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => return self.fail(e.into(), Answer::Cancel),
            }
        }
        let unused: Vec<_> = self.choices.fomod.keys().filter(|k| !self.used_steps.contains(*k)).cloned().collect();
        if !unused.is_empty() {
            let e = anyhow!("--choices names FOMOD steps that were not shown: {}", unused.join(", "));
            return self.fail(e, Answer::Cancel);
        }
        Answer::Accept(())
    }

    fn password(&mut self, archive: &str, wrong: bool) -> Option<String> {
        match (&self.args.password, wrong) {
            (Some(p), false) => Some(p.clone()),
            (Some(_), true) => self.fail(anyhow!("the password given with --password does not open {archive}"), None),
            (None, _) => self.fail(anyhow!("{archive} is password protected: add --password <password>"), None),
        }
    }

    fn fomod_missing(&mut self, missing: &[String]) -> bool {
        // MO2 asks "Install anyway"; the CLI installs and says so
        self.log.push(format!("warning: the FOMOD names files the archive lacks: {}", missing.join(", ")));
        true
    }

    fn manual(&mut self, _name: &mut GuessedName, _tree: &mut FileTree, _checker: &DataChecker) -> Answer<String> {
        match &self.choices.manual {
            Some(root) => Answer::Accept(root.clone()),
            None => {
                let e = anyhow!(
                    "no installer recognises this archive's layout; install it from the window, or name the folder \
                     to use as Data with --choices '{{\"manual\": \"<folder>\"}}'"
                );
                self.fail(e, Answer::Cancel)
            }
        }
    }

    fn root_sort(&mut self, _mod_name: &str, plan: &Plan) -> bool {
        if self.args.no_root_sort {
            return false;
        }
        self.log.push(format!("Root: {}", plan.to_root.join(", ")));
        true
    }

    fn question(&mut self, id: &str, _title: &str, _text: &str, default_yes: bool) -> (bool, bool) {
        let yes = match id {
            "unpackBSA" => self.args.extract_bsa,
            "removeUnpackedBSA" => self.args.remove_bsa,
            _ => default_yes,
        };
        (yes, false)
    }

    fn clash(&mut self, mod_name: &str) -> Clash {
        let backup = self.args.backup;
        let action = match (&self.args.rename, self.args.merge, self.args.replace) {
            (Some(new), ..) if !self.asked_clash => ClashAction::Rename(new.clone()),
            (None, true, _) => ClashAction::Merge,
            (None, _, true) => ClashAction::Replace,
            _ => {
                let e = anyhow!("a mod named '{mod_name}' exists: add --merge, --replace or --rename <name>");
                self.error.get_or_insert(e);
                ClashAction::Cancel
            }
        };
        self.asked_clash = true;
        Clash { action, backup }
    }
}

/// What `strata downloads` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum DownloadAction {
    /// Hide from the Downloads tab (`removed=true` in the .meta).
    Hide,
    /// Show again (`removed=false`).
    Unhide,
    /// Move the archive and its .meta to the trash (`--permanently`: delete for good).
    Delete,
    /// Mark installed (`installed=true`, `uninstalled=false`).
    Installed,
    /// Mark uninstalled (`uninstalled=true`).
    Uninstalled,
    /// Store the mod id and name the file name gives (Nexus is not asked).
    Query,
    /// Install the archives one after the other with the defaults.
    Install,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub(crate) struct DownloadsArgs {
    #[arg(value_enum)]
    pub action: DownloadAction,
    /// Archive files (a bare name is looked up in the downloads folder).
    pub archives: Vec<PathBuf>,
    /// With delete: delete for good instead of moving to the trash.
    #[arg(long)]
    pub permanently: bool,
    /// With install: put the mods at the end of this separator.
    #[arg(long, value_name = "SEPARATOR")]
    pub under: Option<String>,
}

pub(crate) fn downloads_cmd(global: &Global, args: &DownloadsArgs) -> Result<()> {
    use strata_install::downloads as dl;
    let (instance, profile) = open(global)?;
    let dir = &instance.paths.downloads;
    let resolve = |p: &PathBuf| if p.components().count() == 1 && !p.exists() { dir.join(p) } else { p.clone() };
    #[allow(unused_mut)]
    let mut archives: Vec<PathBuf> = args.archives.iter().map(resolve).collect();
    if archives.is_empty() {
        bail!("name the archives to act on");
    }
    let mut failed = 0;
    let mut deployed_change = false;
    for archive in &archives {
        let shown = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let result: Result<String> = (|| {
            #[allow(unused_mut)]
            let mut must_exist = true;
            if must_exist && !archive.is_file() {
                bail!("{} does not exist", archive.display());
            }
            Ok(match args.action {
                DownloadAction::Hide => dl::set_hidden(archive, true).map(|()| "hidden".to_owned())?,
                DownloadAction::Unhide => dl::set_hidden(archive, false).map(|()| "shown".to_owned())?,
                DownloadAction::Installed => strata_install::meta::mark_download_installed(archive).map(|()| "marked installed".to_owned())?,
                DownloadAction::Uninstalled => dl::mark_uninstalled(archive).map(|()| "marked uninstalled".to_owned())?,
                DownloadAction::Delete => {
                    let trash = if args.permanently { None } else { Some(strata_core::Trash::home()?) };
                    dl::delete(archive, trash.as_ref())?;
                    if args.permanently { "deleted".into() } else { "moved to the trash".into() }
                }
                DownloadAction::Query => {
                    let q = dl::query_info_offline(archive, "SkyrimSE")?;
                    match q.mod_id {
                        Some(id) => format!("mod id {id}, name '{}'", q.mod_name),
                        None => format!("no mod id in the file name; name '{}'", q.mod_name),
                    }
                }
                DownloadAction::Install => {
                    let mut ui = CliUi {
                        choices: Choices::default(),
                        args: InstallArgs { archive: Some(archive.clone()), ..InstallArgs::default() },
                        used_steps: HashSet::new(),
                        asked_clash: false,
                        error: None,
                        log: Vec::new(),
                    };
                    let options = Options { under: args.under.clone(), from_downloads: true, ..Options::default() };
                    let done = install_into(&instance, &profile, &SevenZip, &mut ui, &AtomicBool::new(false), archive, &options);
                    let done = match (done, ui.error.take()) {
                        (Ok(d), _) => d,
                        (Err(_), Some(why)) => return Err(why),
                        (Err(e), None) => return Err(e.into()),
                    };
                    deployed_change = true;
                    format!("installed as '{}' ({:?} installer)", done.outcome.name, done.outcome.installer)
                }
            })
        })();
        match result {
            Ok(line) if line.is_empty() => {}
            Ok(line) => println!("{shown}: {line}"),
            Err(e) => {
                failed += 1;
                println!("{shown}: {e:#}");
            }
        }
    }
    if deployed_change && let Some(line) = crate::cmd_modlist::refresh_deploy(global, &profile.name)? {
        println!("{line}");
    }
    if failed > 0 {
        bail!("{failed} of {} failed", archives.len());
    }
    Ok(())
}

/// Sends the profile's layers to a live deploy of the instance in `root` (after an install
/// from the window). Returns a line for the user when something was or could not be
/// refreshed; `None` when nothing is deployed.
pub fn refresh_live_deploy(root: &std::path::Path, profile: &str) -> Result<Option<String>> {
    let global = Global { instance: Some(root.to_owned()), profile: Some(profile.to_owned()), ..Global::default() };
    crate::cmd_modlist::refresh_deploy(&global, profile)
}

/// Installs a finished download with the CLI's default answers (`installDownload`: an
/// existing mod with the same Nexus id is the one updated).
pub(crate) fn install_downloaded(
    instance: &strata_core::Instance,
    profile: &strata_core::Profile,
    args: &InstallArgs,
) -> Result<strata_install::instance::Installed> {
    let archive = args.archive.clone().ok_or_else(|| anyhow!("no archive"))?;
    let mut ui = CliUi {
        choices: parse_choices(args.choices.as_deref())?,
        args: args.clone(),
        used_steps: HashSet::new(),
        asked_clash: false,
        error: None,
        log: Vec::new(),
    };
    let options = Options { under: args.under.clone(), from_downloads: true, ..Options::default() };
    match (install_into(instance, profile, &SevenZip, &mut ui, &AtomicBool::new(false), &archive, &options), ui.error.take()) {
        (Ok(d), _) => Ok(d),
        (Err(_), Some(why)) => Err(why),
        (Err(e), None) => Err(e.into()),
    }
}

pub(crate) fn install_cmd(global: &Global, args: &InstallArgs) -> Result<()> {
    let (instance, profile) = open(global)?;
    let choices = parse_choices(args.choices.as_deref())?;
    let archive = match (&args.archive, &args.reinstall) {
        // absolute, not canonical: a symlink in the downloads folder stays a download there
        // (its .meta goes next to the link), as QFileInfo::absoluteFilePath keeps it
        (Some(a), _) if a.exists() => std::path::absolute(a).with_context(|| format!("cannot find {}", a.display()))?,
        (Some(a), _) => bail!("cannot find {}", a.display()),
        (None, Some(m)) => reinstall_archive(&instance, m)?,
        (None, None) => bail!("give an archive or --reinstall <mod>"),
    };
    let options = Options {
        name: args.name.clone(),
        enable: args.no_enable.then_some(false),
        under: args.under.clone(),
        reinstall: args.reinstall.clone(),
        from_downloads: false,
    };
    let mut ui = CliUi {
        choices,
        args: args.clone(),
        used_steps: HashSet::new(),
        asked_clash: false,
        error: None,
        log: Vec::new(),
    };
    if args.plan {
        let result = preview_into(&instance, &profile, &SevenZip, &mut ui, &archive, &options);
        let p = match (result, ui.error.take()) {
            (Ok(p), _) => p,
            (Err(_), Some(why)) => return Err(why),
            (Err(e), None) => return Err(e.into()),
        };
        if global.json {
            return print_json(&serde_json::json!({ "plan": p, "log": ui.log }));
        }
        for line in &ui.log {
            println!("{line}");
        }
        let clash = if p.exists { " (a mod of that name exists)" } else { "" };
        println!("would install '{}'{clash} with the {:?} installer:", p.name, p.installer);
        for f in &p.files {
            println!("  {f}");
        }
        return Ok(());
    }
    let result = install_into(&instance, &profile, &SevenZip, &mut ui, &AtomicBool::new(false), &archive, &options);
    let done = match (result, ui.error.take()) {
        (Ok(o), _) => o,
        (Err(_), Some(why)) => return Err(why),
        (Err(e), None) => return Err(e.into()),
    };
    let deploy = crate::cmd_modlist::refresh_deploy(global, &profile.name)?;

    if global.json {
        return print_json(&serde_json::json!({
            "installed": done, "profile": profile.name, "log": ui.log, "deploy": deploy
        }));
    }
    for line in &ui.log {
        println!("{line}");
    }
    let outcome = &done.outcome;
    let how = if outcome.merged {
        "merged into"
    } else if outcome.replaced {
        "replaced"
    } else {
        "installed as"
    };
    println!(
        "{how} '{}' ({:?} installer), {} at priority {} in profile '{}'",
        outcome.name,
        outcome.installer,
        if done.enabled { "enabled" } else { "disabled" },
        done.priority,
        profile.name
    );
    if let Some(b) = &done.bsa {
        match &b.error {
            Some(e) => println!("BSA extraction failed: {e}"),
            None => println!("unpacked {}{}", b.archives.join(", "), if b.removed { " (removed)" } else { "" }),
        }
    }
    if let Some(b) = &outcome.backup {
        println!("backup: {}", b.display());
    }
    for note in &outcome.notes {
        println!("note: {note}");
    }
    if outcome.ini_tweaks {
        println!("the mod has INI tweaks (its 'INI Tweaks' folder): pick them in the mod's information, INI Files");
    }
    if let Some(t) = &outcome.trashed {
        println!("old files moved to the trash: {}", t.display());
    }
    if let Some(line) = deploy {
        println!("{line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_parse_and_reject_unknown_keys() {
        let c = parse_choices(Some(r#"{"fomod": {"Step": {"Group": ["A"]}}, "bain": ["00 Core"]}"#)).unwrap();
        assert_eq!(c.fomod["Step"]["Group"], ["A"]);
        assert_eq!(c.bain.unwrap(), ["00 Core"]);
        assert!(parse_choices(Some(r#"{"fomood": {}}"#)).is_err());
        assert!(parse_choices(None).unwrap().fomod.is_empty());
    }
}
