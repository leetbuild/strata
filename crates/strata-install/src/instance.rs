//! Installing into an opened instance: what the CLI and the window share.
//!
//! [`install_into`] = [`crate::install::install`] with the instance's folders, its FOMOD
//! environment ([`InstanceEnv`]), then the mod list ([`crate::profile::add_to_profile`]).
//! Refreshing a live deploy is the caller's (it talks to the overlay daemon).

use std::cell::OnceCell;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use strata_core::{Instance, Profile, Trash};

use crate::archive::Archives;
use crate::fomod::session::{Environment, FileState};
use crate::install::{InstallUi, Outcome, Preview, Target, install, preview};
use crate::profile::{Placed, Placement, add_to_profile};
use crate::settings::InstallSettings;
use crate::Result;

/// The install target of an instance.
pub fn target(instance: &Instance) -> Result<Target> {
    Ok(Target {
        game_name: instance.game.name().to_owned(),
        // IPluginGame::gameShortName; the Game trait has no short name yet (SSE only)
        game_short_name: "SkyrimSE".to_owned(),
        mods_dir: instance.paths.mods.clone(),
        downloads_dir: instance.paths.downloads.clone(),
        categories_dir: instance.root.clone(),
        temp_dir: instance.strata_dir().join("tmp"),
        trash: Trash::home()?,
    })
}

/// `fileDependency` and version answers from the instance.
pub struct InstanceEnv<'a> {
    instance: &'a Instance,
    profile: &'a Profile,
    tree: OnceCell<Option<strata_vfs::Tree>>,
}

impl<'a> InstanceEnv<'a> {
    pub fn new(instance: &'a Instance, profile: &'a Profile) -> Self {
        Self { instance, profile, tree: OnceCell::new() }
    }

    /// The merged tree, built the first time a FOMOD asks.
    fn tree(&self) -> Option<&strata_vfs::Tree> {
        self.tree
            .get_or_init(|| {
                let layers = strata_core::resolve_layers(self.instance, self.profile).ok()?;
                let whiteouts = self.instance.strata_dir().join("whiteouts.txt");
                strata_vfs::Tree::build(layers, strata_vfs::Options::default(), Some(whiteouts)).ok()
            })
            .as_ref()
    }
}

/// The file version of a Windows program (`VS_FIXEDFILEINFO`, found by its signature), as
/// MO2's `getFileVersion` gives it: `a.b.c.d`.
pub fn pe_version(bytes: &[u8]) -> Option<String> {
    let sig = 0xFEEF04BDu32.to_le_bytes();
    let at = bytes.windows(4).position(|w| w == sig)?;
    let word = |o: usize| bytes.get(at + o..at + o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (ms, ls) = (word(8)?, word(12)?);
    Some(format!("{}.{}.{}.{}", ms >> 16, ms & 0xffff, ls >> 16, ls & 0xffff))
}

/// The `ProductVersion` string of a Windows program's version resource (MO2's
/// `getProductVersion`): the UTF-16 key, padding to 4 bytes, then the value.
pub fn pe_product_version(bytes: &[u8]) -> Option<String> {
    let key: Vec<u8> = "ProductVersion\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let at = bytes.windows(key.len()).position(|w| w == key)?;
    let mut i = at + key.len();
    while !i.is_multiple_of(4) {
        i += 2;
    }
    let units: Vec<u16> =
        bytes.get(i..)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|u| *u != 0).collect();
    let text = String::from_utf16_lossy(&units).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// `GameGamebryo::gameVersion`: the file version, or the product version when the file
/// version is the "1.0.0" placeholder (Skyrim SE 1.5.97's exe says 1.0.0.0).
pub fn game_version_of(bytes: &[u8]) -> Option<String> {
    let version = pe_version(bytes)?;
    if version.starts_with("1.0.0")
        && let Some(product) = pe_product_version(bytes)
    {
        return Some(product);
    }
    Some(version)
}

impl Environment for InstanceEnv<'_> {
    /// Missing unless the merged Data folder has it; active when the game always loads it
    /// or `plugins.txt` marks it (MO2 asks its plugin list).
    fn file_state(&self, file: &str) -> FileState {
        let Some(tree) = self.tree() else { return FileState::Missing };
        let data = self.instance.game.data_dir();
        if tree.resolve(&format!("{data}/{file}")).is_none() {
            return FileState::Missing;
        }
        if self.instance.primary_plugins().iter().any(|p| p.eq_ignore_ascii_case(file)) {
            return FileState::Active;
        }
        let list = std::fs::read(self.profile.dir.join("plugins.txt")).unwrap_or_default();
        let active =
            strata_core::PluginListFile::parse(&list).plugins.iter().any(|(name, on)| *on && name.eq_ignore_ascii_case(file));
        if active { FileState::Active } else { FileState::Inactive }
    }

    fn game_version(&self) -> Option<String> {
        game_version_of(&std::fs::read(self.instance.game_path.join(self.instance.game.binary())).ok()?)
    }

    /// The loader's version in the merged game folder (SKSE's loader may live in a mod's
    /// `Root`; MO2 looks in the real game folder only).
    fn script_extender_version(&self) -> Option<String> {
        let tree = self.tree()?;
        let id = tree.resolve("skse64_loader.exe")?;
        pe_version(&std::fs::read(tree.real_path(id)?).ok()?)
    }
}

