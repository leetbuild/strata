//! `strata clone-instance` on generated instances: every path of the ini and the prefix's
//! profile-ini symlinks repointed into the copy, an outside prefix refused or copied, a full
//! copy refused over the limit, and the source never written (its tree, contents, symlinks
//! and modification times are the same before and after).

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn strata(args: &[&str], env: &[(&str, &str)]) -> (bool, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_strata-headless"));
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// Everything about a tree that a write would change.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_owned()];
    while let Some(d) = stack.pop() {
        let meta = std::fs::symlink_metadata(&d).unwrap();
        out.insert(d.clone(), format!("dir {} {}", meta.mtime_nsec(), meta.mtime()));
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let m = std::fs::symlink_metadata(&p).unwrap();
            if m.file_type().is_symlink() {
                out.insert(p.clone(), format!("link {}", std::fs::read_link(&p).unwrap().display()));
            } else if m.is_dir() {
                stack.push(p);
            } else {
                let bytes = std::fs::read(&p).unwrap();
                out.insert(p.clone(), format!("file {} {} {:?}", m.mtime(), m.mtime_nsec(), bytes));
            }
        }
    }
    out
}

/// A generated instance `<dir>/Original` with a prefix (inside it or at `<dir>/SharedPrefix`)
/// holding Fluorine's profile-ini symlinks, and an ini naming the instance in every spelling.
fn instance(dir: &Path, outside_prefix: bool) -> (PathBuf, PathBuf) {
    let src = dir.join("Original");
    let generator = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/gen-instance");
    assert!(Command::new("python3").arg(&generator).arg(&src).stdout(Stdio::null()).status().unwrap().success());
    let profile = src.join("profiles/Default");
    std::fs::write(profile.join("skyrim.ini"), "[General]\n").unwrap();
    std::fs::write(profile.join("skyrimcustom.ini"), "").unwrap();
    std::fs::create_dir_all(src.join("tools")).unwrap();
    let prefix = if outside_prefix { dir.join("SharedPrefix") } else { src.join("Prefix") };
    let my_games = prefix.join("pfx/drive_c/users/steamuser/Documents/My Games/Skyrim Special Edition");
    std::fs::create_dir_all(&my_games).unwrap();
    std::fs::write(prefix.join("pfx/system.reg"), "WINE REGISTRY").unwrap();
    std::os::unix::fs::symlink(profile.join("skyrim.ini"), my_games.join("Skyrim.ini")).unwrap();
    std::os::unix::fs::symlink(profile.join("skyrimcustom.ini"), my_games.join("SkyrimCustom.ini")).unwrap();
    std::os::unix::fs::symlink("Skyrim.ini", my_games.join("relative.ini")).unwrap();

    let s = src.to_string_lossy();
    let w = format!("Z:{}", s.replace('/', "\\\\"));
    let elsewhere = dir.join("Elsewhere").to_string_lossy().replace('/', "\\\\");
    let ini = format!(
        "[General]\r\ngameName=Skyrim Special Edition\r\nselected_profile=@ByteArray(Default)\r\n\
         gamePath=@ByteArray({w}\\\\Game Root)\r\nversion=2.5.2\r\n\
         [customExecutables]\r\nsize=2\r\n1\\title=SKSE\r\n1\\binary={w}\\\\Game Root\\\\skse64_loader.exe\r\n\
         1\\workingDirectory=Z:{s}/Game Root\r\n1\\arguments=\r\n2\\title=SSEEdit\r\n\
         2\\binary={w}\\\\tools\\\\SSEEdit.exe\r\n2\\arguments=-D:\\\"{w}\\\\Game Root\\\\Data\\\" -IKnowWhatImDoing\r\n\
         2\\workingDirectory=\r\n\
         [Settings]\r\nstyle=vs15 Dark.qss\r\n\
         [recentDirectories]\r\nsize=2\r\n1\\name=editExecutableBinary\r\n1\\directory=Z:{s}/tools\r\n\
         2\\name=other\r\n2\\directory=Z:{elsewhere}\r\n\
         [Plugins]\r\nSome%20Unknown%20Plugin\\cache_path={s}/mods/SKSE\r\n\
         [fluorine]\r\nprefix_path={}\r\nproton_path=/nonexistent/GE-Proton10-14\r\n",
        prefix.display()
    );
    std::fs::write(src.join("ModOrganizer.ini"), ini).unwrap();
    (src, prefix)
}

fn scratch() -> tempfile::TempDir {
    // on the same file system as the build (btrfs on both PCs), not /tmp
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
}

