//! What a mod holds (the Content column, `GamebryoModDataContent::getContentsFor` with
//! `SkyrimSEModDataContent`'s choices) and whether it looks like game data at all (the
//! "no valid game data" flag, `SkyrimSEModDataChecker::dataLooksValid`), from the mod's
//! folder. Read from Fluorine's port of the game plugins (`libs/game_bethesda`), which is
//! MO2's.

use std::path::Path;

/// `GamebryoModDataContent::EContent`, plus Leet change 3's "Root Files" right after the
/// game's ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Content {
    Plugin = 0,
    Optional = 1,
    Texture = 2,
    Mesh = 3,
    Bsa = 4,
    Interface = 5,
    Sound = 6,
    Script = 7,
    Skse = 8,
    SkseFiles = 9,
    // 10 = SkyProc patcher: disabled for Skyrim SE
    Mcm = 11,
    Ini = 12,
    FaceGen = 13,
    ModGroup = 14,
    RootFiles = 15,
}

impl Content {
    /// The icon (`getAllContents`); `None` for the contents MO2 only uses as filters.
    pub fn icon(self) -> Option<&'static str> {
        Some(match self {
            Content::Plugin => ":/MO/gui/content/plugin",
            Content::Interface => ":/MO/gui/content/interface",
            Content::Mesh => ":/MO/gui/content/mesh",
            Content::Bsa => ":/MO/gui/content/bsa",
            Content::Script => ":/MO/gui/content/script",
            Content::Skse => ":/MO/gui/content/skse",
            Content::Sound => ":/MO/gui/content/sound",
            Content::Texture => ":/MO/gui/content/texture",
            Content::Mcm => ":/MO/gui/content/menu",
            Content::Ini => ":/MO/gui/content/inifile",
            Content::FaceGen => ":/MO/gui/content/facegen",
            Content::ModGroup => ":/MO/gui/content/modgroup",
            Content::RootFiles => ":/MO/gui/content/rootfiles",
            Content::Optional | Content::SkseFiles => return None,
        })
    }

    /// The name MO2 gives it (tooltips, the filter panel).
    pub fn name(self) -> &'static str {
        match self {
            Content::Plugin => "Plugins (ESP/ESM/ESL)",
            Content::Optional => "Optional Plugins",
            Content::Interface => "Interface",
            Content::Mesh => "Meshes",
            Content::Bsa => "Bethesda Archive",
            Content::Script => "Scripts (Papyrus)",
            Content::Skse => "Script Extender Plugin",
            Content::SkseFiles => "Script Extender Files",
            Content::Sound => "Sound or Music",
            Content::Texture => "Textures",
            Content::Mcm => "MCM Configuration",
            Content::Ini => "INI Files",
            Content::FaceGen => "FaceGen Data",
            Content::ModGroup => "ModGroup Files",
            Content::RootFiles => "Root Files",
        }
    }

    pub const ALL: [Content; 15] = [
        Content::Plugin,
        Content::Optional,
        Content::Texture,
        Content::Mesh,
        Content::Bsa,
        Content::Interface,
        Content::Sound,
        Content::Script,
        Content::Skse,
        Content::SkseFiles,
        Content::Mcm,
        Content::Ini,
        Content::FaceGen,
        Content::ModGroup,
        Content::RootFiles,
    ];
}

/// `SkyrimSEModDataChecker::possibleFolderNames` (compared without case).
const DATA_FOLDERS: &[&str] = &[
    "fonts", "interface", "menus", "meshes", "music", "scripts", "shaders", "sound", "strings", "textures", "trees",
    "video", "facegen", "materials", "skse", "distantlod", "asi", "tools", "mcm", "distantland", "mits",
    "dllplugins", "calientetools", "netscriptframework", "shadersfx", "nemesis_engine", "platform", "grass",
    "lightplacer", "mainmenuwallpapers", "mainmenuvideo", "pbrmaterialobjects", "pbrnifpatcher", "pbrtexturesets",
];
/// `possibleFileExtensions`.
const DATA_EXTENSIONS: &[&str] = &["esp", "esm", "esl", "bsa", "modgroups", "ini"];

/// What one look at a mod folder tells.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    /// Sorted, no duplicates.
    pub contents: Vec<Content>,
    /// `dataLooksValid`, with Leet change 3: a non-empty `Root` folder makes a mod valid.
    pub valid: bool,
}

