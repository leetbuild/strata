//! Strata's own settings (not an instance's): which instance the window opened last.
//!
//! The file is `$XDG_CONFIG_HOME/strata/strata.ini` (`~/.config/strata/strata.ini`), in the
//! same ini format as MO2's files. MO2 keeps the same fact in the Windows registry
//! (`InstanceManager::currentInstance`, `HKCU\Software\Mod Organizer Team\Mod Organizer\CurrentInstance`).

use std::path::{Path, PathBuf};

use strata_ini::{Document, Value};

use strata_core::settings::{Missing, store_ini};
use crate::{Error, Result};

/// `[General]`: strata-ini names it "", as QSettings does.
const SECTION: &str = "";
const LAST_INSTANCE: &str = "last_instance";

/// Where Strata's own settings live, from the environment (`None` without `$HOME`).
pub fn config_file() -> Option<PathBuf> {
    let base = strata_core::dirs::config_home()?;
    Some(base.join("strata/strata.ini"))
}

fn is_instance(dir: &Path) -> bool {
    dir.join("ModOrganizer.ini").is_file()
}

/// The instance folder the window opened last, as stored in `config`.
pub fn last_instance(config: &Path) -> Option<PathBuf> {
    let data = std::fs::read(config).ok()?;
    Document::parse(&data).string(SECTION, LAST_INSTANCE).filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// Stores `instance` as the last one opened. Writes nothing when it already is.
pub fn remember_instance(config: &Path, instance: &Path) -> Result<()> {
    let value = Value::String(instance.to_string_lossy().into_owned());
    store_ini(config, &[(SECTION, LAST_INSTANCE, Some(value))], Missing::Create)?;
    Ok(())
}

/// Which instance the window opens:
///
/// 1. the one given with `-i` (relative to `cwd`), whatever it holds (opening it reports
///    what is wrong);
/// 2. else `cwd` when it is an instance, as the command line does;
/// 3. else the last instance opened, when it still is one.
pub fn choose_instance(given: Option<&Path>, cwd: &Path, config: Option<&Path>) -> Result<PathBuf> {
    if let Some(given) = given {
        return Ok(cwd.join(given));
    }
    if is_instance(cwd) {
        return Ok(cwd.to_owned());
    }
    config.and_then(last_instance).filter(|dir| is_instance(dir)).ok_or(Error::NoInstance)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("ModOrganizer.ini"), "[General]\n").unwrap();
    }

    #[test]
    fn the_given_instance_wins_then_the_current_folder_then_the_last_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b, elsewhere) = (tmp.path().join("a"), tmp.path().join("b"), tmp.path().join("x"));
        instance(&a);
        instance(&b);
        std::fs::create_dir_all(&elsewhere).unwrap();
        let config = tmp.path().join("cfg/strata/strata.ini");

        assert!(matches!(choose_instance(None, &elsewhere, Some(&config)), Err(Error::NoInstance)));
        remember_instance(&config, &a).unwrap();
        assert_eq!(choose_instance(None, &elsewhere, Some(&config)).unwrap(), a);
        assert_eq!(choose_instance(None, &b, Some(&config)).unwrap(), b);
        assert_eq!(choose_instance(Some(Path::new("b")), tmp.path(), Some(&config)).unwrap(), b);
    }

    #[test]
    fn a_last_instance_that_is_gone_is_not_opened() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("strata.ini");
        remember_instance(&config, &tmp.path().join("gone")).unwrap();
        assert!(matches!(choose_instance(None, tmp.path(), Some(&config)), Err(Error::NoInstance)));
    }

    #[test]
    fn remembering_the_same_instance_twice_writes_once_and_keeps_other_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("strata.ini");
        std::fs::write(&config, "[General]\nother=1\n").unwrap();
        remember_instance(&config, Path::new("/x")).unwrap();
        let first = std::fs::metadata(&config).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        remember_instance(&config, Path::new("/x")).unwrap();
        assert_eq!(std::fs::metadata(&config).unwrap().modified().unwrap(), first);
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "[General]\nother=1\nlast_instance=/x\n");
    }
}
