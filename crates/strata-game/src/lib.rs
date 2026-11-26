//! The `Game` trait and its implementations (Skyrim SE first), and the layer model the
//! overlay is built from.
//!
//! A game is the counterpart of one of MO2's `game_bethesda` plugins: it knows the game's
//! folders, files and rules, and nothing about instances or profiles.

mod layer;
pub mod invalidation;
pub mod plugin;
pub mod save;
mod oblivion;
mod skyrimle;
mod skyrimse;

pub use layer::{Layer, LayerKind, Mapping};
pub use plugin::{PluginError, PluginHeader};
pub use save::{SaveError, SaveGame};
pub use oblivion::Oblivion;
pub use skyrimle::SkyrimLE;
pub use skyrimse::SkyrimSE;

/// One supported game.
pub trait Game: Send + Sync {
    /// The name MO2 stores as `gameName` in `ModOrganizer.ini`.
    fn name(&self) -> &'static str;

    /// The game's executable, relative to the game folder.
    fn binary(&self) -> &'static str;

    /// The Steam app id, used for `SteamAppId` and the Proton environment.
    fn steam_app_id(&self) -> u32;

    /// The data folder, relative to the game folder (`Data`).
    fn data_dir(&self) -> &'static str;

    /// The folder name under `Documents/My Games/` (inis, saves, script extender logs).
    fn documents_dir(&self) -> &'static str;

    /// The folder name under `AppData/Local/` (`plugins.txt`, `loadorder.txt`).
    fn appdata_dir(&self) -> &'static str;

    /// The game's ini files, as named in `Documents/My Games/<documents_dir>/`.
    fn ini_files(&self) -> &'static [&'static str];

    /// Plugins the game always loads first, lower case, in load order. They are never
    /// written to `plugins.txt`. The Creation Club list ([`Game::ccc_file`]) comes on top.
    fn primary_plugins(&self) -> &'static [&'static str];

    /// A file in the game folder that names more plugins the game always loads, one per
    /// line (`Skyrim.ccc`: the Creation Club content).
    fn ccc_file(&self) -> Option<&'static str>;

    /// Whether the game knows light plugins (`.esl`, the light flag): `GamePlugins::
    /// lightPluginsAreSupported`. Without them an `.esl` can never be enabled.
    fn light_plugins_supported(&self) -> bool;

    /// The lowest form version the game's own plugins have (Skyrim SE: 44). Plugins below it
    /// were made for an older game (MO2's Form 43 checker). `None`: no such check.
    fn form_version(&self) -> Option<u16>;

    /// The script extender's log and where it is (`SKSE/skse64.log` in the documents
    /// folder; `obse.log` in the game folder); `ScriptExtenderPluginChecker` reads it.
    fn script_extender_log(&self) -> Option<(LogBase, &'static str)>;

    /// The game's name for LOOT (`IPluginGame::lootGameName`, lootcli's `--game`).
    fn loot_name(&self) -> &'static str;

    /// The game's name on Nexus Mods (`IPluginGame::gameNexusName`): the domain in its pages
    /// and in nxm:// links (`skyrimspecialedition`).
    fn nexus_name(&self) -> &'static str;

    /// How `plugins.txt` marks active plugins (`CreationGamePlugins` vs `GamebryoGamePlugins`).
    fn plugin_list_format(&self) -> PluginListFormat;

    /// The script extender's loader in the game folder (`ScriptExtender::loaderName`).
    fn script_extender_loader(&self) -> Option<&'static str>;

    /// The executables MO2 adds when it finds them in the game folder (`IPluginGame::
    /// executables`): title, binary relative to the game folder.
    fn executables(&self) -> &'static [(&'static str, &'static str)];

    /// Archive invalidation, for games with MO2's `BSAInvalidation` feature.
    fn invalidation(&self) -> Option<Invalidation>;

    /// How the game itself orders plugins (`IPluginGame::loadOrderMechanism`).
    fn load_order_mechanism(&self) -> LoadOrderMechanism;

    /// Where a mod's (and overwrite's) files appear in the mounted game folder.
    fn mod_mappings(&self) -> Vec<Mapping>;
}

/// How a game's `plugins.txt` is written and read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginListFormat {
    /// Skyrim SE and later (`CreationGamePlugins`): every plugin but the primary ones, `*`
    /// before active ones.
    Creation,
    /// Skyrim LE and older (`GamebryoGamePlugins`, `SkyrimGamePlugins`): only the active
    /// plugins, primary ones included, no marks.
    Gamebryo,
}

/// Where a script extender writes its log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogBase {
    /// `Documents/My Games/<documents_dir>/`.
    Documents,
    /// The game folder.
    Game,
}

/// How the game decides the load order (`IPluginGame::LoadOrderMechanism`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOrderMechanism {
    /// The order of `plugins.txt` (Skyrim from 1.4.26 on).
    PluginsTxt,
    /// The plugin files' modification times (Oblivion; `GameGamebryo` default).
    FileTime,
}

/// MO2's archive invalidation for a game (`GamebryoBSAInvalidation`): a dummy BSA named in
/// the ini's archive list and `[Archive] bInvalidateOlderFiles=1`, `SInvalidationFile=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalidation {
    /// The dummy archive's name (`Skyrim - Invalidation.bsa`).
    pub bsa_name: &'static str,
    /// Its BSA version (`bsaVersion()`: 0x68 for Skyrim LE).
    pub bsa_version: u32,
    /// The ini that holds the settings (`Skyrim.ini`).
    pub ini: &'static str,
    /// The `[Archive]` keys of the archive list: one (`SArchiveList`), or two that MO2 splits
    /// past 255 characters (`SResourceArchiveList`, `SResourceArchiveList2`).
    pub archive_keys: &'static [&'static str],
}

/// Every game Strata supports, in the order the instance wizard lists them.
pub fn all() -> Vec<Box<dyn Game>> {
    vec![Box::new(SkyrimSE), Box::new(SkyrimLE), Box::new(Oblivion)]
}

/// The game for a `gameName` from `ModOrganizer.ini`, if Strata supports it.
pub fn by_name(name: &str) -> Option<Box<dyn Game>> {
    all().into_iter().find(|g| g.name().eq_ignore_ascii_case(name))
}
