//! The installer dialogs' side of an install: `strata_install`'s questions as JSON messages
//! to a [`Host`] (the window's dialogs, through `strata-ffi`'s `bridge_install.rs`).
//!
//! One method carries every question, `ask(kind, request) -> answer`, both JSON objects, so
//! the bridge stays two functions wide and each dialog's contract is written down here and
//! tested without Qt. `"action"` in an answer is `"ok"`, `"manual"` or `"cancel"` where
//! the dialog has those buttons; `"name"` is the name field when the dialog has one.
//!
//! | kind | request | answer |
//! |---|---|---|
//! | `quick` | `{name, variants}` | `{action, name}` |
//! | `bain_unsure` | `{}` | `{yes}` |
//! | `bain` | `{name, variants, packages: [{name, checked}]}` | `{action, name, selected: [..]}` |
//! | `bundle` | `{archives: [..]}` | `{action, pick}` |
//! | `fomod` | `{name, variants, info: {name, author, version, website}, step: StepView or null, can_back, error}` | `{action: "set" + group, plugin, on / "next" / "back" / "manual" / "cancel", name}` |
//! | `fomod_missing` | `{missing: [..]}` | `{yes}` |
//! | `manual` | `{name, variants, paths: [..], valid}` | `{action, name, data_root, removed: [..]}` |
//! | `root_sort` | `{mod_name, to_root, dropped, unsure, data_split, base}` | `{move}` |
//! | `clash` | `{name}` | `{action: "merge" / "replace" / "rename" / "cancel", name, backup}` |
//! | `question` | `{id, title, text, default}` | `{yes, remember}` (MO2's `QuestionBoxMemory`) |
//! | `password` | `{archive, wrong}` | `{action, password}` (`queryPassword`) |
//! | `ini_tweaks` | `{name}` | `{yes}` ("This mod contains ini tweaks...") |
//!
//! The FOMOD dialog is asked again after every click: the answer `set` ticks or unticks one
//! plugin, and the next request shows the step as it is now (plugin types, the Next / Install
//! label: conditions are live). `next` on a step whose `is_last` is true installs; the dialog
//! closes itself then (and on `manual` / `cancel`).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};
use strata_core::{Instance, Profile};
use strata_install::archive::SevenZip;
use strata_install::checker::DataChecker;
use strata_install::fomod::Info;
use strata_install::fomod::session::Session;
use strata_install::install::{Answer, Clash, ClashAction, InstallUi};
use strata_install::instance::{Installed, Options, install_into};
use strata_install::name::{GuessedName, Quality};
use strata_install::rootsorter::Plan;
use strata_install::tree::FileTree;

/// The dialogs.
pub trait Host {
    /// Shows the dialog `kind` with `request` and returns the user's answer.
    fn ask(&mut self, kind: &str, request: &Value) -> Value;
    /// Extraction progress; returns false when the user cancelled.
    fn progress(&mut self, percent: u8) -> bool;
}

/// [`InstallUi`] over a [`Host`].
pub struct HostUi<'a> {
    host: &'a mut dyn Host,
    cancel: &'a AtomicBool,
}

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").to_owned()
}

fn action(v: &Value) -> &str {
    v["action"].as_str().unwrap_or("cancel")
}

fn named(name: &GuessedName) -> Value {
    json!({ "name": name.value(), "variants": name.variants() })
}

fn take_name(answer: &Value, name: &mut GuessedName) {
    if let Some(n) = answer["name"].as_str().filter(|n| !n.trim().is_empty()) {
        name.update(n, Quality::User);
    }
}

fn answer<T>(answer: &Value, name: &mut GuessedName, ok: impl FnOnce(&Value) -> T) -> Answer<T> {
    match action(answer) {
        "ok" => {
            take_name(answer, name);
            Answer::Accept(ok(answer))
        }
        "manual" => {
            take_name(answer, name);
            Answer::Manual
        }
        _ => Answer::Cancel,
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_owned)).collect()
}

impl<'a> HostUi<'a> {
    pub fn new(host: &'a mut dyn Host, cancel: &'a AtomicBool) -> Self {
        Self { host, cancel }
    }
}

