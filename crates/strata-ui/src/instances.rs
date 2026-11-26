//! The instance manager (MO2's `InstanceManagerDialog` and `CreateInstanceDialog`).
//!
//! MO2 knows two kinds of instance: global ones, each a folder under one root
//! (`%LOCALAPPDATA%/ModOrganizer`, or Leet change 1's `instances_root`), and one portable
//! instance next to the program. Strata's instances are all portable folders (as
//! Fluorine's), so the manager lists:
//!
//! 1. every folder with a `ModOrganizer.ini` directly under the instances root
//!    (`[General] instances_root` in Strata's own `strata.ini`, by default `~/Games`), and
//! 2. every instance the window opened elsewhere (`[General] known_instances`), still there.
//!
//! Rename, delete and create work on those folders. Nothing here touches a prefix outside
//! the instance folder: several instances may share one.

use std::path::{Path, PathBuf};

use strata_core::Instance;
use strata_core::settings::{Missing, store_ini};
use strata_ini::{Document, Value};

use crate::{Error, Result};

const SECTION: &str = "";
const ROOT_KEY: &str = "instances_root";
const KNOWN_KEY: &str = "known_instances";

/// What the instance manager shows of one instance (`Instance::readFromIni`). Read from
/// `ModOrganizer.ini` without opening the instance, so one Strata doesn't support (or a
/// broken one) is still listed, renamed and deleted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstanceInfo {
    /// The folder's name (MO2's `displayName`).
    pub name: String,
    /// The instance folder.
    pub root: PathBuf,
    /// `[Settings] base_directory`, else the folder.
    pub base_dir: PathBuf,
    /// `[General] gameName`.
    pub game_name: String,
    /// `[General] gamePath` on this machine ("" when unset).
    pub game_dir: String,
    /// `[fluorine] prefix_path` ("" when unset).
    pub prefix: String,
    /// `[fluorine] proton_path` ("" when unset).
    pub proton: String,
    /// Whether Strata supports the game.
    pub supported: bool,
}

fn read_doc(path: &Path) -> Option<Document> {
    std::fs::read(path).ok().map(|d| Document::parse(&d))
}

fn is_instance(dir: &Path) -> bool {
    dir.join("ModOrganizer.ini").is_file()
}

/// The default instances root: `~/Games`, a common home for instances.
fn default_root() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Games")
}

/// The instances root stored in `config`, else `~/Games`.
pub fn instances_root(config: Option<&Path>) -> PathBuf {
    config
        .and_then(read_doc)
        .and_then(|d| d.string(SECTION, ROOT_KEY))
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_root)
}

/// Stores the instances root (empty: back to the default).
pub fn set_instances_root(config: &Path, root: &Path) -> Result<()> {
    let value = (!root.as_os_str().is_empty()).then(|| Value::String(root.to_string_lossy().into_owned()));
    store_ini(config, &[(SECTION, ROOT_KEY, value)], Missing::Create)?;
    Ok(())
}

fn known(config: &Path) -> Vec<PathBuf> {
    let Some(doc) = read_doc(config) else { return Vec::new() };
    match doc.value(SECTION, KNOWN_KEY) {
        Some(Value::List(items)) => items.into_iter().filter(|s| !s.is_empty()).map(PathBuf::from).collect(),
        Some(Value::String(s)) if !s.is_empty() => vec![PathBuf::from(s)],
        _ => Vec::new(),
    }
}

fn store_known(config: &Path, list: &[PathBuf]) -> Result<()> {
    let value = (!list.is_empty()).then(|| Value::List(list.iter().map(|p| p.to_string_lossy().into_owned()).collect()));
    store_ini(config, &[(SECTION, KNOWN_KEY, value)], Missing::Create)?;
    Ok(())
}

/// Adds `instance` to the known instances when it is outside the instances root (those
/// inside are found by the scan). Writes nothing when nothing changes.
pub fn remember(config: &Path, instance: &Path) -> Result<()> {
    let root = instances_root(Some(config));
    if instance.parent() == Some(root.as_path()) {
        return Ok(());
    }
    let mut list = known(config);
    if list.iter().any(|p| p == instance) {
        return Ok(());
    }
    list.push(instance.to_owned());
    store_known(config, &list)
}

