//! The installation pipeline: MO2's `InstallationManager::install`, `doInstall`,
//! `testOverwrite` and the end of `OrganizerCore::installArchive`.
//!
//! An archive is listed into a tree, the Root sorter may unwrap and split it, then the
//! installers are tried in priority order until one takes it; its tree is Root-sorted once
//! more, the name clash is settled, the archive is extracted and its files placed as the
//! tree says, and `meta.ini` is written. Every question is a call on [`InstallUi`]: the
//! window answers with dialogs, the CLI with its options.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::archive::Archives;
use crate::checker::DataChecker;
use crate::fomod::session::{Choices, Environment, Session};
use crate::fomod::{Info, ModuleConfig};
use crate::installers::{self, ARCHIVE_EXTENSIONS, Installer};
use crate::meta::{DownloadMeta, InstallMeta, ModMeta, mark_download_installed};
use crate::name::{GuessedName, Quality, interpret_nexus_file_name};
use strata_nexus::links::{modpub_mod_id, modpub_page_url};
use crate::rootsorter::{self, Plan};
use crate::tree::{FileTree, complete_base_name, fold, suffix};
use crate::{Error, Result};

/// Where an install goes, from the instance.
#[derive(Debug, Clone)]
pub struct Target {
    /// `gameName` of `ModOrganizer.ini` (`Skyrim Special Edition`).
    pub game_name: String,
    /// `IPluginGame::gameShortName` (`SkyrimSE`), for ModPub pages.
    pub game_short_name: String,
    pub mods_dir: PathBuf,
    pub downloads_dir: PathBuf,
    /// The folder with `categories.dat` and `nexuscatmap.dat` (the instance folder).
    pub categories_dir: PathBuf,
    /// Scratch space on the same file system as `mods_dir` (`<instance>/strata/tmp`), so
    /// extracted files are moved, not copied.
    pub temp_dir: PathBuf,
    /// Where "Replace" sends the old files (`strata_core::Trash::home()`).
    pub trash: strata_core::Trash,
}

/// A dialog's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer<T> {
    Accept(T),
    /// The "Manual" button: try the manual installer instead.
    Manual,
    Cancel,
}

/// The answer to "a mod of this name exists" (`QueryOverwriteDialog`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClashAction {
    /// Install over the existing files.
    Merge,
    /// Remove the old files (to the trash), keep its `meta.ini`, install fresh.
    Replace,
    /// Ask again with this name.
    Rename(String),
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clash {
    pub action: ClashAction,
    /// Copy the existing mod to `<name>_backup` first ("keep backup").
    pub backup: bool,
}

/// Every question an install may ask. `name` arguments are the mod name being guessed: a
/// dialog that lets the user type a name updates it with [`Quality::User`].
pub trait InstallUi {
    /// The quick installer's dialog (name, Ok / Manual / Cancel).
    fn quick(&mut self, name: &mut GuessedName) -> Answer<()>;
    /// "This may be a BAIN package, install as one?" when some folders don't look valid.
    fn bain_unsure(&mut self) -> bool;
    /// The BAIN dialog: the packages and their default ticks; answers the ticked names.
    fn bain(&mut self, name: &mut GuessedName, packages: &[String], defaults: &[bool]) -> Answer<Vec<String>>;
    /// The bundle installer's choice when the archive holds several archives.
    fn bundle(&mut self, archives: &[String]) -> Answer<String>;
    /// The FOMOD dialog: drive the session (defaults are applied) to its end. The pictures
    /// the steps name (relative to the folder holding `fomod/`) are extracted under
    /// `images` for the length of the call.
    fn fomod(&mut self, name: &mut GuessedName, info: &Info, session: &mut Session, images: &Path) -> Answer<()>;
    /// FOMOD sources missing from the archive: install anyway?
    fn fomod_missing(&mut self, missing: &[String]) -> bool;
    /// The manual installer: the user may edit `tree` and answers the folder to install as
    /// Data (`""` for the top).
    fn manual(&mut self, name: &mut GuessedName, tree: &mut FileTree, checker: &DataChecker) -> Answer<String>;
    /// The Root sorter's confirm dialog: move the listed files?
    fn root_sort(&mut self, mod_name: &str, plan: &Plan) -> bool;
    /// A mod of this name exists.
    fn clash(&mut self, mod_name: &str) -> Clash;
    /// Extraction progress, in percent.
    fn progress(&mut self, _percent: u8) {}
    /// "This mod contains ini tweaks. Do you want to configure them now?" after an install
    /// (`OrganizerCore::doInstall`); yes opens the mod's information on its INI files.
    fn configure_ini_tweaks(&mut self, _mod_name: &str) -> bool {
        false
    }
    /// The archive (file name) is password protected (`InstallationManager::queryPassword`);
    /// `wrong` when the last answer did not open it. `None` cancels the install.
    fn password(&mut self, archive: &str, wrong: bool) -> Option<String>;
    /// A yes / no question with "remember" (MO2's `QuestionBoxMemory`, e.g. BSA Extractor's
    /// `unpackBSA`); answers `(yes, remember)`. Without a dialog: the default, not remembered.
    fn question(&mut self, _id: &str, _title: &str, _text: &str, default_yes: bool) -> (bool, bool) {
        (default_yes, false)
    }
}

