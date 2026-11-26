//! An MO2 instance: the folder with `ModOrganizer.ini` (`docs/DESIGN.md`, "Instance files").

use std::path::{Path, PathBuf};

use strata_game::Game;
use strata_ini::Document;

use crate::{Error, Profile, Result, winepath};

/// The instance's folders, resolved (MO2 `Settings::paths()`; each can be moved in the ini,
/// with `%BASE_DIR%` standing for the base folder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstancePaths {
    pub base: PathBuf,
    pub mods: PathBuf,
    pub downloads: PathBuf,
    pub profiles: PathBuf,
    pub overwrite: PathBuf,
    pub cache: PathBuf,
}

/// The `[fluorine]` keys Strata reads so that an instance set up in Fluorine just works
/// (`docs/DESIGN.md`, Proton rule 1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProtonSettings {
    /// The compatdata-style folder that holds `pfx/`.
    pub prefix: Option<PathBuf>,
    /// The Proton folder (holds the `proton` script).
    pub proton: Option<PathBuf>,
    /// Whether the game needs Steam running (SteamStub DRM).
    pub steam_drm: bool,
    /// Variables the user sets for every launch (`[Settings] strata_environment`, Strata's
    /// own key: one `NAME=value` per line, `#` lines and blank lines ignored), applied on
    /// top of Strata's own (e.g. `PROTON_NO_NTSYNC=1`).
    pub environment: Vec<(String, String)>,
}

/// The `NAME=value` lines of `[Settings] strata_environment`.
pub fn parse_environment(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .filter(|(k, _)| !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .collect()
}

/// An opened instance. Opening only reads; the writing operations (profiles, settings,
/// executables) live in their own modules and re-read the file before they change it.
pub struct Instance {
    /// The instance folder.
    pub root: PathBuf,
    /// `ModOrganizer.ini`, kept whole so unknown keys survive a later write.
    pub ini: Document,
    pub game: Box<dyn Game>,
    /// The game folder (the Game Root), on this machine.
    pub game_path: PathBuf,
    pub paths: InstancePaths,
    /// The profile MO2 had selected.
    pub selected_profile: String,
    pub proton: ProtonSettings,
}

impl std::fmt::Debug for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Instance").field("root", &self.root).field("game", &self.game.name()).finish_non_exhaustive()
    }
}

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|source| Error::Read { path: path.to_owned(), source })
}

impl Instance {
    /// Opens the portable instance in `root`.
    pub fn open(root: &Path) -> Result<Self> {
        let ini_path = root.join("ModOrganizer.ini");
        if !ini_path.is_file() {
            return Err(Error::NotAnInstance { path: root.to_owned() });
        }
        let ini = Document::parse(&read(&ini_path)?);
        let missing = |key| Error::MissingKey { path: ini_path.clone(), section: "General", key };

        let proton = {
            let path = |key| ini.string("fluorine", key).filter(|s| !s.is_empty()).map(PathBuf::from);
            ProtonSettings {
                prefix: path("prefix_path"),
                proton: path("proton_path"),
                steam_drm: ini.value("fluorine", "steam_drm").is_some_and(|v| v.to_bool()),
                environment: parse_environment(&ini.string("Settings", "strata_environment").unwrap_or_default()),
            }
        };
        let pfx = proton.prefix.as_ref().map(|p| p.join("pfx"));
        let host = |stored: &str| winepath::to_host(stored, pfx.as_deref());

        let game_name = ini.string("", "gameName").ok_or_else(|| missing("gameName"))?;
        let game = strata_game::by_name(&game_name).ok_or(Error::UnknownGame { name: game_name })?;
        let game_path = host(&ini.string("", "gamePath").ok_or_else(|| missing("gamePath"))?)?;
        let selected_profile = ini.string("", "selected_profile").ok_or_else(|| missing("selected_profile"))?;

        // Settings::paths(): base_directory defaults to the instance folder, the others to
        // %BASE_DIR%/<default>.
        let base = match ini.string("Settings", "base_directory").filter(|s| !s.is_empty()) {
            Some(stored) => host(&stored)?,
            None => root.to_owned(),
        };
        let dir = |key: &str, default: &str| -> Result<PathBuf> {
            match ini.string("Settings", key).filter(|s| !s.is_empty()) {
                Some(stored) => host(&stored.replace("%BASE_DIR%", &base.to_string_lossy())),
                None => Ok(base.join(default)),
            }
        };
        let paths = InstancePaths {
            mods: dir("mod_directory", "mods")?,
            downloads: dir("download_directory", "downloads")?,
            profiles: dir("profiles_directory", "profiles")?,
            overwrite: dir("overwrite_directory", "overwrite")?,
            cache: dir("cache_directory", "webcache")?,
            base,
        };

        Ok(Self { root: root.to_owned(), ini, game, game_path, paths, selected_profile, proton })
    }