/// What [`install_into`] did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Installed {
    #[serde(flatten)]
    pub outcome: Outcome,
    pub enabled: bool,
    pub priority: usize,
    /// What BSA Extractor unpacked, when it asked and the answer was yes.
    pub bsa: Option<crate::bsa::Extracted>,
    /// The mod has INI tweaks and the user wants to configure them now: the window opens the
    /// mod's information on its INI files (MO2's `ModInfoTabIDs::IniFiles`).
    pub configure_ini: bool,
}

/// What the caller of [`install_into`] asks for besides the archive.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The mod's name (the user's).
    pub name: Option<String>,
    /// Overrides the auto-enable setting when `Some`.
    pub enable: Option<bool>,
    /// "Install to Separator".
    pub under: Option<String>,
    /// "Reinstall mod" (`ModListViewActions::reinstallMod`): the mod of this name is the one
    /// replaced; its installers' previous options are offered and its name is the default.
    pub reinstall: Option<String>,
    /// Installing from the Downloads tab (`OrganizerCore::installDownload`): an existing
    /// mod with the download's Nexus id is the one updated (its name is the default guess,
    /// its BAIN options are offered).
    pub from_downloads: bool,
}

/// The archive "Reinstall" uses for a mod (`reinstallMod`): its `installationFile`, as an
/// absolute path when that still exists, else that file name in the downloads folder.
pub fn reinstall_archive(instance: &Instance, mod_name: &str) -> Result<std::path::PathBuf> {
    let dir = instance.paths.mods.join(mod_name);
    if !dir.is_dir() {
        return Err(crate::Error::Invalid(format!("there is no mod named '{mod_name}'")));
    }
    let file = crate::meta::ModMeta::open(&dir)?.get("", "installationFile").unwrap_or_default();
    if file.is_empty() {
        return Err(crate::Error::Invalid(format!(
            "'{mod_name}' has no installationFile in its meta.ini (installed with an old MO2?): install its archive instead"
        )));
    }
    let path = Path::new(&file);
    let full = if path.is_absolute() && path.exists() {
        path.to_owned()
    } else {
        instance.paths.downloads.join(path.file_name().unwrap_or_default())
    };
    if !full.is_file() {
        return Err(crate::Error::Invalid(format!("the installation file {} no longer exists", full.display())));
    }
    Ok(full)
}

/// `installDownload`'s `currentMod`: the last mod (by name) with the download's game and
/// Nexus id that is not a backup.
fn mod_of_download(instance: &Instance, archive: &Path) -> Result<Option<String>> {
    let Some(dl) = crate::meta::DownloadMeta::read_for(archive)? else { return Ok(None) };
    if dl.mod_id <= 0 {
        return Ok(None);
    }
    let Ok(read) = std::fs::read_dir(&instance.paths.mods) else { return Ok(None) };
    let mut names: Vec<String> = read
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !strata_core::is_backup_name(n))
        .collect();
    names.sort();
    let mut found = None;
    for name in names {
        let meta = crate::meta::ModMeta::open(&instance.paths.mods.join(&name))?;
        let id = meta.get("", "modid").and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(-1);
        let game = meta.get("", "gameName").unwrap_or_default();
        if id == dl.mod_id && (dl.game_name.is_empty() || game.eq_ignore_ascii_case(&dl.game_name)) {
            found = Some(name);
        }
    }
    Ok(found)
}

