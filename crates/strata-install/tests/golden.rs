//! Oracle check 5 (`spec/07-oracle.md`): install golden tests. Each test builds a small
//! archive with `7z`, installs it through the whole pipeline with scripted answers and
//! compares the mod folder with the expected tree. Nothing big is committed.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use strata_install::archive::SevenZip;
use strata_install::checker::DataChecker;
use strata_install::fomod::Info;
use strata_install::fomod::session::{Environment, FileState, Session};
use strata_install::install::{Answer, Clash, ClashAction, InstallUi, Outcome, Previous, Request, Target, install, preview};
use strata_install::installers::Installer;
use strata_install::name::{GuessedName, Quality};
use strata_install::rootsorter::Plan;
use strata_install::tree::FileTree;

fn have_7z() -> bool {
    std::process::Command::new("7z").arg("i").output().is_ok_and(|o| o.status.success())
}

macro_rules! need_7z {
    () => {
        if !have_7z() {
            eprintln!("SKIPPED: golden install test: the 7z program is not installed");
            return;
        }
    };
}

/// A throwaway instance: mods, downloads, temp, trash.
struct World {
    dir: tempfile::TempDir,
    target: Target,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        for d in ["mods", "downloads", "src"] {
            std::fs::create_dir_all(p.join(d)).unwrap();
        }
        std::fs::write(p.join("categories.dat"), "1|Skyrim Special Edition|0\n7|Patches|0\n").unwrap();
        std::fs::write(p.join("nexuscatmap.dat"), "1|Skyrim Special Edition|20\n7|Patches|79\n").unwrap();
        let target = Target {
            game_name: "Skyrim Special Edition".into(),
            game_short_name: "SkyrimSE".into(),
            mods_dir: p.join("mods"),
            downloads_dir: p.join("downloads"),
            categories_dir: p.to_owned(),
            temp_dir: p.join("strata/tmp"),
            trash: strata_core::Trash::at(p.join("Trash")),
        };
        Self { dir, target }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    /// Builds `<where>/<name>` from `(path, content)` pairs; a path ending in `/` is a folder.
    fn archive(&self, place: &str, name: &str, files: &[(&str, &str)]) -> PathBuf {
        self.archive_with(place, name, files, &[])
    }

    /// [`World::archive`] plus files given as bytes.
    fn archive_with(&self, place: &str, name: &str, files: &[(&str, &str)], raw: &[(&str, &[u8])]) -> PathBuf {
        let src = tempfile::tempdir_in(self.path("src")).unwrap();
        for (path, content) in raw {
            let p = src.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
        }
        for (path, content) in files {
            let p = src.path().join(path);
            if path.ends_with('/') {
                std::fs::create_dir_all(&p).unwrap();
            } else {
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, content).unwrap();
            }
        }
        let out = self.path(place).join(name);
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        let status = std::process::Command::new("7z")
            .arg("a")
            .arg("-bso0")
            .arg("-bsp0")
            .arg(&out)
            .arg(".")
            .current_dir(src.path())
            .status()
            .unwrap();
        assert!(status.success());
        out
    }

    /// Installs again over the mod `current` ("Reinstall"): its previous options are offered.
    fn reinstall(&self, ui: &mut Script, archive: &Path, current: &str) -> strata_install::Result<Outcome> {
        let previous = Previous::of(&self.path("mods").join(current), true, true).unwrap();
        let request = Request { name: Some(current.to_owned()), current_name: Some(current.to_owned()), previous };
        install(&self.target, &SevenZip, &Env::default(), ui, &AtomicBool::new(false), archive, &request)
    }

    fn install(&self, ui: &mut Script, archive: &Path, name: Option<&str>) -> strata_install::Result<Outcome> {
        let env = Env::default();
        let request = Request { name: name.map(str::to_owned), ..Request::default() };
        install(&self.target, &SevenZip, &env, ui, &AtomicBool::new(false), archive, &request)
    }

    /// The mod folder's files and folders, sorted, `/` after folders; `meta.ini` left out.
    fn tree(&self, mod_name: &str) -> Vec<String> {
        fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let rel = e.path().strip_prefix(base).unwrap().to_string_lossy().into_owned();
                if e.file_type().unwrap().is_dir() {
                    out.push(format!("{rel}/"));
                    walk(base, &e.path(), out);
                } else if rel != "meta.ini" {
                    out.push(rel);
                }
            }
        }
        let mut out = Vec::new();
        let base = self.target.mods_dir.join(mod_name);
        walk(&base, &base, &mut out);
        out.sort();
        out
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path(rel)).unwrap()
    }
}

#[derive(Default)]
struct Env {}

impl Environment for Env {
    fn file_state(&self, file: &str) -> FileState {
        if file.eq_ignore_ascii_case("SkyUI_SE.esp") { FileState::Active } else { FileState::Missing }
    }
    fn game_version(&self) -> Option<String> {
        Some("1.5.97.0".into())
    }
    fn script_extender_version(&self) -> Option<String> {
        Some("0.2.0.20".into())
    }
}

