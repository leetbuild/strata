//! Crash Log Labeler and Crash Log Viewer, built in (the third-party `crashlogtools` plugin by
//! Parapets, edited by Miss Corruption).
//!
//! The labeler names the game's functions in the call stack of Crash Logger's logs: a frame
//! `SkyrimSE.exe+05B35C7 -> 35565+0x5D7` gets the name the Address Library database has for
//! id 35565, padded into a column after the frame (`crashlogutil.CrashLogProcessor`). The
//! database is meh321's `skyrimae.rename` (`addresslib.DATABASES`), kept in
//! `$XDG_DATA_HOME/strata/crashlogs/`. The plugin pulled it with git unless its
//! `offline_mode` setting was on; Strata downloads the one file.
//!
//! Two changes from the plugin, both so it labels what it should and nothing else:
//! - Crash Logger 1.18+ writes `CALL STACK ([P]robable / [S]tack scan):` and frames like
//!   `[16][P] 0x... SkyrimSE.exe+... -> 517081+0x40\tadd [rax], al`; the plugin only knew
//!   `PROBABLE CALL STACK:` and frames that end at the offset. Both are read; what follows
//!   the offset (the instruction, a source line) stays after the name.
//! - The database names Skyrim **1.6.317+** ids. A 1.5.97 log uses the older ids, which
//!   mean other functions: such a log is left as it is (`Outcome::OtherVersion`) instead of
//!   being given wrong names.
//!
//! The logs are in the prefix's `Documents/My Games/Skyrim Special Edition/SKSE/crash-*.log`
//! unless the merged `SKSE/Plugins/CrashLogger.ini` sets `[Debug] Crashlog Directory`
//! (`crashlogs.CrashLogFinder.get_crash_log_dir`). A labelled log replaces the old one,
//! which goes to the trash first (as the plugin's `moveToTrash`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::bytes::Regex;
use strata_core::{Instance, Profile, Trash};
use strata_game::LayerKind;

use crate::{Error, Result};

/// The plugin names, as MO2 keys their settings.
pub const LABELER: &str = "Crash Log Labeler";
pub const VIEWER: &str = "Crash Log Viewer";

/// The database the plugin follows (`addresslib.DATABASES["Skyrim Special Edition"]`).
pub const DATABASE_URL: &str = "https://raw.githubusercontent.com/meh321/AddressLibraryDatabase/main/skyrimae.rename";
const DATABASE_FILE: &str = "skyrimae.rename";

/// The game the plugin supports (`crashlogs.FINDERS`, Skyrim VR aside: Strata has no VR).
pub fn supported(instance: &Instance) -> bool {
    instance.game.name() == "Skyrim Special Edition"
}

/// `[Plugins] Crash Log Labeler\offline_mode` (default true, as the plugin's setting): no
/// automatic database update.
pub fn offline(instance: &Instance) -> bool {
    instance.ini.value("Plugins", &format!("{LABELER}/offline_mode")).is_none_or(|v| v.to_bool())
}

// ---- where the logs are ----

/// A file of the merged Data folder (`rel` with `/`), the winner: what MO2's `findFiles` finds
/// first. Read from the layers, so it works without a mount.
pub(crate) fn merged_file(instance: &Instance, profile: &Profile, rel: &str) -> Result<Option<PathBuf>> {
    Ok(merged_dirs(instance, profile, rel)?.into_iter().find(|p| p.is_file()))
}

/// Every layer's copy of `rel` (a Data path), the highest priority first.
pub(crate) fn merged_dirs(instance: &Instance, profile: &Profile, rel: &str) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for layer in strata_core::resolve_layers(instance, profile)?.iter().rev() {
        let path = match layer.kind {
            LayerKind::Game => format!("{}/{rel}", instance.game.data_dir()),
            LayerKind::Mod | LayerKind::Overwrite => rel.to_owned(),
            LayerKind::Passthrough => continue,
        };
        if let Some(p) = find_ci(&layer.source, &path) {
            found.push(p);
        }
    }
    Ok(found)
}

/// `rel` below `base`, each part matched without regard to case.
fn find_ci(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut at = base.to_owned();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        let exact = at.join(part);
        if exact.exists() {
            at = exact;
            continue;
        }
        let lower = part.to_lowercase();
        at = std::fs::read_dir(&at).ok()?.flatten().find(|e| e.file_name().to_string_lossy().to_lowercase() == lower)?.path();
    }
    Some(at)
}

