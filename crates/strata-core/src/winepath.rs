//! Paths as MO2 stores them. An instance used under Wine holds Windows paths
//! (`Z:\home\user\Games\...`, `C:\users\...`), one used by a native tool holds Linux paths.
//! Strata reads both and writes back the form it found.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Converts a stored path to a path on this machine.
///
/// - `/home/x` stays as it is.
/// - `Z:\home\x` (or `Z:/home/x`) is `/home/x`: Wine maps `Z:` to `/`.
/// - Any other drive letter lives in the prefix: `C:\users` is `<prefix>/drive_c/users`.
///   Without a prefix that is an error.
///
/// `prefix` is the folder holding `drive_c` (the `pfx` folder of a Proton prefix).
pub fn to_host(stored: &str, prefix: Option<&Path>) -> Result<PathBuf> {
    let bytes = stored.as_bytes();
    let has_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if !has_drive {
        return Ok(PathBuf::from(stored.replace('\\', "/")));
    }
    let drive = (bytes[0] as char).to_ascii_lowercase();
    let rest = stored[2..].replace('\\', "/");
    let rest = rest.trim_start_matches('/');
    if drive == 'z' {
        return Ok(Path::new("/").join(rest));
    }
    match prefix {
        Some(prefix) => Ok(prefix.join(format!("drive_{drive}")).join(rest)),
        None => Err(Error::NoPrefixForDrive { path: stored.to_owned(), drive: drive.to_ascii_uppercase() }),
    }
}

/// The Windows form of a path on this machine, for programs run in Wine: `Z:\home\x`.
pub fn to_wine(host: &Path) -> String {
    format!("Z:{}", host.to_string_lossy().replace('/', "\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_paths_become_host_paths() {
        let pfx = Path::new("/p/pfx");
        assert_eq!(to_host("Z:\\home\\user\\Game Root", None).unwrap(), Path::new("/home/user/Game Root"));
        assert_eq!(to_host("Z:/home/user/mods", None).unwrap(), Path::new("/home/user/mods"));
        assert_eq!(to_host("z:\\", None).unwrap(), Path::new("/"));
        assert_eq!(to_host("/home/user/x", None).unwrap(), Path::new("/home/user/x"));
        assert_eq!(to_host("C:\\users\\steamuser", Some(pfx)).unwrap(), Path::new("/p/pfx/drive_c/users/steamuser"));
        assert!(matches!(to_host("C:\\x", None), Err(Error::NoPrefixForDrive { drive: 'C', .. })));
    }

    #[test]
    fn host_paths_become_wine_paths() {
        assert_eq!(to_wine(Path::new("/home/user/Game Root/Data")), "Z:\\home\\user\\Game Root\\Data");
    }
}
