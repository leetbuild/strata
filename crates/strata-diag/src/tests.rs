use super::*;

const LOG: &str = "SKSE64 runtime: initialize (version = 2.0.20 01050610 1DBAD0F6A2B5B1C, os = 6.2 (9200))\r
plugin directory = Z:\\G\\Game Root\\Data\\SKSE\\Plugins\\\r
checking plugin Z:\\G\\Game Root\\Data\\SKSE\\Plugins\\\\A.dll\r
plugin Z:\\G\\Game Root\\Data\\SKSE\\Plugins\\\\A.dll (00000001 Ahzaab's moreHUD (Inventory) 02020040) loaded correctly (handle 2)\r
plugin Z:\\G\\Game Root\\Data\\SKSE\\Plugins\\\\W.dll (00000001 WaterOnSteroids 00010000) reported as incompatible during load\r
plugin Z:\\G\\Game Root\\Data\\SKSE\\Plugins\\\\N.dll (00000001  00000000) no version data\r
plugin Z:\\G\\Game Root\\Data\\SKSE\\Plugins\\\\F.dll (00000001 Fatal 00000002) disabled, fatal error occurred while loading plugin 5 (handle 9)\r
plugin Z:\\G\\Game Root\\Data\\SKSE\\Plugins\\\\msdia140.dll does not appear to be an SKSE plugin\r
couldn't load plugin Z:\\G\\Game Root\\Data\\SKSE\\Plugins\\\\D.dll (Error 126: The specified module could not be found.)\r
couldn't load plugin Z:\\G\\Game Root\\Data\\SKSE\\Plugins\\\\D.dll (Error 126: The specified module could not be found.)\r
plugin 2 allocated 64 bytes from branch pool\r
";

#[test]
fn script_extender_failures_are_read_as_mo2_reads_them() {
    let got = script_extender_failures(LOG);
    let short: Vec<(String, &str)> =
        got.iter().map(|f| (f.path.rsplit('\\').next().unwrap().to_owned(), f.status.as_str())).collect();
    assert_eq!(
        short,
        [
            ("W.dll".to_owned(), "reported as incompatible during load"),
            ("F.dll".to_owned(), "disabled, fatal error occurred while loading plugin"),
            ("D.dll".to_owned(), "couldn't load (Error 126: The specified module could not be found.)"),
        ]
    );
}

#[test]
fn only_dlls_still_present_are_reported() {
    let problem = script_extender(LOG, |path| path.ends_with("W.dll").then(|| "Water Mod".to_owned())).unwrap();
    assert_eq!(problem.details, ["W.dll (Water Mod): reported as incompatible during load"]);
    assert!(script_extender(LOG, |_| None).is_none());
}

#[test]
fn the_check_7_log_reads() {
    // Read only: the SKSE log of the last `tools/e2e.sh` run, when there is one.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_stage/oracle/e2e/skse64.log");
    let Ok(data) = std::fs::read(&path) else {
        eprintln!("SKIPPED: the_check_7_log_reads: no {} (tools/e2e.sh makes it)", path.display());
        return;
    };
    // MO2 reads it as cp1252; the file is ASCII here.
    let log = String::from_utf8_lossy(&data);
    let failures = script_extender_failures(&log);
    let names: Vec<&str> = failures.iter().map(|f| f.path.rsplit('\\').next().unwrap()).collect();
    assert!(names.iter().all(|n| n.to_ascii_lowercase().ends_with(".dll")), "{failures:?}");
}

#[test]
fn overwrite_is_checked_only_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("SKSE/empty")).unwrap();
    let mut s = BasicSettings::default();
    assert!(overwrite(dir.path(), &s).is_none(), "off by default (Leet)");
    s.check_overwrite = true;
    assert_eq!(overwrite(dir.path(), &s).unwrap().details, ["SKSE"], "a folder counts");
    s.ow_ignore_empty = true;
    assert!(overwrite(dir.path(), &s).is_none(), "empty folders don't");
    std::fs::write(dir.path().join("SKSE/empty/x.log1"), "").unwrap();
    assert!(overwrite(dir.path(), &s).is_some());
    s.ow_ignore_log = true;
    assert!(overwrite(dir.path(), &s).is_none(), "nor logs");
}

