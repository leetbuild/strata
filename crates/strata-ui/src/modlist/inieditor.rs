//! The ini editor (MO2's `tool_inieditor`, shown in uibase's `TextViewer`): the profile's
//! game inis as tabs, saved through the core (`Instance::save_ini`).

use strata_core::Instance;
use strata_core::inis::{IniFile, MAX_EDITABLE_SIZE, ReadOnly};

use crate::Result;

/// One tab of the editor, or a file that gets no tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IniTab {
    /// The game's name for it (`Skyrim.ini`): the tab title, and the name to save under.
    pub name: String,
    /// Where it is (the dialog shows it as the tab's tooltip); empty when unknown.
    pub path: String,
    /// The text, as MO2's `TextViewer::addFile` reads it (`QString(QByteArray)`: UTF-8),
    /// with `\r\n` as `\n` (saving writes `\r\n` again).
    pub text: String,
    pub read_only: bool,
    /// Why there is no tab ("too big", "missing"), or empty.
    pub skipped: String,
}

/// What the dialog shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IniEditorView {
    /// The description line over the tabs (`IniEditor::display`'s two texts).
    pub description: String,
    pub tabs: Vec<IniTab>,
}

/// The view for `profile` ("" = the selected one).
pub fn view(instance: &Instance, profile: &str) -> Result<IniEditorView> {
    let profile = instance.profile((!profile.is_empty()).then_some(profile))?;
    let description = if profile.local_settings {
        "Editing profile-specific INIs. Profile-specific game INI files are Enabled."
    } else {
        "Editing global INIs (in the prefix's 'My Games' folder). Profile-specific game INI files are Disabled."
    };
    let tabs = instance.ini_files(&profile).into_iter().map(tab).collect();
    Ok(IniEditorView { description: description.to_owned(), tabs })
}

fn tab(file: IniFile) -> IniTab {
    let path = file.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
    let skipped = match file.size {
        None => "missing",
        Some(size) if size >= MAX_EDITABLE_SIZE => "too big",
        Some(_) => "",
    };
    let text = match (&file.path, skipped.is_empty()) {
        (Some(p), true) => std::fs::read(p).map(|d| String::from_utf8_lossy(&d).replace("\r\n", "\n")).unwrap_or_default(),
        _ => String::new(),
    };
    IniTab { name: file.name, path, text, read_only: file.read_only, skipped: skipped.to_owned() }
}

/// Saves one tab. `read_only`: 0 refuse, 1 clear the flag, 2 write once (MO2's three
/// buttons). Returns whether the file changed.
pub fn save(instance: &Instance, profile: &str, name: &str, text: &str, read_only: u8) -> Result<bool> {
    let profile = instance.profile((!profile.is_empty()).then_some(profile))?;
    let choice = match read_only {
        1 => ReadOnly::Clear,
        2 => ReadOnly::Once,
        _ => ReadOnly::Refuse,
    };
    Ok(instance.save_ini(&profile, name, text, choice)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_instance::TestInstance;

    #[test]
    fn the_profiles_inis_become_tabs_and_save_with_crlf() {
        let t = TestInstance::new("+A\r\n");
        std::fs::write(t.path("profiles/Default/settings.ini"), "[General]\r\nLocalSettings=true\r\n").unwrap();
        std::fs::write(t.path("profiles/Default/Skyrim.ini"), "[General]\r\nsLanguage=ENGLISH\r\n").unwrap();
        std::fs::write(t.path("profiles/Default/SkyrimPrefs.ini"), vec![b' '; MAX_EDITABLE_SIZE as usize]).unwrap();
        let (instance, _) = t.open();
        let v = view(&instance, "").unwrap();
        assert!(v.description.contains("Enabled"));
        let skipped: Vec<_> = v.tabs.iter().map(|t| (t.name.as_str(), t.skipped.as_str())).collect();
        assert_eq!(skipped, [("Skyrim.ini", ""), ("SkyrimPrefs.ini", "too big"), ("SkyrimCustom.ini", "missing")]);
        assert_eq!(v.tabs[0].text, "[General]\nsLanguage=ENGLISH\n");

        assert!(save(&instance, "Default", "Skyrim.ini", "[General]\nsLanguage=GERMAN\n", 0).unwrap());
        assert_eq!(std::fs::read(t.path("profiles/Default/Skyrim.ini")).unwrap(), b"[General]\r\nsLanguage=GERMAN\r\n");
    }

    #[test]
    fn without_local_inis_or_a_prefix_nothing_is_editable() {
        let t = TestInstance::new("+A\r\n");
        let (instance, _) = t.open();
        let v = view(&instance, "").unwrap();
        assert!(v.description.contains("Disabled"));
        assert!(v.tabs.iter().all(|t| t.skipped == "missing" && t.path.is_empty()));
        assert!(save(&instance, "", "Skyrim.ini", "x", 0).is_err());
    }
}