/// What the caller knows besides the archive (`OrganizerCore::installArchive` /
/// `installDownload`).
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// The name the user gave (`GUESS_USER`).
    pub name: Option<String>,
    /// The mod this install replaces or updates (MO2's `currentMod`); its name is the guess
    /// to beat (`GUESS_PRESET`) unless `name` is given.
    pub current_name: Option<String>,
    /// What the installers remember from that mod's last install.
    pub previous: Previous,
}

/// The options of a mod's previous install, as the installers' `onInstallationStart`
/// read them from `currentMod`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Previous {
    /// `InstallerBAIN`: the `[Plugins] BAIN Installer\optionN` values, ticked by default.
    pub bain: Vec<String>,
    /// `InstallerBundle`: `Bundle Installer\archive`, picked without asking. Only on a
    /// reinstall with the bundle installer's `auto_reinstall` setting on (default).
    pub bundle: Option<String>,
    /// Strata's `Fomod Installer\choices` (MO2 2.5.2 keeps none): ticked again.
    pub fomod: Choices,
}

impl Previous {
    /// Reads the previous options from the mod folder `current`.
    pub fn of(current: &Path, reinstall: bool, bundle_auto_reinstall: bool) -> Result<Self> {
        let meta = ModMeta::open(current)?;
        let bain = meta
            .plugin_settings(Installer::Bain.plugin_name())
            .into_iter()
            .filter(|(k, _)| k.starts_with("option"))
            .map(|(_, v)| v)
            .collect();
        let bundle = if reinstall && bundle_auto_reinstall {
            meta.plugin_settings(Installer::Bundle.plugin_name()).into_iter().find(|(k, _)| k == "archive").map(|(_, v)| v)
        } else {
            None
        };
        let fomod = meta
            .plugin_settings(Installer::Fomod.plugin_name())
            .into_iter()
            .find(|(k, _)| k == FOMOD_CHOICES)
            .and_then(|(_, v)| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        Ok(Self { bain, bundle, fomod })
    }
}

/// The `[Plugins] Fomod Installer\<key>` holding a FOMOD install's choices as JSON.
pub const FOMOD_CHOICES: &str = "choices";

/// What an install did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Outcome {
    pub name: String,
    pub mod_dir: PathBuf,
    pub installer: Installer,
    pub merged: bool,
    pub replaced: bool,
    /// Where the existing mod was copied first.
    pub backup: Option<PathBuf>,
    /// Where the replaced files went (the trash).
    pub trashed: Option<PathBuf>,
    /// The mod has a non-empty `INI Tweaks` folder (`InstallationResult::m_iniTweaks`).
    pub ini_tweaks: bool,
    /// Things that went wrong after the mod was in place (the download's `.meta` could not be
    /// written): the install stands, as in MO2, whose `QSettings` fails silently there.
    pub notes: Vec<String>,
}

impl Outcome {
    /// `InstallationResult::mergedOrReplaced`: such a mod keeps its enabled state.
    pub fn merged_or_replaced(&self) -> bool {
        self.merged || self.replaced
    }
}