#[test]
fn settings_come_from_the_ini() {
    let ini = strata_ini::Document::parse(b"[Plugins]\r\nBasic%20diagnosis%20plugin\\check_overwrite=true\r\n");
    assert!(BasicSettings::from_ini(&ini).check_overwrite);
    assert!(!BasicSettings::from_ini(&strata_ini::Document::parse(b"")).check_overwrite);
}

#[test]
fn profile_tweaks_and_nitpick() {
    let dir = tempfile::tempdir().unwrap();
    assert!(profile_tweaks(dir.path()).is_none());
    std::fs::write(dir.path().join("profile_tweaks.ini"), "").unwrap();
    assert!(profile_tweaks(dir.path()).is_some());
    let s = BasicSettings::default();
    assert!(nitpick(&s, None).is_none());
    assert!(nitpick(&s, Some("Nitpick".into())).is_some());
}

#[test]
fn font_config_reports_fonts_that_are_not_installed() {
    let config = "fontlib \"Interface\\fonts_console.swf\"\nfontlib \"Interface\\fonts_en.swf\"\n\
                  fontlib \"Interface\\fonts_mine.swf\"\nfontlib \"Interface\\fonts_there.swf\"\n\
                  fontlib \"Interface\\fonts_cr.swf\"\r\nfontlib \"Interface\\a\"b.swf\"\nmap \"$ConsoleFont\" = \"Arial\" Normal\n";
    let p = invalid_font_config(config, |path| path.ends_with("there.swf")).unwrap();
    assert_eq!(p.details, ["Interface\\fonts_mine.swf: not installed", "Interface\\fonts_cr.swf: not installed"], "defaults skipped");
    assert!(invalid_font_config("fontlib \"Interface\\fonts_en.swf\"\n", |_| false).is_none());
}

#[test]
fn alternate_sources_are_active_unconverted_mods_of_another_game() {
    let m = |name: &str, active, game: &str, converted| ModSource { name: name.into(), active, game_name: game.into(), converted };
    let mods = [
        m("Own", true, "SkyrimSE", false),
        m("Unset", true, "", false),
        m("LE", true, "Skyrim", false),
        m("Off", false, "Skyrim", false),
        m("Done", true, "Skyrim", true),
        m("Primary", true, "Enderal", false),
    ];
    let p = alternate_game_sources(&mods, "skyrimse", &["Enderal"]).unwrap();
    assert_eq!(p.details, ["LE (from Skyrim): mark it converted once it works"]);
    let s = BasicSettings::default();
    assert!(s.check_font && !s.check_alternate_games, "MO2's defaults");
}

mod lists {
    use strata_core::plugins::{AvailablePlugin, PluginFiles, PluginList, PluginRules};
    use strata_game::plugin::{FLAG_MASTER, build_plugin};

    fn list(specs: &[(&str, u16, &[&str])]) -> (tempfile::TempDir, PluginList) {
        let dir = tempfile::tempdir().unwrap();
        let available = specs
            .iter()
            .map(|(name, form, masters)| {
                let path = dir.path().join(name);
                let flags = if name.ends_with(".esm") { FLAG_MASTER } else { 0 };
                std::fs::write(&path, build_plugin(flags, *form, masters, "", 1)).unwrap();
                AvailablePlugin { name: (*name).into(), origin: "m".into(), path }
            })
            .collect();
        let rules = PluginRules { primary: vec!["skyrim.esm".into()], force_enable_core_files: true, light_plugins_supported: true, format: strata_game::PluginListFormat::Creation };
        let txt = specs.iter().skip(1).map(|(n, ..)| format!("*{n}\r\n")).collect::<String>();
        let files = PluginFiles { plugins_txt: Some(txt.into_bytes()), ..PluginFiles::default() };
        let list = PluginList::load(available, rules, &files);
        (dir, list)
    }

    #[test]
    fn missing_masters_list_their_children() {
        let (_d, l) = list(&[("Skyrim.esm", 44, &[]), ("a.esp", 44, &["Gone.esm"]), ("b.esp", 44, &["gone.esm", "Skyrim.esm"])]);
        let p = super::missing_masters(&l).unwrap();
        assert_eq!(p.details, ["Gone.esm: needed by a.esp, b.esp"]);
    }

    #[test]
    fn form43_skips_light_plugins() {
        let (_d, l) = list(&[("Skyrim.esm", 44, &[]), ("old.esp", 43, &[]), ("old.esl", 43, &[]), ("new.esp", 44, &[])]);
        assert_eq!(super::form43(&l, 44).unwrap().details, ["old.esp (form 43)"]);
    }
}
