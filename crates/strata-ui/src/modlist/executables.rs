//! The executables dialog (`EditExecutablesDialog`, `editexecutablesdialog.ui`).

use std::path::Path;

use strata_core::executables::non_conflicting_title;
use strata_core::profiles::{profile_setting, store_profile_settings};
use strata_core::{Executable, Instance, Profile, winepath};
use strata_ini::Value;

use crate::Result;

/// One executable as the dialog edits it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutableRow {
    pub exe: Executable,
    /// "Create files in mod instead of overwrite": the mod, or empty (the profile's
    /// `[custom_overwrites] <title>`).
    pub output_mod: String,
}

/// The list with each executable's output mod from `profile`.
pub fn rows(instance: &Instance, profile: &Profile) -> Result<Vec<ExecutableRow>> {
    let mut rows = Vec::new();
    for exe in instance.executables() {
        let output_mod = profile_setting(profile, "custom_overwrites", &exe.title)?.map(|v| v.to_text()).unwrap_or_default();
        rows.push(ExecutableRow { exe, output_mod });
    }
    Ok(rows)
}

/// What the dialog stores for a binary picked in the file dialog: the Windows form
/// (`Z:\...`), as the instance's other entries are and as the programs see it in Wine.
pub fn binary_for_storage(host: &Path) -> String {
    winepath::to_wine(host)
}

/// `EditExecutablesDialog::setBinary`'s title: the file name without its extension, made
/// unique among `list` (only when the current title is still a "New Executable").
pub fn title_for_binary(list: &[Executable], current_title: &str, binary: &str) -> Option<String> {
    if !current_title.to_lowercase().starts_with("new executable") {
        return None;
    }
    let name = binary.rsplit(['/', '\\']).next().unwrap_or(binary);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    non_conflicting_title(list, stem)
}

/// `EditExecutablesDialog::commitChanges`: checks the titles (non-empty, unique) and the
/// output mods (`checkOutputMods`: the mod must exist), rewrites binaries in a mod's
/// `Root/` to the game folder (Leet backlog), stores the list, and replaces the profile's
/// `custom_overwrites` of `original` titles with the new ones. Returns whether anything
/// was written.
pub fn commit(instance: &mut Instance, profile: &Profile, original: &[String], rows: &[ExecutableRow]) -> Result<bool> {
    let mods = instance.mod_list(profile)?;
    for (i, row) in rows.iter().enumerate() {
        let title = row.exe.title.trim();
        if title.is_empty() {
            return Err(refused(format!("executable {} has no title", i + 1)));
        }
        if rows[..i].iter().any(|r| r.exe.title.trim() == title) {
            return Err(refused(format!("two executables are called \"{title}\"")));
        }
        if !row.output_mod.is_empty() && !mods.iter().any(|m| m.name == row.output_mod) {
            return Err(refused(format!("the output mod \"{}\" of \"{title}\" does not exist", row.output_mod)));
        }
    }
    let list: Vec<Executable> = rows
        .iter()
        .map(|r| {
            let mut e = r.exe.clone();
            e.title = e.title.trim().to_owned();
            if let Some(fixed) = instance.root_path_in_game(&e.binary) {
                e.binary = fixed;
            }
            if let Some(fixed) = instance.root_path_in_game(&e.working_directory) {
                e.working_directory = fixed;
            }
            e
        })
        .collect();
    let mut changed = instance.store_executables(&list)?;

    let mut values: Vec<(String, Option<Value>)> = original.iter().map(|t| (t.clone(), None)).collect();
    for (row, exe) in rows.iter().zip(&list) {
        if !row.output_mod.is_empty() {
            values.retain(|(t, _)| *t != exe.title);
            values.push((exe.title.clone(), Some(Value::String(row.output_mod.clone()))));
        }
    }
    let values: Vec<(&str, &str, Option<Value>)> = values.iter().map(|(t, v)| ("custom_overwrites", t.as_str(), v.clone())).collect();
    changed |= store_profile_settings(profile, &values)?;
    Ok(changed)
}

fn refused(what: String) -> crate::Error {
    crate::Error::Core(strata_core::Error::Refused { what })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(root: &Path) -> Instance {
        let w = |p: &str, t: &str| {
            std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
            std::fs::write(root.join(p), t).unwrap();
        };
        w("ModOrganizer.ini", &format!("[General]\ngameName=Skyrim Special Edition\nselected_profile=@ByteArray(P)\ngamePath={}\n", root.join("game").display()));
        w("mods/Output/meta.ini", "");
        w("profiles/P/modlist.txt", "+Output\r\n");
        w("profiles/P/settings.ini", "[General]\nLocalSaves=false\n\n[custom_overwrites]\nOld=Output\n");
        Instance::open(root).unwrap()
    }

    #[test]
    fn commit_checks_stores_and_moves_the_output_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let mut inst = instance(tmp.path());
        let profile = inst.profile(None).unwrap();
        let exe = |title: &str, binary: &str| Executable { title: title.into(), binary: binary.into(), ..Default::default() };
        let rows = vec![
            ExecutableRow { exe: exe(" xEdit ", "/t/SSEEdit.exe"), output_mod: "Output".into() },
            ExecutableRow { exe: exe("SKSE", &format!("{}/mods/Output/Root/skse64_loader.exe", tmp.path().display())), output_mod: String::new() },
        ];
        assert!(commit(&mut inst, &profile, &["Old".into()], &rows).unwrap());
        let stored = inst.executables();
        assert_eq!(stored[0].title, "xEdit");
        assert_eq!(stored[1].binary, format!("{}/game/skse64_loader.exe", tmp.path().display()), "Root binary runs from the game folder");
        let settings = std::fs::read_to_string(tmp.path().join("profiles/P/settings.ini")).unwrap();
        assert!(settings.contains("[custom_overwrites]\nxEdit=Output\n") && !settings.contains("Old="), "{settings}");
        let back = super::rows(&inst, &profile).unwrap();
        assert_eq!(back[0].output_mod, "Output");

        let dup = vec![rows[0].clone(), rows[0].clone()];
        assert!(commit(&mut inst, &profile, &[], &dup).unwrap_err().to_string().contains("two executables"));
        let missing = vec![ExecutableRow { exe: exe("A", "/a"), output_mod: "Nope".into() }];
        assert!(commit(&mut inst, &profile, &[], &missing).unwrap_err().to_string().contains("Nope"));
    }

    #[test]
    fn a_new_executable_takes_its_binarys_name() {
        let list = vec![Executable { title: "SSEEdit".into(), ..Default::default() }];
        assert_eq!(title_for_binary(&list, "New Executable (1)", "Z:\\t\\SSEEdit.exe").as_deref(), Some("SSEEdit (1)"));
        assert_eq!(title_for_binary(&list, "Mine", "/t/x.exe"), None);
        assert_eq!(binary_for_storage(Path::new("/home/x/a b.exe")), "Z:\\home\\x\\a b.exe");
    }
}
