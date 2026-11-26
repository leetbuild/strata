//! The `conflicts` area's views: the mod info dialog (`modinfo`) and file previews
//! (`preview`), plus the conflict index of a profile as MO2 builds it (`build_index`).

pub mod bbcode;
pub mod modinfo;
pub mod preview;

use std::collections::HashSet;

use strata_core::{Instance, PluginListFile, Profile, resolve_layers};
use strata_vfs::conflicts::{ArchiveRules, ConflictIndex};

use crate::Result;

/// MO2's archive parsing for the profile, when the instance has it on
/// (`[Settings] archive_parsing_experimental`): the profile's `archives.txt`
/// (`OrganizerCore::enabledArchives`) and its load order (`loadorder.txt`).
pub fn archive_rules(instance: &Instance, profile: &Profile) -> Result<Option<ArchiveRules>> {
    let on = instance.ini.string("Settings", "archive_parsing_experimental").is_some_and(|v| v == "true");
    if !on {
        return Ok(None);
    }
    let read = |name: &str| {
        let path = profile.dir.join(name);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(source) => Err(crate::Error::Read { path, source }),
        }
    };
    let enabled: HashSet<String> = String::from_utf8_lossy(&read("archives.txt")?)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    let load_order = PluginListFile::parse(&read("loadorder.txt")?).plugins.into_iter().map(|(name, _)| name).collect();
    Ok(Some(ArchiveRules { enabled, load_order }))
}

/// The profile's conflict index, with archive contents when MO2's archive parsing is on.
pub fn build_index(instance: &Instance, profile: &Profile) -> Result<ConflictIndex> {
    let layers = resolve_layers(instance, profile)?;
    Ok(ConflictIndex::build(&layers, instance.game.data_dir(), archive_rules(instance, profile)?))
}

/// One provider of a file that has a file on disk (the preview's variants: MO2's
/// `PreviewDialog` shows the file of every mod that has it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// The layer id: the mod's name, `<game>`, `<overwrite>`.
    pub layer: String,
    pub real: std::path::PathBuf,
}

/// `rel` below `base`, matching each part without regard to case (the index keeps one spelling
/// per path; other layers may spell it differently).
fn find_ci(base: &std::path::Path, rel: &str) -> Option<std::path::PathBuf> {
    let mut at = base.to_owned();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        let exact = at.join(part);
        if exact.exists() {
            at = exact;
            continue;
        }
        let lower = part.to_lowercase();
        at = std::fs::read_dir(&at)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().to_lowercase() == lower)?
            .path();
    }
    Some(at)
}

/// Every loose copy of a file of the merged tree, lowest priority first (the last one is what
/// the game sees). `root` picks the Root area (`/Root/...` paths) over Data. Archived copies
/// have no file on disk and are left out.
pub fn file_providers(instance: &Instance, profile: &Profile, root: bool, path: &str) -> Result<Vec<Variant>> {
    use strata_game::LayerKind;
    use strata_vfs::conflicts::Area;
    let layers = resolve_layers(instance, profile)?;
    let index = ConflictIndex::build(&layers, instance.game.data_dir(), None);
    let area = if root { Area::Root } else { Area::Data };
    let Some(origins) = index.origins(area, path) else { return Ok(Vec::new()) };
    let data_dir = instance.game.data_dir();
    let mut out = Vec::new();
    for p in origins.iter().filter(|p| p.archive.is_none()) {
        let Some(layer) = layers.iter().find(|l| l.id == p.layer) else { continue };
        let base = match (layer.kind, root) {
            (LayerKind::Game, _) => find_ci(&layer.source, data_dir),
            (_, true) => find_ci(&layer.source, "Root").or_else(|| Some(layer.source.join("Root"))),
            (_, false) => Some(layer.source.clone()),
        };
        let Some(real) = base.and_then(|b| find_ci(&b, path)).filter(|r| r.is_file()) else { continue };
        out.push(Variant { layer: p.layer.to_owned(), real });
    }
    Ok(out)
}
