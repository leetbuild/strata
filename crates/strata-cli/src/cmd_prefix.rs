//! The `prefix` area (M5): `strata prefix setup`, `strata proton list`, `strata create-portable`.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use strata_core::Instance;
use strata_launch::setup::{self, Outcome};
use strata_launch::{Proton, Steam};

use crate::Global;
use crate::commands::print_json;

/// `strata prefix <action>`.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum PrefixAction {
    /// Make the instance's prefix fit for the game and its tools: create it with the
    /// instance's Proton, then VC++ 2015-2022, d3dcompiler_47, d3dx9/10/11, .NET 8 and 9
    /// Desktop. Each step checks first; a working prefix is left as it is.
    Setup {
        /// Only report what each step finds; change nothing.
        #[arg(long)]
        check: bool,
    },
}

/// `strata proton <action>`.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum ProtonAction {
    /// The Protons Steam knows: compatibilitytools.d and the Steam libraries.
    List,
}

/// `strata create-portable`.
#[derive(Debug, Args, PartialEq, Eq)]
pub(crate) struct CreatePortable {
    /// The new instance folder (must not exist, or be empty).
    pub folder: PathBuf,
    /// The game folder (the one with SkyrimSE.exe).
    #[arg(long, value_name = "PATH")]
    pub game: PathBuf,
    /// The Proton folder (the one with the `proton` script).
    #[arg(long, value_name = "PATH")]
    pub proton: PathBuf,
    /// The prefix folder (holds pfx/); default: <folder>/Prefix. Not created here: run
    /// `strata prefix setup` next.
    #[arg(long = "wine-prefix", value_name = "PATH")]
    pub wine_prefix: Option<PathBuf>,
    /// The game's name as MO2 writes it (`Skyrim Special Edition`, `Skyrim`).
    #[arg(long, default_value = "Skyrim Special Edition")]
    pub game_name: String,
}

/// Opens the instance without a profile (setup does not need one).
fn open_instance(global: &Global) -> Result<Instance> {
    let root = match &global.instance {
        Some(path) => path.clone(),
        None => std::env::current_dir().context("cannot read the current folder")?,
    };
    let root = root.canonicalize().with_context(|| format!("cannot find {}", root.display()))?;
    let mut instance = Instance::open(&root)?;
    if let Some(prefix) = &global.prefix {
        // A prefix that does not exist yet is fine here: setup creates it.
        std::fs::create_dir_all(prefix).with_context(|| format!("cannot create {}", prefix.display()))?;
        instance.proton.prefix = Some(prefix.canonicalize()?);
    }
    Ok(instance)
}

pub(crate) fn prefix(global: &Global, action: &PrefixAction) -> Result<()> {
    let PrefixAction::Setup { check } = action;
    let instance = open_instance(global)?;
    let prefix = instance
        .proton
        .prefix
        .clone()
        .context("this instance names no prefix ([fluorine] prefix_path in ModOrganizer.ini). Give one with --prefix.")?;
    let proton_dir =
        instance.proton.proton.clone().context("this instance names no Proton ([fluorine] proton_path in ModOrganizer.ini).")?;
    let proton = Proton::at(&proton_dir)?;
    let steam = Steam::detect()?;

    // Every run leaves its log in the instance (docs/DESIGN.md, Proton rule 8).
    let logs = instance.root.join("logs");
    std::fs::create_dir_all(&logs)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let log_path = logs.join(format!("strata-prefix-setup-{stamp}.log"));
    let mut log = std::fs::File::create(&log_path).with_context(|| format!("cannot write {}", log_path.display()))?;
    writeln!(log, "# strata prefix setup{}\n# prefix {}\n# proton {}", if *check { " --check" } else { "" }, prefix.display(), proton_dir.display())?;

    let steps = setup::skyrim_steps();
    let mut ctx = setup::Context {
        proton: &proton,
        steam: &steam,
        prefix: &prefix,
        app_id: instance.game.steam_app_id(),
        cache: setup::cache_dir(),
        log: &mut log,
    };
    let quiet = global.json;
    let started = std::time::Instant::now();
    let result = setup::run(&steps, &mut ctx, *check, &mut |step, state| {
        if !quiet {
            eprintln!("{:<15} {state}", step.id());
        }
    });
    let report = match result {
        Ok(report) => report,
        Err(e) => bail!("{e}\n(log: {})", log_path.display()),
    };
    if global.json {
        let steps: Vec<_> = report
            .iter()
            .map(|(id, outcome)| {
                let (state, missing) = match outcome {
                    Outcome::AlreadyDone => ("already done", None),
                    Outcome::Done => ("done", None),
                    Outcome::Missing(m) => ("missing", Some(m.as_str())),
                };
                serde_json::json!({ "step": id, "state": state, "missing": missing })
            })
            .collect();
        return print_json(&serde_json::json!({ "prefix": prefix, "log": log_path, "steps": steps }));
    }
    for (id, outcome) in &report {
        if let Outcome::Missing(m) = outcome {
            println!("{id}: missing {m}");
        }
    }
    let changed = report.iter().filter(|(_, o)| *o == Outcome::Done).count();
    println!(
        "{} in {} s ({changed} steps ran; log: {})",
        if *check { "checked" } else { "prefix ready" },
        started.elapsed().as_secs(),
        log_path.display()
    );
    Ok(())
}

