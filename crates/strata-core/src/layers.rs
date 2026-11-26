//! From a profile to the overlay's layer list (`docs/ARCHITECTURE.md`, "The layer model").

use strata_game::{Layer, LayerKind, Mapping};

use crate::{Instance, ModKind, Profile, Result};

/// The layers of `profile`, lowest priority first: the game folder, every enabled regular
/// mod in priority order, then overwrite.
pub fn resolve_layers(instance: &Instance, profile: &Profile) -> Result<Vec<Layer>> {
    let rules = instance.game.mod_mappings();
    let mut layers = vec![Layer {
        id: "<game>".into(),
        kind: LayerKind::Game,
        source: instance.game_path.clone(),
        rules: vec![Mapping::new("", "")],
    }];
    for entry in instance.mod_list(profile)? {
        if let (true, ModKind::Regular, Some(path)) = (entry.enabled, entry.kind, entry.path) {
            layers.push(Layer { id: entry.name, kind: LayerKind::Mod, source: path, rules: rules.clone() });
        }
    }
    layers.push(Layer {
        id: "<overwrite>".into(),
        kind: LayerKind::Overwrite,
        source: instance.paths.overwrite.clone(),
        rules,
    });
    Ok(layers)
}

/// A profile mount: a folder inside the prefix shown with some of the profile's files laid
/// over it (`docs/DESIGN.md`, "Profile files in the prefix are overlaid, not copied").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileMount {
    /// `appdata` or `documents`.
    pub id: &'static str,
    /// The folder inside the prefix. It is created when missing.
    pub mountpoint: std::path::PathBuf,
    pub layers: Vec<Layer>,
}

/// The profile mounts for running the game in the prefix whose `pfx` folder is `pfx`.
///
/// - `AppData/Local/<game>`: the profile's `plugins.txt` and `loadorder.txt`.
/// - `Documents/My Games/<game>`: the profile's inis (when it has local settings) and its
///   `saves/` in place of the prefix's `Saves` (when it has local saves). Left out when the
///   profile has neither.
///
/// Everything else in those folders is the prefix's own and is written straight through.
pub fn profile_mounts(instance: &Instance, profile: &Profile, pfx: &std::path::Path) -> Vec<ProfileMount> {
    let game = &instance.game;
    // Proton's user is always "steamuser".
    let user = pfx.join("drive_c/users/steamuser");
    let under = |source: &std::path::Path, exclude: Vec<String>| Layer {
        id: "<prefix>".into(),
        kind: LayerKind::Passthrough,
        source: source.to_owned(),
        rules: vec![Mapping { exclude, ..Mapping::new("", "") }],
    };
    let over = |rules| Layer { id: "<profile>".into(), kind: LayerKind::Overwrite, source: profile.dir.clone(), rules };
    let names = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();

    let appdata = user.join("AppData/Local").join(game.appdata_dir());
    let mut mounts = vec![ProfileMount {
        id: "appdata",
        layers: vec![
            under(&appdata, Vec::new()),
            over(vec![Mapping { include: names(&["plugins.txt", "loadorder.txt"]), ..Mapping::new("", "") }]),
        ],
        mountpoint: appdata,
    }];

    if profile.local_settings || profile.local_saves {
        let documents = user.join("Documents/My Games").join(game.documents_dir());
        let mut rules = Vec::new();
        if profile.local_settings {
            rules.push(Mapping { include: names(game.ini_files()), ..Mapping::new("", "") });
        }
        if profile.local_saves {
            rules.push(Mapping::new("saves", "Saves"));
        }
        let hidden = if profile.local_saves { names(&["Saves"]) } else { Vec::new() };
        mounts.push(ProfileMount { id: "documents", layers: vec![under(&documents, hidden), over(rules)], mountpoint: documents });
    }
    mounts
}