/// The FOMOD installer's leftovers for `meta.ini` (`InstallerFomod::onInstallationEnd`).
enum Extra {
    None,
    Fomod { url: String, choices: Choices },
    BainOptions(Vec<String>),
}

/// One installation (MO2's `InstallationManager` during `install`).
pub struct Install<'a> {
    pub target: &'a Target,
    pub archives: &'a dyn Archives,
    pub env: &'a dyn Environment,
    pub ui: &'a mut dyn InstallUi,
    pub cancel: &'a AtomicBool,
    checker: DataChecker,
    root_sort_asked: bool,
    /// Stop before anything is written and say what would be installed.
    preview: bool,
    previous: Previous,
    /// The password the user gave for the archive being installed.
    password: Option<String>,
}

/// What an install would do, from [`preview`]: nothing is written.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Preview {
    pub name: String,
    pub installer: Installer,
    /// The mod folder's files and folders (folders end in `/`), sorted.
    pub files: Vec<String>,
    /// A mod of that name exists (the install would ask merge / replace / rename).
    pub exists: bool,
}

enum Done {
    Installed(Outcome),
    Preview(Preview),
}

/// Installs `archive` as a mod (`OrganizerCore::installArchive`): `name` is the name given
/// by the user, if any. The mod list is the caller's (add the mod, enable it unless
/// [`Outcome::merged_or_replaced`]).
pub fn install(
    target: &Target,
    archives: &dyn Archives,
    env: &dyn Environment,
    ui: &mut dyn InstallUi,
    cancel: &AtomicBool,
    archive: &Path,
    request: &Request,
) -> Result<Outcome> {
    let checker = DataChecker::for_game(&target.game_name).ok_or_else(|| Error::UnsupportedGame(target.game_name.clone()))?;
    let mut guessed = guess_from(request);
    let previous = request.previous.clone();
    let mut run = Install { target, archives, env, ui, cancel, checker, root_sort_asked: false, preview: false, previous, password: None };
    let Done::Installed(outcome) = run.run(archive, &mut guessed, 0)? else {
        unreachable!("not previewing")
    };

    // OrganizerCore::installArchive: a known download records its ids and is marked; the
    // new mod's ModInfo then saves its meta.ini (saveMeta)
    let mut m = ModMeta::open(&outcome.mod_dir)?;
    let in_downloads = is_in(archive, &target.downloads_dir);
    if in_downloads {
        let meta = DownloadMeta::read_for(archive)?.unwrap_or_default();
        m.add_installed_file(meta.mod_id, meta.file_id);
    }
    m.save_meta(&strata_core::Categories::open(&target.categories_dir)?);
    m.save()?;
    let mut outcome = outcome;
    // DownloadManager::markInstalled writes a .meta beside any archive; Strata only marks
    // downloads in the instance's downloads folder and never writes beside an archive that
    // lies elsewhere (a departure from MO2)
    if in_downloads
        && let Err(e) = mark_download_installed(archive)
    {
        outcome.notes.push(format!("the download was not marked installed: {e}"));
    }
    Ok(outcome)
}

/// Runs the installers and their questions like [`install`], but stops before the name
/// clash, the extraction and every write: shows the mod folder that would result.
pub fn preview(
    target: &Target,
    archives: &dyn Archives,
    env: &dyn Environment,
    ui: &mut dyn InstallUi,
    archive: &Path,
    request: &Request,
) -> Result<Preview> {
    let checker = DataChecker::for_game(&target.game_name).ok_or_else(|| Error::UnsupportedGame(target.game_name.clone()))?;
    let mut guessed = guess_from(request);
    let cancel = AtomicBool::new(false);
    let previous = request.previous.clone();
    let mut run =
        Install { target, archives, env, ui, cancel: &cancel, checker, root_sort_asked: false, preview: true, previous, password: None };
    match run.run(archive, &mut guessed, 0)? {
        Done::Preview(p) => Ok(p),
        Done::Installed(_) => unreachable!("previewing"),
    }
}

