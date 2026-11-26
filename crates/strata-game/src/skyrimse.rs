//! Skyrim Special Edition (1.5.97 first), from MO2's `game_skyrimse` plugin.

use crate::{Game, Invalidation, LoadOrderMechanism, LogBase, Mapping, PluginListFormat};

/// Skyrim Special Edition.
#[derive(Debug, Clone, Copy, Default)]
pub struct SkyrimSE;

impl Game for SkyrimSE {
    fn name(&self) -> &'static str {
        "Skyrim Special Edition"
    }

    fn binary(&self) -> &'static str {
        "SkyrimSE.exe"
    }

    fn steam_app_id(&self) -> u32 {
        489_830
    }

    fn data_dir(&self) -> &'static str {
        "Data"
    }

    fn documents_dir(&self) -> &'static str {
        "Skyrim Special Edition"
    }

    fn appdata_dir(&self) -> &'static str {
        "Skyrim Special Edition"
    }

    fn ini_files(&self) -> &'static [&'static str] {
        &["Skyrim.ini", "SkyrimPrefs.ini", "SkyrimCustom.ini"]
    }

    fn primary_plugins(&self) -> &'static [&'static str] {
        // GameSkyrimSE::primaryPlugins; the Creation Club ones come from ccc_file().
        &["skyrim.esm", "update.esm", "dawnguard.esm", "hearthfires.esm", "dragonborn.esm"]
    }

    fn ccc_file(&self) -> Option<&'static str> {
        Some("Skyrim.ccc")
    }

    fn light_plugins_supported(&self) -> bool {
        // CreationGamePlugins::lightPluginsAreSupported
        true
    }

    fn form_version(&self) -> Option<u16> {
        Some(44)
    }

    fn script_extender_log(&self) -> Option<(LogBase, &'static str)> {
        Some((LogBase::Documents, "SKSE/skse64.log"))
    }

    fn loot_name(&self) -> &'static str {
        "skyrimse"
    }

    fn nexus_name(&self) -> &'static str {
        "skyrimspecialedition"
    }

    fn plugin_list_format(&self) -> PluginListFormat {
        PluginListFormat::Creation
    }

    fn script_extender_loader(&self) -> Option<&'static str> {
        Some("skse64_loader.exe")
    }

    fn executables(&self) -> &'static [(&'static str, &'static str)] {
        // GameSkyrimSE::executables (those that live in the game folder).
        &[
            ("SKSE", "skse64_loader.exe"),
            ("Skyrim Special Edition", "SkyrimSE.exe"),
            ("Skyrim Special Edition Launcher", "SkyrimSELauncher.exe"),
            ("Creation Kit", "CreationKit.exe"),
        ]
    }

    fn invalidation(&self) -> Option<Invalidation> {
        // game_skyrimse registers no BSAInvalidation feature.
        None
    }

    fn load_order_mechanism(&self) -> LoadOrderMechanism {
        LoadOrderMechanism::PluginsTxt
    }

    fn mod_mappings(&self) -> Vec<Mapping> {
        bethesda_mappings(self.ini_files())
    }
}

/// The mappings of MO2's Bethesda games with Leet's Root folder: a mod's files are Data
/// files, except its top-level Root folder and MO2's own meta.ini (which MO2 never shows
/// the game either); Root holds game-folder files (Leet change 3), never Data, Data Files,
/// Saves or the game's inis (as Leet's Root deploy, change 6).
pub(crate) fn bethesda_mappings(inis: &[&str]) -> Vec<Mapping> {
    vec![
        Mapping { exclude: vec!["Root".into(), "meta.ini".into()], ..Mapping::new("", "Data") },
        Mapping {
            from: "Root".into(),
            to: String::new(),
            exclude: ["Data", "Data Files", "Saves"].into_iter().chain(inis.iter().copied()).map(String::from).collect(),
            write_back: true,
            include: Vec::new(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn found_by_its_mo2_name() {
        assert_eq!(crate::by_name("Skyrim Special Edition").unwrap().binary(), "SkyrimSE.exe");
        assert_eq!(crate::by_name("skyrim special edition").unwrap().steam_app_id(), 489_830);
        assert!(crate::by_name("Morrowind").is_none());
    }

    #[test]
    fn root_is_kept_out_of_data_and_the_reverse() {
        let rules = SkyrimSE.mod_mappings();
        assert!(rules[0].exclude.iter().any(|e| e == "Root"));
        assert!(rules[1].exclude.iter().any(|e| e == "Data"));
        assert!(rules[1].exclude.iter().any(|e| e == "Skyrim.ini"));
    }
}