/// A folder's entries as MO2's mod file tree has them: `meta.ini` files left out
/// (`QDirFileTree`'s "ignores meta.ini").
fn entries(dir: &Path) -> Vec<(String, bool)> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.file_type().is_ok_and(|t| t.is_dir())))
                .filter(|(name, is_dir)| *is_dir || !name.eq_ignore_ascii_case("meta.ini"))
                .collect()
        })
        .unwrap_or_default()
}

/// A child folder of `dir` named `name` in any case.
fn child(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    entries(dir).into_iter().find(|(n, is_dir)| *is_dir && n.eq_ignore_ascii_case(name)).map(|(n, _)| dir.join(n))
}

fn find_dir(dir: &Path, path: &str) -> Option<std::path::PathBuf> {
    path.split('/').try_fold(dir.to_path_buf(), |at, part| child(&at, part))
}

/// Looks at a mod folder: its top level, `SKSE/Plugins`, the FaceGen folders and `Root`.
pub fn scan(dir: &Path) -> Scan {
    let mut contents = Vec::new();
    let mut valid = false;
    for (name, is_dir) in entries(dir) {
        let lower = name.to_lowercase();
        if is_dir {
            valid |= DATA_FOLDERS.contains(&lower.as_str());
            match lower.as_str() {
                "textures" | "icons" | "bookart" => contents.push(Content::Texture),
                "meshes" => contents.push(Content::Mesh),
                "interface" | "menus" => contents.push(Content::Interface),
                "music" | "sound" => contents.push(Content::Sound),
                "scripts" => contents.push(Content::Script),
                "mcm" => contents.push(Content::Mcm),
                "optional" if !entries(&dir.join(&name)).is_empty() => contents.push(Content::Optional),
                "root" if !entries(&dir.join(&name)).is_empty() => {
                    contents.push(Content::RootFiles);
                    valid = true;
                }
                _ => {}
            }
        } else {
            let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
            valid |= DATA_EXTENSIONS.contains(&ext);
            match ext {
                "esp" | "esm" | "esl" => contents.push(Content::Plugin),
                "bsa" | "ba2" => contents.push(Content::Bsa),
                "ini" => contents.push(Content::Ini),
                "modgroups" => contents.push(Content::ModGroup),
                _ => {}
            }
        }
    }
    if find_dir(dir, "meshes/actors/character/facegendata").is_some()
        || find_dir(dir, "textures/actors/character/facegendata").is_some()
    {
        contents.push(Content::FaceGen);
    }
    // the script extender's plugin path (SKSE/Plugins)
    if let Some(plugins) = find_dir(dir, "SKSE/Plugins") {
        contents.push(Content::SkseFiles);
        if entries(&plugins).iter().any(|(n, is_dir)| !is_dir && n.to_lowercase().ends_with(".dll")) {
            contents.push(Content::Skse);
        }
    }
    contents.sort();
    contents.dedup();
    Scan { contents, valid }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, rel: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"").unwrap();
    }

    #[test]
    fn contents_and_validity_as_the_game_plugin_sees_them() {
        let tmp = tempfile::tempdir().unwrap();
        let m = tmp.path();
        touch(m, "Mod.ESP");
        touch(m, "meta.ini");
        touch(m, "Textures/a.dds");
        touch(m, "SKSE/Plugins/x.dll");
        touch(m, "meshes/actors/character/FaceGenData/f.nif");
        touch(m, "Optional/old.esp");
        touch(m, "settings.ini");
        let s = scan(m);
        use Content::*;
        assert_eq!(s.contents, [Plugin, Optional, Texture, Mesh, Skse, SkseFiles, Ini, FaceGen]);
        assert!(s.valid);

        let docs = tempfile::tempdir().unwrap();
        touch(docs.path(), "readme.txt");
        touch(docs.path(), "meta.ini");
        let s = scan(docs.path());
        assert!(!s.valid);
        assert!(s.contents.is_empty(), "meta.ini is not an INI file content");

        // Leet change 3: a Root folder alone is valid game data
        let root = tempfile::tempdir().unwrap();
        touch(root.path(), "Root/d3d11.dll");
        touch(root.path(), "Optional/.keep");
        let s = scan(root.path());
        assert!(s.valid);
        assert_eq!(s.contents, [Optional, RootFiles]);
    }
}
