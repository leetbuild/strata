//! The profiles dialog (`ProfilesDialog`, `profilesdialog.ui`): its list and its actions.

use strata_core::profiles::{profile_setting, store_profile_settings};
use strata_core::{Instance, Trash};
use strata_ini::Value;

use crate::Result;

/// One row of the profile list with the three checkboxes under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRow {
    pub name: String,
    /// The instance's selected profile (MO2 refuses to rename or delete it).
    pub selected: bool,
    pub local_saves: bool,
    pub local_settings: bool,
    /// `AutomaticArchiveInvalidation` (stored only: Skyrim SE has no invalidation feature).
    pub archive_invalidation: bool,
}

/// Every profile, sorted as MO2 lists them.
pub fn rows(instance: &Instance) -> Result<Vec<ProfileRow>> {
    let mut rows = Vec::new();
    for name in instance.profile_names()? {
        let profile = instance.profile(Some(&name))?;
        let invalidation = profile_setting(&profile, "", "AutomaticArchiveInvalidation")?
            .map_or_else(|| instance.setting_bool("Settings", "profile_archive_invalidation"), |v| v.to_bool());
        rows.push(ProfileRow {
            selected: profile.name == instance.selected_profile,
            local_saves: profile.local_saves,
            local_settings: profile.local_settings,
            archive_invalidation: invalidation,
            name: profile.name,
        });
    }
    Ok(rows)
}

/// The invalidation checkbox (`on_invalidationBox_stateChanged`).
pub fn set_archive_invalidation(instance: &Instance, name: &str, on: bool) -> Result<bool> {
    let profile = instance.profile(Some(name))?;
    let value = Value::String(if on { "true" } else { "false" }.into());
    Ok(store_profile_settings(&profile, &[("", "AutomaticArchiveInvalidation", Some(value))])?)
}

/// Deletes a profile to the desktop's trash (MO2: the Recycle Bin).
pub fn delete(instance: &Instance, name: &str) -> Result<()> {
    instance.delete_profile(name, Some(&Trash::home()?))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_show_the_checkboxes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let w = |p: &str, t: &str| {
            std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
            std::fs::write(root.join(p), t).unwrap();
        };
        w("ModOrganizer.ini", "[General]\ngameName=Skyrim Special Edition\nselected_profile=@ByteArray(B)\ngamePath=/g\n");
        std::fs::create_dir_all(root.join("mods")).unwrap();
        w("profiles/A/modlist.txt", "");
        w("profiles/A/settings.ini", "[General]\nLocalSaves=true\nAutomaticArchiveInvalidation=false\n");
        w("profiles/B/modlist.txt", "");
        let instance = Instance::open(root).unwrap();
        let rows = rows(&instance).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].local_saves && !rows[0].archive_invalidation && !rows[0].selected);
        assert!(rows[1].selected && rows[1].archive_invalidation, "missing: the instance default (true)");
        assert!(set_archive_invalidation(&instance, "B", false).unwrap());
        assert!(!super::rows(&instance).unwrap()[1].archive_invalidation);
    }
}
