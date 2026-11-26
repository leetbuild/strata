use super::*;

fn put(path: &Path, data: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

const USER_REG: &str = "WINE REGISTRY Version 2\n;; All keys relative to \\\\User\\\\S-1-5-21\n\n\
[Software\\\\Wine\\\\DllOverrides] 1795313173\n#time=1dd7a260d2e8254\n\"*d3dx9_43\"=\"native\"\n\"msvcp140\"=\"native,builtin\"\n\n\
[Software\\\\Wine\\\\Other] 1\n\"*d3dx9_42\"=\"native\"\n";

#[test]
fn registry_values_are_read_per_key() {
    let values = reg_values(USER_REG, "Software\\Wine\\DllOverrides");
    assert_eq!(values, [("*d3dx9_43".to_owned(), "\"native\"".to_owned()), ("msvcp140".to_owned(), "\"native,builtin\"".to_owned())]);
    assert!(reg_values(USER_REG, "software\\wine\\other").len() == 1, "keys compare without case");
}

#[test]
fn overrides_count_with_or_without_the_star_and_only_in_the_right_mode() {
    let tmp = tempfile::tempdir().unwrap();
    put(&tmp.path().join("user.reg"), USER_REG.as_bytes());
    let want = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(missing_overrides(tmp.path(), &want(&["d3dx9_43", "d3dx9_42"]), "native"), ["d3dx9_42"]);
    assert!(missing_overrides(tmp.path(), &want(&["msvcp140"]), "native,builtin").is_empty());
    assert_eq!(missing_overrides(tmp.path(), &want(&["msvcp140"]), "native"), ["msvcp140"]);
}

#[test]
fn the_vc_check_reads_both_architectures_and_the_overrides() {
    let tmp = tempfile::tempdir().unwrap();
    let pfx = tmp.path();
    let system = |build: u32| {
        format!(
            "[Software\\\\Microsoft\\\\VisualStudio\\\\14.0\\\\VC\\\\Runtimes\\\\X64] 1\n\"Installed\"=dword:00000001\n\"Bld\"=dword:{build:08x}\n\n\
             [Software\\\\Wow6432Node\\\\Microsoft\\\\VisualStudio\\\\14.0\\\\VC\\\\Runtimes\\\\X86] 1\n\"Installed\"=dword:00000001\n\"Bld\"=dword:{build:08x}\n"
        )
    };
    put(&pfx.join("system.reg"), system(VC_BUILD).as_bytes());
    let overrides: String = VC_DLLS.iter().map(|d| format!("\"*{d}\"=\"native,builtin\"\n")).collect();
    put(&pfx.join("user.reg"), format!("[Software\\\\Wine\\\\DllOverrides] 1\n{overrides}").as_bytes());
    assert_eq!(VcRuntime.missing(pfx), None);
    put(&pfx.join("system.reg"), system(VC_BUILD - 1).as_bytes());
    assert!(VcRuntime.missing(pfx).unwrap().contains("x64 runtime"));
    put(&pfx.join("system.reg"), system(VC_BUILD + 5).as_bytes());
    put(&pfx.join("user.reg"), b"");
    assert_eq!(VcRuntime.missing(pfx).as_deref(), Some("DLL overrides"));
}

#[test]
fn wine_placeholders_are_not_real_dlls() {
    let tmp = tempfile::tempdir().unwrap();
    let mut fake = vec![0u8; 128];
    fake[..2].copy_from_slice(b"MZ");
    fake[64..80].copy_from_slice(b"Wine builtin DLL");
    put(&tmp.path().join("fake.dll"), &fake);
    put(&tmp.path().join("real.dll"), &[b'M'; 600]);
    assert!(!is_real_dll(&tmp.path().join("fake.dll")));
    assert!(is_real_dll(&tmp.path().join("real.dll")));
    assert!(!is_real_dll(&tmp.path().join("absent.dll")));
}

#[test]
fn dotnet_counts_a_newer_patch_of_its_line() {
    let tmp = tempfile::tempdir().unwrap();
    let shared = tmp.path().join("drive_c/Program Files/dotnet/shared");
    assert!(DOTNET8.missing(tmp.path()).is_some());
    std::fs::create_dir_all(shared.join("Microsoft.NETCore.App/8.0.40")).unwrap();
    assert!(DOTNET8.missing(tmp.path()).is_some(), "the desktop part is missing");
    std::fs::create_dir_all(shared.join("Microsoft.WindowsDesktop.App/8.0.33")).unwrap();
    assert!(DOTNET8.missing(tmp.path()).is_some(), "too old");
    std::fs::create_dir_all(shared.join("Microsoft.WindowsDesktop.App/8.0.40")).unwrap();
    assert_eq!(DOTNET8.missing(tmp.path()), None);
    assert!(DOTNET9.missing(tmp.path()).is_some());
}

#[test]
fn d3dx_lists_every_version() {
    let names = d3dx_dlls();
    assert_eq!(names.len(), 20 + 11 + 2 + 2);
    assert!(names.contains(&"d3dcompiler_43".to_owned()));
    assert_eq!(names.first().map(String::as_str), Some("d3dx9_24"));
    assert!(names.contains(&"d3dx11_43".to_owned()));
}

fn context<'a>(root: &Path, proton: &'a Proton, steam: &'a Steam, log: &'a mut Vec<u8>) -> Context<'a> {
    Context { proton, steam, prefix: Box::leak(root.join("Prefix").into_boxed_path()), app_id: 1, cache: root.join("cache"), log }
}

