//! The Elder Scrolls IV: Oblivion (classic, 1.2.0.416), from MO2's `game_oblivion` plugin
//! (Fluorine `libs/game_bethesda/src/games/oblivion`). No Oblivion game folder exists on this
//! PC yet: everything here is checked against generated files only.

use crate::skyrimse::bethesda_mappings;
use crate::{Game, Invalidation, LoadOrderMechanism, LogBase, Mapping, PluginListFormat};

/// Oblivion.
#[derive(Debug, Clone, Copy, Default)]
pub struct Oblivion;

impl Game for Oblivion {
    fn name(&self) -> &'static str {
        // GameOblivion::gameName
        "Oblivion"
    }

    fn binary(&self) -> &'static str {
        "Oblivion.exe"
    }

    fn steam_app_id(&self) -> u32 {
        22_330
    }

    fn data_dir(&self) -> &'static str {
        "Data"
    }

    fn documents_dir(&self) -> &'static str {
        "Oblivion"
    }

    fn appdata_dir(&self) -> &'static str {
        "Oblivion"
    }

    fn ini_files(&self) -> &'static [&'static str] {
        &["Oblivion.ini", "OblivionPrefs.ini"]
    }

    fn primary_plugins(&self) -> &'static [&'static str] {
        &["oblivion.esm", "update.esm"]
    }

    fn ccc_file(&self) -> Option<&'static str> {
        None
    }

    fn light_plugins_supported(&self) -> bool {
        false
    }

    fn form_version(&self) -> Option<u16> {
        None
    }

    fn script_extender_log(&self) -> Option<(LogBase, &'static str)> {
        // ScriptExtenderPluginChecker: "Oblivion": obse.log in the game folder.
        Some((LogBase::Game, "obse.log"))
    }

    fn loot_name(&self) -> &'static str {
        "oblivion"
    }

    fn nexus_name(&self) -> &'static str {
        "oblivion"
    }

    fn plugin_list_format(&self) -> PluginListFormat {
        // GameOblivion registers GamebryoGamePlugins.
        PluginListFormat::Gamebryo
    }

    fn script_extender_loader(&self) -> Option<&'static str> {
        // OblivionScriptExtender::BinaryName
        Some("obse_loader.exe")
    }

    fn executables(&self) -> &'static [(&'static str, &'static str)] {
        // GameOblivion::executables, those in the game folder.
        &[
            ("Oblivion", "Oblivion.exe"),
            ("Oblivion Launcher", "OblivionLauncher.exe"),
            ("Oblivion Mod Manager", "OblivionModManager.exe"),
            ("Construction Set", "TESConstructionSet.exe"),
        ]
    }

    fn invalidation(&self) -> Option<Invalidation> {
        // OblivionBSAInvalidation, OblivionDataArchives
        Some(Invalidation {
            bsa_name: "Oblivion - Invalidation.bsa",
            bsa_version: 0x67,
            ini: "Oblivion.ini",
            archive_keys: &["SArchiveList"],
        })
    }

    fn load_order_mechanism(&self) -> LoadOrderMechanism {
        // GameGamebryo::loadOrderMechanism
        LoadOrderMechanism::FileTime
    }

    fn mod_mappings(&self) -> Vec<Mapping> {
        bethesda_mappings(self.ini_files())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn found_by_its_mo2_name() {
        let game = crate::by_name("Oblivion").unwrap();
        assert_eq!(game.binary(), "Oblivion.exe");
        assert_eq!(game.load_order_mechanism(), LoadOrderMechanism::FileTime);
        assert_eq!(game.script_extender_log(), Some((LogBase::Game, "obse.log")));
        assert_eq!(game.invalidation().unwrap().bsa_version, 0x67);
    }
}
