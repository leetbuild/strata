//! Skyrim (2011, "LE", 1.9.32), from MO2's `game_skyrim` plugin (Fluorine `libs/game_bethesda/
//! src/games/skyrim`).

use crate::skyrimse::bethesda_mappings;
use crate::{Game, Invalidation, LoadOrderMechanism, LogBase, Mapping, PluginListFormat};

/// Skyrim (Legendary Edition).
#[derive(Debug, Clone, Copy, Default)]
pub struct SkyrimLE;

impl Game for SkyrimLE {
    fn name(&self) -> &'static str {
        // GameSkyrim::gameName
        "Skyrim"
    }

    fn binary(&self) -> &'static str {
        "TESV.exe"
    }

    fn steam_app_id(&self) -> u32 {
        72_850
    }

    fn data_dir(&self) -> &'static str {
        "Data"
    }

    fn documents_dir(&self) -> &'static str {
        "Skyrim"
    }

    fn appdata_dir(&self) -> &'static str {
        // GameSkyrim::initializeProfile: localAppFolder() + "/Skyrim"
        "Skyrim"
    }

    fn ini_files(&self) -> &'static [&'static str] {
        &["Skyrim.ini", "SkyrimPrefs.ini"]
    }

    fn primary_plugins(&self) -> &'static [&'static str] {
        &["skyrim.esm", "update.esm"]
    }

    fn ccc_file(&self) -> Option<&'static str> {
        None
    }

    fn light_plugins_supported(&self) -> bool {
        // GamebryoGamePlugins::lightPluginsAreSupported: false.
        false
    }

    fn form_version(&self) -> Option<u16> {
        // MO2's Form 43 checker is for Skyrim SE only; 43 is LE's own version.
        None
    }

    fn script_extender_log(&self) -> Option<(LogBase, &'static str)> {
        // ScriptExtenderPluginChecker: "Skyrim": SKSE/skse.log in the documents folder.
        Some((LogBase::Documents, "SKSE/skse.log"))
    }

    fn loot_name(&self) -> &'static str {
        "skyrim"
    }

    fn nexus_name(&self) -> &'static str {
        "skyrim"
    }

    fn plugin_list_format(&self) -> PluginListFormat {
        PluginListFormat::Gamebryo
    }

    fn script_extender_loader(&self) -> Option<&'static str> {
        // SkyrimScriptExtender::BinaryName
        Some("skse_loader.exe")
    }

    fn executables(&self) -> &'static [(&'static str, &'static str)] {
        // GameSkyrim::executables, those in the game folder.
        &[
            ("SKSE", "skse_loader.exe"),
            ("SBW", "SBW.exe"),
            ("Skyrim", "TESV.exe"),
            ("Skyrim Launcher", "SkyrimLauncher.exe"),
            ("Creation Kit", "CreationKit.exe"),
        ]
    }

    fn invalidation(&self) -> Option<Invalidation> {
        // SkyrimBSAInvalidation
        Some(Invalidation {
            bsa_name: "Skyrim - Invalidation.bsa",
            bsa_version: 0x68,
            ini: "Skyrim.ini",
            // SkyrimDataArchives
            archive_keys: &["SResourceArchiveList", "SResourceArchiveList2"],
        })
    }

    fn load_order_mechanism(&self) -> LoadOrderMechanism {
        // GameSkyrim::loadOrderMechanism: PluginsTxt for TESV.exe 1.4.26 and later (on Windows;
        // Fluorine's Linux build skips the version check and says FileTime).
        LoadOrderMechanism::PluginsTxt
    }

    fn mod_mappings(&self) -> Vec<Mapping> {
        bethesda_mappings(self.ini_files())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn found_by_its_mo2_name_and_not_mixed_up_with_se() {
        assert_eq!(crate::by_name("Skyrim").unwrap().binary(), "TESV.exe");
        assert_eq!(crate::by_name("Skyrim Special Edition").unwrap().binary(), "SkyrimSE.exe");
        assert_eq!(SkyrimLE.plugin_list_format(), PluginListFormat::Gamebryo);
        assert!(!SkyrimLE.light_plugins_supported());
        assert!(SkyrimLE.mod_mappings()[1].exclude.iter().any(|e| e == "SkyrimPrefs.ini"));
    }
}
