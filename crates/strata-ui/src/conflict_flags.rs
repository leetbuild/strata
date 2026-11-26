//! The mod list's Conflicts column, the "contains hidden files" flag and the conflict
//! highlight on selection, from the conflict index (`strata-vfs/src/conflicts.rs`, MO2's
//! `ModInfoWithConflictInfo`).

use std::collections::HashSet;

use strata_core::{Color, Instance, Profile, color_from_value, resolve_layers};
use strata_vfs::conflicts::{ConflictIndex, ConflictState, Flags};

use crate::Result;

/// One mod's conflicts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictRow {
    /// The icons in MO2's four slots (`ModConflictIconDelegate::getIconsForFlags`): loose
    /// files, loose beating archives, archives beaten by loose, archives among themselves.
    pub icons: [Option<&'static str>; 4],
    /// The tooltip lines (`ModList::getConflictFlagText`).
    pub texts: Vec<&'static str>,
    pub hidden_files: bool,
}

/// What a selection highlights (`ModListView::setOverwriteMarkers` + `markerColor`): mod
/// names per colour, strongest first as MO2 picks them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Markers {
    /// Mods that win files from the selection (red by default).
    pub overwriting: HashSet<String>,
    /// Mods the selection wins files from (green).
    pub overwritten: HashSet<String>,
    /// Archive against archive: mods whose archives win (magenta) / lose (cyan).
    pub archive_overwriting: HashSet<String>,
    pub archive_overwritten: HashSet<String>,
}

/// The highlight colours (`ColorSettings::modlist*`), from `[Settings]`, else MO2's defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerColors {
    pub overwriting_loose: Color,
    pub overwritten_loose: Color,
    pub overwriting_archive: Color,
    pub overwritten_archive: Color,
}

impl MarkerColors {
    pub fn of(instance: &Instance) -> Self {
        let get = |key: &str, default: Color| {
            instance.ini.value("Settings", key).as_ref().and_then(color_from_value).unwrap_or(default)
        };
        let c = |r, g, b| Color { r, g, b, a: 64 };
        Self {
            overwriting_loose: get("overwritingLooseFilesColor", c(255, 0, 0)),
            overwritten_loose: get("overwrittenLooseFilesColor", c(0, 255, 0)),
            overwriting_archive: get("overwritingArchiveFilesColor", c(255, 0, 255)),
            overwritten_archive: get("overwrittenArchiveFilesColor", c(0, 255, 255)),
        }
    }
}

/// The conflict index of a profile.
pub struct ConflictView {
    index: ConflictIndex,
}

fn icon_loose(state: ConflictState) -> Option<(&'static str, &'static str)> {
    match state {
        ConflictState::Mixed => Some((":/MO/gui/emblem_conflict_mixed", "Loose files Overwrites & Overwritten")),
        ConflictState::Overwrite => Some((":/MO/gui/emblem_conflict_overwrite", "Overwrites loose files")),
        ConflictState::Overwritten => Some((":/MO/gui/emblem_conflict_overwritten", "Overwritten loose files")),
        ConflictState::Redundant => Some((":/MO/gui/emblem_conflict_redundant", "Redundant")),
        ConflictState::None => None,
    }
}

/// `getConflictFlags` turned into the delegate's four slots.
pub fn row_of(flags: &Flags) -> ConflictRow {
    let mut row = ConflictRow { hidden_files: flags.hidden_files, ..ConflictRow::default() };
    if let Some((icon, text)) = icon_loose(flags.state) {
        row.icons[0] = Some(icon);
        row.texts.push(text);
    }
    let (loose_over, loose_under) = match flags.archive_loose_state {
        ConflictState::Mixed => (true, true),
        ConflictState::Overwrite => (true, false),
        ConflictState::Overwritten => (false, true),
        _ => (false, false),
    };
    if loose_over {
        row.icons[1] = Some(":/MO/gui/archive_loose_conflict_overwrite");
        row.texts.push("Overwrites an archive with loose files");
    }
    if loose_under {
        row.icons[2] = Some(":/MO/gui/archive_loose_conflict_overwritten");
        row.texts.push("Archive is overwritten by loose files");
    }
    match flags.archive_state {
        ConflictState::Mixed => {
            row.icons[3] = Some(":/MO/gui/archive_conflict_mixed");
            row.texts.push("Archive files overwrites & overwritten");
        }
        ConflictState::Overwrite => {
            row.icons[3] = Some(":/MO/gui/archive_conflict_winner");
            row.texts.push("Overwrites another archive file");
        }
        ConflictState::Overwritten => {
            row.icons[3] = Some(":/MO/gui/archive_conflict_loser");
            row.texts.push("Overwritten by another archive file");
        }
        _ => {}
    }
    row
}