pub(crate) fn proton(global: &Global, action: &ProtonAction) -> Result<()> {
    let ProtonAction::List = action;
    let steam = Steam::detect()?;
    let found = strata_launch::steam::find_protons(&steam);
    if global.json {
        let found: Vec<_> = found
            .iter()
            .map(|p| serde_json::json!({ "name": p.name, "path": p.path, "runtime": p.runtime, "runtime_installed": p.runtime_installed }))
            .collect();
        return print_json(&serde_json::json!({ "protons": found }));
    }
    if found.is_empty() {
        println!("no Proton found in {}", steam.root.display());
    }
    for p in found {
        let runtime = match (p.runtime, p.runtime_installed) {
            (None, _) => "no runtime".to_owned(),
            (Some(id), true) => format!("runtime {id}"),
            (Some(id), false) => format!("runtime {id} NOT INSTALLED (steam steam://install/{id})"),
        };
        println!("{:<28} {runtime:<20} {}", p.name, p.path.display());
    }
    Ok(())
}

pub(crate) fn create_portable(args: &CreatePortable) -> Result<()> {
    let folder = strata_ui::instances::create(&strata_ui::instances::NewInstance {
        folder: args.folder.clone(),
        game_name: args.game_name.clone(),
        game_dir: args.game.clone(),
        proton: args.proton.clone(),
        prefix: args.wine_prefix.clone(),
        local_inis: true,
        local_saves: true,
    })?;
    println!("instance created: {}\n  next: strata prefix setup -i {:?}", folder.display(), folder);
    Ok(())
}

/// What Fluorine writes into `dxvk.conf` for ENB (`protonlauncher.cpp`): ENB under DXVK
/// misbehaves with the graphics pipeline library.
pub(crate) const ENB_DXVK_CONF: &str = "dxvk.enableGraphicsPipelineLibrary = False\n";

/// `docs/DESIGN.md`, Proton rule 6: when the merged game folder has ENB (`d3d11.dll` with
/// `enbseries.ini` or `enblocal.ini` next to the exe) and no `dxvk.conf`, writes one into
/// overwrite's `Root/`, so it shows next to the game's exe in the overlay and the real folder
/// stays untouched. A `dxvk.conf` from any layer, or `DXVK_CONFIG_FILE` in the environment,
/// wins. Returns the file written.
pub(crate) fn ensure_enb_dxvk_conf(instance: &Instance, profile: &strata_core::Profile) -> Result<Option<PathBuf>> {
    if std::env::var_os("DXVK_CONFIG_FILE").is_some_and(|v| !v.is_empty()) {
        return Ok(None);
    }
    let tree = strata_ui::profile_tree::merged_tree(instance, profile)?;
    let has = |name: &str| tree.resolve(name).and_then(|id| tree.node(id)).is_some_and(|n| !n.is_dir());
    let enb = has("d3d11.dll") && (has("enbseries.ini") || has("enblocal.ini"));
    if !enb || has("dxvk.conf") {
        return Ok(None);
    }
    let path = instance.paths.overwrite.join("Root/dxvk.conf");
    std::fs::create_dir_all(path.parent().unwrap_or(&instance.paths.overwrite))?;
    std::fs::write(&path, ENB_DXVK_CONF).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(Some(path))
}