/// The folder of the crash logs, or `None` when the game is not supported or the instance
/// names no prefix.
pub fn log_dir(instance: &Instance, profile: &Profile) -> Result<Option<PathBuf>> {
    if !supported(instance) {
        return Ok(None);
    }
    let Some(prefix) = instance.proton.prefix.as_ref() else { return Ok(None) };
    let pfx = prefix.join("pfx");
    let default = pfx.join("drive_c/users/steamuser/Documents/My Games").join(instance.game.documents_dir()).join("SKSE");
    // CrashLogger.ini's own folder, when it names one that exists
    if let Some(ini) = merged_file(instance, profile, "SKSE/Plugins/CrashLogger.ini")? {
        let text = std::fs::read(&ini).map_err(|source| Error::Read { path: ini.clone(), source })?;
        if let Some(dir) = ini_value(&String::from_utf8_lossy(&text), "Debug", "Crashlog Directory")
            && let Ok(host) = strata_core::winepath::to_host(&dir, Some(&pfx))
            && host.is_dir()
        {
            return Ok(Some(host));
        }
    }
    Ok(Some(default))
}

/// One value of a plain ini (configparser: keys without regard to case, `=` or `:`).
fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            inside = name.trim() == section;
        } else if inside
            && let Some((k, v)) = line.split_once(['=', ':'])
            && k.trim().eq_ignore_ascii_case(key)
        {
            let v = v.trim().trim_matches('"');
            return (!v.is_empty()).then(|| v.to_owned());
        }
    }
    None
}

/// The crash logs (`crash-*.log`) of `dir`, newest first (the viewer sorts by name,
/// descending; the names are times).
pub fn logs(dir: &Path) -> Vec<PathBuf> {
    let mut logs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("crash-") && n.ends_with(".log"))
        })
        .collect();
    logs.sort();
    logs.reverse();
    logs
}

// ---- the database ----

/// Address Library ids and their names.
#[derive(Debug, Default)]
pub struct Names(HashMap<u64, String>);

impl Names {
    /// A `.rename` file: a version line, then `<id> <name>` per line (`IdScanner`).
    pub fn parse(data: &[u8]) -> Self {
        let text = String::from_utf8_lossy(data);
        let mut map = HashMap::new();
        for line in text.lines().skip(1) {
            let mut parts = line.split_whitespace();
            if let (Some(id), Some(name)) = (parts.next(), parts.next())
                && let Ok(id) = id.parse()
            {
                map.insert(id, name.to_owned());
            }
        }
        Self(map)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).map_err(|source| Error::Read { path: path.to_owned(), source })?;
        Ok(Self::parse(&data))
    }

    /// The name of `id`, without the trailing `_` / `*` marks (`add_name`'s `rstrip("_*")`).
    pub fn get(&self, id: u64) -> Option<&str> {
        self.0.get(&id).map(|n| n.trim_end_matches(['_', '*'])).filter(|n| !n.is_empty())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Where the database is kept: `$XDG_DATA_HOME/strata/crashlogs/skyrimae.rename`
/// (`STRATA_CRASHLOG_DATABASE` overrides it, for tests).
pub fn database_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("STRATA_CRASHLOG_DATABASE") {
        return Some(PathBuf::from(path));
    }
    Some(strata_core::dirs::data_home()?.join("strata/crashlogs").join(DATABASE_FILE))
}

/// The database, or `None` when it was never downloaded.
pub fn database() -> Result<Option<Names>> {
    match database_path() {
        Some(path) if path.is_file() => Ok(Some(Names::load(&path)?)),
        _ => Ok(None),
    }
}

/// Downloads the database (the plugin's `update_database`); returns its path and how many
/// names it has.
pub fn update_database() -> Result<(PathBuf, usize)> {
    let path = database_path().ok_or_else(|| Error::Invalid("neither XDG_DATA_HOME nor HOME is set".into()))?;
    let net = |e: ureq::Error| Error::Invalid(format!("cannot download {DATABASE_URL}: {e}. Check the network."));
    let mut reply = ureq::get(DATABASE_URL).call().map_err(net)?;
    let data = reply.body_mut().with_config().limit(64 << 20).read_to_vec().map_err(net)?;
    let names = Names::parse(&data);
    if names.is_empty() {
        return Err(Error::Invalid(format!("{DATABASE_URL} gave no names; the old database is kept")));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|source| Error::Write { path: dir.to_owned(), source })?;
    }
    strata_core::write::write_if_changed(&path, &data)?;
    Ok((path, names.len()))
}

// ---- labelling ----

