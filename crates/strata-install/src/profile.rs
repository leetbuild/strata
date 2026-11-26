//! After an install: the new mod's place in the profile's mod list.
//!
//! - `Profile::refreshModStatus` puts a mod the list does not name on top (highest priority),
//!   disabled; `OrganizerCore::installArchive` then enables it when "auto-enable installed
//!   mods" is on (Leet change 11) and the mod was not merged or replaced.
//! - "Install to Separator" (a third-party MO2 plugin): the new mod goes
//!   to the end of a chosen separator, as MO2 places a mod installed onto a separator
//!   (`priorityAfterSeparator`).


use strata_core::separators::priority_for_mod_in_separator;
use strata_core::{Instance, ModKind, ModList, ModListEvent, Profile};

use crate::install::Outcome;
use crate::{Error, Result};

/// Where and how the new mod lands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placement {
    /// Enable a freshly installed mod.
    pub enable: bool,
    /// Put a freshly installed mod at the end of this separator (its name with or without
    /// `_separator`).
    pub under: Option<String>,
}

/// What [`add_to_profile`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub enabled: bool,
    pub priority: usize,
    pub events: Vec<ModListEvent>,
}

/// Puts the installed mod in the profile's mod list and saves it.
pub fn add_to_profile(instance: &Instance, profile: &Profile, outcome: &Outcome, placement: &Placement) -> Result<Placed> {
    let mut list = ModList::load(instance, profile)?;
    let index = list.index_of(&outcome.name).ok_or_else(|| Error::Invalid(format!("'{}' is not in the mod list", outcome.name)))?;
    let fresh = !outcome.merged_or_replaced() && !list.entries()[index].listed;
    let mut events = Vec::new();
    if fresh {
        if placement.enable {
            events.extend(list.set_enabled(&[&outcome.name], true)?);
        }
        if let Some(under) = &placement.under {
            let sep = separator_index(&list, under)?;
            // as `ModList::create_mod` does for a mod made "in" a separator: before its first
            // sub-separator, not inside the last one
            let target = priority_for_mod_in_separator(list.entries(), sep).unwrap_or(usize::MAX);
            events.extend(list.set_priority(&outcome.name, target)?);
        }
    }
    // a listed name that was not in `modlist.txt` yet must be written, even unchanged
    if !events.is_empty() || !list.entries()[index].listed || fresh {
        list.save()?;
    }
    let entry = list.get(&outcome.name).ok_or_else(|| Error::Invalid(format!("'{}' left the mod list", outcome.name)))?;
    Ok(Placed { enabled: entry.enabled, priority: entry.priority, events })
}

fn separator_index(list: &ModList, name: &str) -> Result<usize> {
    let full = if name.ends_with("_separator") { name.to_owned() } else { format!("{name}_separator") };
    list.index_of(&full)
        .filter(|&i| list.entries()[i].kind == ModKind::Separator)
        .ok_or_else(|| Error::Invalid(format!("there is no separator named '{name}'")))
}

/// The separators of a list, as the user names them (without `_separator`), lowest
/// priority first: the choices of "install under separator".
pub fn separators(instance: &Instance, profile: &Profile) -> Result<Vec<String>> {
    let list = ModList::load(instance, profile)?;
    Ok(list
        .entries()
        .iter()
        .filter(|e| e.kind == ModKind::Separator)
        .map(|e| e.name.trim_end_matches("_separator").to_owned())
        .collect())
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use crate::installers::Installer;

    fn instance(dir: &Path) -> (Instance, Profile) {
        std::fs::write(
            dir.join("ModOrganizer.ini"),
            "[General]\ngameName=Skyrim Special Edition\ngamePath=/tmp/none\nselected_profile=p\n",
        )
        .unwrap();
        for m in ["A", "Tools_separator", "B", "Next_separator", "New"] {
            std::fs::create_dir_all(dir.join("mods").join(m)).unwrap();
        }
        std::fs::create_dir_all(dir.join("profiles/p")).unwrap();
        // top line = highest priority: Next_separator, B, Tools_separator, A
        std::fs::write(dir.join("profiles/p/modlist.txt"), "# h\r\n+Next_separator\r\n+B\r\n+Tools_separator\r\n+A\r\n").unwrap();
        let i = Instance::open(dir).unwrap();
        let p = i.profile(None).unwrap();
        (i, p)
    }

    fn outcome(name: &str, merged: bool) -> Outcome {
        Outcome {
            name: name.into(),
            mod_dir: Default::default(),
            installer: Installer::Quick,
            merged,
            replaced: false,
            backup: None,
            trashed: None,
            ini_tweaks: false,
            notes: Vec::new(),
        }
    }

    fn lines(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("profiles/p/modlist.txt")).unwrap().replace("\r\n", " ")
    }

    #[test]
    fn new_mod_on_top_enabled() {
        let d = tempfile::tempdir().unwrap();
        let (i, p) = instance(d.path());
        let placed = add_to_profile(&i, &p, &outcome("New", false), &Placement { enable: true, under: None }).unwrap();
        assert!(placed.enabled);
        assert_eq!(placed.priority, 4);
        assert!(lines(d.path()).contains("generated by Mod Organizer. +New +Next_separator"));
    }

    #[test]
    fn under_a_separator_goes_to_its_end() {
        let d = tempfile::tempdir().unwrap();
        let (i, p) = instance(d.path());
        assert_eq!(separators(&i, &p).unwrap(), ["Tools", "Next"]);
        let placed = add_to_profile(&i, &p, &outcome("New", false), &Placement { enable: false, under: Some("Tools".into()) }).unwrap();
        assert!(!placed.enabled);
        // A, Tools, B, New, Next: New after B, inside Tools
        assert_eq!(placed.priority, 3);
        assert!(lines(d.path()).ends_with("+Next_separator -New +B +Tools_separator +A "), "{}", lines(d.path()));
        let p2 = i.profile(None).unwrap();
        assert!(add_to_profile(&i, &p2, &outcome("A", false), &Placement { enable: true, under: Some("Nope".into()) }).is_ok(), "listed mods are not moved");
    }

    #[test]
    fn merged_mods_keep_their_place_and_state() {
        let d = tempfile::tempdir().unwrap();
        let (i, p) = instance(d.path());
        std::fs::write(d.path().join("profiles/p/modlist.txt"), "# h\r\n-New\r\n+A\r\n").unwrap();
        let p = i.profile(Some(&p.name)).unwrap();
        let placed = add_to_profile(&i, &p, &outcome("New", true), &Placement { enable: true, under: None }).unwrap();
        assert!(!placed.enabled);
    }

}