/// `writeArchiveList`: the list joined with ", " in the first key. With two keys
/// (`SkyrimDataArchives`) a list over 255 characters is split at the last comma before
/// character 256 and the rest goes to the second key (which MO2 otherwise leaves as it was);
/// with one (`OblivionDataArchives`, `SArchiveList`) it all goes there.
fn write_archive_list(data: &[u8], list: &[String], keys: &[&str]) -> Vec<u8> {
    use strata_game::invalidation::ini_set;
    let joined = list.join(", ");
    let (first_key, second_key) = (keys.first().copied().unwrap_or("SArchiveList"), keys.get(1).copied());
    if let Some(second_key) = second_key.filter(|_| joined.len() > 255) {
        // QString::lastIndexOf(",", 256)
        let split = joined.char_indices().rev().find(|(i, c)| *i <= 256 && *c == ',').map_or(joined.len(), |(i, _)| i);
        let first = ini_set(data, "Archive", first_key, &joined[..split]);
        ini_set(&first, "Archive", second_key, joined.get(split + 2..).unwrap_or(""))
    } else {
        ini_set(data, "Archive", first_key, &joined)
    }
}

/// What [`prepare_invalidation`] changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct InvalidationChanges {
    pub ini: Option<PathBuf>,
    pub dummy: Option<PathBuf>,
}

