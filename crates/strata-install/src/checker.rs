//! "Does this folder look like the game's Data folder?" (MO2's `ModDataChecker`).
//!
//! The rules are the game plugin's: `GamebryoModDataChecker::dataLooksValid` with the
//! folder names and extensions of `SkyrimSEModDataChecker` (Fluorine `libs/game_bethesda`,
//! `skyrimsemoddatachecker.h`), plus Leet change 3: a non-empty top-level `Root` folder
//! makes a mod valid (`GameFeatures::CombinedModDataChecker::dataLooksValid`). The SSE
//! checker never answers "fixable", so there is no fixer.
//!
//! Strata keeps the lists here, keyed by the game's name, until the `Game` trait grows a
//! data checker (`docs/ARCHITECTURE.md` gives it that job).

use crate::tree::{Dir, Node, fold, suffix};

/// The name of a mod's game-folder files folder (`AppConfig::rootFolderName()`).
pub const ROOT: &str = "Root";

/// A game's Data checker.
#[derive(Debug, Clone)]
pub struct DataChecker {
    /// The game's data folder name (`Data`), used by the quick installer's "Data + docs"
    /// rule.
    pub data_dir: &'static str,
    folders: &'static [&'static str],
    extensions: &'static [&'static str],
}

const SSE_FOLDERS: &[&str] = &[
    "fonts",
    "interface",
    "menus",
    "meshes",
    "music",
    "scripts",
    "shaders",
    "sound",
    "strings",
    "textures",
    "trees",
    "video",
    "facegen",
    "materials",
    "skse",
    "distantlod",
    "asi",
    "Tools",
    "MCM",
    "distantland",
    "mits",
    "dllplugins",
    "CalienteTools",
    "NetScriptFramework",
    "shadersfx",
    "Nemesis_Engine",
    "Platform",
    "grass",
    "LightPlacer",
    "mainmenuwallpapers",
    "MainMenuVideo",
    "PBRMaterialObjects",
    "PBRNifPatcher",
    "PBRTextureSets",
];

const SSE_EXTENSIONS: &[&str] = &["esp", "esm", "esl", "bsa", "modgroups", "ini"];

impl DataChecker {
    /// The checker for a game by its MO2 `gameName` (`ModOrganizer.ini`), if Strata knows
    /// the game.
    pub fn for_game(game_name: &str) -> Option<Self> {
        game_name.eq_ignore_ascii_case("Skyrim Special Edition").then_some(Self {
            data_dir: "Data",
            folders: SSE_FOLDERS,
            extensions: SSE_EXTENSIONS,
        })
    }

    /// `dataLooksValid`: one top-level folder with a known name or one file with a known
    /// extension is enough; so is a non-empty `Root` folder.
    pub fn looks_valid(&self, tree: &Dir) -> bool {
        if tree.child(ROOT).and_then(Node::as_dir).is_some_and(|root| !root.is_empty()) {
            return true;
        }
        tree.entries().iter().any(|entry| match entry {
            Node::Dir(d) => self.folders.iter().any(|f| fold(f) == fold(&d.name)),
            Node::File(f) => {
                let ext = fold(suffix(&f.name));
                self.extensions.contains(&ext.as_str())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::listing;

    #[test]
    fn sse_rules() {
        let c = DataChecker::for_game("Skyrim Special Edition").unwrap();
        assert!(c.looks_valid(&listing(&["Meshes/a.nif"])));
        assert!(c.looks_valid(&listing(&["SKSE/Plugins/a.dll"])));
        assert!(c.looks_valid(&listing(&["Plugin.ESP"])));
        assert!(c.looks_valid(&listing(&["Root/enbseries.ini"])));
        assert!(!c.looks_valid(&listing(&["Root/"])), "an empty Root folder doesn't count");
        assert!(!c.looks_valid(&listing(&["Wrapper/meshes/a.nif"])));
        assert!(!c.looks_valid(&listing(&["a.ba2"])), "SSE drops ba2 from the list");
        assert!(DataChecker::for_game("Oblivion").is_none());
    }
}
