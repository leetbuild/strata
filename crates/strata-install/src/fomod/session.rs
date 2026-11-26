//! Running a FOMOD: which steps show, what is ticked by default, which files go where.
//!
//! The logic of MO2's `FomodInstallerDialog` without the widgets: `testCondition`,
//! `testVisible`, `getPluginDependencyType`, `displayCurrentPage` (the defaults, as in
//! upstream `modorganizer-installer_fomod`: they are applied each time a step is entered
//! going forward), `nextPage` / `on_prevBtn_clicked`, `updateNextbtnText` (a "select at
//! least one" group blocks Next) and `updateTree` (the file list, sorted by priority).
//!
//! The shell drives a [`Session`] step by step; the CLI applies `--choices` or the defaults.
//!
//! One deliberate difference: MO2 caches step visibility in `m_PageVisible` from the first
//! *visible* step, so when the first step is hidden the cache is shifted by one and a hidden
//! step can count as visible (its "select all" files get installed). Strata computes the
//! visibility of each step from the steps before it.

use super::{Composite, Condition, FileInstall, GroupType, ModuleConfig, Operator, PluginType, Step};
use crate::{Error, Result};

/// A file's state for `fileDependency` (MO2: the plugin list's state of the file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Missing,
    Inactive,
    Active,
}

impl FileState {
    fn as_str(self) -> &'static str {
        match self {
            FileState::Missing => "Missing",
            FileState::Inactive => "Inactive",
            FileState::Active => "Active",
        }
    }
}

/// What a FOMOD may ask about the installation.
pub trait Environment {
    /// The state of a plugin file (MO2 `InstallerFomod::fileState`: plugins only).
    fn file_state(&self, file: &str) -> FileState;
    /// The game's version (`IPluginGame::gameVersion`), e.g. `1.5.97.0`.
    fn game_version(&self) -> Option<String>;
    /// The script extender's version (`ScriptExtender::getExtenderVersion`); `None` when
    /// it is not installed.
    fn script_extender_version(&self) -> Option<String>;
}

/// The FOMM version MO2 claims for `fommDependency`.
const FOMM_VERSION: &str = "0.13.21";

/// `FOMOD::Version`: four numbers read with `istream >>`, one character skipped between
/// them; reading stops (the rest stays 0) at the first thing that is not a number.
fn version(v: &str) -> [i64; 4] {
    let mut out = [0; 4];
    let mut rest = v;
    for (i, slot) in out.iter_mut().enumerate() {
        if i > 0 {
            let mut chars = rest.chars();
            if chars.next().is_none() {
                break;
            }
            rest = chars.as_str();
        }
        let trimmed = rest.trim_start();
        let digits = trimmed
            .char_indices()
            .take_while(|(j, c)| c.is_ascii_digit() || (*j == 0 && (*c == '-' || *c == '+')))
            .count();
        match trimmed[..digits].parse() {
            Ok(n) => *slot = n,
            Err(_) => break,
        }
        rest = &trimmed[digits..];
    }
    out
}

/// One plugin as the step shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PluginView {
    pub name: String,
    pub description: String,
    pub image: Option<String>,
    pub kind: PluginType,
    pub checked: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GroupView {
    pub name: String,
    pub kind: GroupType,
    pub plugins: Vec<PluginView>,
}

/// The current step, for a dialog.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StepView {
    pub index: usize,
    pub name: String,
    pub groups: Vec<GroupView>,
    /// Groups that need a selection before Next (`updateNextbtnText`).
    pub blocking: Vec<String>,
    /// No visible step comes after this one: the button says Install.
    pub is_last: bool,
}

/// A FOMOD being installed.
pub struct Session<'a> {
    config: &'a ModuleConfig,
    env: &'a dyn Environment,
    /// `[step][group][plugin]`.
    checked: Vec<Vec<Vec<bool>>>,
    enabled: Vec<Vec<Vec<bool>>>,
    /// The step shown; `None` when no step is visible (MO2's "small install").
    current: Option<usize>,
    /// The steps passed through, for Back.
    history: Vec<usize>,
    /// The choices of the previous install of this mod, ticked again when a step is entered.
    previous: Choices,
}

/// Ticked plugins by step and group name: what a FOMOD install chose. Strata keeps it in the
/// mod's `meta.ini` (MO2 2.5.2 does not remember FOMOD choices) to offer it on a reinstall.
pub type Choices = std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<String>>>;