/// A frame with an Address Library id: `\t[ 16][P] 0x... SkyrimSE.exe+2F27BE0 -> 517081+0x40`
/// (group 1 = the frame up to the offset, `id`, then the rest of the line).
static FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\t\[ *\d+\](?:\[[A-Z]\])? 0x[0-9A-F]+ .*\+[0-9A-F]+ -> (?P<id>\d+)\+0x[0-9A-F]+)(?P<rest>.*)$").unwrap()
});
static VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^Skyrim (?:SSE|VR) v(\d+)\.(\d+)").unwrap());

/// What labelling did to one log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Names were added (the old log is in the trash).
    Labelled(usize),
    /// Nothing to add: no frame with a known id, or already labelled.
    Unchanged,
    /// A log of a game version the database does not cover (`1.5.97`).
    OtherVersion(String),
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Labelled(n) => write!(f, "labelled {n} frame(s)"),
            Self::Unchanged => write!(f, "nothing to label"),
            Self::OtherVersion(v) => {
                write!(f, "Skyrim {v}: the names database covers 1.6.317 and later, whose ids differ; left as it is")
            }
        }
    }
}

fn strip_eol(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// The game version of a log (`Skyrim SSE v1.5.97` → `1.5.97`'s major and minor).
fn version(text: &[u8]) -> Option<(u32, u32, String)> {
    text.split(|b| *b == b'\n').take(10).find_map(|line| {
        let c = VERSION.captures(strip_eol(line))?;
        let num = |i| std::str::from_utf8(&c[i]).ok()?.parse().ok();
        let shown = String::from_utf8_lossy(strip_eol(line)).trim_start_matches("Skyrim SSE v").trim_start_matches("Skyrim VR v").to_owned();
        Some((num(1)?, num(2)?, shown))
    })
}

/// The labelled text of a log, or the reason it stays as it is (`CrashLogProcessor.process_log`).
/// Lines keep their own line ends.
pub fn label(text: &[u8], names: &Names) -> std::result::Result<(Vec<u8>, usize), Outcome> {
    if let Some((major, minor, shown)) = version(text)
        && (major, minor) < (1, 6)
    {
        return Err(Outcome::OtherVersion(shown));
    }
    let lines: Vec<&[u8]> = text.split_inclusive(|b| *b == b'\n').collect();
    // the call stack: after its heading, up to an empty line or REGISTERS:
    let Some(start) = lines.iter().position(|l| {
        let l = strip_eol(l);
        l == b"PROBABLE CALL STACK:" || (l.starts_with(b"CALL STACK") && l.ends_with(b":"))
    }) else {
        return Err(Outcome::Unchanged);
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| strip_eol(l).is_empty() || strip_eol(l) == b"REGISTERS:")
        .map_or(lines.len(), |i| start + 1 + i);
    let stack = &lines[start + 1..end];
    // the column: one past the longest frame with an id
    let width = stack.iter().filter_map(|l| FRAME.captures(strip_eol(l))).map(|c| c[1].len() + 1).max();
    let Some(width) = width else { return Err(Outcome::Unchanged) };
    let mut out = Vec::with_capacity(text.len() + 4096);
    for l in &lines[..=start] {
        out.extend_from_slice(l);
    }
    let mut count = 0;
    for l in stack {
        let body = strip_eol(l);
        let eol = &l[body.len()..];
        let Some(c) = FRAME.captures(body) else {
            out.extend_from_slice(l);
            continue;
        };
        let id = std::str::from_utf8(&c["id"]).ok().and_then(|s| s.parse().ok());
        let rest = &c["rest"];
        let name = id.and_then(|id| names.get(id));
        match name {
            // already labelled: the rest starts with the padding and the name
            Some(name) if !rest.trim_ascii_start().starts_with(name.as_bytes()) => {
                let frame = &c[1];
                out.extend_from_slice(frame);
                out.resize(out.len() + width - frame.len(), b' ');
                out.extend_from_slice(name.as_bytes());
                out.extend_from_slice(rest);
                out.extend_from_slice(eol);
                count += 1;
            }
            _ => out.extend_from_slice(l),
        }
    }
    // the plugin puts back the empty line a log without one before REGISTERS: lacks
    if end < lines.len() && strip_eol(lines[end]) == b"REGISTERS:" {
        out.extend_from_slice(if lines[end].ends_with(b"\r\n") { b"\r\n" } else { b"\n" });
    }
    for l in &lines[end..] {
        out.extend_from_slice(l);
    }
    if count == 0 {
        return Err(Outcome::Unchanged);
    }
    Ok((out, count))
}

/// Labels one log in place: the old one goes to `trash` first.
pub fn label_file(path: &Path, names: &Names, trash: &Trash) -> Result<Outcome> {
    let text = std::fs::read(path).map_err(|source| Error::Read { path: path.to_owned(), source })?;
    match label(&text, names) {
        Ok((labelled, count)) => {
            trash.put(path)?;
            std::fs::write(path, labelled).map_err(|source| Error::Write { path: path.to_owned(), source })?;
            Ok(Outcome::Labelled(count))
        }
        Err(outcome) => Ok(outcome),
    }
}

/// Each log with what labelling did, and a note for the user (no database, a failed update).
pub type Labelled = (Vec<(PathBuf, Outcome)>, Option<String>);

/// Labels `logs` with the database (downloading it first unless offline, as the plugin's
/// `onFinishedRunCallback`). No database: each log is `Unchanged` and the note says so.
pub fn label_logs(instance: &Instance, logs: &[PathBuf], trash: &Trash) -> Result<Labelled> {
    if logs.is_empty() {
        return Ok((Vec::new(), None));
    }
    let mut note = None;
    if !offline(instance)
        && let Err(e) = update_database()
    {
        note = Some(format!("{e}"));
    }
    let Some(names) = database()? else {
        let note = "no names database yet: `strata crashlogs update` (or the viewer's Update button) downloads it".to_owned();
        return Ok((logs.iter().map(|p| (p.clone(), Outcome::Unchanged)).collect(), Some(note)));
    };
    let mut out = Vec::new();
    for log in logs {
        out.push((log.clone(), label_file(log, &names, trash)?));
    }
    Ok((out, note))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &[u8] = b"1\n36564 Main::Update_*\n12 Foo_\n517081 BSSomething::sub_*\n";

    fn ae(text: &str) -> String {
        text.replace("Skyrim SSE v1.5.97", "Skyrim SSE v1.6.1170")
    }

    const NEW: &str = "CRASH TIME: x\r\nSkyrim SSE v1.5.97\r\n\r\nCALL STACK ([P]robable / [S]tack scan):\r\n\
\t[ 0][P] 0x6FFFEA4DCF11                           d3d12core.dll+02BCF11\r\n\
\t[16][P] 0x000142F27BE0     SkyrimSE.exe+2F27BE0 -> 517081+0x40\tadd [rax], al\r\n\
\t[18][P] 0x0001405B35C7     SkyrimSE.exe+05B35C7 -> 36564+0x5D7\ttest dil, dil\r\n\
\t[70][S] 0x0001436DB310     SkyrimSE.exe+36DB310 -> 810147+0xE7AE0\tcall 0x00000001436DB315\r\n\
\r\nREGISTERS:\r\n\tRAX 0x0\r\n";

    #[test]
    fn names_lose_their_marks() {
        let names = Names::parse(NAMES);
        assert_eq!(names.get(36564), Some("Main::Update"));
        assert_eq!(names.get(12), Some("Foo"));
        assert_eq!(names.get(1), None);
    }

    #[test]
    fn a_new_format_log_gets_a_column_of_names_and_keeps_its_instructions() {
        let names = Names::parse(NAMES);
        let (out, count) = label(ae(NEW).as_bytes(), &names).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(count, 2);
        let line = out.lines().find(|l| l.contains("36564+0x5D7")).unwrap();
        assert!(line.ends_with("Main::Update\ttest dil, dil"), "{line:?}");
        let other = out.lines().find(|l| l.contains("517081+0x40")).unwrap();
        // the same column for both: the longest frame (810147+0xE7AE0) plus one
        let col = |l: &str, n: &str| l.find(n).unwrap();
        assert_eq!(col(line, "Main::Update"), col(other, "BSSomething::sub"));
        assert!(out.contains("-> 810147+0xE7AE0\tcall"), "an unknown id stays as it was");
        assert!(out.ends_with("\r\nREGISTERS:\r\n\tRAX 0x0\r\n"));
        // labelling twice changes nothing
        assert_eq!(label(out.as_bytes(), &names), Err(Outcome::Unchanged));
    }

    #[test]
    fn a_1_5_97_log_is_left_alone() {
        let names = Names::parse(NAMES);
        assert_eq!(label(NEW.as_bytes(), &names), Err(Outcome::OtherVersion("1.5.97".into())));
    }

    #[test]
    fn the_old_format_labels_as_the_plugin_did() {
        let names = Names::parse(NAMES);
        let log = "Skyrim SSE v1.6.640\nPROBABLE CALL STACK:\n\t[0] 0x7FF6 SkyrimSE.exe+05B35C7 -> 36564+0x5D7\n\t[1] 0x7FF7 SkyrimSE.exe+1 -> 99+0x1\nREGISTERS:\nRAX\n";
        let (out, _) = label(log.as_bytes(), &names).unwrap();
        let out = String::from_utf8(out).unwrap();
        // no empty line before REGISTERS: the plugin adds one
        assert_eq!(
            out,
            "Skyrim SSE v1.6.640\nPROBABLE CALL STACK:\n\t[0] 0x7FF6 SkyrimSE.exe+05B35C7 -> 36564+0x5D7 Main::Update\n\t[1] 0x7FF7 SkyrimSE.exe+1 -> 99+0x1\n\nREGISTERS:\nRAX\n"
        );
    }

    #[test]
    fn the_users_real_log_is_read_whole() {
        // crash-2026-11-25-19-39-28.log from a real prefix (1.5.97), made an AE log
        let real = include_bytes!("../tests/data/crash-2026-11-25-19-39-28.log");
        let names = Names::parse(NAMES);
        assert_eq!(label(real, &names), Err(Outcome::OtherVersion("1.5.97".into())));
        let text = String::from_utf8_lossy(real).replace("Skyrim SSE v1.5.97", "Skyrim SSE v1.6.1170");
        let (out, count) = label(text.as_bytes(), &names).unwrap();
        assert_eq!(count, 1, "one id of that log is in the test names (517081)");
        let out = String::from_utf8_lossy(&out);
        assert_eq!(out.lines().count(), text.lines().count());
        assert!(out.lines().any(|l| l.contains("-> 517081+0x40") && l.contains("BSSomething::sub\tadd [rax], al")));
    }

    #[test]
    fn a_file_is_labelled_in_place_and_the_old_one_trashed() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("crash-2026-01-01-00-00-00.log");
        std::fs::write(&log, ae(NEW)).unwrap();
        let trash = Trash::at(dir.path().join("Trash"));
        let names = Names::parse(NAMES);
        assert_eq!(label_file(&log, &names, &trash).unwrap(), Outcome::Labelled(2));
        assert!(std::fs::read_to_string(&log).unwrap().contains("Main::Update"));
        let trashed = dir.path().join("Trash/files/crash-2026-01-01-00-00-00.log");
        assert_eq!(std::fs::read_to_string(trashed).unwrap(), ae(NEW));
        assert_eq!(label_file(&log, &names, &trash).unwrap(), Outcome::Unchanged);
    }

    #[test]
    fn logs_are_listed_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["crash-2026-01-02.log", "crash-2026-01-01.log", "skse64.log", "crash-x.txt"] {
            std::fs::write(dir.path().join(n), "").unwrap();
        }
        let names: Vec<_> = logs(dir.path()).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["crash-2026-01-02.log", "crash-2026-01-01.log"]);
    }

    #[test]
    fn the_folder_is_the_prefixs_or_crashlogger_ini_s() {
        let t = crate::test_instance::TestInstance::new("+Crash Logger\r\n");
        let prefix = t.root.join("prefix");
        let ini = t.root.join("ModOrganizer.ini");
        let mut text = std::fs::read_to_string(&ini).unwrap();
        text.push_str(&format!("[fluorine]\r\nprefix_path={}\r\n", prefix.display()));
        std::fs::write(&ini, text).unwrap();
        let (instance, profile) = t.open();
        let docs = prefix.join("pfx/drive_c/users/steamuser/Documents/My Games/Skyrim Special Edition/SKSE");
        assert_eq!(log_dir(&instance, &profile).unwrap(), Some(docs.clone()));
        // CrashLogger.ini names a folder: used when it exists
        let plugins = t.root.join("mods/Crash Logger/SKSE/Plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("CrashLogger.ini"), "[Debug]\r\nCrashlog Directory = C:\\Logs\r\n").unwrap();
        assert_eq!(log_dir(&instance, &profile).unwrap(), Some(docs));
        let logs = prefix.join("pfx/drive_c/Logs");
        std::fs::create_dir_all(&logs).unwrap();
        assert_eq!(log_dir(&instance, &profile).unwrap(), Some(logs));
    }

    #[test]
    fn crashlogger_ini_values() {
        let ini = "\u{feff}[Debug]\r\n; c\r\nCrashlog Directory = \"C:\\Logs\"\r\n";
        assert_eq!(ini_value(ini, "Debug", "crashlog directory").as_deref(), Some("C:\\Logs"));
        assert_eq!(ini_value("[Debug]\nCrashlog Directory=\n", "Debug", "Crashlog Directory"), None);
    }
}
