//! The mod info dialog (`ModInfoDialog`, `modinfodialog*.cpp`), as plain Rust: what each tab
//! lists, and the edits it makes (all through files the core owns).
//!
//! Tabs, in MO2's order (`ModInfoDialog::createTabs`): text files, ini files, images, optional
//! plugins, conflicts (general and advanced), categories, Nexus info, notes, file tree. The file
//! tabs share one walk of the mod folder; each file goes to the first tab that wants it
//! (`ModInfoDialog::feedFiles`).

use std::path::{Path, PathBuf};

use strata_core::{Categories, Endorsed, Instance, ModKind, ModMeta, Profile};
use strata_vfs::conflicts::{Area, ConflictIndex, FileConflict, Provider};

use crate::{Error, Result};

/// `TextFilesTab::wantsFile`.
const TEXT_EXTENSIONS: &[&str] = &[".txt", ".json", ".cfg", ".log", ".toml"];
/// Image formats: what Qt reads on this system (`QImageReader::supportedImageFormats`, which
/// MO2's `ImagesTab` asks) plus `dds`, decoded by `preview`.
const IMAGE_EXTENSIONS: &[&str] = &[
    ".bmp", ".dds", ".gif", ".ico", ".jpeg", ".jpg", ".pbm", ".pgm", ".png", ".ppm", ".svg", ".tga", ".tif", ".tiff",
    ".webp", ".xbm", ".xpm",
];
/// `ESPsTab::feedFile`.
const PLUGIN_EXTENSIONS: &[&str] = &[".esp", ".esm", ".esl"];

/// A plugin of the mod: active when it sits in the mod's top folder, where the game sees it
/// (`ESPItem`); optional when it is anywhere below.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    /// Relative to the mod folder, `/`-separated.
    pub path: String,
    pub active: bool,
}

/// The mod's files, split as the file tabs take them. Paths relative to the mod folder, sorted
/// without regard to case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Files {
    pub text: Vec<String>,
    pub ini: Vec<String>,
    pub images: Vec<String>,
    pub plugins: Vec<Plugin>,
}

/// One row of the general conflicts tab (`GeneralConflictsTab::addFiles`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneralRow {
    /// `/meshes/x.nif`, `/Root/x.dll`.
    pub shown: String,
    /// Overwrites: every other provider, `, `-joined, lowest first. Overwritten: the winner.
    /// No conflict: empty.
    pub others: String,
    /// The file on disk in this mod (for previews and "open"); `None` when it is in an archive.
    pub real: Option<PathBuf>,
    /// The other mods of the row, one by one ("go to"), lowest first.
    pub mods: Vec<String>,
}

/// One row of the advanced conflicts tab (`AdvancedConflictsTab::createItem`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvancedRow {
    pub shown: String,
    /// Providers below this mod, `, `-joined, lowest first; and only the nearest.
    pub before_all: String,
    pub before_nearest: String,
    /// Providers above this mod, `, `-joined, lowest first; and only the nearest.
    pub after_all: String,
    pub after_nearest: String,
    pub real: Option<PathBuf>,
    /// Every other provider of the file, lowest first ("go to").
    pub mods: Vec<String>,
}

/// The conflicts tabs of one mod.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conflicts {
    pub overwrites: Vec<GeneralRow>,
    pub overwritten: Vec<GeneralRow>,
    pub no_conflict: Vec<GeneralRow>,
    /// Every file of the mod (no-conflict ones have empty before and after).
    pub advanced: Vec<AdvancedRow>,
}

/// One category of `categories.dat`, with its state for this mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryRow {
    pub id: i32,
    pub name: String,
    pub parent: i32,
    pub checked: bool,
}

/// The Nexus tab's fields, read only (no network here).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Nexus {
    pub mod_id: i64,
    pub version: String,
    pub newest_version: String,
    pub url: String,
    pub custom_url: bool,
    pub description: String,
    pub endorsed: String,
    pub tracked: bool,
    pub game_name: String,
    pub last_query: String,
}

/// Everything the dialog shows for one mod.
#[derive(Debug, Clone)]
pub struct ModInfo {
    pub name: String,
    pub dir: PathBuf,
    pub is_separator: bool,
    pub files: Files,
    pub conflicts: Conflicts,
    pub categories: Vec<CategoryRow>,
    /// The primary category (0: none).
    pub primary_category: i32,
    pub nexus: Nexus,
    pub comments: String,
    pub notes: String,
}

fn ends_with_any(name: &str, list: &[&str]) -> bool {
    let lower = name.to_lowercase();
    list.iter().any(|e| lower.ends_with(e))
}