/// Scripted answers, and a record of what was asked.
#[derive(Default)]
struct Script {
    asked: Vec<String>,
    root_sort: bool,
    bain: Option<Vec<String>>,
    bundle: Option<String>,
    /// For each FOMOD step entered: `(group, plugin, on)` clicks.
    fomod: Vec<Vec<(usize, usize, bool)>>,
    clash: Vec<Clash>,
    manual_root: Option<String>,
    /// Answers to yes / no questions by id: (yes, remember).
    questions: Vec<(&'static str, bool, bool)>,
    /// Answers to the password prompt, in order; `None` once they run out.
    passwords: Vec<String>,
    /// Set to true at the first extraction progress report.
    cancel_on_progress: Option<std::sync::Arc<AtomicBool>>,
    progress: Vec<u8>,
}

impl InstallUi for Script {
    fn quick(&mut self, _name: &mut GuessedName) -> Answer<()> {
        self.asked.push("quick".into());
        Answer::Accept(())
    }
    fn bain_unsure(&mut self) -> bool {
        self.asked.push("bain_unsure".into());
        true
    }
    fn bain(&mut self, _name: &mut GuessedName, packages: &[String], defaults: &[bool]) -> Answer<Vec<String>> {
        self.asked.push(format!("bain {packages:?} {defaults:?}"));
        match &self.bain {
            Some(pick) => Answer::Accept(pick.clone()),
            None => Answer::Accept(packages.iter().zip(defaults).filter(|(_, d)| **d).map(|(p, _)| p.clone()).collect()),
        }
    }
    fn bundle(&mut self, archives: &[String]) -> Answer<String> {
        self.asked.push(format!("bundle {archives:?}"));
        Answer::Accept(self.bundle.clone().unwrap_or_else(|| archives[0].clone()))
    }
    fn fomod(&mut self, name: &mut GuessedName, info: &Info, session: &mut Session, images: &Path) -> Answer<()> {
        self.asked.push(format!("fomod {} {:?}", name.value(), info.name));
        assert!(images.join("fomod").is_dir(), "the pictures are extracted for the dialog");
        let mut steps = self.fomod.iter();
        while let Some(view) = session.current() {
            self.asked.push(format!("step {}", view.name));
            for &(g, p, on) in steps.next().into_iter().flatten() {
                session.set(g, p, on).unwrap();
            }
            if !session.next_step().unwrap() {
                break;
            }
        }
        Answer::Accept(())
    }
    fn fomod_missing(&mut self, missing: &[String]) -> bool {
        self.asked.push(format!("missing {missing:?}"));
        true
    }
    fn manual(&mut self, name: &mut GuessedName, _tree: &mut FileTree, _checker: &DataChecker) -> Answer<String> {
        self.asked.push("manual".into());
        name.update("Manual Name", Quality::User);
        match &self.manual_root {
            Some(r) => Answer::Accept(r.clone()),
            None => Answer::Cancel,
        }
    }
    fn root_sort(&mut self, mod_name: &str, plan: &Plan) -> bool {
        self.asked.push(format!("root_sort {mod_name} {:?}", plan.to_root));
        self.root_sort
    }
    fn question(&mut self, id: &str, _title: &str, _text: &str, default_yes: bool) -> (bool, bool) {
        self.asked.push(format!("question {id}"));
        self.questions.iter().find(|(q, _, _)| *q == id).map_or((default_yes, false), |(_, y, r)| (*y, *r))
    }
    fn clash(&mut self, mod_name: &str) -> Clash {
        self.asked.push(format!("clash {mod_name}"));
        self.clash.remove(0)
    }
    fn password(&mut self, archive: &str, wrong: bool) -> Option<String> {
        self.asked.push(format!("password {archive} {wrong}"));
        (!self.passwords.is_empty()).then(|| self.passwords.remove(0))
    }
    fn progress(&mut self, percent: u8) {
        self.progress.push(percent);
        if let Some(c) = &self.cancel_on_progress {
            c.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

#[test]
fn simple_archive_from_downloads_with_meta() {
    need_7z!();
    let w = World::new();
    let a = w.archive("downloads", "Cool Mod-1234-1-0.7z", &[("meshes/a.nif", "n"), ("Cool.esp", "p")]);
    std::fs::write(
        format!("{}.meta", a.display()),
        "[General]\ngameName=skyrimse\nmodID=1234\nfileID=99\nname=Cool Mod Main\nmodName=Cool Mod\nversion=1.0\ncategory=79\nrepository=Nexus\nfileCategory=1\n",
    )
    .unwrap();
    let mut ui = Script::default();
    let out = w.install(&mut ui, &a, None).unwrap();
    assert_eq!(out.name, "Cool Mod");
    assert_eq!(out.installer, Installer::Quick);
    assert!(!out.merged_or_replaced());
    assert_eq!(ui.asked, ["quick"]);
    assert_eq!(w.tree("Cool Mod"), ["Cool.esp", "meshes/", "meshes/a.nif"]);
    // doInstall's keys, then saveMeta's (version and category rewritten, the rest appended)
    let meta = w.read("mods/Cool Mod/meta.ini");
    assert!(meta.contains("\nnexusLastModified=20"), "{meta}");
    let meta: String = meta.lines().filter(|l| !l.starts_with("nexusLastModified=")).map(|l| format!("{l}\n")).collect();
    assert_eq!(
        meta,
        "[General]\ngameName=skyrimse\nmodid=1234\nversion=1.0.0.0\nnewestVersion=\ncategory=\"7,\"\nnexusFileStatus=1\n\
         installationFile=Cool Mod-1234-1-0.7z\nrepository=Nexus\nignoredVersion=\ncomments=\nnotes=\nnexusDescription=\n\
         url=\nhasCustomURL=false\nlastNexusQuery=\nlastNexusUpdate=\nnexusCategory=0\nconverted=false\nvalidated=false\n\
         color=@Variant(\\0\\0\\0\\x43\\0\\xff\\xff\\0\\0\\0\\0\\0\\0\\0\\0)\ntracked=0\n\
         [installedFiles]\n1\\modid=1234\n1\\fileid=99\nsize=1\n"
    );
    let meta = w.read("downloads/Cool Mod-1234-1-0.7z.meta");
    assert!(meta.contains("installed=true") && meta.contains("uninstalled=false"));
    assert!(std::fs::read_dir(w.path("strata/tmp")).unwrap().next().is_none(), "temp folders are removed");
}

#[test]
fn wrapped_archive_outside_downloads_guesses_the_name() {
    need_7z!();
    let w = World::new();
    let a = w.archive("elsewhere", "SkyUI_5_2_SE-12604-5-2SE.7z", &[("SkyUI/Interface/x.swf", "i"), ("SkyUI/SkyUI_SE.esp", "e")]);
    let mut ui = Script::default();
    let out = w.install(&mut ui, &a, None).unwrap();
    assert_eq!(out.name, "SkyUI_5_2_SE");
    assert_eq!(w.tree("SkyUI_5_2_SE"), ["Interface/", "Interface/x.swf", "SkyUI_SE.esp"]);
    let meta = w.read("mods/SkyUI_5_2_SE/meta.ini");
    assert!(meta.contains("modid=12604\n"), "{meta}");
    assert!(meta.contains(&format!("installationFile={}\n", a.display())), "full path outside downloads");
    assert!(meta.contains("version=d20"), "date version without a .meta");
    assert!(meta.starts_with("[General]\ngameName=SkyrimSE\n"), "the game's short name without a .meta");
    assert!(!meta.contains("installedFiles"), "not a known download");
}

#[test]
fn user_name_wins_and_data_plus_docs_merge() {
    need_7z!();
    let w = World::new();
    let a = w.archive("downloads", "x.zip", &[("Data/textures/t.dds", "t"), ("readme.txt", "r")]);
    let mut ui = Script::default();
    let out = w.install(&mut ui, &a, Some("My Name")).unwrap();
    assert_eq!(out.name, "My Name");
    assert_eq!(w.tree("My Name"), ["readme.txt", "textures/", "textures/t.dds"]);
}

#[test]
fn skse_layout_is_root_sorted_once() {
    need_7z!();
    let w = World::new();
    let a = w.archive(
        "downloads",
        "skse64_2_00_20.7z",
        &[
            ("skse64_2_00_20/skse64_loader.exe", "x"),
            ("skse64_2_00_20/skse64_1_5_97.dll", "d"),
            ("skse64_2_00_20/skse64_readme.txt", "r"),
            ("skse64_2_00_20/Data/Scripts/Actor.pex", "p"),
            ("skse64_2_00_20/src/main.cpp", "c"),
        ],
    );
    let mut ui = Script { root_sort: true, ..Script::default() };
    let out = w.install(&mut ui, &a, Some("SKSE64")).unwrap();
    assert_eq!(out.installer, Installer::Quick, "after the split the archive is plain Data + Root");
    assert_eq!(ui.asked.iter().filter(|a| a.starts_with("root_sort")).count(), 1);
    assert_eq!(
        w.tree("SKSE64"),
        ["Root/", "Root/skse64_1_5_97.dll", "Root/skse64_loader.exe", "Scripts/", "Scripts/Actor.pex"]
    );
}

#[test]
fn enb_declined_stays_where_it_was() {
    need_7z!();
    let w = World::new();
    let a = w.archive("downloads", "enb.zip", &[("d3d11.dll", "d"), ("enbseries.ini", "e"), ("enbseries/a.fx", "f")]);
    let mut ui = Script { root_sort: false, ..Script::default() };
    w.install(&mut ui, &a, Some("ENB")).unwrap();
    assert_eq!(w.tree("ENB"), ["d3d11.dll", "enbseries.ini", "enbseries/", "enbseries/a.fx"]);
    assert_eq!(ui.asked.iter().filter(|a| a.starts_with("root_sort")).count(), 1, "declining is not asked twice");

    let mut ui = Script { root_sort: true, clash: vec![Clash { action: ClashAction::Replace, backup: false }], ..Script::default() };
    let out = w.install(&mut ui, &a, Some("ENB")).unwrap();
    assert!(out.replaced);
    assert_eq!(w.tree("ENB"), ["Root/", "Root/d3d11.dll", "Root/enbseries.ini", "Root/enbseries/", "Root/enbseries/a.fx"]);
    assert!(out.trashed.unwrap().join("d3d11.dll").is_file(), "the old files went to the trash");
}

#[test]
fn bain_package_installs_selected_folders_in_order() {
    need_7z!();
    let w = World::new();
    let a = w.archive(
        "downloads",
        "bain.7z",
        &[
            ("00 Core/Core.esp", "c"),
            ("10 Hi/textures/a.dds", "hi"),
            ("20 Lo/textures/a.dds", "lo"),
            ("Docs/readme.txt", "r"),
            ("package.txt", "p"),
        ],
    );
    let mut ui = Script { bain: Some(vec!["20 Lo".into(), "00 Core".into()]), ..Script::default() };
    let out = w.install(&mut ui, &a, Some("Bain")).unwrap();
    assert_eq!(out.installer, Installer::Bain);
    assert_eq!(ui.asked, [r#"bain ["00 Core", "10 Hi", "20 Lo"] [true, false, false]"#]);
    assert_eq!(w.tree("Bain"), ["Core.esp", "textures/", "textures/a.dds"]);
    assert_eq!(w.read("mods/Bain/textures/a.dds"), "lo");
    let meta = w.read("mods/Bain/meta.ini");
    assert!(meta.contains("[Plugins]\nBAIN%20Installer\\option0=00 Core\nBAIN%20Installer\\option1=20 Lo\n"), "{meta}");
}

const MODULE_CONFIG: &str = r#"<?xml version="1.0" encoding="UTF-16"?>
<config>
  <moduleName>Fancy</moduleName>
  <requiredInstallFiles><folder source="Core" destination=""/></requiredInstallFiles>
  <installSteps order="Explicit">
    <installStep name="Size">
      <optionalFileGroups>
        <group name="Resolution" type="SelectExactlyOne">
          <plugins order="Explicit">
            <plugin name="2K"><description>big</description>
              <files><folder source="2K" destination="textures" priority="1"/></files>
              <conditionFlags><flag name="res">2k</flag></conditionFlags>
              <typeDescriptor><type name="Recommended"/></typeDescriptor></plugin>
            <plugin name="1K"><description>small</description>
              <files><folder source="1K" destination="textures" priority="1"/></files>
              <conditionFlags><flag name="res">1k</flag></conditionFlags>
              <typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
    <installStep name="Patches">
      <visible><flagDependency flag="res" value="2k"/></visible>
      <optionalFileGroups>
        <group name="Patches" type="SelectAny">
          <plugins>
            <plugin name="SkyUI patch"><description/>
              <files><file source="Patches\skyui.esp" destination="Fancy - SkyUI.esp"/></files>
              <typeDescriptor><dependencyType><defaultType name="Optional"/><patterns><pattern>
                <dependencies><fileDependency file="SkyUI_SE.esp" state="Active"/></dependencies>
                <type name="Recommended"/></pattern></patterns></dependencyType></typeDescriptor></plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
  </installSteps>
  <conditionalFileInstalls><patterns>
    <pattern><dependencies><flagDependency flag="res" value="1k"/></dependencies>
      <files><file source="Extra/lowres.ini" destination="Fancy.ini"/></files></pattern>
  </patterns></conditionalFileInstalls>
</config>"#;

/// FOMODs are often UTF-16 with a BOM.
fn utf16(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    bytes
}

fn fomod_archive(w: &World) -> PathBuf {
    let config = utf16(MODULE_CONFIG);
    w.archive_with(
        "downloads",
        "Fancy-555-1-0.7z",
        &[
            ("Fancy/fomod/info.xml", "<fomod><Name>Fancy Textures</Name><Version>2.1</Version><Website>https://example.org/fancy</Website></fomod>"),
            ("Fancy/fomod/shot.png", "png"),
            ("Fancy/Core/Fancy.esp", "esp"),
            ("Fancy/2K/a.dds", "2k"),
            ("Fancy/1K/a.dds", "1k"),
            ("Fancy/Patches/skyui.esp", "patch"),
            ("Fancy/Extra/lowres.ini", "ini"),
        ],
        &[("Fancy/fomod/ModuleConfig.xml", &config)],
    )
}

#[test]
fn fomod_defaults_follow_types_and_conditions() {
    need_7z!();
    let w = World::new();
    let a = fomod_archive(&w);
    let mut ui = Script::default();
    let out = w.install(&mut ui, &a, None).unwrap();
    assert_eq!(out.installer, Installer::Fomod);
    assert_eq!(out.name, "Fancy Textures", "info.xml's name is metadata");
    assert_eq!(ui.asked, ["fomod Fancy Textures Some(\"Fancy Textures\")", "step Size", "step Patches"]);
    assert_eq!(w.tree("Fancy Textures"), ["Fancy - SkyUI.esp", "Fancy.esp", "textures/", "textures/a.dds"]);
    assert_eq!(w.read("mods/Fancy Textures/textures/a.dds"), "2k");
    let meta = w.read("mods/Fancy Textures/meta.ini");
    assert!(meta.contains("version=2.1.0.0\n") && meta.contains("modid=555\n"), "{meta}");
    assert!(meta.contains("url=https://example.org/fancy\nhasCustomURL=true\n"), "{meta}");
}

#[test]
fn fomod_choices_change_steps_and_conditional_files() {
    need_7z!();
    let w = World::new();
    let a = fomod_archive(&w);
    let mut ui = Script { fomod: vec![vec![(0, 1, true)]], ..Script::default() };
    w.install(&mut ui, &a, Some("Low")).unwrap();
    assert_eq!(ui.asked.iter().filter(|a| a.starts_with("step")).collect::<Vec<_>>(), ["step Size"], "Patches hidden");
    assert_eq!(w.tree("Low"), ["Fancy.esp", "Fancy.ini", "textures/", "textures/a.dds"]);
    assert_eq!(w.read("mods/Low/textures/a.dds"), "1k");
}

#[test]
fn preview_writes_nothing() {
    need_7z!();
    let w = World::new();
    let a = fomod_archive(&w);
    let before = std::fs::read(format!("{}", a.display())).unwrap();
    let mut ui = Script { fomod: vec![vec![(0, 1, true)]], ..Script::default() };
    let p = preview(&w.target, &SevenZip, &Env::default(), &mut ui, &a, &Request::default()).unwrap();
    assert_eq!(p.installer, Installer::Fomod);
    assert_eq!(p.name, "Fancy Textures");
    assert_eq!(p.files, ["Fancy.esp", "Fancy.ini", "textures/", "textures/a.dds"]);
    assert!(!p.exists);
    assert!(std::fs::read_dir(&w.target.mods_dir).unwrap().next().is_none(), "no mod folder");
    assert!(!w.path("downloads/Fancy-555-1-0.7z.meta").exists(), "the download is not marked");
    assert_eq!(std::fs::read(&a).unwrap(), before);
}

#[test]
fn bundle_installs_the_inner_archive() {
    need_7z!();
    let w = World::new();
    let inner = w.archive("src", "Inner Mod.zip", &[("Inner.esp", "i")]);
    let bytes = std::fs::read(&inner).unwrap();
    let a = {
        let src = tempfile::tempdir_in(w.path("src")).unwrap();
        std::fs::create_dir(src.path().join("pack")).unwrap();
        std::fs::write(src.path().join("pack/Inner Mod.zip"), bytes).unwrap();
        std::fs::write(src.path().join("pack/readme.txt"), "r").unwrap();
        let out = w.path("downloads/Bundle.7z");
        let ok = std::process::Command::new("7z").args(["a", "-bso0", "-bsp0"]).arg(&out).arg(".").current_dir(src.path()).status().unwrap();
        assert!(ok.success());
        out
    };
    let mut ui = Script::default();
    let out = w.install(&mut ui, &a, None).unwrap();
    assert_eq!(out.installer, Installer::Bundle);
    // both file names are "good" guesses; the inner one comes later and wins, as in MO2
    assert_eq!(out.name, "Inner Mod");
    assert_eq!(w.tree("Inner Mod"), ["Inner.esp"]);
    let meta = w.read("mods/Inner Mod/meta.ini");
    assert!(meta.contains(&format!("installationFile={}\n", a.display())), "{meta}");
}

#[test]
fn clash_merge_rename_and_backup() {
    need_7z!();
    let w = World::new();
    let a = w.archive("downloads", "m.7z", &[("a.esp", "1")]);
    let b = w.archive("downloads", "n.7z", &[("b.esp", "2")]);
    w.install(&mut Script::default(), &a, Some("Mod")).unwrap();

    let mut ui = Script { clash: vec![Clash { action: ClashAction::Merge, backup: true }], ..Script::default() };
    let out = w.install(&mut ui, &b, Some("mod")).unwrap();
    assert!(out.merged);
    assert_eq!(out.name, "Mod", "names compare without case; the folder keeps its spelling");
    assert_eq!(w.tree("Mod"), ["a.esp", "b.esp"]);
    assert!(out.backup.unwrap().join("a.esp").is_file());

    let mut ui = Script {
        clash: vec![
            Clash { action: ClashAction::Rename("Mod".into()), backup: false },
            Clash { action: ClashAction::Rename("Mod 2".into()), backup: false },
        ],
        ..Script::default()
    };
    let out = w.install(&mut ui, &b, Some("Mod")).unwrap();
    assert_eq!(out.name, "Mod 2");
    assert_eq!(ui.asked, ["quick", "clash Mod", "clash Mod"]);

    let mut ui = Script { clash: vec![Clash { action: ClashAction::Cancel, backup: false }], ..Script::default() };
    assert!(matches!(w.install(&mut ui, &b, Some("Mod")), Err(strata_install::Error::Cancelled)));
}

#[test]
fn unknown_layout_goes_to_the_manual_installer() {
    need_7z!();
    let w = World::new();
    let a = w.archive("downloads", "odd.7z", &[("one/meshes/a.nif", "m"), ("two/notes.txt", "n")]);
    let mut ui = Script { manual_root: Some("one".into()), ..Script::default() };
    let out = w.install(&mut ui, &a, None).unwrap();
    assert_eq!(out.installer, Installer::Manual);
    assert_eq!(out.name, "Manual Name");
    assert_eq!(w.tree("Manual Name"), ["meshes/", "meshes/a.nif"]);

    let mut ui = Script::default();
    assert!(matches!(w.install(&mut ui, &a, None), Err(strata_install::Error::Cancelled)));
    assert!(!w.target.mods_dir.join("odd").exists());
}

#[test]
fn unsupported_and_broken_archives_fail_clearly() {
    need_7z!();
    let w = World::new();
    std::fs::write(w.path("downloads/x.txt"), "x").unwrap();
    let e = w.install(&mut Script::default(), &w.path("downloads/x.txt"), None).unwrap_err();
    assert!(e.to_string().contains(".txt"), "{e}");
    std::fs::write(w.path("downloads/broken.7z"), "not an archive").unwrap();
    let e = w.install(&mut Script::default(), &w.path("downloads/broken.7z"), None).unwrap_err();
    assert!(matches!(e, strata_install::Error::Archive { .. }), "{e}");
}

/// BSA Extractor: a mod from Skyrim LE's Nexus (an "alternate source") with a BSA is offered
/// for unpacking; the answers are Yes and "remove", the second one remembered.
#[test]
fn bsa_from_an_alternate_source_is_unpacked() {
    need_7z!();
    let w = World::new();
    // build the BSA with strata-archive's packer
    let loose = w.path("src/loose");
    std::fs::create_dir_all(loose.join("textures")).unwrap();
    std::fs::write(loose.join("textures/a.dds"), "dds").unwrap();
    let bsa = w.path("src/Old.bsa");
    let files = [strata_archive::pack::PackFile { path: "textures/a.dds".into(), source: loose.join("textures/a.dds") }];
    strata_archive::pack::write_bsa(&bsa, &files, strata_archive::pack::PackOptions { version: 105, compress: false }).unwrap();
    let bytes = std::fs::read(&bsa).unwrap();
    let a = {
        let src = tempfile::tempdir_in(w.path("src")).unwrap();
        std::fs::write(src.path().join("Old.esp"), "p").unwrap();
        std::fs::write(src.path().join("Old.bsa"), &bytes).unwrap();
        let out = w.path("downloads/Old-1-1.7z");
        let ok = std::process::Command::new("7z").args(["a", "-bso0", "-bsp0"]).arg(&out).arg(".").current_dir(src.path()).status().unwrap();
        assert!(ok.success());
        out
    };
    std::fs::write(format!("{}.meta", a.display()), "[General]\ngameName=Skyrim\nmodID=1\nfileID=2\n").unwrap();
    std::fs::write(w.path("ModOrganizer.ini"), "[General]\n").unwrap();
    let mut ui = Script { questions: vec![("unpackBSA", true, false), ("removeUnpackedBSA", true, true)], ..Script::default() };
    let out = w.install(&mut ui, &a, Some("Old")).unwrap();
    let ini = strata_ini::Document::parse(&std::fs::read(w.path("ModOrganizer.ini")).unwrap());
    let done = strata_install::bsa::offer(&mut ui, w.dir.path(), &ini, &out.mod_dir, "SkyrimSE").unwrap().unwrap();
    assert_eq!(done.archives, ["Old.bsa"]);
    assert!(done.removed && done.error.is_none());
    assert_eq!(w.tree("Old"), ["Old.esp", "textures/", "textures/a.dds"]);
    assert_eq!(w.read("mods/Old/textures/a.dds"), "dds");
    assert!(w.read("ModOrganizer.ini").contains("removeUnpackedBSA=16384"), "remembered");
    assert!(ui.asked.contains(&"question unpackBSA".to_owned()));
}


fn replace() -> Vec<Clash> {
    vec![Clash { action: ClashAction::Replace, backup: false }]
}

#[test]
fn reinstall_offers_the_previous_bain_packages() {
    need_7z!();
    let w = World::new();
    let files = [("00 Core/Core.esp", "c"), ("10 Hi/textures/a.dds", "hi"), ("20 Lo/textures/a.dds", "lo")];
    let a = w.archive("downloads", "bain.7z", &files);
    let mut ui = Script { bain: Some(vec!["20 Lo".into(), "00 Core".into()]), ..Script::default() };
    w.install(&mut ui, &a, Some("Bain")).unwrap();
    // the dialog's defaults are now 00 Core and 20 Lo; taking them installs the same
    let mut ui = Script { clash: replace(), ..Script::default() };
    let out = w.reinstall(&mut ui, &a, "Bain").unwrap();
    assert!(out.replaced);
    assert_eq!(ui.asked[0], r#"bain ["00 Core", "10 Hi", "20 Lo"] [true, false, true]"#);
    assert_eq!(w.read("mods/Bain/textures/a.dds"), "lo");
    // without the previous mod: only 00
    let mut ui = Script::default();
    w.install(&mut ui, &a, Some("Other")).unwrap();
    assert_eq!(ui.asked[0], r#"bain ["00 Core", "10 Hi", "20 Lo"] [true, false, false]"#);
}

#[test]
fn reinstall_ticks_the_previous_fomod_choices() {
    need_7z!();
    let w = World::new();
    let a = fomod_archive(&w);
    let mut ui = Script { fomod: vec![vec![(0, 1, true)]], ..Script::default() };
    w.install(&mut ui, &a, Some("Low")).unwrap();
    let meta = w.read("mods/Low/meta.ini");
    assert!(meta.contains("Fomod%20Installer\\choices="), "{meta}");
    // no clicks this time: the 1K choice comes back, so Patches stays hidden
    let mut ui = Script { clash: replace(), ..Script::default() };
    w.reinstall(&mut ui, &a, "Low").unwrap();
    assert_eq!(ui.asked.iter().filter(|a| a.starts_with("step")).collect::<Vec<_>>(), ["step Size"]);
    assert_eq!(w.read("mods/Low/textures/a.dds"), "1k");
    // a fresh install of the same archive starts from the defaults
    let mut ui = Script::default();
    w.install(&mut ui, &a, Some("Fresh")).unwrap();
    assert_eq!(w.read("mods/Fresh/textures/a.dds"), "2k");
}

#[test]
fn reinstall_picks_the_previous_bundle_archive() {
    need_7z!();
    let w = World::new();
    let one = std::fs::read(w.archive("src", "One.zip", &[("One.esp", "1")])).unwrap();
    let two = std::fs::read(w.archive("src", "Two.zip", &[("Two.esp", "2")])).unwrap();
    let src = tempfile::tempdir_in(w.path("src")).unwrap();
    std::fs::write(src.path().join("One.zip"), one).unwrap();
    std::fs::write(src.path().join("Two.zip"), two).unwrap();
    let a = w.path("downloads/Both.7z");
    let ok = std::process::Command::new("7z").args(["a", "-bso0", "-bsp0"]).arg(&a).arg(".").current_dir(src.path()).status();
    assert!(ok.unwrap().success());
    let mut ui = Script { bundle: Some("Two.zip".into()), ..Script::default() };
    w.install(&mut ui, &a, Some("Pair")).unwrap();
    assert!(w.read("mods/Pair/meta.ini").contains("Bundle%20Installer\\archive=Two.zip"));
    let mut ui = Script { clash: replace(), ..Script::default() };
    w.reinstall(&mut ui, &a, "Pair").unwrap();
    assert!(!ui.asked.iter().any(|q| q.starts_with("bundle")), "{:?}", ui.asked);
    assert_eq!(w.tree("Pair"), ["Two.esp"]);
}

#[test]
fn password_protected_archive_asks_until_it_opens() {
    need_7z!();
    let w = World::new();
    let src = tempfile::tempdir_in(w.path("src")).unwrap();
    std::fs::write(src.path().join("Secret.esp"), "s").unwrap();
    let a = w.path("downloads/Locked.7z");
    let ok = std::process::Command::new("7z")
        .args(["a", "-bso0", "-bsp0", "-pHunter2", "-mhe=on"])
        .arg(&a)
        .arg(".")
        .current_dir(src.path())
        .status();
    assert!(ok.unwrap().success());
    let mut ui = Script { passwords: vec!["wrong".into(), "Hunter2".into()], ..Script::default() };
    let out = w.install(&mut ui, &a, None).unwrap();
    assert_eq!(&ui.asked[..2], ["password Locked.7z false", "password Locked.7z true"]);
    assert_eq!(w.tree(&out.name), ["Secret.esp"]);
    // no answer: cancelled, nothing installed
    let mut ui = Script::default();
    assert!(matches!(w.install(&mut ui, &a, Some("Again")), Err(strata_install::Error::Cancelled)));
    assert!(!w.path("mods/Again").exists());
}

/// A big archive (about 300 MB of incompressible data): cancelling at the first progress
/// report stops 7-Zip, leaves no mod folder and no extracted files behind.
#[test]
fn cancelling_a_big_extraction_leaves_nothing() {
    need_7z!();
    let w = World::new();
    let src = tempfile::tempdir_in(w.path("src")).unwrap();
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut chunk = vec![0u8; 1 << 20];
    for i in 0..3 {
        let mut data = Vec::with_capacity(100 << 20);
        for _ in 0..100 {
            for b in chunk.iter_mut() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *b = x as u8;
            }
            data.extend_from_slice(&chunk);
        }
        std::fs::create_dir_all(src.path().join("textures")).unwrap();
        std::fs::write(src.path().join(format!("textures/big{i}.dds")), &data).unwrap();
    }
    let a = w.path("downloads/Big.7z");
    let ok = std::process::Command::new("7z").args(["a", "-bso0", "-bsp0", "-mx1"]).arg(&a).arg(".").current_dir(src.path()).status();
    assert!(ok.unwrap().success());
    drop(src);

    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let mut ui = Script { cancel_on_progress: Some(cancel.clone()), ..Script::default() };
    let started = std::time::Instant::now();
    let request = Request { name: Some("Big".into()), ..Request::default() };
    let result = install(&w.target, &SevenZip, &Env::default(), &mut ui, &cancel, &a, &request);
    assert!(matches!(result, Err(strata_install::Error::Cancelled)), "{result:?}");
    assert!(!ui.progress.is_empty() && ui.progress.iter().all(|p| *p < 100), "{:?}", ui.progress);
    assert!(!w.path("mods/Big").exists(), "the new mod folder is removed");
    let left: Vec<_> = std::fs::read_dir(&w.target.temp_dir).map(|d| d.flatten().collect()).unwrap_or_default();
    assert!(left.is_empty(), "temp files left: {left:?}");
    assert!(!w.path("downloads/Big.7z.meta").exists(), "a cancelled install marks nothing");
    eprintln!("cancelled after {:?}", started.elapsed());
}

#[test]
fn ini_tweaks_folder_is_reported() {
    need_7z!();
    let w = World::new();
    let a = w.archive("downloads", "Tweaks.7z", &[("Tweaks.esp", "e"), ("INI Tweaks/Faster.ini", "[x]")]);
    assert!(w.install(&mut Script::default(), &a, None).unwrap().ini_tweaks);
    let b = w.archive("downloads", "Plain.7z", &[("Plain.esp", "e"), ("INI Tweaks/", "")]);
    assert!(!w.install(&mut Script::default(), &b, None).unwrap().ini_tweaks, "an empty folder does not count");
}

/// A download whose `.meta` cannot be written (a read-only downloads folder): the mod is
/// installed, the `.meta` is not written, and the outcome says so.
#[test]
fn an_unwritable_download_meta_does_not_fail_the_install() {
    use std::os::unix::fs::PermissionsExt;
    need_7z!();
    let w = World::new();
    let a = w.archive("downloads", "Locked Down.7z", &[("Locked.esp", "e")]);
    let dl = w.path("downloads");
    std::fs::set_permissions(&dl, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = w.install(&mut Script::default(), &a, None);
    std::fs::set_permissions(&dl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = result.unwrap();
    assert_eq!(w.tree(&out.name), ["Locked.esp"]);
    assert!(out.notes.iter().any(|n| n.contains("not marked installed")), "{:?}", out.notes);
    assert!(!w.path("downloads/Locked Down.7z.meta").exists());
}

/// An archive outside the instance's downloads folder: no `.meta` is written or changed
/// beside it (MO2 would mark it installed). Inside the downloads folder it is marked.
#[test]
fn only_downloads_in_the_downloads_folder_are_marked() {
    need_7z!();
    let w = World::new();
    // outside, with a .meta: left byte for byte, mtime and all
    let a = w.archive("elsewhere", "Outside-1-0.7z", &[("Outside.esp", "e")]);
    let meta = w.path("elsewhere/Outside-1-0.7z.meta");
    std::fs::write(&meta, "[General]\nmodID=1\ninstalled=false\n").unwrap();
    let before = (std::fs::read(&meta).unwrap(), std::fs::metadata(&meta).unwrap().modified().unwrap());
    std::thread::sleep(std::time::Duration::from_millis(20));
    let out = w.install(&mut Script::default(), &a, Some("Outside")).unwrap();
    assert_eq!(w.tree(&out.name), ["Outside.esp"]);
    assert!(out.notes.is_empty(), "{:?}", out.notes);
    assert_eq!((std::fs::read(&meta).unwrap(), std::fs::metadata(&meta).unwrap().modified().unwrap()), before);
    // outside, without a .meta: none is created
    let b = w.archive("elsewhere", "Bare.7z", &[("Bare.esp", "e")]);
    w.install(&mut Script::default(), &b, Some("Bare")).unwrap();
    assert!(!w.path("elsewhere/Bare.7z.meta").exists());
    let names: Vec<_> = std::fs::read_dir(w.path("elsewhere")).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(names.len(), 3, "only the two archives and the old .meta: {names:?}");
    // inside the downloads folder: marked installed, as MO2 does
    let c = w.archive("downloads", "Inside.7z", &[("Inside.esp", "e")]);
    w.install(&mut Script::default(), &c, Some("Inside")).unwrap();
    let text = std::fs::read_to_string(w.path("downloads/Inside.7z.meta")).unwrap();
    assert!(text.contains("installed=true") && text.contains("uninstalled=false"), "{text}");
}
