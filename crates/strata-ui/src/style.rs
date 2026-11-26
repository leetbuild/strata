//! Which `.qss` stylesheet the window uses and where it is found (`docs/DESIGN.md`, "Style").

use std::path::{Path, PathBuf};

use strata_ini::Document;

/// A sheet a desktop theme renders for Qt apps (`riki.qss`, the author's theme): the default
/// when the instance names no style and the sheet is in one of the search folders (it
/// replaces Leet change 8's pick from the desktop's light/dark setting).
pub const PREFERRED_STYLE: &str = "riki.qss";

/// The default when no `riki.qss` is found: MO2's dark sheet, bundled with Strata.
pub const FALLBACK_STYLE: &str = "vs15 Dark.qss";

/// The sheet for an instance that names none (and for a window with no instance):
/// [`PREFERRED_STYLE`] when [`find`] finds it in `dirs`, else [`FALLBACK_STYLE`]. The one
/// place the default is chosen.
pub fn default_style(dirs: &[PathBuf]) -> &'static str {
    if find(PREFERRED_STYLE, dirs).is_some() { PREFERRED_STYLE } else { FALLBACK_STYLE }
}

/// The folder of the running program (`QCoreApplication::applicationDirPath`), where the
/// bundled `stylesheets/` are.
pub fn program_dir() -> PathBuf {
    strata_core::dirs::own_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default()
}

/// The instance's style: `[Settings] style` of `ModOrganizer.ini` (`InterfaceSettings::styleName`),
/// a file name such as `Night Eyes.qss`, or a Qt style name such as `Fusion`. An empty or
/// missing value means "not set" (Leet change 8 dropped MO2's "None").
pub fn instance_style(ini: &Document) -> Option<String> {
    ini.string("Settings", "style").map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

/// The folders searched for a sheet, first match wins:
///
/// 1. `<instance>/stylesheets`: sheets that belong to one instance;
/// 2. `<data home>/strata/stylesheets` (`~/.local/share/strata/stylesheets`): the user's own
///    and rendered ones (a desktop theme);
/// 3. `<program folder>/stylesheets`, then `<program folder>/../share/strata/stylesheets`: the
///    sheets bundled with Strata (MO2's own), in a build folder or installed;
/// 4. `<data home>/fluorine/bin/stylesheets`: Fluorine's folder, read only. Themes made for
///    Fluorine render there, and instances name them, so they keep working in Strata.
///
/// MO2 itself only looks in its program folder (`MOApplication::setStyleFile`).
pub fn search_dirs(instance: Option<&Path>, program_dir: &Path, data_home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(instance) = instance {
        dirs.push(instance.join("stylesheets"));
    }
    if let Some(home) = data_home {
        dirs.push(home.join("strata/stylesheets"));
    }
    dirs.push(program_dir.join("stylesheets"));
    // an installed Strata: <prefix>/bin/strata, <prefix>/share/strata/stylesheets
    if let Some(prefix) = program_dir.parent() {
        dirs.push(prefix.join("share/strata/stylesheets"));
    }
    if let Some(home) = data_home {
        dirs.push(home.join("fluorine/bin/stylesheets"));
    }
    dirs
}

/// `$XDG_DATA_HOME`, or `~/.local/share`.
pub fn data_home() -> Option<PathBuf> {
    strata_core::dirs::data_home()
}

/// The sheet `name` (`Night Eyes.qss`, or `Night Eyes` without the extension) in the first of
/// `dirs` that has it. Symlinks are followed (desktop themes link their rendered sheets in);
/// the path returned is the one found, not the link's target, so `url(...)` in the sheet
/// resolves next to where it was found, as in MO2.
pub fn find(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let file = if name.to_lowercase().ends_with(".qss") { name.to_owned() } else { format!("{name}.qss") };
    if file.contains('/') {
        return None;
    }
    dirs.iter().map(|dir| dir.join(&file)).find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_riki_when_found_else_vs15_dark() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = search_dirs(None, &tmp.path().join("prog"), Some(&tmp.path().join("share")));
        for dir in &dirs {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(dirs[1].join("vs15 Dark.qss"), "a").unwrap();
        assert_eq!(default_style(&dirs), "vs15 Dark.qss", "no riki.qss anywhere");
        // where a desktop theme renders it today: Fluorine's folder (the last one searched)
        std::fs::write(dirs.last().unwrap().join("riki.qss"), "b").unwrap();
        assert_eq!(default_style(&dirs), "riki.qss");
    }

    #[test]
    fn the_instance_style_is_read_as_mo2_stores_it() {
        let ini = Document::parse(b"[Settings]\r\nstyle=Night Eyes.qss\r\n");
        assert_eq!(instance_style(&ini).as_deref(), Some("Night Eyes.qss"));
        assert_eq!(instance_style(&Document::parse(b"[Settings]\nstyle=\n")), None);
        assert_eq!(instance_style(&Document::parse(b"[General]\n")), None);
    }

    #[test]
    fn the_first_folder_with_the_sheet_wins_and_symlinks_are_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let instance = tmp.path().join("inst");
        let program = tmp.path().join("opt/bin");
        let home = tmp.path().join("share");
        let dirs = search_dirs(Some(&instance), &program, Some(&home));
        assert_eq!(dirs[0], instance.join("stylesheets"));
        assert_eq!(dirs[2], program.join("stylesheets"));
        assert_eq!(dirs[3], tmp.path().join("opt/share/strata/stylesheets"), "installed under a prefix");
        for dir in &dirs {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(dirs[2].join("vs15 Dark.qss"), "a").unwrap();
        std::fs::write(dirs[4].join("riki.qss"), "b").unwrap();
        assert_eq!(find("vs15 Dark", &dirs), Some(dirs[2].join("vs15 Dark.qss")));
        assert_eq!(find("riki.qss", &dirs), Some(dirs[4].join("riki.qss")));

        // a link in the user's folder, to a sheet rendered elsewhere, comes first
        let rendered = tmp.path().join("rendered.qss");
        std::fs::write(&rendered, "c").unwrap();
        std::os::unix::fs::symlink(&rendered, dirs[1].join("riki.qss")).unwrap();
        assert_eq!(find("riki.qss", &dirs), Some(dirs[1].join("riki.qss")));
        assert_eq!(find("missing", &dirs), None);
        assert_eq!(find("../rendered", &dirs), None);
    }
}