    /// Strata's own folder in the instance (`<instance>/strata`: whiteouts, journal).
    pub fn strata_dir(&self) -> PathBuf {
        self.root.join("strata")
    }

    /// The names of the instance's profiles, sorted.
    pub fn profile_names(&self) -> Result<Vec<String>> {
        let dir = &self.paths.profiles;
        let entries = std::fs::read_dir(dir).map_err(|source| Error::Read { path: dir.clone(), source })?;
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            // hidden folders are no profiles (QDir lists none on Linux); Strata builds new
            // profiles under such names
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        Ok(names)
    }

    /// Opens a profile by name (any case, as on Windows where MO2 runs), or the selected
    /// one for `None`.
    pub fn profile(&self, name: Option<&str>) -> Result<Profile> {
        let name = name.unwrap_or(&self.selected_profile);
        let mut dir = self.paths.profiles.join(name);
        let mut real = name.to_owned();
        if !dir.is_dir()
            && let Some(other) = self.profile_names().unwrap_or_default().into_iter().find(|n| n.eq_ignore_ascii_case(name))
        {
            dir = self.paths.profiles.join(&other);
            real = other;
        }
        if !dir.is_dir() {
            return Err(Error::NoSuchProfile {
                name: name.to_owned(),
                dir: self.paths.profiles.clone(),
                available: self.profile_names().unwrap_or_default().join(", "),
            });
        }
        Profile::open(&real, &dir)
    }
}

impl Instance {
    /// The plugins the game always loads, in load order, as named by the game and its
    /// Creation Club list (`GameSkyrimSE::primaryPlugins` + `CCPlugins`): each name once,
    /// compared without case. The list file is read from the real game folder, as MO2 does.
    /// A plugin named here loads only if its file exists in the merged Data folder.
    pub fn primary_plugins(&self) -> Vec<String> {
        let mut plugins: Vec<String> = self.game.primary_plugins().iter().map(|p| (*p).to_owned()).collect();
        let listed = self.game.ccc_file().and_then(|name| std::fs::read(self.game_path.join(name)).ok());
        for line in String::from_utf8_lossy(&listed.unwrap_or_default()).lines() {
            let name = line.trim();
            if !name.is_empty() && !plugins.iter().any(|p| p.eq_ignore_ascii_case(name)) {
                plugins.push(name.to_owned());
            }
        }
        plugins
    }
}

#[cfg(test)]
mod environment_tests {
    #[test]
    fn environment_lines() {
        let env = super::parse_environment("PROTON_NO_NTSYNC=1\n# a note\n\n DXVK_HUD = fps \nbad line\n=x\nA B=1\nEMPTY=\n");
        assert_eq!(
            env,
            [("PROTON_NO_NTSYNC".into(), "1".into()), ("DXVK_HUD".into(), "fps".into()), ("EMPTY".into(), String::new())]
        );
        // as the settings dialog stores it (QSettings' escaped string)
        let ini = strata_ini::Document::parse(b"[Settings]\r\nstrata_environment=\"PROTON_NO_NTSYNC=1\\nDXVK_HUD=fps\"\r\n");
        let text = ini.string("Settings", "strata_environment").unwrap();
        assert_eq!(super::parse_environment(&text).len(), 2, "{text:?}");
    }
}