/// Archive invalidation as MO2 prepares it when a profile is used
/// (`GamebryoBSAInvalidation::prepareProfile`, for games that have it: Skyrim LE). With the
/// profile's `AutomaticArchiveInvalidation` on (default: `[Settings]
/// profile_archive_invalidation`, true): `[Archive] bInvalidateOlderFiles=1`, the dummy BSA
/// first in `SResourceArchiveList`, `SInvalidationFile=`, and the dummy BSA in Data. Off:
/// the dummy out of the list and `SInvalidationFile=ArchiveInvalidation.txt`.
///
/// One difference on purpose: MO2 writes (or deletes) the dummy BSA in the real game
/// folder; Strata puts it into overwrite (the overlay shows it in Data) and never touches
/// the game folder. The ini is the profile's (local settings) or the prefix's.
pub(crate) fn prepare_invalidation(instance: &Instance, profile: &strata_core::Profile) -> Result<InvalidationChanges> {
    use strata_game::invalidation::{dummy_bsa, ini_get, ini_set};
    let mut changes = InvalidationChanges::default();
    let Some(inv) = instance.game.invalidation() else { return Ok(changes) };
    let settings = std::fs::read(profile.dir.join("settings.ini")).map(|d| strata_ini::Document::parse(&d)).unwrap_or_default();
    let default = instance.ini.value("Settings", "profile_archive_invalidation").is_none_or(|v| v.to_bool());
    let active = settings.value("", "AutomaticArchiveInvalidation").map_or(default, |v| v.to_bool());

    let ini_dir = if profile.local_settings {
        profile.dir.clone()
    } else {
        match &instance.proton.prefix {
            Some(p) => p.join("pfx/drive_c/users/steamuser/Documents/My Games").join(instance.game.documents_dir()),
            None => return Ok(changes),
        }
    };
    let ini_path = std::fs::read_dir(&ini_dir)
        .ok()
        .and_then(|entries| {
            entries.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
                p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(inv.ini))
            })
        })
        .unwrap_or_else(|| ini_dir.join(inv.ini));
    let original = std::fs::read(&ini_path).unwrap_or_default();
    let mut ini = original.clone();

    // GamebryoDataArchives::archives: both keys, split at commas, trimmed.
    let list_of = |data: &[u8]| -> Vec<String> {
        inv.archive_keys
            .iter()
            .filter_map(|k| ini_get(data, "Archive", k))
            .filter(|v| !v.is_empty())
            .flat_map(|v| v.split(',').map(|s| s.trim().to_owned()).collect::<Vec<_>>())
            .collect()
    };
    let write_list = |data: &[u8], list: &[String]| write_archive_list(data, list, inv.archive_keys);

    if ini_get(&ini, "Archive", "bInvalidateOlderFiles").and_then(|v| v.parse::<i64>().ok()) != Some(1) {
        ini = ini_set(&ini, "Archive", "bInvalidateOlderFiles", "1");
    }
    let mut list = list_of(&ini);
    let listed = list.iter().any(|a| a.eq_ignore_ascii_case(inv.bsa_name));
    if active {
        if !listed {
            list.insert(0, inv.bsa_name.to_owned());
            ini = write_list(&ini, &list);
        }
        if ini_get(&ini, "Archive", "SInvalidationFile").as_deref() != Some("") {
            ini = ini_set(&ini, "Archive", "SInvalidationFile", "");
        }
        let tree = strata_ui::profile_tree::merged_tree(instance, profile)?;
        let data = instance.game.data_dir();
        if tree.resolve(&format!("{data}/{}", inv.bsa_name)).is_none() {
            let path = instance.paths.overwrite.join(inv.bsa_name);
            std::fs::create_dir_all(&instance.paths.overwrite)?;
            std::fs::write(&path, dummy_bsa(inv.bsa_version)).with_context(|| format!("cannot write {}", path.display()))?;
            changes.dummy = Some(path);
        }
    } else {
        if listed {
            list.retain(|a| !a.eq_ignore_ascii_case(inv.bsa_name));
            ini = write_list(&ini, &list);
        }
        if ini_get(&ini, "Archive", "SInvalidationFile").as_deref() != Some("ArchiveInvalidation.txt") {
            ini = ini_set(&ini, "Archive", "SInvalidationFile", "ArchiveInvalidation.txt");
        }
    }
    if ini != original {
        std::fs::create_dir_all(&ini_dir)?;
        strata_core::write::write_if_changed(&ini_path, &ini).with_context(|| format!("cannot write {}", ini_path.display()))?;
        changes.ini = Some(ini_path);
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_archive_invalidation_is_prepared_in_the_profile_ini_and_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("root");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        std::fs::write(game.join("TESV.exe"), "").unwrap();
        let proton = tmp.path().join("P");
        std::fs::create_dir_all(&proton).unwrap();
        std::fs::write(proton.join("proton"), "").unwrap();
        let folder = tmp.path().join("LE");
        create_portable(&CreatePortable { folder: folder.clone(), game: game.clone(), proton, wine_prefix: None, game_name: "Skyrim".into() })
            .unwrap();
        let ini = folder.join("profiles/Default/skyrim.ini");
        std::fs::write(&ini, "[Archive]\r\nSResourceArchiveList=Skyrim - Misc.bsa, Skyrim - Shaders.bsa\r\nSInvalidationFile=ArchiveInvalidation.txt\r\n").unwrap();
        let open = || {
            let i = Instance::open(&folder).unwrap();
            let p = i.profile(None).unwrap();
            (i, p)
        };
        let (i, p) = open();
        assert_eq!(i.game.name(), "Skyrim");
        let changes = prepare_invalidation(&i, &p).unwrap();
        assert_eq!(changes.ini.as_deref(), Some(ini.as_path()), "the existing file, any case");
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(text.contains("SResourceArchiveList=Skyrim - Invalidation.bsa, Skyrim - Misc.bsa, Skyrim - Shaders.bsa\r\n"), "{text}");
        assert!(text.contains("SInvalidationFile=\r\n") && text.contains("bInvalidateOlderFiles=1\r\n"), "{text}");
        let dummy = changes.dummy.unwrap();
        assert!(dummy.ends_with("overwrite/Skyrim - Invalidation.bsa"));
        assert_eq!(std::fs::read(&dummy).unwrap(), strata_game::invalidation::dummy_bsa(0x68));
        assert!(!game.join("Data/Skyrim - Invalidation.bsa").exists(), "the game folder is not touched");
        assert_eq!(prepare_invalidation(&i, &p).unwrap(), InvalidationChanges::default(), "a second time changes nothing");

        // Off in the profile: the dummy leaves the list.
        std::fs::write(folder.join("profiles/Default/settings.ini"), "[General]\r\nLocalSettings=true\r\nAutomaticArchiveInvalidation=false\r\n").unwrap();
        let (i, p) = open();
        prepare_invalidation(&i, &p).unwrap();
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(text.contains("SResourceArchiveList=Skyrim - Misc.bsa, Skyrim - Shaders.bsa\r\n"), "{text}");
        assert!(text.contains("SInvalidationFile=ArchiveInvalidation.txt\r\n"), "{text}");
    }


    #[test]
    fn a_long_archive_list_is_split_as_mo2_splits_it() {
        use strata_game::invalidation::ini_get;
        let list: Vec<String> = (0..30).map(|i| format!("Archive Number {i:02}.bsa")).collect();
        let keys = ["SResourceArchiveList", "SResourceArchiveList2"];
        let out = write_archive_list(b"[Archive]\r\n", &list, &keys);
        let first = ini_get(&out, "Archive", "SResourceArchiveList").unwrap();
        let second = ini_get(&out, "Archive", "SResourceArchiveList2").unwrap();
        assert!(first.len() <= 256, "{}", first.len());
        assert_eq!(format!("{first}, {second}"), list.join(", "));
        let short = write_archive_list(b"", &list[..2], &keys);
        assert_eq!(ini_get(&short, "Archive", "SResourceArchiveList2"), None);
        // Oblivion: one key, never split.
        let one = write_archive_list(b"", &list, &["SArchiveList"]);
        assert_eq!(ini_get(&one, "Archive", "SArchiveList").unwrap(), list.join(", "));
    }


    #[test]
    fn the_dummy_bsas_read_back_through_strata_archive() {
        // MO2's dummy archive for Oblivion (v103) and Skyrim LE (v104), read by the BSA reader.
        let tmp = tempfile::tempdir().unwrap();
        for version in [0x67u32, 0x68] {
            let path = tmp.path().join(format!("dummy-{version}.bsa"));
            std::fs::write(&path, strata_game::invalidation::dummy_bsa(version)).unwrap();
            let archive = strata_archive::GameArchive::open(&path).unwrap();
            let names: Vec<&str> = archive.files().iter().map(|f| f.path.as_str()).collect();
            assert_eq!(names, ["dummy.dds"], "version {version:#x}");
        }
    }


    #[test]
    fn an_oblivion_instance_opens_and_lists_its_plugins() {
        // Generated data only: no Oblivion game folder exists on this PC.
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("Oblivion");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        std::fs::write(game.join("Oblivion.exe"), "").unwrap();
        std::fs::write(game.join("obse_loader.exe"), "").unwrap();
        // Oblivion-style headers: a 20-byte record header, then HEDR.
        let plugin = |flags: u32, masters: &[&str]| {
            let mut fields = b"HEDR\x0c\x00".to_vec();
            fields.extend_from_slice(&1.0f32.to_le_bytes());
            fields.extend_from_slice(&1u32.to_le_bytes());
            fields.extend_from_slice(&0u32.to_le_bytes());
            for m in masters {
                fields.extend_from_slice(b"MAST");
                fields.extend_from_slice(&(m.len() as u16 + 1).to_le_bytes());
                fields.extend_from_slice(m.as_bytes());
                fields.push(0);
            }
            let mut out = b"TES4".to_vec();
            out.extend_from_slice(&(fields.len() as u32).to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&[0; 8]);
            out.extend_from_slice(&fields);
            out
        };
        std::fs::write(game.join("Data/Oblivion.esm"), plugin(1, &[])).unwrap();
        let proton = tmp.path().join("P");
        std::fs::create_dir_all(&proton).unwrap();
        std::fs::write(proton.join("proton"), "").unwrap();
        let folder = tmp.path().join("OB");
        create_portable(&CreatePortable { folder: folder.clone(), game: game.clone(), proton, wine_prefix: None, game_name: "Oblivion".into() })
            .unwrap();
        std::fs::create_dir_all(folder.join("mods/M")).unwrap();
        std::fs::write(folder.join("mods/M/Knights.esp"), plugin(0, &["Oblivion.esm"])).unwrap();
        std::fs::write(folder.join("mods/M/Off.esp"), plugin(0, &[])).unwrap();
        let h = "# This file was automatically generated by Mod Organizer.\r\n";
        std::fs::write(folder.join("profiles/Default/modlist.txt"), format!("{h}+M\r\n")).unwrap();
        std::fs::write(folder.join("profiles/Default/plugins.txt"), format!("{h}Oblivion.esm\r\nKnights.esp\r\n")).unwrap();
        std::fs::write(folder.join("profiles/Default/loadorder.txt"), format!("{h}Oblivion.esm\r\nOff.esp\r\nKnights.esp\r\n")).unwrap();

        let instance = Instance::open(&folder).unwrap();
        assert_eq!(instance.game.name(), "Oblivion");
        assert_eq!(instance.executables().iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["Oblivion"]);
        let profile = instance.profile(None).unwrap();
        let tree = strata_ui::profile_tree::merged_tree(&instance, &profile).unwrap();
        let available = strata_ui::profile_tree::available_plugins(&instance, &tree);
        let list = instance.plugin_list(&profile, available).unwrap();
        let order: Vec<(String, bool)> = list.plugins().map(|p| (p.name.clone(), p.enabled)).collect();
        assert_eq!(order, [("Oblivion.esm".into(), true), ("Off.esp".into(), false), ("Knights.esp".into(), true)]);
        assert_eq!(list.get("Knights.esp").unwrap().masters, ["Oblivion.esm"]);
        assert_eq!(list.plugins_txt(), format!("{h}Oblivion.esm\r\nKnights.esp\r\n").into_bytes());
    }

    #[test]
    fn enb_gets_a_dxvk_conf_in_overwrite_root() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("Game Root");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        std::fs::write(game.join("SkyrimSE.exe"), "").unwrap();
        let proton = tmp.path().join("P");
        std::fs::create_dir_all(&proton).unwrap();
        std::fs::write(proton.join("proton"), "").unwrap();
        let folder = tmp.path().join("I");
        create_portable(&CreatePortable { folder: folder.clone(), game, proton, wine_prefix: None, game_name: "Skyrim Special Edition".into() })
            .unwrap();
        let enb = folder.join("mods/ENB/Root");
        std::fs::create_dir_all(&enb).unwrap();
        std::fs::write(enb.join("d3d11.dll"), "").unwrap();
        std::fs::write(folder.join("profiles/Default/modlist.txt"), "+ENB\r\n").unwrap();
        let open = || {
            let instance = Instance::open(&folder).unwrap();
            let profile = instance.profile(None).unwrap();
            (instance, profile)
        };
        let (i, p) = open();
        assert_eq!(ensure_enb_dxvk_conf(&i, &p).unwrap(), None, "d3d11.dll alone is not ENB");
        std::fs::write(enb.join("enblocal.ini"), "").unwrap();
        let (i, p) = open();
        let written = ensure_enb_dxvk_conf(&i, &p).unwrap().unwrap();
        assert!(written.ends_with("overwrite/Root/dxvk.conf"));
        assert_eq!(std::fs::read_to_string(&written).unwrap(), ENB_DXVK_CONF);
        std::fs::write(&written, "user's own").unwrap();
        assert_eq!(ensure_enb_dxvk_conf(&i, &p).unwrap(), None, "an existing dxvk.conf is kept");
        assert_eq!(std::fs::read_to_string(&written).unwrap(), "user's own");
    }

    #[test]
    fn a_new_instance_opens_and_names_its_executables() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("Game Root");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        for exe in ["SkyrimSE.exe", "skse64_loader.exe"] {
            std::fs::write(game.join(exe), "").unwrap();
        }
        let proton = tmp.path().join("GE-Proton");
        std::fs::create_dir_all(&proton).unwrap();
        std::fs::write(proton.join("proton"), "").unwrap();
        let folder = tmp.path().join("New Instance");
        create_portable(&CreatePortable {
            folder: folder.clone(),
            game: game.clone(),
            proton: proton.clone(),
            wine_prefix: None,
            game_name: "Skyrim Special Edition".into(),
        })
        .unwrap();

        let instance = Instance::open(&folder).unwrap();
        assert_eq!(instance.game_path, game.canonicalize().unwrap());
        assert_eq!(instance.proton.prefix.as_deref(), Some(folder.canonicalize().unwrap().join("Prefix").as_path()));
        assert!(instance.proton.steam_drm);
        let exes = instance.executables();
        assert_eq!(exes.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), ["SKSE", "Skyrim Special Edition"]);
        assert!(exes[0].binary.ends_with("Game Root\\skse64_loader.exe"), "{}", exes[0].binary);
        let profile = instance.profile(None).unwrap();
        assert!(profile.local_saves && profile.local_settings);
        assert!(instance.mod_list(&profile).unwrap().is_empty());

        // Not over an existing instance.
        assert!(
            create_portable(&CreatePortable { folder, game, proton, wine_prefix: None, game_name: "Skyrim Special Edition".into() })
                .is_err()
        );
    }
}