/// Replaces `from` by `to` in the known instances and as the last instance (a rename);
/// `to` = None drops it (a delete).
fn forget_or_move(config: &Path, from: &Path, to: Option<&Path>) -> Result<()> {
    let mut list = known(config);
    let before = list.len();
    list.retain(|p| p != from);
    if list.len() != before {
        if let Some(to) = to {
            list.push(to.to_owned());
        }
        store_known(config, &list)?;
    }
    if crate::config::last_instance(config).as_deref() == Some(from) {
        let value = to.map(|t| Value::String(t.to_string_lossy().into_owned()));
        store_ini(config, &[(SECTION, "last_instance", value)], Missing::Create)?;
    }
    Ok(())
}

/// Reads what the manager shows of the instance in `root`.
pub fn info(root: &Path) -> InstanceInfo {
    let doc = read_doc(&root.join("ModOrganizer.ini")).unwrap_or_default();
    let text = |section: &str, key: &str| doc.string(section, key).unwrap_or_default();
    let game_name = text("", "gameName");
    let stored_game = text("", "gamePath");
    let game_dir = if stored_game.is_empty() {
        String::new()
    } else {
        strata_core::winepath::to_host(&stored_game, None)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(stored_game)
    };
    let base = text("Settings", "base_directory");
    let base_dir = if base.is_empty() {
        root.to_owned()
    } else {
        strata_core::winepath::to_host(&base, None).unwrap_or_else(|_| PathBuf::from(base))
    };
    InstanceInfo {
        name: root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        root: root.to_owned(),
        base_dir,
        supported: strata_game::by_name(&game_name).is_some(),
        game_name,
        game_dir,
        prefix: text("fluorine", "prefix_path"),
        proton: text("fluorine", "proton_path"),
    }
}

/// Every instance the manager lists: the instances root's, then the known ones, sorted by
/// name without case (MO2 sorts with `naturalCompare`). `extra` (the open instance) is
/// always listed.
pub fn list(config: Option<&Path>, extra: Option<&Path>) -> Vec<InstanceInfo> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut add = |p: PathBuf| {
        let p = p.canonicalize().unwrap_or(p);
        if is_instance(&p) && !roots.contains(&p) {
            roots.push(p);
        }
    };
    if let Ok(entries) = std::fs::read_dir(instances_root(config)) {
        for e in entries.filter_map(|e| e.ok()) {
            add(e.path());
        }
    }
    if let Some(config) = config {
        known(config).into_iter().for_each(&mut add);
        if let Some(last) = crate::config::last_instance(config) {
            add(last);
        }
    }
    if let Some(extra) = extra {
        add(extra.to_owned());
    }
    let mut out: Vec<InstanceInfo> = roots.iter().map(|r| info(r)).collect();
    out.sort_by(|a, b| natural_key(&a.name).cmp(&natural_key(&b.name)).then_with(|| a.root.cmp(&b.root)));
    out
}

/// A sort key that orders "Instance 2" before "Instance 10" and ignores case.
fn natural_key(name: &str) -> Vec<(u8, u64, String)> {
    let mut out = Vec::new();
    let mut chars = name.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            let mut n = String::new();
            while let Some(&d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                n.push(d);
                chars.next();
            }
            out.push((0, n.parse().unwrap_or(u64::MAX), String::new()));
        } else {
            let mut s = String::new();
            while let Some(&d) = chars.peek().filter(|d| !d.is_ascii_digit()) {
                s.extend(d.to_lowercase());
                chars.next();
            }
            out.push((1, 0, s));
        }
    }
    out
}

/// MO2's instance name check (`validInstanceName` / the dialog's error label): "" when the
/// name is good, else why not.
pub fn name_problem(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." || name.contains(['/', '\0']) || trimmed != name {
        return "The instance name must be a valid folder name.".into();
    }
    String::new()
}