/// "(reflink copy)" where the file system shares blocks (btrfs, XFS), else "(full copy)" (ext4,
/// the CI runners).
fn copy_kind(dir: &Path) -> &'static str {
    let a = dir.join(".reflink-probe");
    std::fs::write(&a, b"x").unwrap();
    let ok = std::process::Command::new("cp")
        .args(["--reflink=always"])
        .arg(&a)
        .arg(dir.join(".reflink-probe2"))
        .status()
        .is_ok_and(|s| s.success());
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(dir.join(".reflink-probe2"));
    if ok { "(reflink copy)" } else { "(full copy)" }
}

#[test]
fn clone_rewrites_every_path_and_relinks_without_touching_the_source() {
    let tmp = scratch();
    let (src, _) = instance(tmp.path(), false);
    let before = snapshot(tmp.path());
    let dst = tmp.path().join("Clone");
    let (ok, out) = strata(&["clone-instance", src.to_str().unwrap(), dst.to_str().unwrap()], &[]);
    assert!(ok, "{out}");
    assert!(out.contains(copy_kind(tmp.path())), "{out}");
    for key in [
        "gamePath",
        "[customExecutables] 1/binary",
        "[customExecutables] 1/workingDirectory",
        "[customExecutables] 2/binary",
        "[customExecutables] 2/arguments",
        "[recentDirectories] 1/directory",
        "[Plugins] Some Unknown Plugin/cache_path",
        "[fluorine] prefix_path",
    ] {
        assert!(out.contains(&format!("  {key}\n")), "{key} not reported:\n{out}");
    }
    assert!(out.contains("WARNING: points outside the copy: [recentDirectories] 2/directory"), "{out}");
    assert!(out.contains("WARNING: points outside the copy: [fluorine] proton_path"), "{out}");
    let ini = std::fs::read_to_string(dst.join("ModOrganizer.ini")).unwrap();
    assert!(!ini.contains("Original"), "{ini}");
    assert!(ini.contains("2\\arguments=-D:\\\"Z:"), "the quoted argument keeps its shape: {ini}");
    let my_games = dst.join("Prefix/pfx/drive_c/users/steamuser/Documents/My Games/Skyrim Special Edition");
    assert_eq!(std::fs::read_link(my_games.join("Skyrim.ini")).unwrap(), dst.join("profiles/Default/skyrim.ini"));
    assert_eq!(std::fs::read_link(my_games.join("SkyrimCustom.ini")).unwrap(), dst.join("profiles/Default/skyrimcustom.ini"));
    assert_eq!(std::fs::read_link(my_games.join("relative.ini")).unwrap(), PathBuf::from("Skyrim.ini"));
    assert!(out.contains("symlink repointed:"), "{out}");
    // the info the command ends with names the copy only
    let info = out.split("\n\n").last().unwrap();
    assert!(info.contains(&format!("instance        {}", dst.display())), "{info}");
    assert!(!info.contains("Original"), "{info}");
    let (ok, info) = strata(&["info", "-i", dst.to_str().unwrap()], &[]);
    assert!(ok && !info.contains("Original"), "{info}");
    assert_eq!(snapshot(&src), before.into_iter().filter(|(p, _)| p.starts_with(&src)).collect(), "the source changed");
    // a second clone onto an existing folder is refused
    let (ok, out) = strata(&["clone-instance", src.to_str().unwrap(), dst.to_str().unwrap()], &[]);
    assert!(!ok && out.contains("exists already"), "{out}");
}

#[test]
fn an_outside_prefix_is_refused_or_copied() {
    let tmp = scratch();
    let (src, prefix) = instance(tmp.path(), true);
    let before = snapshot(tmp.path());
    let dst = tmp.path().join("Clone");
    let (ok, out) = strata(&["clone-instance", src.to_str().unwrap(), dst.to_str().unwrap()], &[]);
    assert!(!ok && out.contains("prefix_path") && out.contains("--prefix-copy"), "{out}");
    assert!(!dst.exists(), "nothing is copied when refused");

    let (ok, out) = strata(&["clone-instance", src.to_str().unwrap(), dst.to_str().unwrap(), "--prefix-copy"], &[]);
    assert!(ok, "{out}");
    assert!(out.contains("copied the prefix") && out.contains(copy_kind(tmp.path())), "{out}");
    let ini = std::fs::read_to_string(dst.join("ModOrganizer.ini")).unwrap();
    assert!(ini.contains(&format!("prefix_path={}", dst.join("Prefix").display())), "{ini}");
    assert!(!ini.contains("SharedPrefix") && !ini.contains("Original"), "{ini}");
    let link = dst.join("Prefix/pfx/drive_c/users/steamuser/Documents/My Games/Skyrim Special Edition/Skyrim.ini");
    assert_eq!(std::fs::read_link(link).unwrap(), dst.join("profiles/Default/skyrim.ini"));
    let after = snapshot(tmp.path());
    for root in [&src, &prefix] {
        let pick = |m: &BTreeMap<PathBuf, String>| -> Vec<(PathBuf, String)> {
            m.iter().filter(|(p, _)| p.starts_with(root)).map(|(p, v)| (p.clone(), v.clone())).collect()
        };
        assert_eq!(pick(&after), pick(&before), "{} changed", root.display());
    }
}

