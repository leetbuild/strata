//! The layer model: what the core hands to the overlay (`docs/ARCHITECTURE.md`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What a layer is, which decides how it is treated (only `Overwrite` is written to).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayerKind {
    /// The game's own folder: the bottom layer.
    Game,
    /// An enabled mod.
    Mod,
    /// The instance's overwrite folder (or, in a profile mount, the profile folder): where
    /// new files and changed copies go. Deleting a file below it leaves a whiteout.
    Overwrite,
    /// A real folder that programs may write straight into: the prefix folder under a
    /// profile mount. New files that no `Overwrite` layer above accepts land here.
    Passthrough,
}

/// A rule placing part of a layer's folder in the mounted tree.
///
/// Paths use `/`, are relative, and "" means the folder itself. Names compare without
/// regard to case, as everything in the overlay does.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Mapping {
    /// Subfolder of the layer's source ("" = the whole source).
    pub from: String,
    /// Where it appears, relative to the mount ("" = the mount's root).
    pub to: String,
    /// Entries directly inside `from` that this rule leaves out (for example `Root` under
    /// the rule that maps a mod to `Data`, or `Data` under the rule that maps `Root`).
    #[serde(default)]
    pub exclude: Vec<String>,
    /// When not empty: the only entries directly inside `from` that this rule covers (a
    /// profile mount shows just `plugins.txt` and `loadorder.txt` of the profile folder).
    #[serde(default)]
    pub include: Vec<String>,
    /// When a program changes one of these files, the change goes to the file itself
    /// instead of a copy in overwrite (Leet change 6: an `enbseries.ini` saved by ENB's
    /// editor goes back into its mod's `Root`).
    #[serde(default)]
    pub write_back: bool,
}

/// One layer of the overlay. Layers are ordered low to high priority; for each path the
/// highest layer that has it wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layer {
    /// A name for people and logs: the mod's name, `<game>` or `<overwrite>`.
    pub id: String,
    pub kind: LayerKind,
    /// The folder on disk.
    pub source: PathBuf,
    pub rules: Vec<Mapping>,
}

impl Mapping {
    /// A rule placing all of `from` at `to`.
    pub fn new(from: &str, to: &str) -> Self {
        Self { from: from.into(), to: to.into(), ..Self::default() }
    }

    /// Whether the rule covers an entry named `name` directly inside `from` (any case).
    pub fn covers(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        !self.exclude.iter().any(|e| e.to_lowercase() == name)
            && (self.include.is_empty() || self.include.iter().any(|i| i.to_lowercase() == name))
    }
}

impl LayerKind {
    /// Whether programs' writes may land in a layer of this kind.
    pub fn is_writable(self) -> bool {
        matches!(self, LayerKind::Overwrite | LayerKind::Passthrough)
    }
}