/// Why `root` cannot be renamed or deleted now ("" when it can): an overlay daemon of this
/// instance runs (a deploy, a game or tool started from it).
pub fn busy(root: &Path) -> String {
    let socket = strata_vfs::daemon::socket_path(root);
    if strata_vfs::daemon::is_running(&socket) {
        return "Strata's overlay runs for this instance (it is deployed, or a program started from it still \
                runs). Undeploy it or end the program first."
            .into();
    }
    String::new()
}

/// Replaces `old` (in its host form and its Wine form `Z:\...`) at the start of every path in
/// `ModOrganizer.ini`'s values with `new`. Returns the new file, or None when nothing named
/// the old folder.
fn rewrite_paths(data: &[u8], old: &Path, new: &Path) -> Option<Vec<u8>> {
    let mut doc = Document::parse(data);
    let pairs = [
        (old.to_string_lossy().into_owned(), new.to_string_lossy().into_owned()),
        (strata_core::winepath::to_wine(old), strata_core::winepath::to_wine(new)),
        // QSettings files written by MO2 on Windows sometimes hold forward slashes
        (strata_core::winepath::to_wine(old).replace('\\', "/"), strata_core::winepath::to_wine(new).replace('\\', "/")),
    ];
    let replace = |text: &str| -> Option<String> {
        let mut out = text.to_owned();
        let mut changed = false;
        for (from, to) in &pairs {
            let mut i = 0;
            while let Some(found) = out[i..].find(from.as_str()) {
                let at = i + found;
                let end = at + from.len();
                let boundary = out[end..].chars().next().is_none_or(|c| c == '/' || c == '\\' || c == '"' || c == ',');
                if boundary {
                    out.replace_range(at..end, to);
                    changed = true;
                    i = at + to.len();
                } else {
                    i = end;
                }
            }
        }
        changed.then_some(out)
    };
    let mut edits: Vec<(String, String, Value)> = Vec::new();
    for entry in doc.entries() {
        let value = match entry.value() {
            Value::String(s) => replace(&s).map(Value::String),
            Value::ByteArray(b) => std::str::from_utf8(&b).ok().and_then(|s| replace(s)).map(|s| Value::ByteArray(s.into_bytes())),
            Value::List(items) => {
                let new: Vec<_> = items.iter().map(|s| replace(s)).collect();
                new.iter().any(Option::is_some).then(|| {
                    Value::List(new.into_iter().zip(&items).map(|(n, o)| n.unwrap_or_else(|| o.clone())).collect())
                })
            }
            Value::Invalid | Value::Other(_) => None,
        };
        if let Some(value) = value {
            edits.push((entry.section.to_owned(), entry.key.to_owned(), value));
        }
    }
    if edits.is_empty() {
        return None;
    }
    for (section, key, value) in edits {
        doc.set(&section, &key, &value);
    }
    Some(doc.to_bytes())
}

/// Renames the instance folder (MO2's `InstanceManagerDialog::rename`) and rewrites the
/// paths into it in its `ModOrganizer.ini` (game folder, prefix, executables, ...). Refuses
/// when the instance is busy or the new name is taken. Returns the new folder.
pub fn rename(config: Option<&Path>, root: &Path, new_name: &str) -> Result<PathBuf> {
    let problem = name_problem(new_name);
    if !problem.is_empty() {
        return Err(Error::Invalid(problem));
    }
    let busy = busy(root);
    if !busy.is_empty() {
        return Err(Error::Invalid(busy));
    }
    let parent = root.parent().ok_or_else(|| Error::Invalid(format!("{} has no parent folder", root.display())))?;
    let target = parent.join(new_name);
    if target == root {
        return Ok(target);
    }
    if target.exists() && !target.as_os_str().eq_ignore_ascii_case(root.as_os_str()) {
        return Err(Error::Invalid("An instance with this name already exists.".into()));
    }
    std::fs::rename(root, &target).map_err(|source| Error::Write { path: target.clone(), source })?;
    let ini = target.join("ModOrganizer.ini");
    if let Ok(data) = std::fs::read(&ini)
        && let Some(new) = rewrite_paths(&data, root, &target)
    {
        strata_core::write::write_if_changed(&ini, &new)?;
    }
    if let Some(config) = config {
        forget_or_move(config, root, Some(&target))?;
    }
    Ok(target)
}