impl ConflictView {
    /// Scans the profile's enabled mods (a 1,200-mod list: seconds; on a worker).
    pub fn build(instance: &Instance, profile: &Profile) -> Result<Self> {
        let layers = resolve_layers(instance, profile)?;
        Ok(Self { index: ConflictIndex::build(&layers, instance.game.data_dir(), crate::conflicts::archive_rules(instance, profile)?) })
    }

    /// After the profile's layers changed (a toggle, a move): only the layers concerned are
    /// recomputed. Returns the mods whose flags changed.
    pub fn update(&mut self, instance: &Instance, profile: &Profile) -> Result<Vec<String>> {
        Ok(self.index.update(&resolve_layers(instance, profile)?))
    }

    /// A mod's row; the default for mods without conflicts or not enabled, and for Overwrite
    /// ("don't do flags for overwrite").
    pub fn row(&self, name: &str) -> ConflictRow {
        self.index.flags(name).map(|f| row_of(&f)).unwrap_or_default()
    }

    /// What selecting `names` highlights.
    pub fn markers(&self, names: &[String]) -> Markers {
        let mut m = Markers::default();
        for name in names {
            let Some(f) = self.index.flags(name) else { continue };
            // ModInfo's lists: who this mod overwrites is drawn "overwritten", and so on
            m.overwritten.extend(f.overwrites.iter().chain(&f.archive_loose_overwrites).cloned());
            m.overwriting.extend(f.overwritten_by.iter().chain(&f.archive_loose_overwritten_by).cloned());
            m.archive_overwritten.extend(f.archive_overwrites.iter().cloned());
            m.archive_overwriting.extend(f.archive_overwritten_by.iter().cloned());
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_instance::TestInstance;

    #[test]
    fn flags_icons_and_markers_follow_the_index() {
        let t = TestInstance::new("+High\r\n+Low\r\n+Alone\r\n");
        for (m, file) in [("High", "textures/a.dds"), ("Low", "textures/a.dds"), ("Alone", "meshes/b.nif")] {
            let p = t.folder(m).join(file);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, m).unwrap();
        }
        std::fs::write(t.folder("Alone").join("x.esp.mohidden"), b"").unwrap();
        let (instance, profile) = t.open();
        let view = ConflictView::build(&instance, &profile).unwrap();

        let high = view.row("High");
        assert_eq!(high.icons[0], Some(":/MO/gui/emblem_conflict_overwrite"));
        assert_eq!(high.texts, ["Overwrites loose files"]);
        // Low's only file is won by High: redundant
        assert_eq!(view.row("Low").icons[0], Some(":/MO/gui/emblem_conflict_redundant"));
        assert!(view.row("Alone").hidden_files);
        assert_eq!(view.row("Alone").icons, [None; 4]);
        assert_eq!(view.row("Overwrite"), ConflictRow::default());

        let m = view.markers(&["High".to_owned()]);
        assert!(m.overwritten.contains("Low"));
        assert!(m.overwriting.is_empty());
        let m = view.markers(&["Low".to_owned()]);
        assert!(m.overwriting.contains("High"));

        let colors = MarkerColors::of(&instance);
        assert_eq!(colors.overwriting_loose, Color { r: 255, g: 0, b: 0, a: 64 });
    }
}
