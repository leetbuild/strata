//! The profile's game inis for the ini editor (MO2's `tool_inieditor`): which file each
//! game ini is (`Profile::absoluteIniFilePath`) and saving an edited text back
//! (`TextViewer::saveFile`).

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use crate::profiles::find_any_case;
use crate::write::write_if_changed;
use crate::{Error, Instance, Profile, Result};

/// MO2's ini editor refuses files of this size or more (`IniEditor::display`).
pub const MAX_EDITABLE_SIZE: u64 = 1024 * 1024;

/// One of the game's inis as the profile sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IniFile {
    /// The name the game gives it (`Skyrim.ini`).
    pub name: String,
    /// Where it is: in the profile with local settings on, else in the prefix's
    /// `Documents/My Games/<game>`. `None` when it is not in the profile and the instance
    /// names no prefix.
    pub path: Option<PathBuf>,
    /// Its size, when it exists.
    pub size: Option<u64>,
    /// Whether the user's write bit is off (players set `Skyrim.ini` read-only so the game
    /// can't reset it).
    pub read_only: bool,
}

impl IniFile {
    /// Whether the editor shows it: it exists and is under [`MAX_EDITABLE_SIZE`].
    pub fn editable(&self) -> bool {
        self.size.is_some_and(|s| s < MAX_EDITABLE_SIZE)
    }
}

/// What to do when the file to save is read-only (the three buttons of MO2's "INI file is
/// read-only" question in `TextViewer::saveFile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnly {
    /// Don't write: an error says the file is read-only ("Skip this file").
    Refuse,
    /// Write and leave the file writable ("Clear the read-only flag").
    Clear,
    /// Write and make it read-only again ("Allow the write once").
    Once,
}

impl Instance {
    /// The game's inis for `profile`, in the game's order (`IPluginGame::iniFiles`), each
    /// placed as `Profile::absoluteIniFilePath` places it. A profile file is matched
    /// without case (profiles made on Windows hold `skyrim.ini`).
    pub fn ini_files(&self, profile: &Profile) -> Vec<IniFile> {
        let (documents, _) = self.prefix_game_dirs();
        self.game
            .ini_files()
            .iter()
            .map(|name| {
                let dir = if profile.local_settings { Some(profile.dir.clone()) } else { documents.clone() };
                let path = dir.map(|d| find_any_case(&d, name).unwrap_or_else(|| d.join(name)));
                let meta = path.as_ref().and_then(|p| std::fs::metadata(p).ok()).filter(|m| m.is_file());
                IniFile {
                    name: (*name).to_owned(),
                    path,
                    size: meta.as_ref().map(|m| m.len()),
                    read_only: meta.is_some_and(|m| m.permissions().mode() & 0o200 == 0),
                }
            })
            .collect()
    }

    /// Saves the edited `text` of the game ini `name` of `profile`. As MO2's
    /// `TextViewer::saveFile`, every `\n` is written as `\r\n` (a text that already has
    /// `\r\n` keeps it). Nothing is written when the bytes are the same; the file keeps its
    /// permissions except as `read_only` says. Returns whether the file was written.
    pub fn save_ini(&self, profile: &Profile, name: &str, text: &str, read_only: ReadOnly) -> Result<bool> {
        let file = self
            .ini_files(profile)
            .into_iter()
            .find(|f| f.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| Error::Refused {
                what: format!("\"{name}\" is not one of the game's inis ({})", self.game.ini_files().join(", ")),
            })?;
        let path = file.path.ok_or_else(|| Error::Refused {
            what: format!(
                "{name} is not in the profile (local inis are off) and the instance names no prefix to find it in. \
                 Set the prefix, or switch local inis on for the profile."
            ),
        })?;
        let mode = std::fs::metadata(&path).map(|m| m.permissions().mode() & 0o7777).ok();
        if file.read_only && read_only == ReadOnly::Refuse {
            return Err(Error::Refused {
                what: format!("{} is read-only. Clear its read-only flag, or allow the write once.", path.display()),
            });
        }
        let data = crlf(text);
        let written = write_if_changed(&path, &data)?;
        if let Some(mode) = mode {
            let mode = match read_only {
                ReadOnly::Clear => mode | 0o200,
                _ => mode,
            };
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                .map_err(|source| Error::Write { path: path.clone(), source })?;
        }
        Ok(written)
    }
}

/// `toUtf8().replace('\n', "\r\n")`, without doubling a `\r` already there.
fn crlf(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + text.len() / 16);
    let mut previous = 0u8;
    for &b in text.as_bytes() {
        if b == b'\n' && previous != b'\r' {
            out.push(b'\r');
        }
        out.push(b);
        previous = b;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_ends_are_written_as_mo2_writes_them() {
        assert_eq!(crlf("[General]\nsLanguage=ENGLISH\n"), b"[General]\r\nsLanguage=ENGLISH\r\n");
        assert_eq!(crlf("a\r\nb\nc"), b"a\r\nb\r\nc");
    }
}