fn shape<T: Clone>(config: &ModuleConfig, f: impl Fn(GroupType) -> T) -> Vec<Vec<Vec<T>>> {
    config.steps.iter().map(|s| s.groups.iter().map(|g| vec![f(g.kind); g.plugins.len()]).collect()).collect()
}

impl<'a> Session<'a> {
    /// Starts the installer: checks `moduleDependencies` and enters the first visible step.
    pub fn new(config: &'a ModuleConfig, env: &'a dyn Environment) -> Result<Self> {
        let mut session = Session {
            config,
            env,
            // controls of a "select all" group start ticked
            checked: shape(config, |k| k == GroupType::SelectAll),
            enabled: shape(config, |k| k != GroupType::SelectAll),
            current: None,
            history: Vec::new(),
            previous: Choices::new(),
        };
        if let Some(deps) = &config.dependencies {
            let (ok, why) = session.test(0, deps);
            if !ok {
                return Err(Error::Fomod(format!("the mod's requirements are not met: {why}")));
            }
        }
        session.current = (0..config.steps.len()).find(|&i| session.visible(i));
        if let Some(i) = session.current {
            session.apply_defaults(i);
        }
        Ok(session)
    }

    fn step(&self, i: usize) -> &'a Step {
        &self.config.steps[i]
    }

    /// `testVisible`: a step without a condition is visible; otherwise its condition is
    /// tested with the flags of the visible steps before it.
    pub fn visible(&self, i: usize) -> bool {
        self.visibility(i + 1)[i]
    }

    /// The visibility of steps `0..n`.
    fn visibility(&self, n: usize) -> Vec<bool> {
        let mut vis = Vec::with_capacity(n);
        for i in 0..n.min(self.config.steps.len()) {
            let shown = match &self.step(i).visible {
                None => true,
                Some(c) => self.test_with(i, c, &vis).0,
            };
            vis.push(shown);
        }
        vis
    }

    fn test(&self, max: usize, c: &Composite) -> (bool, String) {
        let vis = self.visibility(max);
        self.test_with(max, c, &vis)
    }

    /// `testCondition(maxIndex, SubCondition)`.
    fn test_with(&self, max: usize, c: &Composite, vis: &[bool]) -> (bool, String) {
        for cond in &c.conditions {
            let (ok, why) = self.test_one(max, cond, vis);
            match c.operator {
                Operator::Or if ok => return (true, why),
                Operator::And if !ok => return (false, why),
                _ => {}
            }
        }
        match c.operator {
            Operator::And => (true, String::new()),
            Operator::Or => (false, "no condition of an 'Or' was met".to_owned()),
        }
    }

    fn test_one(&self, max: usize, cond: &Condition, vis: &[bool]) -> (bool, String) {
        match cond {
            Condition::Nested(c) => self.test_with(max, c, vis),
            Condition::Flag { name, value } => self.test_flag(max, name, value, vis),
            Condition::File { file, state } => {
                let found = self.env.file_state(file).as_str();
                (found == state, format!("the file '{file}' should be {state}, but is {found}"))
            }
            Condition::Game { version: v } => {
                let have = self.env.game_version().unwrap_or_default();
                (version(v) <= version(&have), format!("the game must be version {v} or newer, found '{have}'"))
            }
            Condition::Fomm { version: v } => (
                version(v) <= version(FOMM_VERSION),
                format!("needs FOMM {v}, Strata acts as {FOMM_VERSION}"),
            ),
            Condition::ScriptExtender { version: v } => {
                let have = self.env.script_extender_version().unwrap_or_else(|| "not installed".to_owned());
                (version(v) <= version(&have), format!("the script extender must be version {v} or newer, found '{have}'"))
            }
        }
    }

    /// `testCondition(maxIndex, flag, value)`: the latest visible step before `max` with a
    /// ticked plugin setting the flag decides; an unset flag matches only an empty value.
    fn test_flag(&self, max: usize, name: &str, value: &str, vis: &[bool]) -> (bool, String) {
        for i in (0..max.min(vis.len())).rev() {
            if !vis[i] {
                continue;
            }
            for (g, group) in self.step(i).groups.iter().enumerate() {
                for (p, plugin) in group.plugins.iter().enumerate() {
                    if !self.checked[i][g][p] {
                        continue;
                    }
                    if let Some((_, v)) = plugin.flags.iter().find(|(n, _)| n == name) {
                        return (v == value, format!("the flag '{name}' is '{v}', not '{value}'"));
                    }
                }
            }
        }
        (value.is_empty(), format!("the flag '{name}' is not set"))
    }

    /// `getPluginDependencyType` for a plugin of step `i`.
    fn plugin_type(&self, i: usize, g: usize, p: usize) -> PluginType {
        let info = &self.step(i).groups[g].plugins[p].type_info;
        info.patterns.iter().find(|(c, _)| self.test(i, c).0).map_or(info.default, |(_, t)| *t)
    }

    /// `QAbstractButton::setChecked` with radio buttons' exclusivity: ticking one unticks
    /// the others, and the ticked radio button cannot be unticked.
    fn set_raw(&mut self, i: usize, g: usize, p: usize, on: bool) {
        let radio = self.step(i).groups[g].kind.is_radio();
        if radio {
            if on {
                self.checked[i][g].iter_mut().for_each(|c| *c = false);
            } else if self.checked[i][g][p] {
                return;
            }
        }
        self.checked[i][g][p] = on;
    }

    /// Remembers a previous install's choices: the current step and each step entered later
    /// get them ticked over the defaults, where step, group and plugin names still match and
    /// the plugin can be changed (an archive's new version may have renamed things: those
    /// are left at their defaults).
    pub fn remember(&mut self, previous: Choices) {
        self.previous = previous;
        if let Some(i) = self.current {
            self.apply_previous(i);
        }
    }

    fn apply_previous(&mut self, i: usize) {
        let step = self.step(i);
        let Some(groups) = self.previous.get(&step.name) else { return };
        for (g, group) in step.groups.iter().enumerate() {
            let Some(wanted) = groups.get(&group.name) else { continue };
            if group.kind == GroupType::SelectAll {
                continue;
            }
            let want = |p: usize| wanted.contains(&group.plugins[p].name);
            let radio = matches!(group.kind, GroupType::SelectExactlyOne | GroupType::SelectAtMostOne);
            if radio && wanted.len() > 1 {
                continue;
            }
            let locked = (0..group.plugins.len()).any(|p| !self.enabled[i][g][p] && self.checked[i][g][p] != want(p));
            if locked || (group.kind == GroupType::SelectExactlyOne && !(0..group.plugins.len()).any(want)) {
                continue;
            }
            if group.kind == GroupType::SelectAtLeastOne && !(0..group.plugins.len()).any(want) {
                continue;
            }
            for (p, checked) in self.checked[i][g].iter_mut().enumerate() {
                *checked = want(p);
            }
        }
    }

    /// What is ticked in the steps the user went through (Back's history and the current
    /// step), for [`Session::remember`] next time.
    pub fn choices(&self) -> Choices {
        let mut out = Choices::new();
        for &i in self.history.iter().chain(self.current.iter()) {
            let step = self.step(i);
            let groups = out.entry(step.name.clone()).or_default();
            for (g, group) in step.groups.iter().enumerate() {
                let ticked: Vec<String> = group
                    .plugins
                    .iter()
                    .enumerate()
                    .filter(|(p, _)| self.checked[i][g][*p])
                    .map(|(_, plugin)| plugin.name.clone())
                    .collect();
                groups.insert(group.name.clone(), ticked);
            }
        }
        out
    }

    /// `displayCurrentPage`: plugin types decide what is ticked and what can be changed.
    fn apply_defaults(&mut self, i: usize) {
        for g in 0..self.step(i).groups.len() {
            let kind = self.step(i).groups[g].kind;
            if kind == GroupType::SelectAll {
                continue;
            }
            let must_select_one = matches!(kind, GroupType::SelectExactlyOne | GroupType::SelectAtLeastOne);
            let mut may_select_more = true;
            let mut first_optional = None;
            let mut first_couldbe = None;
            let count = self.step(i).groups[g].plugins.len();
            for p in 0..count {
                let t = self.plugin_type(i, g, p);
                self.enabled[i][g][p] = true;
                match t {
                    PluginType::Required => {
                        if kind.is_radio() {
                            self.enabled[i][g].iter_mut().for_each(|e| *e = false);
                        } else {
                            self.enabled[i][g][p] = false;
                        }
                        self.set_raw(i, g, p, true);
                    }
                    PluginType::Recommended => {
                        if may_select_more || !must_select_one {
                            self.set_raw(i, g, p, true);
                        }
                    }
                    PluginType::Optional => {
                        first_optional.get_or_insert(p);
                    }
                    PluginType::NotUsable => {
                        self.set_raw(i, g, p, false);
                        self.enabled[i][g][p] = false;
                    }
                    PluginType::CouldBeUsable => {
                        first_couldbe.get_or_insert(p);
                    }
                }
                if self.checked[i][g][p] {
                    may_select_more = false;
                }
            }
            if may_select_more {
                if kind == GroupType::SelectAtMostOne {
                    // the "None" radio button
                    self.checked[i][g].iter_mut().for_each(|c| *c = false);
                } else if must_select_one
                    && let Some(p) = first_optional.or(first_couldbe).or((count > 0).then_some(0))
                {
                    self.set_raw(i, g, p, true);
                }
            }
        }
    }

    /// The step shown, `None` when there is none (nothing to choose).
    pub fn current(&self) -> Option<StepView> {
        let i = self.current?;
        let step = self.step(i);
        let groups = step
            .groups
            .iter()
            .enumerate()
            .map(|(g, group)| GroupView {
                name: group.name.clone(),
                kind: group.kind,
                plugins: group
                    .plugins
                    .iter()
                    .enumerate()
                    .map(|(p, plugin)| PluginView {
                        name: plugin.name.clone(),
                        description: plugin.description.clone(),
                        image: plugin.image.clone(),
                        kind: self.plugin_type(i, g, p),
                        checked: self.checked[i][g][p],
                        enabled: self.enabled[i][g][p],
                    })
                    .collect(),
            })
            .collect();
        let blocking = step
            .groups
            .iter()
            .enumerate()
            .filter(|(g, group)| group.kind == GroupType::SelectAtLeastOne && !self.checked[i][*g].iter().any(|c| *c))
            .map(|(_, group)| group.name.clone())
            .collect();
        Some(StepView { index: i, name: step.name.clone(), groups, blocking, is_last: self.next_visible(i).is_none() })
    }

    fn next_visible(&self, i: usize) -> Option<usize> {
        let vis = self.visibility(self.config.steps.len());
        (i + 1..self.config.steps.len()).find(|&j| vis[j])
    }

    /// The user clicks a plugin of the current step. Disabled plugins and unticking the
    /// ticked radio button are refused.
    pub fn set(&mut self, group: usize, plugin: usize, on: bool) -> Result<()> {
        let i = self.current.ok_or_else(|| Error::Fomod("there is no step to choose in".to_owned()))?;
        let Some(&enabled) = self.enabled[i].get(group).and_then(|g| g.get(plugin)) else {
            return Err(Error::Fomod(format!("step '{}' has no plugin {group}/{plugin}", self.step(i).name)));
        };
        let g = &self.step(i).groups[group];
        let name = &g.plugins[plugin].name;
        if !enabled && self.checked[i][group][plugin] != on {
            return Err(Error::Fomod(format!("'{name}' in '{}' cannot be changed", g.name)));
        }
        if !on && g.kind == GroupType::SelectExactlyOne && self.checked[i][group][plugin] {
            return Err(Error::Fomod(format!("'{}' needs exactly one choice: pick another instead", g.name)));
        }
        if !on && g.kind == GroupType::SelectAtMostOne {
            // clicking "None"
            self.checked[i][group].iter_mut().for_each(|c| *c = false);
            return Ok(());
        }
        self.set_raw(i, group, plugin, on);
        Ok(())
    }

    /// Next: enters the next visible step and applies its defaults. Returns false when there
    /// is none (the dialog would install now). Fails while a group needs a selection.
    pub fn next_step(&mut self) -> Result<bool> {
        let Some(view) = self.current() else { return Ok(false) };
        if !view.blocking.is_empty() {
            return Err(Error::Fomod(format!("these groups need a selection: {}", view.blocking.join(", "))));
        }
        let i = view.index;
        match self.next_visible(i) {
            Some(j) => {
                self.history.push(i);
                self.current = Some(j);
                self.apply_defaults(j);
                self.apply_previous(j);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Whether Back has a step to return to.
    pub fn can_back(&self) -> bool {
        !self.history.is_empty()
    }

    /// Back: returns to the step Next came from. Its choices are kept.
    pub fn back(&mut self) -> bool {
        match self.history.pop() {
            Some(i) => {
                self.current = Some(i);
                true
            }
            None => false,
        }
    }

    /// `updateTree`'s list: required files, the conditional installs whose condition holds,
    /// the ticked plugins' files of the visible steps; sorted by priority, then document
    /// order. Later entries overwrite earlier ones.
    pub fn files(&self) -> Vec<&'a FileInstall> {
        let n = self.config.steps.len();
        let mut list: Vec<&FileInstall> = self.config.required_files.iter().collect();
        for c in &self.config.conditional_installs {
            if self.test(n, &c.condition).0 {
                list.extend(&c.files);
            }
        }
        let vis = self.visibility(n);
        for (i, step) in self.config.steps.iter().enumerate() {
            if !vis[i] {
                continue;
            }
            for (g, group) in step.groups.iter().enumerate() {
                for (p, plugin) in group.plugins.iter().enumerate() {
                    if self.checked[i][g][p] {
                        list.extend(&plugin.files);
                    }
                }
            }
        }
        list.sort_by_key(|f| (f.priority, f.sequence));
        list
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::SAMPLE;
    use super::*;

    struct Env {
        zeta: FileState,
        game: &'static str,
    }

    impl Environment for Env {
        fn file_state(&self, file: &str) -> FileState {
            if file.eq_ignore_ascii_case("zeta.esp") { self.zeta } else { FileState::Missing }
        }
        fn game_version(&self) -> Option<String> {
            Some(self.game.to_owned())
        }
        fn script_extender_version(&self) -> Option<String> {
            None
        }
    }

    fn config() -> ModuleConfig {
        ModuleConfig::parse(SAMPLE.as_bytes()).unwrap()
    }

    fn sources(s: &Session) -> Vec<String> {
        s.files().iter().map(|f| f.source.clone()).collect()
    }

    #[test]
    fn versions_compare_like_istream() {
        assert_eq!(version("1.5.97.0"), [1, 5, 97, 0]);
        assert_eq!(version("1.2"), [1, 2, 0, 0]);
        assert_eq!(version("not installed"), [0, 0, 0, 0]);
        assert_eq!(version("2.0.20b"), [2, 0, 20, 0]);
        assert!(version("1.5.97") <= version("1.5.97.0"));
        assert!(version("1.6") > version("1.5.97.0"));
    }

    #[test]
    fn defaults_recommended_flags_and_visibility() {
        let c = config();
        let env = Env { zeta: FileState::Active, game: "1.5.97.0" };
        let mut s = Session::new(&c, &env).unwrap();
        let view = s.current().unwrap();
        assert_eq!(view.name, "Options");
        assert!(view.groups[0].plugins[0].checked, "2K is recommended");
        assert!(!view.groups[0].plugins[1].checked);
        assert!(view.groups[1].plugins[0].checked && !view.groups[1].plugins[0].enabled, "select all");
        assert!(!view.is_last, "2K sets res=2k, so Patches shows");
        assert!(s.next_step().unwrap());
        let view = s.current().unwrap();
        assert_eq!(view.name, "Patches");
        assert!(view.is_last);
        assert_eq!(view.groups[0].plugins[0].name, "Alpha");
        assert!(!view.groups[0].plugins[0].enabled, "not usable");
        assert_eq!(view.groups[0].plugins[1].kind, PluginType::Recommended, "Zeta.esp is active");
        assert!(view.groups[0].plugins[1].checked);
        assert!(!s.next_step().unwrap());
        // the game is 1.5.97.0, so the Or of the conditional install holds
        assert_eq!(sources(&s), ["extra/readme.txt", "core", "patches/zeta.esp", "tex/2k"]);
    }

    #[test]
    fn choosing_1k_hides_the_patches_step() {
        let c = config();
        let env = Env { zeta: FileState::Missing, game: "1.5.80.0" };
        let mut s = Session::new(&c, &env).unwrap();
        s.set(0, 1, true).unwrap();
        let view = s.current().unwrap();
        assert!(!view.groups[0].plugins[0].checked, "radio: 2K unticked");
        assert!(view.is_last);
        assert!(s.set(0, 1, false).is_err(), "exactly one: cannot untick");
        assert!(s.set(1, 0, false).is_err(), "select all cannot change");
        assert!(!s.next_step().unwrap());
        // res=1k satisfies the conditional install even on an old game
        assert_eq!(sources(&s), ["extra/readme.txt", "core", "tex/1k"]);
    }

    #[test]
    fn back_keeps_choices_and_next_reapplies_defaults() {
        let c = config();
        let env = Env { zeta: FileState::Active, game: "1.5.97.0" };
        let mut s = Session::new(&c, &env).unwrap();
        s.next_step().unwrap();
        s.set(0, 1, false).unwrap();
        assert!(s.back());
        assert_eq!(s.current().unwrap().name, "Options");
        s.next_step().unwrap();
        assert!(s.current().unwrap().groups[0].plugins[1].checked, "upstream MO2 ticks recommended again");
    }

    #[test]
    fn unmet_module_dependencies_stop_the_install() {
        let xml = r#"<config><moduleDependencies><fileDependency file="Zeta.esp" state="Active"/></moduleDependencies></config>"#;
        let c = ModuleConfig::parse(xml.as_bytes()).unwrap();
        let env = Env { zeta: FileState::Inactive, game: "" };
        assert!(Session::new(&c, &env).is_err());
        let env = Env { zeta: FileState::Active, game: "" };
        let s = Session::new(&c, &env).unwrap();
        assert!(s.current().is_none(), "no steps: small install");
    }

    #[test]
    fn at_least_one_blocks_next_until_ticked() {
        let xml = r#"<config><installSteps><installStep name="s"><optionalFileGroups>
            <group name="g" type="SelectAtLeastOne"><plugins>
              <plugin name="a"><typeDescriptor><type name="NotUsable"/></typeDescriptor></plugin>
              <plugin name="b"><typeDescriptor><type name="NotUsable"/></typeDescriptor></plugin>
            </plugins></group>
            <group name="h" type="SelectAtMostOne"><plugins>
              <plugin name="x"><typeDescriptor><type name="Recommended"/></typeDescriptor></plugin>
              <plugin name="y"><typeDescriptor><type name="Recommended"/></typeDescriptor></plugin>
            </plugins></group>
          </optionalFileGroups></installStep></installSteps></config>"#;
        let c = ModuleConfig::parse(xml.as_bytes()).unwrap();
        let env = Env { zeta: FileState::Missing, game: "" };
        let mut s = Session::new(&c, &env).unwrap();
        let view = s.current().unwrap();
        // MO2 ticks controls[0] when nothing is usable; NotUsable unticks only what it sees
        assert!(view.groups[0].plugins[0].checked);
        assert!(view.blocking.is_empty());
        // at most one, two recommended: the radio buttons leave the last one ticked
        assert!(!view.groups[1].plugins[0].checked && view.groups[1].plugins[1].checked);
        s.set(1, 1, false).unwrap();
        assert!(s.current().unwrap().groups[1].plugins.iter().all(|p| !p.checked), "None");
        assert!(!s.next_step().unwrap());
    }

    #[test]
    fn previous_choices_are_ticked_again() {
        let c = config();
        let env = Env { zeta: FileState::Active, game: "1.5.97.0" };
        // first install: 2K (default), then untick Zeta in Patches
        let mut s = Session::new(&c, &env).unwrap();
        s.next_step().unwrap();
        s.set(0, 1, false).unwrap();
        let chosen = s.choices();
        assert_eq!(chosen["Options"]["Textures"], ["2K"]);
        assert_eq!(chosen["Options"]["Lonely"], ["Only"]);
        assert!(chosen["Patches"]["Patches"].is_empty());

        // the reinstall starts from those choices, not from the defaults
        let mut s = Session::new(&c, &env).unwrap();
        s.remember(chosen.clone());
        s.next_step().unwrap();
        assert!(!s.current().unwrap().groups[0].plugins[1].checked, "Zeta stays unticked");
        assert_eq!(s.choices(), chosen);

        // 1K chosen before: ticked on the first step at once, which hides Patches
        let mut previous = Choices::new();
        previous.entry("Options".into()).or_default().insert("Textures".into(), vec!["1K".into()]);
        // names that no longer exist are ignored
        previous.entry("Options".into()).or_default().insert("Gone".into(), vec!["X".into()]);
        previous.entry("Old step".into()).or_default().insert("G".into(), vec!["P".into()]);
        let mut s = Session::new(&c, &env).unwrap();
        s.remember(previous);
        let view = s.current().unwrap();
        assert!(view.groups[0].plugins[1].checked && !view.groups[0].plugins[0].checked);
        assert!(view.is_last);
        // a radio group never gets two, a disabled plugin is never changed
        let mut bad = Choices::new();
        bad.entry("Options".into()).or_default().insert("Textures".into(), vec!["1K".into(), "2K".into()]);
        bad.entry("Options".into()).or_default().insert("Lonely".into(), vec![]);
        let mut s = Session::new(&c, &env).unwrap();
        s.remember(bad);
        let view = s.current().unwrap();
        assert!(view.groups[0].plugins[0].checked, "2K default kept");
        assert!(view.groups[1].plugins[0].checked, "select all kept");
    }
}