/// One thing "Delete instance" offers to delete (MO2's `Instance::objectsForDeletion`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionCandidate {
    pub path: PathBuf,
    /// The instance folder itself: always deleted. The rest is unticked by default.
    pub mandatory: bool,
}

/// The instance folder, then every folder of its `[Settings]` (base, mods, downloads,
/// profiles, overwrite, cache) that lies outside it. The game folder and a prefix outside
/// the instance are never offered: a prefix may be shared by other instances.
pub fn deletion_candidates(root: &Path) -> Vec<DeletionCandidate> {
    let mut out = vec![DeletionCandidate { path: root.to_owned(), mandatory: true }];
    if let Ok(instance) = Instance::open(root) {
        let p = &instance.paths;
        for dir in [&p.base, &p.mods, &p.downloads, &p.profiles, &p.overwrite, &p.cache] {
            let dir = dir.canonicalize().unwrap_or_else(|_| dir.clone());
            if dir.exists() && !dir.starts_with(root) && !out.iter().any(|c| c.path == dir) {
                out.push(DeletionCandidate { path: dir, mandatory: false });
            }
        }
    }
    out
}

/// Deletes `paths` (to the trash with `trash`), the instance's entries in `config` with
/// them. `root` must be the first path and is refused while busy.
pub fn delete(config: Option<&Path>, root: &Path, paths: &[PathBuf], trash: bool) -> Result<()> {
    let busy = busy(root);
    if !busy.is_empty() {
        return Err(Error::Invalid(busy));
    }
    let bin = if trash { Some(strata_core::trash::Trash::home()?) } else { None };
    for path in paths {
        strata_core::trash::remove(path, bin.as_ref())?;
    }
    if let Some(config) = config {
        forget_or_move(config, root, None)?;
    }
    Ok(())
}

/// A game the wizard offers (MO2's game page): its name, and where Steam has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameChoice {
    pub name: String,
    /// The game's exe, relative to its folder (what the folder must hold).
    pub binary: String,
    /// The game's folder in a Steam library, "" when Steam doesn't have it.
    pub steam_dir: String,
}

/// The games Strata supports, with their Steam folders when installed.
pub fn games() -> Vec<GameChoice> {
    let steam = strata_launch::Steam::detect().ok();
    strata_game::all()
        .into_iter()
        .map(|g| GameChoice {
            name: g.name().to_owned(),
            binary: g.binary().to_owned(),
            steam_dir: steam
                .as_ref()
                .and_then(|s| s.app_dir(g.steam_app_id()))
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        })
        .collect()
}

/// The Protons on this machine (as `strata proton list`): name, folder, and a note when
/// the Steam Linux Runtime it needs is missing ("" when fine).
pub fn protons() -> Vec<(String, PathBuf, String)> {
    let Ok(steam) = strata_launch::Steam::detect() else { return Vec::new() };
    strata_launch::steam::find_protons(&steam)
        .into_iter()
        .map(|p| {
            let note = match (p.runtime, p.runtime_installed) {
                (Some(id), false) => format!("needs Steam Linux Runtime {id}: steam steam://install/{id}"),
                _ => String::new(),
            };
            (p.name, p.path, note)
        })
        .collect()
}

/// What a new instance is made from (MO2's `CreateInstanceDialog::CreationInfo`, with the
/// Proton fields Strata needs instead of the global / portable choice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewInstance {
    /// The new instance folder (must not exist, or be empty).
    pub folder: PathBuf,
    /// `gameName` as MO2 writes it.
    pub game_name: String,
    /// The game folder (holds the game's exe).
    pub game_dir: PathBuf,
    /// The Proton folder (holds the `proton` script).
    pub proton: PathBuf,
    /// The prefix folder (holds `pfx/`); None: `<folder>/Prefix`.
    pub prefix: Option<PathBuf>,
    /// The profile's local inis and saves (MO2's profile settings checkboxes; on in all the
    /// user's instances).
    pub local_inis: bool,
    pub local_saves: bool,
}