/// The request for the core: the current mod, if any, and its previous options.
fn request(instance: &Instance, archive: &Path, options: &Options) -> Result<crate::install::Request> {
    let current = match &options.reinstall {
        Some(name) => Some(name.clone()),
        None if options.from_downloads => mod_of_download(instance, archive)?,
        None => None,
    };
    let previous = match &current {
        Some(name) => {
            let auto = instance
                .ini
                .value("Plugins", "Bundle Installer/auto_reinstall")
                .is_none_or(|v| v.to_bool());
            crate::install::Previous::of(&instance.paths.mods.join(name), options.reinstall.is_some(), auto)?
        }
        None => crate::install::Previous::default(),
    };
    let mut name = options.name.clone();
    if options.reinstall.is_some() && name.is_none() {
        // reinstallMod passes the mod's name as the user's
        name.clone_from(&current);
    }
    Ok(crate::install::Request { name, current_name: current, previous })
}

/// Installs `archive` into the instance and puts the mod in the profile's list.
pub fn install_into(
    instance: &Instance,
    profile: &Profile,
    archives: &dyn Archives,
    ui: &mut dyn InstallUi,
    cancel: &AtomicBool,
    archive: &Path,
    options: &Options,
) -> Result<Installed> {
    strata_core::write::recover(&instance.strata_dir())?;
    let settings = InstallSettings::read(&instance.ini);
    let under = options.under.as_deref().filter(|_| settings.install_to_separator);
    if let Some(under) = under {
        let wanted = under.trim_end_matches("_separator");
        if !crate::profile::separators(instance, profile)?.iter().any(|s| s.eq_ignore_ascii_case(wanted)) {
            return Err(crate::Error::Invalid(format!("there is no separator named '{under}'; nothing was installed")));
        }
    }
    let target = target(instance)?;
    let env = InstanceEnv::new(instance, profile);
    let outcome = install(&target, archives, &env, ui, cancel, archive, &request(instance, archive, options)?)?;
    let placement = Placement {
        enable: options.enable.unwrap_or(settings.auto_enable),
        under: under.map(str::to_owned),
    };
    let Placed { enabled, priority, .. } = add_to_profile(instance, profile, &outcome, &placement)?;
    let bsa = crate::bsa::offer(ui, &instance.root, &instance.ini, &outcome.mod_dir, &target.game_short_name)?;
    let configure_ini = outcome.ini_tweaks && ui.configure_ini_tweaks(&outcome.name);
    Ok(Installed {
        outcome,
        enabled,
        priority,
        bsa,
        configure_ini,
    })
}

/// [`crate::install::preview`] for the instance: nothing is written.
pub fn preview_into(
    instance: &Instance,
    profile: &Profile,
    archives: &dyn Archives,
    ui: &mut dyn InstallUi,
    archive: &Path,
    options: &Options,
) -> Result<Preview> {
    let target = target(instance)?;
    let env = InstanceEnv::new(instance, profile);
    preview(&target, archives, &env, ui, archive, &request(instance, archive, options)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(file: (u32, u32), product: &str) -> Vec<u8> {
        let mut bytes = vec![0u8; 100];
        bytes.extend(0xFEEF04BDu32.to_le_bytes());
        bytes.extend(0x0001_0000u32.to_le_bytes());
        bytes.extend(file.0.to_le_bytes());
        bytes.extend(file.1.to_le_bytes());
        bytes.extend([0u8; 2]);
        bytes.extend("ProductVersion\0".encode_utf16().flat_map(u16::to_le_bytes));
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        bytes.extend(format!("{product}\0").encode_utf16().flat_map(u16::to_le_bytes));
        bytes
    }

    #[test]
    fn versions_from_the_version_resource() {
        let skse = resource((2 << 16, 20 << 16), "2, 0, 20, 0");
        assert_eq!(pe_version(&skse).as_deref(), Some("2.0.20.0"));
        assert_eq!(game_version_of(&skse).as_deref(), Some("2.0.20.0"));
        // Skyrim SE 1.5.97: the fixed file info says 1.0.0.0
        let game = resource((1 << 16, 0), "1.5.97.0");
        assert_eq!(game_version_of(&game).as_deref(), Some("1.5.97.0"));
    }

    /// The real game exe of `STRATA_REAL_GAME` (read only), when it is set: a 1.5.97 copy.
    #[test]
    fn real_skyrim_exe() {
        let Some(game) = std::env::var_os("STRATA_REAL_GAME") else {
            return eprintln!("SKIPPED: real_skyrim_exe: STRATA_REAL_GAME is not set (real data, read only)");
        };
        let path = format!("{}/SkyrimSE.exe", game.to_string_lossy());
        let Ok(bytes) = std::fs::read(&path) else {
            return eprintln!("SKIPPED: real_skyrim_exe: no {path} (real data, read only)");
        };
        assert_eq!(game_version_of(&bytes).as_deref(), Some("1.5.97.0"));
    }
}
