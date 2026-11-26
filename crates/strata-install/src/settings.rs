//! The install settings stored in `ModOrganizer.ini`.
//!
//! - `[Settings] auto_enable_install` (Leet change 11, on by default).
//! - Third-party MO2 plugins built in (each behind a setting): their settings keep the
//!   plugins' own `[Plugins]` keys, so an instance that had them configured keeps its
//!   choices. A plugin's on/off switch is MO2's `<plugin>\enabled` key.
//!   - "Install to Separator" (`Install to Separator\enabled`, on;
//!     `remember_last_separator`, on; `last_separator\<profile>`).

use strata_ini::{Document, Value};

pub const INSTALL_TO_SEPARATOR: &str = "Install to Separator";

/// The settings, read once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallSettings {
    pub auto_enable: bool,
    pub install_to_separator: bool,
    pub remember_last_separator: bool,
}

fn flag(ini: &Document, section: &str, key: &str, default: bool) -> bool {
    ini.value(section, key).map_or(default, |v| v.to_bool())
}

impl InstallSettings {
    pub fn read(ini: &Document) -> Self {
        let plugin = |name: &str, key: &str, default| flag(ini, "Plugins", &format!("{name}/{key}"), default);
        Self {
            auto_enable: flag(ini, "Settings", "auto_enable_install", true),
            install_to_separator: plugin(INSTALL_TO_SEPARATOR, "enabled", true),
            remember_last_separator: plugin(INSTALL_TO_SEPARATOR, "remember_last_separator", true),
        }
    }
}

/// The separator last chosen for this profile (Install to Separator's `last_separator`).
pub fn last_separator(ini: &Document, profile: &str) -> Option<String> {
    ini.string("Plugins", &format!("{INSTALL_TO_SEPARATOR}/last_separator/{profile}")).filter(|s| !s.is_empty())
}

/// Stores the separator chosen for this profile; returns whether the document changed.
pub fn set_last_separator(ini: &mut Document, profile: &str, separator: Option<&str>) -> bool {
    ini.set(
        "Plugins",
        &format!("{INSTALL_TO_SEPARATOR}/last_separator/{profile}"),
        &Value::String(separator.unwrap_or("").to_owned()),
    )
}

/// Stores "auto-enable installed mods" in the instance's `ModOrganizer.ini`.
pub fn store_auto_enable(instance_root: &std::path::Path, on: bool) -> crate::Result<()> {
    let value = Value::String(if on { "true" } else { "false" }.to_owned());
    strata_core::settings::store_instance_ini(instance_root, &[("Settings", "auto_enable_install", Some(value))])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_real_keys() {
        let s = InstallSettings::read(&Document::parse(b""));
        assert!(s.auto_enable && s.install_to_separator);
        #[allow(unused_mut)]
        let mut ini = b"[Settings]\nauto_enable_install=false\n[Plugins]\nInstall%20to%20Separator\\last_separator\\red=Tools\n".to_vec();
        let doc = Document::parse(&ini);
        let s = InstallSettings::read(&doc);
        assert!(!s.auto_enable);
        assert_eq!(last_separator(&doc, "red").as_deref(), Some("Tools"));
        let mut doc = doc;
        assert!(set_last_separator(&mut doc, "red", Some("Next")));
        assert_eq!(last_separator(&doc, "red").as_deref(), Some("Next"));
        assert!(String::from_utf8(doc.to_bytes()).unwrap().contains("Install%20to%20Separator\\last_separator\\red=Next"));
    }

    #[test]
    fn auto_enable_is_stored() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("ModOrganizer.ini"), "[General]\ngameName=x\n[Settings]\nstyle=a\n").unwrap();
        store_auto_enable(d.path(), false).unwrap();
        let text = std::fs::read_to_string(d.path().join("ModOrganizer.ini")).unwrap();
        assert_eq!(text, "[General]\ngameName=x\n[Settings]\nstyle=a\nauto_enable_install=false\n");
        assert!(!InstallSettings::read(&Document::parse(text.as_bytes())).auto_enable);
    }
}