/// The `ModOrganizer.ini` of a new portable instance, as MO2 writes one after its first
/// start (`[General]` game, path, profile, version; `[Settings]` folders under
/// `%BASE_DIR%`, local inis and saves; the executables found in the game folder) plus the
/// `[fluorine]` keys Strata reads for Proton.
pub fn portable_ini(game_def: &dyn strata_game::Game, game: &Path, prefix: &Path, proton: &Path) -> Document {
    let game_name = game_def.name();
    let mut ini = Document::default();
    let s = |v: &str| Value::String(v.to_owned());
    let wine_game = strata_core::winepath::to_wine(game);
    ini.set("", "gameName", &s(game_name));
    ini.set("", "selected_profile", &Value::ByteArray(b"Default".to_vec()));
    ini.set("", "gamePath", &Value::ByteArray(wine_game.clone().into_bytes()));
    ini.set("", "game_edition", &s("Steam"));
    ini.set("", "version", &s("2.5.2"));
    ini.set("", "first_start", &s("false"));
    ini.set("", "portable", &s("true"));
    let found: Vec<_> = game_def.executables().iter().filter(|(_, exe)| game.join(exe).is_file()).collect();
    ini.set("customExecutables", "size", &s(&found.len().to_string()));
    for (i, (title, exe)) in found.iter().enumerate() {
        let n = i + 1;
        let key = |k: &str| format!("{n}/{k}");
        ini.set("customExecutables", &key("arguments"), &s(""));
        ini.set("customExecutables", &key("binary"), &s(&format!("{wine_game}\\{exe}")));
        ini.set("customExecutables", &key("hide"), &s("false"));
        ini.set("customExecutables", &key("ownicon"), &s("false"));
        ini.set("customExecutables", &key("steamAppID"), &s(""));
        ini.set("customExecutables", &key("title"), &s(title));
        ini.set("customExecutables", &key("toolbar"), &s("true"));
        ini.set("customExecutables", &key("workingDirectory"), &s(&wine_game));
    }
    ini.set("Settings", "download_directory", &s("%BASE_DIR%/downloads"));
    ini.set("Settings", "mod_directory", &s("%BASE_DIR%/mods"));
    ini.set("Settings", "overwrite_directory", &s("%BASE_DIR%/overwrite"));
    ini.set("Settings", "profile_local_inis", &s("true"));
    ini.set("Settings", "profile_local_saves", &s("true"));
    ini.set("fluorine", "prefix_path", &s(&prefix.to_string_lossy()));
    ini.set("fluorine", "proton_path", &s(&proton.to_string_lossy()));
    ini.set("fluorine", "steam_drm", &s("true"));
    ini
}