/// The name guesses the caller brings: the current mod's (`GUESS_PRESET`), the user's.
fn guess_from(request: &Request) -> GuessedName {
    let mut guessed = GuessedName::default();
    if let Some(n) = &request.current_name {
        guessed.update(n, Quality::Preset);
    }
    if let Some(n) = &request.name {
        guessed.update(n, Quality::User);
    }
    guessed
}

fn is_in(file: &Path, dir: &Path) -> bool {
    match (file.parent().and_then(|p| p.canonicalize().ok()), dir.canonicalize().ok()) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// `"d" + lastModified.toString("yyyy.M.d")`: the version of an archive without one.
fn date_version(path: &Path) -> String {
    let secs = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as libc::time_t);
    let (y, m, d, ..) = local_time(secs);
    format!("d{y}.{m}.{d}")
}

/// Local calendar time: (year, month, day, hour, minute, second).
fn local_time(secs: libc::time_t) -> (i32, i32, i32, i32, i32, i32) {
    // SAFETY: localtime_r only writes the tm we pass.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    (tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// `CategoryFactory::resolveNexusID` + `getCategoryID`: the instance category a Nexus
/// category maps to (`nexuscatmap.dat`: `id|name|nexusId`), when that category exists in
/// `categories.dat`; 0 otherwise. MO2 asks what to do with an unmapped category; Strata
/// installs without one.
pub fn nexus_category(dir: &Path, nexus_id: i64) -> i64 {
    if nexus_id == 0 {
        return 0;
    }
    let lines = |file: &str| std::fs::read_to_string(dir.join(file)).unwrap_or_default();
    let ids: Vec<i64> = lines("categories.dat").lines().filter_map(|l| l.split('|').next()?.trim().parse().ok()).collect();
    lines("nexuscatmap.dat")
        .lines()
        .filter_map(|l| {
            let mut parts = l.split('|');
            let id: i64 = parts.next()?.trim().parse().ok()?;
            let nexus: i64 = parts.nth(1)?.trim().parse().ok()?;
            (nexus == nexus_id).then_some(id)
        })
        .find(|id| ids.contains(id))
        .unwrap_or(0)
}

impl Install<'_> {
    /// `InstallationManager::install`.
    fn run(&mut self, archive: &Path, name: &mut GuessedName, mod_id_in: i64) -> Result<Done> {
        self.root_sort_asked = false;
        self.password = None;
        let file_name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = suffix(&file_name).to_owned();
        if !ARCHIVE_EXTENSIONS.contains(&fold(&ext).as_str()) {
            return Err(Error::Unsupported(ext));
        }
        name.update(complete_base_name(&file_name), Quality::Fallback);

        let mut meta = InstallMeta {
            mod_id: mod_id_in,
            file_category: 1,
            repository: "Nexus".to_owned(),
            ..InstallMeta::default()
        };
        if let Some(dl) = DownloadMeta::read_for(archive)? {
            meta.game_name = dl.game_name;
            meta.mod_id = dl.mod_id;
            name.update(&dl.name, Quality::Fallback);
            name.update(&dl.mod_name, Quality::Meta);
            meta.version = dl.version;
            meta.newest_version = dl.newest_version;
            meta.category = nexus_category(&self.target.categories_dir, dl.category);
            meta.repository = dl.repository;
            meta.file_category = dl.file_category;
        }
        if meta.game_name.is_empty() {
            // MO2 writes "" here, then ModInfoRegular::saveMeta replaces it with the managed
            // game's short name (readMeta keeps its default for an empty value)
            meta.game_name.clone_from(&self.target.game_short_name);
        }
        if modpub_mod_id(&file_name).is_some() {
            meta.repository = "ModPub".to_owned();
            meta.page_url = modpub_page_url(&file_name, &self.target.game_short_name).unwrap_or_default();
        }
        if meta.version.is_empty() {
            meta.version = date_version(archive);
        }
        let (guessed_name, guessed_id) = interpret_nexus_file_name(&file_name);
        let other_site = !meta.repository.is_empty() && !meta.repository.eq_ignore_ascii_case("Nexus");
        if meta.mod_id == 0
            && let Some(id) = guessed_id.filter(|_| !other_site)
        {
            meta.mod_id = id;
        }
        name.update(&guessed_name, Quality::Good);
        meta.installation_file = if is_in(archive, &self.target.downloads_dir) {
            file_name.clone()
        } else {
            std::path::absolute(archive).unwrap_or_else(|_| archive.to_owned()).to_string_lossy().into_owned()
        };

        let listing = self.with_password(archive, |a, p, _| a.list(archive, p))?;
        let mut tree = FileTree::from_listing(listing.iter().map(|e| (e.path.as_str(), e.is_dir)));

        // sort game-folder files into Root before any installer sees the archive
        let plan = rootsorter::plan_archive(&tree, |d| self.checker.looks_valid(d));
        self.sort_root(&mut tree, name.value().to_owned(), &plan);

        let mut manual_only = false;
        for installer in Installer::BY_PRIORITY {
            if manual_only && installer != Installer::Manual {
                continue;
            }
            let mut work = tree.clone();
            let answer = match installer {
                Installer::Fomod => self.fomod(archive, &mut work, name, &mut meta)?,
                Installer::Quick => self.quick(&mut work, name),
                Installer::Bain => self.bain(&mut work, name),
                Installer::Bundle => match self.bundle(archive, &tree, name, meta.mod_id)? {
                    Step::Done(done) => return Ok(done),
                    Step::Answer(a) => a,
                },
                Installer::Manual => self.manual(&mut work, name),
            };
            match answer {
                None => continue,
                Some(Answer::Manual) => manual_only = true,
                Some(Answer::Cancel) => return Err(Error::Cancelled),
                Some(Answer::Accept(extra)) => {
                    // sort what the installer produced (FOMOD, BAIN, manual, ...)
                    let plan = rootsorter::plan_installed(&work);
                    self.sort_root(&mut work, name.value().to_owned(), &plan);
                    if self.preview {
                        let exists = self.existing(name.value()).is_some();
                        return Ok(Done::Preview(Preview { name: name.value().to_owned(), installer, files: work.paths(), exists }));
                    }
                    let mut outcome = self.do_install(archive, &work, name, &meta, installer)?;
                    outcome.ini_tweaks = work.find_dir("INI Tweaks").is_some_and(|d| !d.is_empty());
                    let mut m = ModMeta::open(&outcome.mod_dir)?;
                    match extra {
                        Extra::None => {}
                        Extra::Fomod { url, choices } => {
                            m.set_url_if_empty(&url);
                            let json = serde_json::to_string(&choices).unwrap_or_default();
                            m.set_plugin_settings(installer.plugin_name(), &[(FOMOD_CHOICES.to_owned(), json)], true);
                        }
                        Extra::BainOptions(options) => {
                            let settings: Vec<(String, String)> =
                                options.into_iter().enumerate().map(|(i, o)| (format!("option{i}"), o)).collect();
                            m.set_plugin_settings(installer.plugin_name(), &settings, true);
                        }
                    }
                    m.save()?;
                    return Ok(Done::Installed(outcome));
                }
            }
        }
        Err(Error::NoInstaller)
    }

    /// Runs an archive operation, asking for the password when the archive wants one (and
    /// again when it was wrong) until it works or the user cancels. The listing comes first,
    /// so the extractions later have the password already.
    fn with_password<T>(
        &mut self,
        archive: &Path,
        op: impl Fn(&dyn Archives, Option<&str>, &mut dyn InstallUi) -> Result<T>,
    ) -> Result<T> {
        loop {
            let wrong = match op(self.archives, self.password.as_deref(), &mut *self.ui) {
                Err(Error::PasswordNeeded(_)) => self.password.is_some(),
                Err(Error::WrongPassword(_)) => true,
                other => return other,
            };
            let file = archive.file_name().unwrap_or_default().to_string_lossy().into_owned();
            match self.ui.password(&file, wrong) {
                Some(p) if !p.is_empty() => self.password = Some(p),
                _ => return Err(Error::Cancelled),
            }
        }
    }

    /// `InstallationManager::sortRootFiles`: asks once per install.
    fn sort_root(&mut self, tree: &mut FileTree, mod_name: String, plan: &Plan) {
        if plan.is_empty() || self.root_sort_asked {
            return;
        }
        self.root_sort_asked = true;
        if self.ui.root_sort(&mod_name, plan) {
            rootsorter::apply(tree, plan);
        }
    }

    fn quick(&mut self, tree: &mut FileTree, name: &mut GuessedName) -> Option<Answer<Extra>> {
        let base = installers::quick_base(tree, &self.checker)?;
        Some(match self.ui.quick(name) {
            Answer::Accept(()) => {
                installers::quick_install(tree, &base, &self.checker);
                Answer::Accept(Extra::None)
            }
            Answer::Manual => Answer::Manual,
            Answer::Cancel => Answer::Cancel,
        })
    }

    fn bain(&mut self, tree: &mut FileTree, name: &mut GuessedName) -> Option<Answer<Extra>> {
        let (packages, invalid) = installers::bain_packages(tree, &self.checker);
        if packages.len() < 2 || (invalid > 0 && !self.ui.bain_unsure()) {
            return None;
        }
        let defaults = installers::bain_defaults(&packages, &self.previous.bain);
        Some(match self.ui.bain(name, &packages, &defaults) {
            Answer::Accept(mut selected) => {
                installers::bain_install(tree, &selected);
                // the options are a std::set<QString, FileNameComparator>
                selected.sort_by_key(|s| fold(s));
                selected.dedup_by_key(|s| fold(s));
                Answer::Accept(Extra::BainOptions(selected))
            }
            Answer::Manual => Answer::Manual,
            Answer::Cancel => Answer::Cancel,
        })
    }

    fn manual(&mut self, tree: &mut FileTree, name: &mut GuessedName) -> Option<Answer<Extra>> {
        Some(match self.ui.manual(name, tree, &self.checker) {
            Answer::Accept(root) => {
                if !installers::manual_install(tree, &root) {
                    return Some(Answer::Cancel);
                }
                Answer::Accept(Extra::None)
            }
            Answer::Manual | Answer::Cancel => Answer::Cancel,
        })
    }

    fn fomod(
        &mut self,
        archive: &Path,
        tree: &mut FileTree,
        name: &mut GuessedName,
        meta: &mut InstallMeta,
    ) -> Result<Option<Answer<Extra>>> {
        if !installers::is_fomod(tree) {
            return Ok(None);
        }
        let fomod_dir = installers::fomod_dir(tree).unwrap_or_default();
        let base = fomod_dir.rsplit_once('/').map_or("", |(b, _)| b).to_owned();
        let tmp = self.temp()?;
        let files = installers::fomod_files_to_read(tree);
        self.with_password(archive, |a, p, _| a.extract_files(archive, p, &files, tmp.path()))?;
        let read = |file: &str| -> Result<Option<Vec<u8>>> {
            let Some(crate::tree::Node::File(f)) = tree.find(&format!("{fomod_dir}/{file}")) else { return Ok(None) };
            let path = tmp.path().join(&f.source);
            std::fs::read(&path).map(Some).map_err(|e| Error::io(&path, e))
        };
        let info = match read("info.xml")? {
            Some(bytes) => Info::parse(&bytes).unwrap_or_default(),
            None => Info::default(),
        };
        let bytes = read("ModuleConfig.xml")?.ok_or_else(|| Error::Fomod("ModuleConfig.xml is missing".to_owned()))?;
        let config = ModuleConfig::parse(&bytes)?;
        let mut session = Session::new(&config, self.env)?;
        session.remember(self.previous.fomod.clone());

        if let Some(n) = info.name.as_deref() {
            name.update(n, Quality::Meta);
        }
        if let Some(v) = info.version.as_deref().filter(|v| !v.is_empty()) {
            meta.version = v.to_owned();
        }
        if let Some(id) = info.id {
            meta.mod_id = id;
        }
        match self.ui.fomod(name, &info, &mut session, &tmp.path().join(&base)) {
            Answer::Manual => return Ok(Some(Answer::Manual)),
            Answer::Cancel => return Ok(Some(Answer::Cancel)),
            Answer::Accept(()) => {}
        }
        let (new, missing) = installers::fomod_install(tree, &base, &session.files());
        if !missing.is_empty() && !self.ui.fomod_missing(&missing) {
            return Ok(Some(Answer::Cancel));
        }
        *tree = new;
        let choices = session.choices();
        Ok(Some(Answer::Accept(Extra::Fomod { url: info.website.unwrap_or_default(), choices })))
    }

    fn bundle(&mut self, archive: &Path, tree: &FileTree, name: &mut GuessedName, mod_id: i64) -> Result<Step> {
        let found = installers::bundle_archives(tree);
        let (inner, chosen) = match found.as_slice() {
            [] => return Ok(Step::Answer(None)),
            [one] => (one.clone(), false),
            many if self.previous.bundle.as_ref().is_some_and(|p| many.contains(p)) => {
                (self.previous.bundle.clone().unwrap_or_default(), true)
            }
            many => match self.ui.bundle(many) {
                Answer::Accept(pick) => (pick, true),
                Answer::Manual => return Ok(Step::Answer(Some(Answer::Manual))),
                Answer::Cancel => return Err(Error::Cancelled),
            },
        };
        let Some(crate::tree::Node::File(file)) = tree.find(&inner) else {
            return Err(Error::Invalid(format!("'{inner}' is not in the archive")));
        };
        let tmp = self.temp()?;
        self.with_password(archive, |a, p, _| {
            a.extract_files(archive, p, std::slice::from_ref(&file.source), tmp.path())
        })?;
        let outcome = match self.run(&tmp.path().join(&file.source), name, mod_id)? {
            Done::Installed(outcome) => outcome,
            Done::Preview(p) => return Ok(Step::Done(Done::Preview(Preview { installer: Installer::Bundle, ..p }))),
        };
        // InstallerBundle::onInstallationEnd
        let mut m = ModMeta::open(&outcome.mod_dir)?;
        m.set_installation_file(&std::path::absolute(archive).unwrap_or_else(|_| archive.to_owned()).to_string_lossy());
        if chosen {
            m.set_plugin_settings(Installer::Bundle.plugin_name(), &[("archive".to_owned(), inner)], false);
        }
        m.save()?;
        Ok(Step::Done(Done::Installed(Outcome { installer: Installer::Bundle, ..outcome })))
    }

    fn temp(&self) -> Result<tempfile::TempDir> {
        std::fs::create_dir_all(&self.target.temp_dir).map_err(|e| Error::io(&self.target.temp_dir, e))?;
        tempfile::Builder::new()
            .prefix("install-")
            .tempdir_in(&self.target.temp_dir)
            .map_err(|e| Error::io(&self.target.temp_dir, e))
    }

    /// The existing mod folder with this name, compared without case (a Windows folder).
    fn existing(&self, name: &str) -> Option<PathBuf> {
        let exact = self.target.mods_dir.join(name);
        if exact.exists() {
            return Some(exact);
        }
        std::fs::read_dir(&self.target.mods_dir)
            .ok()?
            .flatten()
            .find(|e| fold(&e.file_name().to_string_lossy()) == fold(name))
            .map(|e| e.path())
    }

    /// `doInstall` + `testOverwrite` + the extraction.
    fn do_install(
        &mut self,
        archive: &Path,
        tree: &FileTree,
        name: &mut GuessedName,
        meta: &InstallMeta,
        installer: Installer,
    ) -> Result<Outcome> {
        if name.value().is_empty() {
            return Err(Error::Invalid("the archive gives no usable mod name: give one with --name".to_owned()));
        }
        let (mut merged, mut replaced, mut backup, mut trashed) = (false, false, None, None);
        while let Some(existing) = self.existing(name.value()) {
            let clash = self.ui.clash(name.value());
            if clash.backup && clash.action != ClashAction::Cancel {
                let to = backup_name(&existing);
                copy_dir(&existing, &to)?;
                backup = Some(to);
            }
            match clash.action {
                ClashAction::Cancel => return Err(Error::Cancelled),
                ClashAction::Rename(new) => {
                    name.update(&new, Quality::User);
                    if fold(name.value()) == fold(&existing.file_name().unwrap_or_default().to_string_lossy()) {
                        // the same name again: ask again, as MO2's loop does
                        continue;
                    }
                }
                ClashAction::Merge => {
                    merged = true;
                    break;
                }
                ClashAction::Replace => {
                    let meta_ini = std::fs::read(existing.join("meta.ini")).ok();
                    trashed = Some(self.target.trash.put(&existing)?);
                    std::fs::create_dir(&existing).map_err(|e| Error::io(&existing, e))?;
                    if let Some(bytes) = meta_ini {
                        let path = existing.join("meta.ini");
                        std::fs::write(&path, bytes).map_err(|e| Error::io(&path, e))?;
                    }
                    replaced = true;
                    break;
                }
            }
        }
        let mod_dir = match self.existing(name.value()) {
            Some(dir) => dir,
            None => {
                let dir = self.target.mods_dir.join(name.value());
                std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
                dir
            }
        };
        let fresh = !merged && !replaced;
        if let Err(e) = self.extract(archive, tree, &mod_dir) {
            if fresh {
                let _ = std::fs::remove_dir_all(&mod_dir);
            }
            return Err(e);
        }
        let mut m = ModMeta::open(&mod_dir)?;
        m.write_install(meta);
        m.save()?;
        let name = mod_dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        Ok(Outcome { name, mod_dir, installer, merged, replaced, backup, trashed, ini_tweaks: false, notes: Vec::new() })
    }

    /// Extracts the archive and puts each file of the tree in its place under `mod_dir`.
    fn extract(&mut self, archive: &Path, tree: &FileTree, mod_dir: &Path) -> Result<()> {
        let files = tree.files();
        let tmp = self.temp()?;
        if !files.is_empty() {
            let cancel = self.cancel;
            self.with_password(archive, |a, password, ui| {
                let progress = std::cell::RefCell::new(ui);
                a.extract(archive, password, tmp.path(), &|p| progress.borrow_mut().progress(p), cancel)
            })?;
        }
        let mut uses: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for (_, source) in &files {
            *uses.entry(source).or_default() += 1;
        }
        for path in tree.paths().iter().filter(|p| p.ends_with('/')) {
            let dir = resolve(mod_dir, path);
            std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        }
        for (dest, source) in &files {
            let from = tmp.path().join(source);
            let to = resolve(mod_dir, dest);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            }
            if to.is_dir() {
                std::fs::remove_dir_all(&to).map_err(|e| Error::io(&to, e))?;
            }
            let left = uses.get_mut(source).expect("counted above");
            *left -= 1;
            if *left > 0 || std::fs::rename(&from, &to).is_err() {
                std::fs::copy(&from, &to).map_err(|e| Error::io(&from, e))?;
            }
        }
        Ok(())
    }
}