fn fake_proton(root: &Path) -> (Proton, Steam) {
    put(&root.join("Proton/proton"), b"");
    std::fs::create_dir_all(root.join("Steam/steamapps")).unwrap();
    (Proton::at(&root.join("Proton")).unwrap(), Steam::at(&root.join("Steam")))
}

#[test]
fn downloads_are_checked_and_cached() {
    let tmp = tempfile::tempdir().unwrap();
    let (proton, steam) = fake_proton(tmp.path());
    let mut log = Vec::new();
    let mut ctx = context(tmp.path(), &proton, &steam, &mut log);
    put(&tmp.path().join("src/file.bin"), b"hello");
    let url: &'static str = Box::leak(format!("file://{}", tmp.path().join("src/file.bin").display()).into_boxed_str());
    let good = Download { url, sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824" };
    let path = ctx.fetch(&good).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    std::fs::remove_file(tmp.path().join("src/file.bin")).unwrap();
    assert_eq!(ctx.fetch(&good).unwrap(), path, "the cached copy is used");

    put(&tmp.path().join("src/other.bin"), b"not hello");
    let url: &'static str = Box::leak(format!("file://{}", tmp.path().join("src/other.bin").display()).into_boxed_str());
    let bad = Download { url, sha256: good.sha256 };
    assert!(matches!(ctx.fetch(&bad), Err(SetupError::Checksum { .. })));
    assert!(!ctx.cache.join("other.bin").exists(), "a file with the wrong sum is not kept");
}

struct Fake {
    done: std::cell::Cell<bool>,
    fail_after: bool,
}

impl Step for Fake {
    fn id(&self) -> &'static str {
        "fake"
    }
    fn title(&self) -> &'static str {
        "a fake step"
    }
    fn missing(&self, _: &Path) -> Option<String> {
        (!self.done.get()).then(|| "not yet".into())
    }
    fn apply(&self, _: &mut Context<'_>) -> Result<()> {
        self.done.set(!self.fail_after);
        Ok(())
    }
}

#[test]
fn steps_run_once_and_must_pass_their_check() {
    let tmp = tempfile::tempdir().unwrap();
    let (proton, steam) = fake_proton(tmp.path());
    let mut log = Vec::new();
    let mut ctx = context(tmp.path(), &proton, &steam, &mut log);
    let steps: Vec<Box<dyn Step>> = vec![Box::new(Fake { done: false.into(), fail_after: false })];
    let mut seen = Vec::new();
    let check = run(&steps, &mut ctx, true, &mut |_, s| seen.push(s.to_owned())).unwrap();
    assert_eq!(check[0].1, Outcome::Missing("not yet".into()));
    assert_eq!(run(&steps, &mut ctx, false, &mut |_, _| {}).unwrap()[0].1, Outcome::Done);
    assert_eq!(run(&steps, &mut ctx, false, &mut |_, _| {}).unwrap()[0].1, Outcome::AlreadyDone);

    let broken: Vec<Box<dyn Step>> = vec![Box::new(Fake { done: false.into(), fail_after: true })];
    assert!(matches!(run(&broken, &mut ctx, false, &mut |_, _| {}), Err(SetupError::NotDone { .. })));
}