/// The file tabs' lists for a mod folder (`ModInfoDialog::feedFiles`: every regular file,
/// recursively; text first, then ini (not `meta.ini`, `IniFilesTab::wantsFile`), images,
/// plugins).
pub fn classify(dir: &Path) -> Files {
    let mut files = Files::default();
    let mut stack = vec![(dir.to_owned(), String::new())];
    while let Some((real, rel)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&real) else { continue };
        for e in entries.flatten() {
            let Ok(name) = e.file_name().into_string() else { continue };
            let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let Ok(meta) = std::fs::metadata(e.path()) else { continue };
            if meta.is_dir() {
                stack.push((e.path(), path));
            } else if !meta.is_file() {
            } else if ends_with_any(&name, TEXT_EXTENSIONS) {
                files.text.push(path);
            } else if ends_with_any(&name, &[".ini"]) {
                // MO2 compares with case: only `meta.ini` itself is left out.
                if !path.ends_with("meta.ini") {
                    files.ini.push(path);
                }
            } else if ends_with_any(&name, IMAGE_EXTENSIONS) {
                files.images.push(path);
            } else if ends_with_any(&name, PLUGIN_EXTENSIONS) {
                files.plugins.push(Plugin { active: !path.contains('/'), path });
            }
        }
    }
    let key = |s: &String| s.to_lowercase();
    files.text.sort_by_cached_key(key);
    files.ini.sort_by_cached_key(key);
    files.images.sort_by_cached_key(key);
    files.plugins.sort_by_cached_key(|p| p.path.to_lowercase());
    files
}

fn provider_text(p: &Provider<'_>) -> String {
    p.layer.to_owned()
}

