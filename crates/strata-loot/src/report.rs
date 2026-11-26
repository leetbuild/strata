//! lootcli's report (`modorganizer-lootcli`, `LOOTWorker::createJsonReport`): what MO2's LOOT
//! dialog shows after a sort, read from libloot's database once the plugins are loaded.
//! Serialised with lootcli's JSON names (`Loot::processOutputFile` in MO2 reads them).

use std::path::Path;

use libloot::metadata::{File, Message, MessageContent, MessageType, PluginCleaningData, select_message_content};
use libloot::{EvalMode, Game, MergeMode};
use serde::Serialize;

use crate::{Error, SortInput, loot};

/// The language messages are picked in (MO2 passes its interface language; Strata is English).
const LANGUAGE: &str = MessageContent::DEFAULT_LANGUAGE;

/// lootcli's report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// The masterlist's (and userlist's) general messages whose conditions hold.
    pub messages: Vec<ReportMessage>,
    /// The loaded plugins that have something to say, in the new load order.
    pub plugins: Vec<PluginReport>,
}

/// A message: `type` is `info`, `warn` or `error`; `text` is markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

/// A file a plugin names (`name`; `displayName` when the masterlist gives one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRef {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub display_name: String,
}

/// Dirty (or verified clean) info whose CRC matches the plugin's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dirty {
    pub crc: u32,
    pub itm: u32,
    pub deleted_references: u32,
    pub deleted_navmesh: u32,
    pub cleaning_utility: String,
    /// The detail message (markdown), "" for none.
    pub info: String,
}

/// One plugin's part of the report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginReport {
    pub name: String,
    /// Incompatible files that are present.
    pub incompatibilities: Vec<FileRef>,
    pub messages: Vec<ReportMessage>,
    pub dirty: Vec<Dirty>,
    pub clean: Vec<Dirty>,
    /// Masters no loaded plugin provides.
    pub missing_masters: Vec<String>,
    /// Required files that are absent (Strata's addition: lootcli leaves them out, LOOT shows
    /// them).
    pub missing_requirements: Vec<FileRef>,
    pub loads_archive: bool,
    pub is_master: bool,
    pub is_light_master: bool,
}

impl PluginReport {
    /// Whether there is anything to show beyond the flags.
    fn has_content(&self) -> bool {
        !(self.incompatibilities.is_empty()
            && self.messages.is_empty()
            && self.dirty.is_empty()
            && self.clean.is_empty()
            && self.missing_masters.is_empty()
            && self.missing_requirements.is_empty())
    }
}

fn message(m: &Message) -> Option<ReportMessage> {
    let text = select_message_content(m.content(), LANGUAGE)?.text().to_owned();
    let kind = match m.message_type() {
        MessageType::Say => "info",
        MessageType::Warn => "warn",
        MessageType::Error => "error",
    };
    (!text.is_empty()).then(|| ReportMessage { kind: kind.to_owned(), text })
}

fn dirty(d: &PluginCleaningData) -> Dirty {
    Dirty {
        crc: d.crc(),
        itm: d.itm_count(),
        deleted_references: d.deleted_reference_count(),
        deleted_navmesh: d.deleted_navmesh_count(),
        cleaning_utility: d.cleaning_utility().to_owned(),
        info: select_message_content(d.detail(), LANGUAGE).map(|c| c.text().to_owned()).unwrap_or_default(),
    }
}

fn file_ref(f: &File) -> FileRef {
    FileRef { name: f.name().as_str().to_owned(), display_name: f.display_name().unwrap_or_default().to_owned() }
}

/// Builds the report after a sort: `order` is the new load order.
pub(crate) fn build(game: &Game, input: &SortInput, order: &[String]) -> Result<Report, Error> {
    let db = game.database();
    let db = db.read().map_err(loot)?;
    let messages = db
        .general_messages(MergeMode::WithUserMetadata, EvalMode::Evaluate)
        .map_err(loot)?
        .iter()
        .filter_map(message)
        .collect();
    let mut plugins = Vec::new();
    for name in order {
        let Some(plugin) = game.plugin(name) else { continue };
        let mut report = PluginReport {
            name: name.clone(),
            loads_archive: plugin.loads_archive(),
            is_master: plugin.is_master(),
            is_light_master: plugin.is_light_plugin(),
            ..PluginReport::default()
        };
        if let Some(meta) = db.plugin_metadata(name, MergeMode::WithUserMetadata, EvalMode::Evaluate).map_err(loot)? {
            report.messages = meta.messages().iter().filter_map(message).collect();
            report.dirty = meta.dirty_info().iter().map(dirty).collect();
            report.clean = meta.clean_info().iter().map(dirty).collect();
            report.incompatibilities = meta
                .incompatibilities()
                .iter()
                .filter(|f| game.plugin(f.name().as_str()).is_some() || file_exists(input, f.name().as_str()))
                .map(file_ref)
                .collect();
            report.missing_requirements = meta
                .requirements()
                .iter()
                .filter(|f| game.plugin(f.name().as_str()).is_none() && !file_exists(input, f.name().as_str()))
                .map(file_ref)
                .collect();
        }
        report.missing_masters =
            plugin.masters().unwrap_or_default().into_iter().filter(|m| game.plugin(m).is_none()).collect();
        if report.has_content() {
            plugins.push(report);
        }
    }
    Ok(Report { messages, plugins })
}

/// Whether a file the masterlist names (relative to Data, `../` for the game folder) is in the
/// merged Data folder: a mod's folder (its `Root/` for `../`) or the game's. Case is ignored,
/// as on Windows.
fn file_exists(input: &SortInput, name: &str) -> bool {
    let name = name.replace('\\', "/");
    if let Some(rest) = name.strip_prefix("../") {
        input.data_paths.iter().any(|d| find_ci(&d.join("Root"), rest)) || find_ci(&input.game_path, rest)
    } else {
        input.data_paths.iter().any(|d| find_ci(d, &name)) || find_ci(&input.game_path.join("Data"), &name)
    }
}

/// Whether `rel` exists under `base`, each component matched without regard to case.
fn find_ci(base: &Path, rel: &str) -> bool {
    let mut at = base.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty() && *p != ".") {
        let exact = at.join(part);
        if exact.exists() {
            at = exact;
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&at) else { return false };
        let lower = part.to_lowercase();
        match entries.flatten().find(|e| e.file_name().to_string_lossy().to_lowercase() == lower) {
            Some(e) => at = e.path(),
            None => return false,
        }
    }
    true
}
