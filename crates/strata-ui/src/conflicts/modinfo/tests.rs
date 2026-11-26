use super::*;

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A small instance: game, mods A (low) and B (high), a separator, a disabled mod.
fn instance() -> (tempfile::TempDir, Instance, Profile) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let game = root.join("game");
    put(&game, "SkyrimSE.exe", "");
    put(&game, "Data/Skyrim.esm", "");
    put(
        root,
        "ModOrganizer.ini",
        &format!("[General]\ngameName=Skyrim Special Edition\nselected_profile=Default\ngamePath={}\n", game.display()),
    );
    put(root, "categories.dat", "1|Armour|0\n2|Weapons|0\n3|Heavy|1\n");
    put(root, "profiles/Default/modlist.txt", "+B\n+A\n-Off\n+Tools_separator\n");
    put(root, "mods/A/meta.ini", "[General]\ncategory=\"3,1\"\nmodid=42\nversion=1.2\ncomments=hello\nnexusDescription=\"[b]Bold[/b] text\"\n");
    put(root, "mods/A/readme.txt", "read me");
    put(root, "mods/A/docs/Notes.JSON", "{}");
    put(root, "mods/A/settings.ini", "[x]");
    put(root, "mods/A/textures/a.dds", "dds");
    put(root, "mods/A/textures/shared.dds", "a");
    put(root, "mods/A/Root/skse64_loader.exe", "a");
    put(root, "mods/A/a.esp", "");
    put(root, "mods/A/optional/extra.esp", "");
    put(root, "mods/A/screens/shot.PNG", "png");
    put(root, "mods/B/textures/shared.dds", "b");
    put(root, "mods/B/Root/skse64_loader.exe", "b");
    put(root, "mods/Off/x.txt", "");
    std::fs::create_dir_all(root.join("mods/Tools_separator")).unwrap();
    std::fs::create_dir_all(root.join("overwrite")).unwrap();
    let instance = Instance::open(root).unwrap();
    let profile = instance.profile(None).unwrap();
    (tmp, instance, profile)
}

#[test]
fn files_go_to_the_first_tab_that_wants_them() {
    let (_tmp, instance, profile) = instance();
    let info = ModInfo::open(&instance, &profile, "A").unwrap();
    assert_eq!(info.files.text, ["docs/Notes.JSON", "readme.txt"]);
    assert_eq!(info.files.ini, ["settings.ini"], "meta.ini is left out");
    assert_eq!(info.files.images, ["screens/shot.PNG", "textures/a.dds", "textures/shared.dds"]);
    assert_eq!(
        info.files.plugins,
        [Plugin { path: "a.esp".into(), active: true }, Plugin { path: "optional/extra.esp".into(), active: false }]
    );
}

#[test]
fn conflicts_tabs_list_data_and_root_files() {
    let (_tmp, instance, profile) = instance();
    let a = ModInfo::open(&instance, &profile, "A").unwrap();
    let shown = |rows: &[GeneralRow]| rows.iter().map(|r| (r.shown.clone(), r.others.clone())).collect::<Vec<_>>();
    assert_eq!(
        shown(&a.conflicts.overwritten),
        [("/textures/shared.dds".to_owned(), "B".to_owned()), ("/Root/skse64_loader.exe".to_owned(), "B".to_owned())]
    );
    assert!(a.conflicts.overwrites.is_empty());
    assert!(a.conflicts.no_conflict.iter().any(|r| r.shown == "/a.esp"));
    let root_row = a.conflicts.overwritten.iter().find(|r| r.shown.starts_with("/Root/")).unwrap();
    assert_eq!(root_row.real.as_deref(), Some(a.dir.join("Root/skse64_loader.exe").as_path()));

    let b = ModInfo::open(&instance, &profile, "B").unwrap();
    assert_eq!(shown(&b.conflicts.overwrites)[0], ("/textures/shared.dds".to_owned(), "A".to_owned()));
    let adv = b.conflicts.advanced.iter().find(|r| r.shown == "/textures/shared.dds").unwrap();
    assert_eq!((adv.before_all.as_str(), adv.before_nearest.as_str(), adv.after_all.as_str()), ("A", "A", ""));
    assert_eq!(adv.mods, ["A"]);
    assert_eq!(b.conflicts.overwrites[0].mods, ["A"]);

    let off = ModInfo::open(&instance, &profile, "Off").unwrap();
    assert!(off.conflicts.advanced.is_empty(), "a disabled mod has no conflicts (MO2: no origin)");
}

