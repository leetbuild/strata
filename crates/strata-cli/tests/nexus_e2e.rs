//! The nxm:// path end to end against a local fake Nexus: `strata nexus login --key-stdin`
//! (the key goes to a stand-in secret-tool), `strata nxm <link> --install` (file info, mod
//! info, download link with the link's key, the download, its `.meta`, the install), then
//! `strata nexus check-updates` and `status`.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn strata(env: &[(&str, &Path)], api: &str, args: &[&str], stdin: Option<&str>) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_strata-headless"));
    cmd.args(args).env("STRATA_NEXUS_API", api).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    if let Some(text) = stdin {
        child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

#[test]
fn nxm_link_downloads_and_installs() {
    if Command::new("7z").arg("i").stdout(Stdio::null()).status().is_err() {
        eprintln!("SKIPPED: nxm_link_downloads_and_installs: the 7z program is not installed");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let inst = tmp.path().join("inst");
    let generator = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/gen-instance");
    assert!(Command::new("python3").arg(&generator).arg(&inst).stdout(Stdio::null()).status().unwrap().success());

    // the mod archive the fake CDN serves
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("textures")).unwrap();
    std::fs::write(src.join("Cool.esp"), b"plugin").unwrap();
    std::fs::write(src.join("textures/c.dds"), b"dds").unwrap();
    let archive = tmp.path().join("cool.7z");
    let ok = Command::new("7z").args(["a", "-bso0", "-bsp0"]).arg(&archive).arg(".").current_dir(&src).status().unwrap();
    assert!(ok.success());
    let server = strata_nexus::fake::serve();
    server.add(strata_nexus::fake::nexus_file_routes(
        &server.base,
        "skyrimspecialedition",
        5,
        77,
        "Cool Mod",
        "Cool Mod-5-1-2-1700000000.7z",
        "1.2",
        std::fs::read(&archive).unwrap(),
    ));
    let secret = strata_nexus::keystore::fake_secret_tool(tmp.path()).unwrap();
    let env = [("STRATA_SECRET_TOOL", secret.as_path())];
    let api = server.api();
    let i = inst.to_str().unwrap();

    let (ok, out) = strata(&env, &api, &["nexus", "status"], None);
    assert!(!ok && out.contains("strata nexus login"), "{out}");
    let (ok, out) = strata(&env, &api, &["nexus", "login", "--key-stdin"], Some("KEY-9\n"));
    assert!(ok, "{out}");
    assert!(out.contains("logged in as tester"), "{out}");
    assert_eq!(std::fs::read_to_string(tmp.path().join("secret")).unwrap(), "KEY-9");

    let link = "nxm://skyrimspecialedition/mods/5/files/77?key=abc&expires=1800000000&user_id=1";
    let (ok, out) = strata(&env, &api, &["nxm", link, "--install", "-i", i], None);
    assert!(ok, "{out}");
    let downloads = inst.join("downloads");
    assert!(downloads.join("Cool Mod-5-1-2-1700000000.7z").is_file(), "{out}");
    let meta = std::fs::read_to_string(downloads.join("Cool Mod-5-1-2-1700000000.7z.meta")).unwrap();
    assert!(meta.contains("installed=true") && meta.contains("modID=5") && meta.contains("fileID=77"), "{meta}");
    let mod_dir = inst.join("mods/Cool Mod");
    assert!(mod_dir.join("Cool.esp").is_file() && mod_dir.join("textures/c.dds").is_file(), "{out}");
    let mod_meta = std::fs::read_to_string(mod_dir.join("meta.ini")).unwrap();
    for line in ["modid=5", "version=1.2.0.0", "1\\fileid=77", "category=\""] {
        assert!(mod_meta.contains(line), "{line} in {mod_meta}");
    }
    let modlist = std::fs::read_to_string(inst.join("profiles/Default/modlist.txt")).unwrap();
    assert!(modlist.contains("+Cool Mod"), "{modlist}");
    let requests = server.requests.lock().unwrap().clone();
    assert!(requests.iter().any(|r| r.contains("download_link?key=abc&expires=1800000000 apikey=KEY-9")), "{requests:?}");

    // a newer version on Nexus: the update check writes newestVersion
    let mut newer = strata_nexus::fake::nexus_file_routes(&server.base, "skyrimspecialedition", 5, 77, "Cool Mod", "x.7z", "1.3", vec![]);
    newer.retain(|path, _| path == "/v1/games/skyrimspecialedition/mods/5");
    server.add(newer);
    let (ok, out) = strata(&env, &api, &["nexus", "check-updates", "-i", i], None);
    assert!(ok, "{out}");
    assert!(out.contains("Cool Mod: newest 1.3"), "{out}");
    let mod_meta = std::fs::read_to_string(mod_dir.join("meta.ini")).unwrap();
    assert!(mod_meta.contains("newestVersion=1.3") && mod_meta.contains("lastNexusQuery=20"), "{mod_meta}");

    let (ok, out) = strata(&env, &api, &["nexus", "status"], None);
    assert!(ok && out.contains("tester") && out.contains("2499 today"), "{out}");
    assert!(!out.contains("KEY-9"), "the key is never printed");
    let (ok, _) = strata(&env, &api, &["nexus", "logout"], None);
    assert!(ok && !tmp.path().join("secret").exists());
}