#[test]
fn a_big_full_copy_needs_permission() {
    let tmp = scratch();
    let (src, _) = instance(tmp.path(), false);
    // /dev/shm is tmpfs: no reflinks from the build's btrfs
    let Ok(other) = tempfile::tempdir_in("/dev/shm") else {
        eprintln!("SKIPPED: a_big_full_copy_needs_permission: no writable /dev/shm");
        return;
    };
    let dst = other.path().join("Clone");
    let limit = [("STRATA_CLONE_FULL_COPY_LIMIT", "1000")];
    let (ok, out) = strata(&["clone-instance", src.to_str().unwrap(), dst.to_str().unwrap()], &limit);
    assert!(!ok && out.contains("full copy") && out.contains("--allow-full-copy"), "{out}");
    assert!(!dst.exists());
    let (ok, out) = strata(&["clone-instance", src.to_str().unwrap(), dst.to_str().unwrap(), "--allow-full-copy"], &limit);
    assert!(ok, "{out}");
    assert!(out.contains("(full copy)"), "{out}");
}

/// A shared prefix whose `My Games` inis link to ANOTHER instance's profile (seen on the desktop:
/// a shared prefix links to another instance's): repointed to the copy's profile; a link with no such
/// file in the copy's profile makes the command fail, and the other instance is never touched.
#[test]
fn prefix_links_into_another_instance_are_repointed_or_refused() {
    let tmp = scratch();
    let (src, prefix) = instance(tmp.path(), true);
    let other = tmp.path().join("Other/profiles/Default");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("SkyrimPrefs.ini"), "other").unwrap();
    std::fs::write(src.join("profiles/Default/skyrimprefs.ini"), "mine").unwrap();
    let my_games = prefix.join("pfx/drive_c/users/steamuser/Documents/My Games/Skyrim Special Edition");
    std::os::unix::fs::symlink(other.join("SkyrimPrefs.ini"), my_games.join("SkyrimPrefs.ini")).unwrap();
    let before = snapshot(tmp.path());
    let dst = tmp.path().join("Clone");
    let (ok, out) = strata(&["clone-instance", src.to_str().unwrap(), dst.to_str().unwrap(), "--prefix-copy"], &[]);
    assert!(ok, "{out}");
    assert!(out.contains("symlink repointed from another instance's profile"), "{out}");
    let copied = dst.join("Prefix/pfx/drive_c/users/steamuser/Documents/My Games/Skyrim Special Edition");
    assert_eq!(std::fs::read_link(copied.join("SkyrimPrefs.ini")).unwrap(), dst.join("profiles/Default/skyrimprefs.ini"));

    // a link the copy's profile has no file for
    std::os::unix::fs::symlink(other.join("Odd.ini"), my_games.join("Odd.ini")).unwrap();
    let before_odd = snapshot(tmp.path());
    let dst2 = tmp.path().join("Clone2");
    let (ok, out) = strata(&["clone-instance", src.to_str().unwrap(), dst2.to_str().unwrap(), "--prefix-copy"], &[]);
    assert!(!ok && out.contains("ERROR: the game would write through this symlink") && out.contains("Odd.ini"), "{out}");
    let after = snapshot(tmp.path());
    let outside_clones = |m: &BTreeMap<PathBuf, String>| -> Vec<(PathBuf, String)> {
        m.iter()
            .filter(|(p, _)| !p.starts_with(&dst) && !p.starts_with(&dst2) && p.as_path() != tmp.path())
            .map(|(p, v)| (p.clone(), v.clone()))
            .collect()
    };
    assert_eq!(outside_clones(&after), outside_clones(&before_odd), "source, shared prefix or other instance changed");
    assert!(before.len() < before_odd.len());
}
