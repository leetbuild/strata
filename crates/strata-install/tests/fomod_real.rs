//! A real FOMOD: `fixtures/po3-papyrus-extender-6.5.2-ModuleConfig.xml`, from powerofthree's
//! Papyrus Extender 6.5.2 (GPL-3, GitHub release `powerofthree.s.Papyrus.Extender.FOMOD.Installer.7z`;
//! UTF-8 with a BOM, CRLF). Its one choice is picked by `gameDependency` patterns, so the default
//! depends on the game version, as in MO2.

use strata_install::fomod::ModuleConfig;
use strata_install::fomod::session::{Environment, FileState, Session};

const XML: &[u8] = include_bytes!("fixtures/po3-papyrus-extender-6.5.2-ModuleConfig.xml");

struct Game(&'static str);

impl Environment for Game {
    fn file_state(&self, _file: &str) -> FileState {
        FileState::Missing
    }
    fn game_version(&self) -> Option<String> {
        Some(self.0.to_owned())
    }
    fn script_extender_version(&self) -> Option<String> {
        None
    }
}

fn install_for(version: &'static str) -> (String, Vec<String>) {
    let config = ModuleConfig::parse(XML).unwrap();
    let env = Game(version);
    let mut session = Session::new(&config, &env).unwrap();
    let view = session.current().unwrap();
    let picked = view.groups[0].plugins.iter().find(|p| p.checked).unwrap().name.clone();
    assert!(!session.next_step().unwrap(), "one step");
    let files = session.files().iter().map(|f| format!("{} -> {}", f.source, f.destination)).collect();
    (picked, files)
}

#[test]
fn the_game_version_picks_the_dll() {
    let (picked, files) = install_for("1.5.97.0");
    assert_eq!(picked, "SSE v1.5.97 (\"Special Edition\")");
    assert_eq!(files, ["Required -> ", "SE/SKSE/Plugins -> SKSE/Plugins"]);

    assert_eq!(install_for("1.6.1170.0").0, "SSE/AE v1.6.1170 (\"Anniversary/Special Edition\")");
    // 1.7 matches the first pattern of the first plugin
    assert_eq!(install_for("1.7.104.0").0, "SSE/AE v1.7.104+ (\"Anniversary/Special Edition\")");
}

#[test]
fn reads_the_whole_file() {
    let config = ModuleConfig::parse(XML).unwrap();
    assert_eq!(config.module_name, "powerofthree's Papyrus Extender");
    assert_eq!(config.required_files.len(), 1);
    assert_eq!(config.steps.len(), 1);
    assert_eq!(config.steps[0].groups[0].plugins.len(), 3, "explicit order kept");
}