impl InstallUi for HostUi<'_> {
    fn quick(&mut self, name: &mut GuessedName) -> Answer<()> {
        let a = self.host.ask("quick", &named(name));
        answer(&a, name, |_| ())
    }

    fn bain_unsure(&mut self) -> bool {
        self.host.ask("bain_unsure", &json!({}))["yes"].as_bool().unwrap_or(false)
    }

    fn bain(&mut self, name: &mut GuessedName, packages: &[String], defaults: &[bool]) -> Answer<Vec<String>> {
        let mut req = named(name);
        req["packages"] = packages.iter().zip(defaults).map(|(p, d)| json!({ "name": p, "checked": d })).collect();
        let a = self.host.ask("bain", &req);
        answer(&a, name, |a| strings(&a["selected"]))
    }

    fn bundle(&mut self, archives: &[String]) -> Answer<String> {
        let a = self.host.ask("bundle", &json!({ "archives": archives }));
        match action(&a) {
            "ok" if archives.contains(&text(&a, "pick")) => Answer::Accept(text(&a, "pick")),
            "manual" => Answer::Manual,
            _ => Answer::Cancel,
        }
    }

    fn fomod(&mut self, name: &mut GuessedName, info: &Info, session: &mut Session, images: &Path) -> Answer<()> {
        let info = json!({ "name": info.name, "author": info.author, "version": info.version, "website": info.website });
        let mut error = String::new();
        loop {
            let mut req = named(name);
            req["info"] = info.clone();
            req["step"] = serde_json::to_value(session.current()).unwrap_or(Value::Null);
            req["can_back"] = json!(session.can_back());
            req["error"] = json!(std::mem::take(&mut error));
            req["images"] = json!(images.to_string_lossy());
            let a = self.host.ask("fomod", &req);
            take_name(&a, name);
            match action(&a) {
                "set" => {
                    let (g, p) = (a["group"].as_u64().unwrap_or(0) as usize, a["plugin"].as_u64().unwrap_or(0) as usize);
                    if let Err(e) = session.set(g, p, a["on"].as_bool().unwrap_or(false)) {
                        error = e.to_string();
                    }
                }
                "next" | "ok" => match session.next_step() {
                    Ok(true) => {}
                    Ok(false) => return Answer::Accept(()),
                    Err(e) => error = e.to_string(),
                },
                "back" => {
                    session.back();
                }
                "manual" => return Answer::Manual,
                _ => return Answer::Cancel,
            }
        }
    }

    fn fomod_missing(&mut self, missing: &[String]) -> bool {
        self.host.ask("fomod_missing", &json!({ "missing": missing }))["yes"].as_bool().unwrap_or(false)
    }

    fn manual(&mut self, name: &mut GuessedName, tree: &mut FileTree, checker: &DataChecker) -> Answer<String> {
        let mut req = named(name);
        req["paths"] = json!(tree.paths());
        req["valid"] = json!(checker.looks_valid(tree));
        req["data_dir"] = json!(checker.data_dir);
        let a = self.host.ask("manual", &req);
        if action(&a) != "ok" {
            return Answer::Cancel;
        }
        take_name(&a, name);
        for path in strings(&a["removed"]) {
            tree.remove(path.trim_end_matches('/'));
        }
        Answer::Accept(text(&a, "data_root"))
    }

    fn root_sort(&mut self, mod_name: &str, plan: &Plan) -> bool {
        let req = json!({
            "mod_name": mod_name, "to_root": plan.to_root, "dropped": plan.dropped, "unsure": plan.unsure,
            "data_split": plan.data_split, "base": plan.base,
        });
        self.host.ask("root_sort", &req)["move"].as_bool().unwrap_or(false)
    }

    fn clash(&mut self, mod_name: &str) -> Clash {
        let a = self.host.ask("clash", &json!({ "name": mod_name }));
        let backup = a["backup"].as_bool().unwrap_or(false);
        let action = match action(&a) {
            "merge" => ClashAction::Merge,
            "replace" => ClashAction::Replace,
            "rename" => ClashAction::Rename(text(&a, "name")),
            _ => ClashAction::Cancel,
        };
        Clash { action, backup }
    }

    fn question(&mut self, id: &str, title: &str, text: &str, default_yes: bool) -> (bool, bool) {
        let a = self.host.ask("question", &json!({ "id": id, "title": title, "text": text, "default": default_yes }));
        (a["yes"].as_bool().unwrap_or(default_yes), a["remember"].as_bool().unwrap_or(false))
    }

    fn password(&mut self, archive: &str, wrong: bool) -> Option<String> {
        let a = self.host.ask("password", &json!({ "archive": archive, "wrong": wrong }));
        (action(&a) == "ok").then(|| text(&a, "password"))
    }

    fn configure_ini_tweaks(&mut self, mod_name: &str) -> bool {
        self.host.ask("ini_tweaks", &json!({ "name": mod_name }))["yes"].as_bool().unwrap_or(false)
    }

    fn progress(&mut self, percent: u8) {
        if !self.host.progress(percent) {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// Installs `archive` into the instance with the dialogs of `host`. `under` is the separator
/// to install into ("Install to Separator"), empty for none; with the plugin's
/// `remember_last_separator` it is stored for the profile.
pub fn install(instance: &Instance, profile: &Profile, host: &mut dyn Host, archive: &Path, under: Option<&str>) -> strata_install::Result<Installed> {
    let cancel = AtomicBool::new(false);
    let mut ui = HostUi::new(host, &cancel);
    // the Downloads tab installs from the downloads folder (installDownload: an existing mod
    // with the same Nexus id is the one updated); "Install mod..." from elsewhere does not
    let from_downloads = archive.parent().and_then(|p| p.canonicalize().ok()) == instance.paths.downloads.canonicalize().ok();
    let options = Options { under: under.map(str::to_owned), from_downloads, ..Options::default() };
    let done = install_into(instance, profile, &SevenZip, &mut ui, &cancel, archive, &options)?;
    let settings = strata_install::settings::InstallSettings::read(&instance.ini);
    if settings.install_to_separator && settings.remember_last_separator {
        remember_separator(instance, profile, under)?;
    }
    Ok(done)
}

/// Stores the last separator chosen for the profile in `ModOrganizer.ini`.
fn remember_separator(instance: &Instance, profile: &Profile, under: Option<&str>) -> strata_install::Result<()> {
    // the key and value of strata_install::settings::set_last_separator
    let key = format!("{}/last_separator/{}", strata_install::settings::INSTALL_TO_SEPARATOR, profile.name);
    let value = strata_ini::Value::String(under.unwrap_or("").to_owned());
    strata_core::settings::store_instance_ini(&instance.root, &[("Plugins", &key, Some(value))])?;
    Ok(())
}

/// The separators "install under" offers (lowest priority first) and the one to preselect.
pub fn separator_choices(instance: &Instance, profile: &Profile) -> (Vec<String>, Option<String>) {
    let list = strata_install::profile::separators(instance, profile).unwrap_or_default();
    let last = strata_install::settings::last_separator(&instance.ini, &profile.name).filter(|l| list.contains(l));
    (list, last)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers from a script: `(kind, answer)` in order; records the requests.
    struct Script {
        answers: Vec<(&'static str, Value)>,
        asked: Vec<(String, Value)>,
    }

    impl Host for Script {
        fn ask(&mut self, kind: &str, request: &Value) -> Value {
            self.asked.push((kind.to_owned(), request.clone()));
            let (k, a) = self.answers.remove(0);
            assert_eq!(k, kind, "asked {kind} with {request}");
            a
        }
        fn progress(&mut self, _percent: u8) -> bool {
            true
        }
    }

    const XML: &str = r#"<config><moduleName>M</moduleName><installSteps order="Explicit">
      <installStep name="One"><optionalFileGroups><group name="G" type="SelectExactlyOne"><plugins order="Explicit">
        <plugin name="A"><files><file source="a.esp"/></files><conditionFlags><flag name="f">a</flag></conditionFlags><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
        <plugin name="B"><files><file source="b.esp"/></files><conditionFlags><flag name="f">b</flag></conditionFlags><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
      </plugins></group></optionalFileGroups></installStep>
      <installStep name="Two"><visible><flagDependency flag="f" value="b"/></visible><optionalFileGroups><group name="H" type="SelectAny"><plugins>
        <plugin name="C"><files><file source="c.esp"/></files><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
      </plugins></group></optionalFileGroups></installStep></installSteps></config>"#;

    struct NoEnv;
    impl strata_install::fomod::session::Environment for NoEnv {
        fn file_state(&self, _: &str) -> strata_install::fomod::session::FileState {
            strata_install::fomod::session::FileState::Missing
        }
        fn game_version(&self) -> Option<String> {
            None
        }
        fn script_extender_version(&self) -> Option<String> {
            None
        }
    }

    #[test]
    fn fomod_dialog_conversation() {
        let config = strata_install::fomod::ModuleConfig::parse(XML.as_bytes()).unwrap();
        let env = NoEnv;
        let mut session = Session::new(&config, &env).unwrap();
        let mut host = Script {
            answers: vec![
                ("fomod", json!({ "action": "set", "group": 0, "plugin": 1, "on": true })),
                ("fomod", json!({ "action": "next" })),
                ("fomod", json!({ "action": "back" })),
                ("fomod", json!({ "action": "set", "group": 0, "plugin": 1, "on": false })),
                ("fomod", json!({ "action": "next" })),
                ("fomod", json!({ "action": "next", "name": "Chosen" })),
            ],
            asked: Vec::new(),
        };
        let cancel = AtomicBool::new(false);
        let mut ui = HostUi::new(&mut host, &cancel);
        let mut name = GuessedName::default();
        name.update("Archive", Quality::Fallback);
        let info = Info { name: Some("M".into()), ..Info::default() };
        assert_eq!(ui.fomod(&mut name, &info, &mut session, Path::new("/tmp")), Answer::Accept(()));
        assert_eq!(name.value(), "Chosen");
        let steps: Vec<_> = host.asked.iter().map(|(_, r)| (r["step"]["name"].clone(), r["step"]["is_last"].clone())).collect();
        // A is ticked by default: One is the last step; after picking B, Two shows
        assert_eq!(steps[0], (json!("One"), json!(true)));
        assert_eq!(steps[1], (json!("One"), json!(false)));
        assert_eq!(steps[2].0, json!("Two"));
        assert_eq!(host.asked[2].1["can_back"], json!(true));
        assert!(host.asked[4].1["error"].as_str().unwrap().contains("exactly one"), "unticking a radio is refused");
        let files: Vec<_> = session.files().iter().map(|f| f.source.clone()).collect();
        assert_eq!(files, ["b.esp"]);
    }

    #[test]
    fn simple_dialogs_map_to_answers() {
        let mut host = Script {
            answers: vec![
                ("quick", json!({ "action": "ok", "name": "Typed" })),
                ("clash", json!({ "action": "rename", "name": "Other", "backup": true })),
                ("bain", json!({ "action": "ok", "selected": ["10 B"] })),
                ("root_sort", json!({ "move": true })),
                ("bundle", json!({ "action": "ok", "pick": "nope.7z" })),
            ],
            asked: Vec::new(),
        };
        let cancel = AtomicBool::new(false);
        let mut ui = HostUi::new(&mut host, &cancel);
        let mut name = GuessedName::default();
        assert_eq!(ui.quick(&mut name), Answer::Accept(()));
        assert_eq!(name.value(), "Typed");
        assert_eq!(ui.clash("X"), Clash { action: ClashAction::Rename("Other".into()), backup: true });
        assert_eq!(ui.bain(&mut name, &["00 A".into(), "10 B".into()], &[true, false]), Answer::Accept(vec!["10 B".into()]));
        assert!(ui.root_sort("X", &Plan { to_root: vec!["a.dll".into()], ..Plan::default() }));
        assert_eq!(ui.bundle(&["a.7z".into()]), Answer::Cancel, "a pick that is not offered cancels");
        assert_eq!(host.asked[2].1["packages"][0], json!({ "name": "00 A", "checked": true }));
    }

    #[test]
    fn password_and_ini_tweaks() {
        let mut host = Script {
            answers: vec![
                ("password", json!({ "action": "ok", "password": "pw" })),
                ("password", json!({ "action": "cancel", "password": "" })),
                ("ini_tweaks", json!({ "yes": true })),
            ],
            asked: Vec::new(),
        };
        let cancel = AtomicBool::new(false);
        let mut ui = HostUi::new(&mut host, &cancel);
        assert_eq!(ui.password("a.7z", false).as_deref(), Some("pw"));
        assert_eq!(ui.password("a.7z", true), None);
        assert!(ui.configure_ini_tweaks("Mod"));
        assert_eq!(host.asked[1].1, json!({ "archive": "a.7z", "wrong": true }));
        assert_eq!(host.asked[2].1, json!({ "name": "Mod" }));
    }
}