/// Creates a portable instance (Fluorine's `create-portable`, MO2's
/// `CreateInstanceDialog::finish`): the folders, a `Default` profile and `ModOrganizer.ini`.
/// The prefix is not created here: `strata prefix setup` does that. Returns the folder.
pub fn create(new: &NewInstance) -> Result<PathBuf> {
    let invalid = |m: String| Error::Invalid(m);
    let folder = &new.folder;
    if folder.exists() && std::fs::read_dir(folder).map_err(|source| Error::Read { path: folder.clone(), source })?.next().is_some() {
        return Err(invalid(format!("{} exists and is not empty. Give a new folder.", folder.display())));
    }
    let Some(game_def) = strata_game::by_name(&new.game_name) else {
        let names: Vec<_> = strata_game::all().iter().map(|g| g.name()).collect();
        return Err(invalid(format!("Strata does not support the game \"{}\" yet (supported: {})", new.game_name, names.join(", "))));
    };
    let game = new.game_dir.canonicalize().map_err(|_| invalid(format!("cannot find the game folder {}", new.game_dir.display())))?;
    if !game.join(game_def.binary()).is_file() {
        return Err(invalid(format!("{} has no {}: give the game folder", game.display(), game_def.binary())));
    }
    let proton = new.proton.canonicalize().map_err(|_| invalid(format!("cannot find {}", new.proton.display())))?;
    strata_launch::Proton::at(&proton).map_err(|e| invalid(e.to_string()))?;
    let write_err = |path: &Path| {
        let path = path.to_owned();
        move |source| Error::Write { path, source }
    };
    std::fs::create_dir_all(folder).map_err(write_err(folder))?;
    let folder = folder.canonicalize().map_err(|source| Error::Read { path: folder.clone(), source })?;
    let prefix = match &new.prefix {
        Some(p) => std::path::absolute(p).map_err(|source| Error::Read { path: p.clone(), source })?,
        None => folder.join("Prefix"),
    };
    for dir in ["mods", "downloads", "overwrite", "profiles/Default/saves", "logs"] {
        let d = folder.join(dir);
        std::fs::create_dir_all(&d).map_err(write_err(&d))?;
    }
    let header = format!("{}\r\n", strata_core::MODLIST_HEADER);
    let profile = folder.join("profiles/Default");
    for name in ["modlist.txt", "plugins.txt", "loadorder.txt"] {
        let p = profile.join(name);
        std::fs::write(&p, &header).map_err(write_err(&p))?;
    }
    let settings = format!("[General]\r\nLocalSaves={}\r\nLocalSettings={}\r\n", new.local_saves, new.local_inis);
    let p = profile.join("settings.ini");
    std::fs::write(&p, settings).map_err(write_err(&p))?;
    // QSettings writes the [General] header; keys before any section mean the same to it.
    let mut ini = b"[General]\n".to_vec();
    let mut doc = portable_ini(game_def.as_ref(), &game, &prefix, &proton);
    doc.set("Settings", "profile_local_inis", &Value::String(new.local_inis.to_string()));
    doc.set("Settings", "profile_local_saves", &Value::String(new.local_saves.to_string()));
    ini.extend(doc.to_bytes());
    let p = folder.join("ModOrganizer.ini");
    std::fs::write(&p, ini).map_err(write_err(&p))?;
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(dir: &Path, game: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("ModOrganizer.ini"), format!("[General]\r\ngameName={game}\r\n")).unwrap();
    }

    #[test]
    fn the_list_has_the_roots_instances_and_the_known_ones_sorted_naturally() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Games");
        let config = tmp.path().join("cfg/strata.ini");
        set_instances_root(&config, &root).unwrap();
        instance(&root.join("Modded 10"), "Skyrim Special Edition");
        instance(&root.join("modded 2"), "Skyrim");
        std::fs::create_dir_all(root.join("not one")).unwrap();
        let away = tmp.path().join("elsewhere/Laptop SSE");
        instance(&away, "Fallout 76");
        remember(&config, &away).unwrap();
        remember(&config, &away).unwrap();
        remember(&config, &root.join("Modded 10")).unwrap(); // inside the root: not stored
        assert_eq!(known(&config), [away.clone()]);

        let names: Vec<_> = list(Some(&config), None).into_iter().map(|i| (i.name, i.supported)).collect();
        assert_eq!(names, [("Laptop SSE".to_owned(), false), ("modded 2".into(), true), ("Modded 10".into(), true)]);
        std::fs::remove_dir_all(&away).unwrap();
        assert_eq!(list(Some(&config), None).len(), 2, "a known instance that is gone is not listed");
    }

    #[test]
    fn info_reads_the_game_and_paths_in_host_form() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("SSE");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("ModOrganizer.ini"),
            "[General]\r\ngameName=Skyrim Special Edition\r\ngamePath=@ByteArray(Z:\\\\home\\\\user\\\\Game Root)\r\n\r\n[fluorine]\r\nprefix_path=/p\r\n",
        )
        .unwrap();
        let i = info(&dir);
        assert_eq!((i.name.as_str(), i.game_dir.as_str(), i.prefix.as_str()), ("SSE", "/home/user/Game Root", "/p"));
        assert_eq!(i.base_dir, dir);
    }

    #[test]
    fn rename_moves_the_folder_and_rewrites_its_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("Old");
        std::fs::create_dir_all(old.join("Game Root")).unwrap();
        let w = strata_core::winepath::to_wine(&old).replace('\\', "\\\\");
        let ini = format!(
            "[General]\r\ngameName=Skyrim\r\ngamePath=@ByteArray({w}\\\\Game Root)\r\n\r\n[customExecutables]\r\n1\\binary={w}\\\\Game Root\\\\TESV.exe\r\n\
             1\\workingDirectory={w}Er\r\n\r\n[fluorine]\r\nprefix_path={}/Prefix\r\nproton_path=/x\r\n",
            old.display()
        );
        std::fs::write(old.join("ModOrganizer.ini"), &ini).unwrap();
        let config = tmp.path().join("strata.ini");
        crate::config::remember_instance(&config, &old).unwrap();

        assert!(rename(Some(&config), &old, "a/b").is_err());
        let new = rename(Some(&config), &old, "New").unwrap();
        assert!(!old.exists());
        let i = info(&new);
        assert_eq!(i.game_dir, new.join("Game Root").to_string_lossy());
        assert_eq!(i.prefix, new.join("Prefix").to_string_lossy());
        let doc = Document::parse(&std::fs::read(new.join("ModOrganizer.ini")).unwrap());
        assert_eq!(doc.string("customExecutables", "1/binary").unwrap(), format!("{}\\Game Root\\TESV.exe", strata_core::winepath::to_wine(&new)));
        // only whole folder names are replaced
        assert_eq!(doc.string("customExecutables", "1/workingDirectory").unwrap(), format!("{}Er", strata_core::winepath::to_wine(&old)));
        assert_eq!(crate::config::last_instance(&config).unwrap(), new);

        instance(&tmp.path().join("Taken"), "Skyrim");
        assert!(rename(Some(&config), &new, "Taken").is_err());
    }

    #[test]
    fn delete_offers_the_folder_and_removes_what_was_ticked() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Gone");
        instance(&dir, "Skyrim");
        let c = deletion_candidates(&dir);
        assert_eq!(c, [DeletionCandidate { path: dir.clone(), mandatory: true }]);
        let config = tmp.path().join("strata.ini");
        remember(&config, &dir).unwrap();
        delete(Some(&config), &dir, &[dir.clone()], false).unwrap();
        assert!(!dir.exists());
        assert!(known(&config).is_empty());
    }

    #[test]
    fn create_makes_a_portable_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("root");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("SkyrimSE.exe"), "").unwrap();
        std::fs::write(game.join("skse64_loader.exe"), "").unwrap();
        let proton = tmp.path().join("P");
        std::fs::create_dir_all(&proton).unwrap();
        std::fs::write(proton.join("proton"), "").unwrap();
        let mut new = NewInstance {
            folder: tmp.path().join("New SSE"),
            game_name: "Skyrim Special Edition".into(),
            game_dir: game.clone(),
            proton,
            prefix: None,
            local_inis: true,
            local_saves: false,
        };
        let folder = create(&new).unwrap();
        let i = Instance::open(&folder).unwrap();
        assert_eq!(i.game.name(), "Skyrim Special Edition");
        assert_eq!(i.proton.prefix.as_deref(), Some(folder.join("Prefix").as_path()));
        let settings = std::fs::read_to_string(folder.join("profiles/Default/settings.ini")).unwrap();
        assert!(settings.contains("LocalSaves=false") && settings.contains("LocalSettings=true"));
        assert!(create(&new).is_err(), "an instance folder that is not empty is refused");
        new.folder = tmp.path().join("other");
        new.game_dir = tmp.path().to_owned();
        assert!(create(&new).unwrap_err().to_string().contains("has no SkyrimSE.exe"));
    }

    #[test]
    fn names_must_be_folder_names() {
        assert!(name_problem("Modded").is_empty());
        for bad in ["", " ", "..", "a/b", " lead"] {
            assert!(!name_problem(bad).is_empty(), "{bad:?}");
        }
    }
}
