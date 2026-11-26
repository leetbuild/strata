//! The Saves tab (`SavesTab::refreshSaveList`): the profile's saves, newest first, read with
//! the core's save reader, with the plugins each one misses.

use crate::html::escape;
use std::path::PathBuf;

use strata_core::plugins::PluginList;
use strata_core::{Instance, Profile};

use crate::Result;

/// One save game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRow {
    /// `GamebryoSaveGame::getName`: `Name, #12, Level 30, Whiterun`; the file name for a save
    /// that can't be read.
    pub name: String,
    /// Relative to the saves folder (the File column).
    pub file: String,
    pub path: PathBuf,
    /// When the game wrote it (Unix seconds), as MO2 computes it.
    pub created: Option<i64>,
    /// The info MO2's save widget shows, as HTML: character, location, date, missing plugins.
    pub tooltip: String,
    /// Plugins the save needs that the profile doesn't load.
    pub missing: Vec<String>,
    /// The screenshot (RGBA, `width` x `height`), empty when the save has none.
    pub screenshot: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The saves; with `plugins` (the current plugin list) each save's plugin lists are read too
/// and the missing ones named.
pub fn rows(instance: &Instance, profile: &Profile, plugins: Option<&PluginList>) -> Result<Vec<SaveRow>> {
    let dir = instance.saves_dir(profile);
    let entries = instance.saves(profile, plugins.is_some())?;
    Ok(entries
        .into_iter()
        .map(|entry| {
            let file = dir
                .as_ref()
                .and_then(|d| entry.path.strip_prefix(d).ok())
                .unwrap_or(&entry.path)
                .to_string_lossy()
                .into_owned();
            match &entry.save {
                Ok(save) => {
                    let missing = plugins
                        .and_then(|list| instance.missing_plugins(profile, list, save).ok())
                        .unwrap_or_default();
                    let mut tip = format!(
                        "<b>{}</b>, level {}<br>{}<br>{}<br>{}",
                        escape(&save.player_name),
                        save.player_level,
                        escape(&save.location),
                        escape(&save.race),
                        escape(&save.time_of_day)
                    );
                    if !missing.is_empty() {
                        tip.push_str("<br><br><b>Missing plugins</b>:<ul>");
                        for m in &missing {
                            let from = if m.providers.is_empty() {
                                String::new()
                            } else {
                                format!(" (in {})", escape(&m.providers.join(", ")))
                            };
                            tip.push_str(&format!("<li><font color=\"red\">{}</font>{from}</li>", escape(&m.name)));
                        }
                        tip.push_str("</ul>");
                    }
                    if !entry.has_cosave {
                        tip.push_str("<br><i>No script extender co-save</i>");
                    }
                    // RGB screenshots (LE) become RGBA, so the shell has one format
                    let s = &save.screenshot;
                    let screenshot = if s.alpha {
                        s.pixels.clone()
                    } else {
                        s.pixels.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect()
                    };
                    SaveRow {
                        name: format!("{}, #{}, Level {}, {}", save.player_name, save.save_number, save.player_level, save.location),
                        file,
                        path: entry.path.clone(),
                        created: Some(save.created),
                        tooltip: tip,
                        missing: missing.into_iter().map(|m| m.name).collect(),
                        screenshot,
                        width: s.width,
                        height: s.height,
                    }
                }
                Err(e) => SaveRow {
                    name: file.clone(),
                    file,
                    path: entry.path.clone(),
                    created: None,
                    tooltip: format!("<b>Cannot read this save</b>: {}", escape(e)),
                    missing: Vec::new(),
                    screenshot: Vec::new(),
                    width: 0,
                    height: 0,
                },
            }
        })
        .collect())
}