enum Step {
    Done(Done),
    Answer(Option<Answer<Extra>>),
}

/// `base/rel`, where each existing folder is found without case (merging into a mod whose
/// folders are spelled differently must not make a second folder on Linux).
fn resolve(base: &Path, rel: &str) -> PathBuf {
    let mut path = base.to_owned();
    for part in crate::tree::split(rel) {
        let exact = path.join(part);
        if exact.exists() {
            path = exact;
            continue;
        }
        let found = std::fs::read_dir(&path)
            .ok()
            .and_then(|rd| rd.flatten().find(|e| fold(&e.file_name().to_string_lossy()) == fold(part)))
            .map(|e| e.path());
        path = found.unwrap_or(exact);
    }
    path
}

/// `generateBackupName`: `<dir>_backup`, then `_backup2`, `_backup3`...
fn backup_name(dir: &Path) -> PathBuf {
    let base = format!("{}_backup", dir.display());
    let mut candidate = PathBuf::from(&base);
    let mut i = 2;
    while candidate.exists() {
        candidate = PathBuf::from(format!("{base}{i}"));
        i += 1;
    }
    candidate
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).map_err(|e| Error::io(to, e))?;
    for entry in std::fs::read_dir(from).map_err(|e| Error::io(from, e))? {
        let entry = entry.map_err(|e| Error::io(from, e))?;
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(|e| Error::io(entry.path(), e))?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| Error::io(entry.path(), e))?;
        }
    }
    Ok(())
}