fn join(list: &[Provider<'_>]) -> String {
    list.iter().map(provider_text).collect::<Vec<_>>().join(", ")
}

/// The conflicts tabs for `name` from the index. `dir` is the mod folder (for real paths).
pub fn conflicts(index: &ConflictIndex, name: &str, dir: &Path) -> Conflicts {
    let mut out = Conflicts::default();
    let Some(files) = index.files_of(name) else { return out };
    for f in files {
        let FileConflict { area, path, origins } = &f;
        let Some(me) = origins.iter().position(|p| p.layer == name) else { continue };
        let real = origins[me].archive.is_none().then(|| match area {
            Area::Data => dir.join(path),
            Area::Root => dir.join("Root").join(path),
        });
        let shown = f.shown();
        let winner = origins.len() - 1;
        let names = |list: &[Provider<'_>]| list.iter().map(provider_text).collect::<Vec<_>>();
        let row = |others: String, mods: Vec<String>| GeneralRow { shown: shown.clone(), others, real: real.clone(), mods };
        if me != winner {
            out.overwritten.push(row(provider_text(&origins[winner]), names(&origins[winner..])));
        } else if origins.len() > 1 {
            out.overwrites.push(row(join(&origins[..winner]), names(&origins[..winner])));
        } else {
            out.no_conflict.push(row(String::new(), Vec::new()));
        }
        let (below, above) = (&origins[..me], &origins[me + 1..]);
        out.advanced.push(AdvancedRow {
            shown,
            before_all: join(below),
            before_nearest: below.last().map(provider_text).unwrap_or_default(),
            after_all: join(above),
            after_nearest: above.first().map(provider_text).unwrap_or_default(),
            real,
            mods: below.iter().chain(above).map(provider_text).collect(),
        });
    }
    out
}

fn endorsed_text(e: Endorsed) -> &'static str {
    match e {
        Endorsed::True => "endorsed",
        Endorsed::False => "not endorsed",
        Endorsed::Never => "won't endorse",
        Endorsed::Unknown => "unknown",
    }
}

impl ModInfo {
    /// Reads what the dialog shows for the mod `name` of the profile. Overwrite and foreign
    /// mods have their own dialogs in MO2 and are refused here.
    pub fn open(instance: &Instance, profile: &Profile, name: &str) -> Result<Self> {
        let entry = instance
            .mod_list(profile)?
            .into_iter()
            .find(|m| m.name == name)
            .ok_or_else(|| Error::Invalid(format!("there is no mod named {name:?} in this profile")))?;
        let (ModKind::Regular | ModKind::Separator | ModKind::Backup, Some(dir)) = (entry.kind, entry.path) else {
            return Err(Error::Invalid(format!("{name:?} has no mod folder: MO2 shows no mod information for it")));
        };
        let is_separator = entry.kind == ModKind::Separator;
        let meta = ModMeta::open(&dir)?;
        let files = if is_separator { Files::default() } else { classify(&dir) };
        let conflicts = if is_separator || !entry.enabled {
            Conflicts::default()
        } else {
            conflicts(&super::build_index(instance, profile)?, name, &dir)
        };
        let ids = meta.categories();
        let categories = Categories::open(&instance.root)?
            .list
            .into_iter()
            .map(|c| CategoryRow { checked: ids.contains(&c.id), id: c.id, name: c.name, parent: c.parent })
            .collect();
        let nexus = Nexus {
            mod_id: meta.mod_id(),
            version: meta.version(),
            newest_version: meta.newest_version(),
            url: meta.url(),
            custom_url: meta.has_custom_url(),
            description: description_html(&meta.nexus_description()),
            endorsed: endorsed_text(meta.endorsed()).to_owned(),
            tracked: meta.tracked(),
            game_name: meta.game_name(),
            last_query: meta.last_nexus_query(),
        };
        Ok(Self {
            name: name.to_owned(),
            is_separator,
            files,
            conflicts,
            categories,
            primary_category: ids.first().copied().unwrap_or(0),
            nexus,
            comments: meta.comments(),
            notes: meta.notes(),
            dir,
        })
    }
}

/// The real path of a file of the mod, refusing paths that leave the folder.
fn inside(dir: &Path, rel: &str) -> Result<PathBuf> {
    if rel.is_empty() || rel.starts_with('/') || rel.split('/').any(|p| p == ".." || p.is_empty()) {
        return Err(Error::Invalid(format!("{rel:?} is not a path inside the mod")));
    }
    Ok(dir.join(rel))
}

/// Saves the text and ini tabs' editor (`TextEditor::save`): the bytes as given. The file must
/// exist already (the tabs only edit files they listed).
pub fn save_file(dir: &Path, rel: &str, content: &[u8]) -> Result<()> {
    let path = inside(dir, rel)?;
    if !path.is_file() {
        return Err(Error::Invalid(format!("{} is gone: it was not saved", path.display())));
    }
    strata_core::write::write_if_changed(&path, content).map(|_| ()).map_err(Error::from)
}

/// Saves the notes tab: the Notes column text (`comments`) and the notes (HTML), through the
/// core's `meta.ini` view. Returns whether the file changed.
/// A value that reads the same as before is not written (a missing key stays missing).
pub fn save_notes(dir: &Path, comments: &str, notes: &str) -> Result<bool> {
    let mut meta = ModMeta::open(dir)?;
    let mut changed = false;
    if meta.comments() != comments {
        changed |= meta.set_comments(comments);
    }
    if meta.notes() != notes {
        changed |= meta.set_notes(notes);
    }
    Ok(changed && meta.save()?)
}

/// Saves the categories tab (`CategoriesTab::save`): the checked ids and the primary one.
/// A primary that is not checked counts as checked.
/// Unchanged categories are not written (a mod without the key keeps it absent).
pub fn save_categories(dir: &Path, primary: i32, checked: &[i32]) -> Result<bool> {
    let mut meta = ModMeta::open(dir)?;
    let old = meta.categories();
    let mut wanted: Vec<i32> = checked.iter().copied().filter(|id| *id > 0).collect();
    if primary > 0 && !wanted.contains(&primary) {
        wanted.push(primary);
    }
    let mut old_sorted = old.clone();
    old_sorted.sort_unstable();
    wanted.sort_unstable();
    wanted.dedup();
    if old_sorted == wanted && old.first().copied().unwrap_or(0) == primary.max(0) {
        return Ok(false);
    }
    let primary = (primary > 0).then_some(primary);
    Ok(meta.set_categories(primary, checked) && meta.save()?)
}

/// `ESPsTab::onActivate`: moves an optional plugin to the mod's top folder as `new_name`.
/// Refuses when a file of that name is already there (MO2 then asks for another name).
/// Returns the new relative path.
pub fn activate_plugin(dir: &Path, rel: &str, new_name: &str) -> Result<String> {
    let from = inside(dir, rel)?;
    if new_name.contains('/') {
        return Err(Error::Invalid(format!("{new_name:?} is not a file name")));
    }
    let to = inside(dir, new_name)?;
    if to.exists() {
        return Err(Error::Invalid(format!("a file named {new_name} exists: enter a new name")));
    }
    std::fs::rename(&from, &to).map_err(|source| Error::Write { path: to, source })?;
    Ok(new_name.to_owned())
}

/// `ESPsTab::onDeactivate`: moves an active plugin back to `back_to` (where it was made
/// active from in this dialog) or into `optional/`. Returns the new relative path.
pub fn deactivate_plugin(dir: &Path, rel: &str, back_to: Option<&str>) -> Result<String> {
    let from = inside(dir, rel)?;
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let target = match back_to.filter(|b| !b.is_empty()) {
        Some(back) => back.to_owned(),
        None => format!("optional/{name}"),
    };
    let to = inside(dir, &target)?;
    if to.exists() {
        return Err(Error::Invalid(format!("{target} exists already: the plugin was not moved")));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Write { path: parent.to_owned(), source })?;
    }
    std::fs::rename(&from, &to).map_err(|source| Error::Write { path: to, source })?;
    Ok(target)
}


/// The Nexus tab's description: the BBCode converted as MO2 does, or MO2's note when there is
/// none (`ModInfoDialogNexus::updateWebpage`).
fn description_html(bbcode: &str) -> String {
    if bbcode.is_empty() {
        "<div style=\"text-align: center;\"><p>This mod does not have a valid Nexus ID. You can add a custom web \
         page for it in the \"Custom URL\" box below.</p></div>"
            .to_owned()
    } else {
        super::bbcode::to_html(bbcode)
    }
}

#[cfg(test)]
mod tests;
