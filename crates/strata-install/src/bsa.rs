//! "BSA Extractor" (MO2's stock `bsa_extractor` plugin, `BsaExtractor::modInstalledHandler`):
//! after an install, offer to unpack the mod's top-level `*.bsa` / `*.ba2` into its folder,
//! then whether to remove them.
//!
//! - With `only_alternate_source` (on by default) only mods from an alternate game source
//!   are asked: `gameName` in `meta.ini` is neither the game's short name nor one of its
//!   primary sources (`ModInfoRegular` constructor) and the mod is not marked `converted`.
//! - The two questions are MO2's `QuestionBoxMemory` ones, `unpackBSA` and
//!   `removeUnpackedBSA`, both defaulting to No. A remembered answer is read from
//!   `ModOrganizer.ini` `[DialogChoices]` (`QDialogButtonBox::Yes` = 16384, `No` = 65536) and
//!   not asked again; a "remember" from the dialog is stored there.
//! - The unpacking is the `conflicts` area's `strata_archive::pack::extract_mod_archives`.

use std::path::Path;

use strata_ini::{Document, Value};

use crate::install::InstallUi;
use crate::Result;

const YES: &str = "16384";
const NO: &str = "65536";

/// The plugin's name, for its `[Plugins]` settings.
pub const PLUGIN: &str = "BSA Extractor";

fn archives(mod_dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(mod_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            let l = n.to_lowercase();
            l.ends_with(".bsa") || l.ends_with(".ba2")
        })
        .collect();
    out.sort();
    out
}

/// `ModInfoRegular::m_IsAlternate && !m_Converted` for SSE (short name `SkyrimSE`, primary
/// source `SkyrimSE`).
pub fn is_alternate(mod_dir: &Path, game_short_name: &str) -> Result<bool> {
    let meta = strata_core::ModMeta::open(mod_dir)?;
    let game = meta.game_name();
    Ok(!game.eq_ignore_ascii_case(game_short_name) && !meta.converted())
}

/// The remembered answer of a `QuestionBoxMemory` dialog, if any.
fn remembered(ini: &Document, id: &str) -> Option<bool> {
    match ini.string("DialogChoices", id).as_deref() {
        Some(YES) => Some(true),
        Some(NO) => Some(false),
        _ => None,
    }
}

fn remember(instance_root: &Path, id: &str, yes: bool) -> Result<()> {
    let value = Value::String(if yes { YES } else { NO }.to_owned());
    strata_core::settings::store_instance_ini(instance_root, &[("DialogChoices", id, Some(value))])?;
    Ok(())
}

/// Asks question `id` unless the answer is remembered; stores a "remember".
fn ask(ui: &mut dyn InstallUi, instance_root: &Path, ini: &Document, id: &str, title: &str, text: &str) -> Result<bool> {
    if let Some(answer) = remembered(ini, id) {
        return Ok(answer);
    }
    let (yes, keep) = ui.question(id, title, text, false);
    if keep {
        remember(instance_root, id, yes)?;
    }
    Ok(yes)
}

/// What [`offer`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Extracted {
    pub archives: Vec<String>,
    pub removed: bool,
    /// Why extraction failed, if it did (the install itself stands).
    pub error: Option<String>,
}

/// The handler. `ini` is the instance's `ModOrganizer.ini` as read at the start.
pub fn offer(
    ui: &mut dyn InstallUi,
    instance_root: &Path,
    ini: &Document,
    mod_dir: &Path,
    game_short_name: &str,
) -> Result<Option<Extracted>> {
    let plugin = |key: &str, default: bool| ini.value("Plugins", &format!("{PLUGIN}/{key}")).map_or(default, |v| v.to_bool());
    if !plugin("enabled", true) {
        return Ok(None);
    }
    if plugin("only_alternate_source", true) && !is_alternate(mod_dir, game_short_name)? {
        return Ok(None);
    }
    let found = archives(mod_dir);
    if found.is_empty() {
        return Ok(None);
    }
    let unpack = ask(
        ui,
        instance_root,
        ini,
        "unpackBSA",
        "Extract BSA",
        "This mod contains at least one BSA. Do you want to unpack it?\n(If you don't know about BSAs, just select no)",
    )?;
    if !unpack {
        return Ok(None);
    }
    let remove = ask(ui, instance_root, ini, "removeUnpackedBSA", "Remove extracted archives", "Do you wish to remove BSAs after extraction completed?\n")?;
    Ok(Some(match strata_archive::pack::extract_mod_archives(mod_dir, remove) {
        Ok(done) => Extracted {
            archives: done.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect(),
            removed: remove,
            error: None,
        },
        Err(e) => Extracted { archives: found, removed: false, error: Some(e.to_string()) },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternate_sources_and_remembered_answers() {
        let d = tempfile::tempdir().unwrap();
        let m = d.path().join("mod");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("meta.ini"), "[General]\ngameName=skyrimse\n").unwrap();
        assert!(!is_alternate(&m, "SkyrimSE").unwrap());
        std::fs::write(m.join("meta.ini"), "[General]\ngameName=Skyrim\n").unwrap();
        assert!(is_alternate(&m, "SkyrimSE").unwrap());
        std::fs::write(m.join("meta.ini"), "[General]\ngameName=Skyrim\nconverted=true\n").unwrap();
        assert!(!is_alternate(&m, "SkyrimSE").unwrap());

        let ini = Document::parse(b"[DialogChoices]\nunpackBSA=65536\nremoveUnpackedBSA=16384\nremoveMods=16384\n");
        assert_eq!(remembered(&ini, "unpackBSA"), Some(false));
        assert_eq!(remembered(&ini, "removeUnpackedBSA"), Some(true));
        assert_eq!(remembered(&ini, "other"), None);

        std::fs::write(d.path().join("ModOrganizer.ini"), "[General]\n").unwrap();
        remember(d.path(), "unpackBSA", true).unwrap();
        let text = std::fs::read_to_string(d.path().join("ModOrganizer.ini")).unwrap();
        assert!(text.contains("[DialogChoices]\nunpackBSA=16384\n"), "{text}");
    }
}
