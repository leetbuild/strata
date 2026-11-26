//! Deleting through the desktop's trash (freedesktop.org Trash specification 1.0), where
//! MO2 uses the Recycle Bin (`shellDelete(..., recycle)`): a deleted profile, cleared
//! overwrite or local saves can be got back from the file manager.
//!
//! Only the home trash (`$XDG_DATA_HOME/Trash`) is used, and only for files on its file
//! system: a move there is a rename. Anything else is refused with a message, never copied
//! and deleted behind the user's back.

use std::path::{Path, PathBuf};

use crate::time::LocalTime;
use crate::{Error, Result};

/// A trash folder (with `files/` and `info/`).
#[derive(Debug, Clone)]
pub struct Trash {
    dir: PathBuf,
}

impl Trash {
    /// The user's home trash: `$XDG_DATA_HOME/Trash`, by default `~/.local/share/Trash`.
    pub fn home() -> Result<Self> {
        match crate::dirs::data_home() {
            Some(data) => Ok(Self::at(data.join("Trash"))),
            None => Err(Error::Refused { what: "neither XDG_DATA_HOME nor HOME is set, so there is no trash to move files to".into() }),
        }
    }

    /// A trash in `dir` (tests).
    pub fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Moves `path` (a file or folder) to the trash and returns where it went.
    pub fn put(&self, path: &Path) -> Result<PathBuf> {
        let absolute = std::path::absolute(path).map_err(|source| Error::Read { path: path.to_owned(), source })?;
        let files = self.dir.join("files");
        let info = self.dir.join("info");
        for dir in [&files, &info] {
            std::fs::create_dir_all(dir).map_err(|source| Error::Write { path: dir.clone(), source })?;
        }
        if !same_device(&absolute, &files) {
            return Err(Error::Refused {
                what: format!(
                    "{} is on another file system than the trash ({}); delete it permanently instead",
                    absolute.display(),
                    self.dir.display()
                ),
            });
        }
        let base = absolute.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "unnamed".into());
        // The info file is created first and exclusively: it reserves the name.
        let (name, info_path, mut handle) = (1..=1000)
            .map(|n| if n == 1 { base.clone() } else { format!("{base}.{n}") })
            .find_map(|name| {
                let file = info.join(format!("{name}.trashinfo"));
                std::fs::OpenOptions::new().write(true).create_new(true).open(&file).ok().map(|f| (name, file, f))
            })
            .ok_or_else(|| Error::Refused { what: format!("cannot record {} in the trash ({})", absolute.display(), info.display()) })?;
        use std::io::Write;
        let now = LocalTime::now();
        let text = format!(
            "[Trash Info]\nPath={}\nDeletionDate={:04}-{:02}-{:02}T{:02}:{:02}:{:02}\n",
            percent_encode(&absolute),
            now.year,
            now.month,
            now.day,
            now.hour,
            now.minute,
            now.second
        );
        handle.write_all(text.as_bytes()).map_err(|source| Error::Write { path: info_path.clone(), source })?;
        let target = files.join(&name);
        if let Err(source) = std::fs::rename(&absolute, &target) {
            let _ = std::fs::remove_file(&info_path);
            return Err(Error::Write { path: absolute, source });
        }
        Ok(target)
    }
}

fn same_device(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev(),
        _ => false,
    }
}

/// The spec's `Path=` value: the path's bytes, `%XX` for anything but unreserved
/// characters and `/`.
fn percent_encode(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Removes `path` (a file or a whole folder) for good, or moves it to `trash`.
pub fn remove(path: &Path, trash: Option<&Trash>) -> Result<()> {
    match trash {
        Some(t) => t.put(path).map(|_| ()),
        None => {
            let meta = std::fs::symlink_metadata(path).map_err(|source| Error::Read { path: path.to_owned(), source })?;
            let result = if meta.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
            result.map_err(|source| Error::Write { path: path.to_owned(), source })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trashing_moves_the_file_and_writes_its_info() {
        let tmp = tempfile::tempdir().unwrap();
        let trash = Trash::at(tmp.path().join("Trash"));
        let victim = tmp.path().join("my profile");
        std::fs::create_dir(&victim).unwrap();
        std::fs::write(victim.join("modlist.txt"), "x").unwrap();
        let went = trash.put(&victim).unwrap();
        assert!(!victim.exists());
        assert_eq!(std::fs::read(went.join("modlist.txt")).unwrap(), b"x");
        let info = std::fs::read_to_string(tmp.path().join("Trash/info/my profile.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath=/"), "{info}");
        assert!(info.contains("my%20profile\nDeletionDate=20"), "{info}");

        // a second one with the same name gets another
        std::fs::create_dir(&victim).unwrap();
        assert!(trash.put(&victim).unwrap().ends_with("my profile.2"));
    }
}