#[test]
fn categories_nexus_notes() {
    let (_tmp, instance, profile) = instance();
    let a = ModInfo::open(&instance, &profile, "A").unwrap();
    assert_eq!(a.primary_category, 3);
    let checked: Vec<i32> = a.categories.iter().filter(|c| c.checked).map(|c| c.id).collect();
    assert_eq!(checked, [1, 3]);
    assert_eq!((a.nexus.mod_id, a.nexus.version.as_str()), (42, "1.2"));
    assert_eq!(a.comments, "hello");
    assert_eq!(a.nexus.description, "<b>Bold</b> text", "the BBCode is shown as HTML (MO2's convertToHTML)");

    let sep = ModInfo::open(&instance, &profile, "Tools_separator").unwrap();
    assert!(sep.is_separator && sep.files == Files::default());
    assert!(sep.nexus.description.contains("does not have a valid Nexus ID"), "MO2's note when there is none");
    assert!(ModInfo::open(&instance, &profile, "nope").is_err());
}

#[test]
fn edits_go_through_meta_ini_and_the_files() {
    let (_tmp, instance, profile) = instance();
    let dir = instance.paths.mods.join("A");
    assert!(save_notes(&dir, "short", "<b>long</b>").unwrap());
    assert!(!save_notes(&dir, "short", "<b>long</b>").unwrap(), "unchanged: not written");
    assert!(save_categories(&dir, 2, &[1]).unwrap());
    let a = ModInfo::open(&instance, &profile, "A").unwrap();
    assert_eq!((a.comments.as_str(), a.notes.as_str(), a.primary_category), ("short", "<b>long</b>", 2));
    let text = std::fs::read_to_string(dir.join("meta.ini")).unwrap();
    assert!(text.contains("category=\"2,1\""), "{text}");
    assert!(text.contains("modid=42"), "other keys kept: {text}");

    // A mod without these keys keeps them absent when nothing was changed.
    let b = instance.paths.mods.join("B");
    assert!(!save_notes(&b, "", "").unwrap());
    assert!(!save_categories(&b, 0, &[]).unwrap());
    assert!(!b.join("meta.ini").exists(), "nothing written for an unchanged mod");
    assert!(!save_categories(&dir, 2, &[1, 2]).unwrap(), "same set, same primary");

    save_file(&dir, "settings.ini", b"[y]").unwrap();
    assert_eq!(std::fs::read(dir.join("settings.ini")).unwrap(), b"[y]");
    assert!(save_file(&dir, "../escape.txt", b"x").is_err());
    assert!(save_file(&dir, "missing.txt", b"x").is_err());
}

#[test]
fn optional_plugins_move_in_and_out() {
    let (_tmp, instance, _) = instance();
    let dir = instance.paths.mods.join("A");
    assert!(activate_plugin(&dir, "optional/extra.esp", "a.esp").is_err(), "name taken");
    assert_eq!(activate_plugin(&dir, "optional/extra.esp", "extra.esp").unwrap(), "extra.esp");
    assert!(dir.join("extra.esp").is_file());
    assert_eq!(deactivate_plugin(&dir, "extra.esp", Some("optional/extra.esp")).unwrap(), "optional/extra.esp");
    assert_eq!(deactivate_plugin(&dir, "a.esp", None).unwrap(), "optional/a.esp");
    assert!(dir.join("optional/a.esp").is_file() && !dir.join("a.esp").exists());
}

#[test]
fn providers_of_a_file_for_the_preview() {
    let (_tmp, instance, profile) = instance();
    let v = crate::conflicts::file_providers(&instance, &profile, false, "TEXTURES/Shared.dds").unwrap();
    let names: Vec<&str> = v.iter().map(|x| x.layer.as_str()).collect();
    assert_eq!(names, ["A", "B"]);
    assert_eq!(std::fs::read(&v[1].real).unwrap(), b"b");
    let r = crate::conflicts::file_providers(&instance, &profile, true, "skse64_loader.exe").unwrap();
    assert_eq!(r.iter().map(|x| x.layer.as_str()).collect::<Vec<_>>(), ["A", "B"]);
    let g = crate::conflicts::file_providers(&instance, &profile, false, "skyrim.esm").unwrap();
    assert_eq!(g[0].layer, "<game>");
    assert!(crate::conflicts::file_providers(&instance, &profile, false, "nothing.txt").unwrap().is_empty());
}
