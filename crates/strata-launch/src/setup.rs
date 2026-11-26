//! Prefix setup (`spec/04-proton.md`, rule 4): an explicit list of steps that make a prefix
//! fit for Skyrim and its tools. Every step checks before it acts, so a second run on a
//! working prefix changes nothing (it does not even start Wine).
//!
//! Lessons and sources from Fluorine's `prefixsetuprunner.cpp`: the prefix is made by
//! `proton waitforexitandrun wineboot -u` inside the Proton's runtime (`stepProtonInit`), with
//! ntsync off for that call; downloads are pinned by sha256 (its URLs and hashes, re-checked
//! 2026-11-24); Microsoft installers run with `/install /quiet /norestart` and `mshtml`
//! disabled; DLL overrides are set before an installer runs. What Strata leaves out on
//! purpose: FAudio and the XACT/XAudio cabinets (rule 3: Proton's own audio), drive letter
//! clean-up, Windows 11 mode, the .NET SDK and NuGet policy, game detection.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::{Proton, ProtonCall, Steam, wait_for_prefix};

/// Why a step failed. Each message says what to do.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("download of {url} failed: {reason}. Check the network and run the setup again.")]
    Download { url: String, reason: String },
    #[error(
        "{file} has sha256 {got}, expected {want}. The download is not the file Strata knows; \
         it was not used. Run the setup again; if it persists, the URL serves another file."
    )]
    Checksum { file: String, got: String, want: String },
    #[error("{0}")]
    Launch(#[from] crate::Error),
    #[error("{what}: {source}")]
    Io { what: String, source: std::io::Error },
    #[error("{step}: {program} ended with code {code}. See the setup log.")]
    ExitCode { step: String, program: String, code: i32 },
    #[error("{step}: the step ran but its check still fails ({detail}). See the setup log.")]
    NotDone { step: String, detail: String },
    #[error("{step}: programs still run in the prefix after {seconds} s. See the setup log.")]
    Timeout { step: String, seconds: u64 },
}

type Result<T> = std::result::Result<T, SetupError>;

fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> SetupError {
    let what = what.into();
    move |source| SetupError::Io { what, source }
}

/// A file Strata downloads, pinned by its sha256.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Download {
    pub url: &'static str,
    pub sha256: &'static str,
}

impl Download {
    /// The file name the cache keeps it under (the URL's last part).
    pub fn file_name(&self) -> &'static str {
        self.url.rsplit('/').next().unwrap_or(self.url)
    }
}

/// The lowercase hex sha256 of a file.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::Digest;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Where downloads are kept: `$XDG_CACHE_HOME/strata/downloads` (shared by all prefixes).
pub fn cache_dir() -> PathBuf {
    strata_core::dirs::cache_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("strata/downloads")
}

// Pinned files. VC, d3dcompiler_47 and DirectX: Fluorine's URLs and hashes, downloaded again
// and checked 2026-11-24. .NET: Microsoft's release metadata of 2026-11-10 (its sha512
// checked against the download, the sha256 computed from it).
pub const VC_X86: Download = Download {
    url: "https://download.visualstudio.microsoft.com/download/pr/355d2512-13c2-400a-bf9f-8a296abb5932/F0BAB33A302B3CDB2E11113760D016F54FD3D2632C65BA7834FAC4F0ABD7F1A3/VC_redist.x86.exe",
    sha256: "f0bab33a302b3cdb2e11113760d016f54fd3d2632c65ba7834fac4f0abd7f1a3",
};
pub const VC_X64: Download = Download {
    url: "https://download.visualstudio.microsoft.com/download/pr/ebdab8e5-1d7b-4d9f-a11b-cbb1720c3b12/843068991DAAA1F73AD9F6239BCE4D0F6A07A51F18C37EA2A867E9BECA71295C/VC_redist.x64.exe",
    sha256: "843068991daaa1f73ad9f6239bce4d0f6a07a51f18c37ea2a867e9beca71295c",
};
pub const D3DCOMPILER_47_32: Download = Download {
    url: "https://github.com/mozilla/fxc2/raw/master/dll/d3dcompiler_47_32.dll",
    sha256: "2ad0d4987fc4624566b190e747c9d95038443956ed816abfd1e2d389b5ec0851",
};
pub const D3DCOMPILER_47_64: Download = Download {
    url: "https://github.com/mozilla/fxc2/raw/master/dll/d3dcompiler_47.dll",
    sha256: "4432bbd1a390874f3f0a503d45cc48d346abc3a8c0213c289f4b615bf0ee84f3",
};
pub const DIRECTX_JUN2010: Download = Download {
    url: "https://download.microsoft.com/download/8/4/a/84a35bf1-dafe-4ae8-82af-ad2ae20b6b14/directx_Jun2010_redist.exe",
    sha256: "053f76dcbb28802e23341b6a787e3b0791c0fa5c8d4d011b1044172dbf89c73b",
};
pub const DOTNET8_DESKTOP: Download = Download {
    url: "https://builds.dotnet.microsoft.com/dotnet/WindowsDesktop/8.0.34/windowsdesktop-runtime-8.0.34-win-x64.exe",
    sha256: "37aa8a02acc1973ddd7e3de993a02007ccef26b48fa35b09d087fa8ab4602795",
};
pub const DOTNET9_DESKTOP: Download = Download {
    url: "https://builds.dotnet.microsoft.com/dotnet/WindowsDesktop/9.0.23/windowsdesktop-runtime-9.0.23-win-x64.exe",
    sha256: "eefb60bd0f3fb33c0787074cebc0467ffba32bcb01088613121649fc3bdb4a27",
};

/// The DLLs `vcrun2022` makes native-first (winetricks' `load_vcrun2022` list): without the
/// overrides Wine keeps its own builtin copies, which lack what new SKSE plugins call.
const VC_DLLS: &[&str] = &[
    "concrt140",
    "msvcp140",
    "msvcp140_1",
    "msvcp140_2",
    "msvcp140_atomic_wait",
    "msvcp140_codecvt_ids",
    "vcamp140",
    "vccorlib140",
    "vcomp140",
    "vcruntime140",
    "vcruntime140_1",
];

/// The d3dx DLLs taken from the DirectX June 2010 redistributable (what winetricks' `d3dx9`,
/// `d3dx10` and `d3dx11_43` install), for both architectures, and the shader compilers of
/// their era (`d3dcompiler_42`, `d3dcompiler_43`, in Fluorine's `DIRECTX_NATIVE_DLLS` too):
/// the native d3dx DLLs compile shaders through them, and Wine's builtin compiler under a
/// native d3dx9 made ENB crash Skyrim LE in a fresh prefix.
pub fn d3dx_dlls() -> Vec<String> {
    let mut names: Vec<String> = (24..=43).map(|v| format!("d3dx9_{v}")).collect();
    names.extend((33..=43).map(|v| format!("d3dx10_{v}")));
    names.extend(["d3dx11_42", "d3dx11_43", "d3dcompiler_42", "d3dcompiler_43"].map(String::from));
    names
}

/// What a step needs to run Wine and fetch files.
pub struct Context<'a> {
    pub proton: &'a Proton,
    pub steam: &'a Steam,
    /// The folder that holds (or will hold) `pfx/`.
    pub prefix: &'a Path,
    pub app_id: u32,
    pub cache: PathBuf,
    /// Every step's output and every command line go here.
    pub log: &'a mut dyn Write,
}

impl Context<'_> {
    fn pfx(&self) -> PathBuf {
        self.prefix.join("pfx")
    }

    fn note(&mut self, text: &str) {
        let _ = writeln!(self.log, "{text}");
    }

    /// The file in the cache, downloaded with `curl` when missing or wrong, then checked.
    pub fn fetch(&mut self, download: &Download) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.cache).map_err(io(format!("cannot create {}", self.cache.display())))?;
        let path = self.cache.join(download.file_name());
        if sha256_file(&path).is_ok_and(|h| h == download.sha256) {
            self.note(&format!("cached: {} (sha256 ok)", path.display()));
            return Ok(path);
        }
        self.note(&format!("downloading {}", download.url));
        let temp = self.cache.join(format!(".{}.download", download.file_name()));
        let status = std::process::Command::new("curl")
            .args(["--fail", "--silent", "--show-error", "--location", "--max-time", "900", "--output"])
            .arg(&temp)
            .arg(download.url)
            .status()
            .map_err(|e| SetupError::Download { url: download.url.into(), reason: format!("cannot run curl: {e}") })?;
        if !status.success() {
            let _ = std::fs::remove_file(&temp);
            return Err(SetupError::Download { url: download.url.into(), reason: format!("curl ended with {status}") });
        }
        let got = sha256_file(&temp).map_err(io("cannot read the download"))?;
        if got != download.sha256 {
            let _ = std::fs::remove_file(&temp);
            return Err(SetupError::Checksum { file: download.file_name().into(), got, want: download.sha256.into() });
        }
        std::fs::rename(&temp, &path).map_err(io(format!("cannot move the download to {}", path.display())))?;
        Ok(path)
    }

    /// Runs `proton <verb> <args>` in the runtime, waits for it and for the prefix to go
    /// quiet, and returns the exit code. Output goes to the log.
    fn proton(&mut self, step: &str, verb: &'static str, args: &[&str], existing: bool) -> Result<i32> {
        let call = ProtonCall {
            proton: self.proton,
            steam: self.steam,
            prefix: self.prefix,
            verb,
            args: args.iter().map(Into::into).collect(),
            app_id: self.app_id,
            expose: vec![self.cache.clone()],
        };
        let mut plan = call.plan(existing)?;
        // No Gecko download prompt from installers (Fluorine and winetricks do the same).
        plan.env.insert("WINEDLLOVERRIDES".into(), "mshtml=d".into());
        // Proton's own log stays quiet; the step log gets the output.
        let _ = writeln!(self.log, "--- {step}\n{}", plan.describe());
        let _ = self.log.flush();
        let output = plan
            .command()
            .stdin(Stdio::null())
            .output()
            .map_err(io(format!("cannot start {}", plan.program.display())))?;
        let _ = self.log.write_all(&output.stdout);
        let _ = self.log.write_all(&output.stderr);
        let code = output.status.code().unwrap_or(-1);
        let _ = writeln!(self.log, "--- {step}: exit code {code}");
        const WAIT: u64 = 600;
        if !wait_for_prefix(&self.pfx(), Duration::from_millis(500), Some(Duration::from_secs(WAIT))) {
            return Err(SetupError::Timeout { step: step.into(), seconds: WAIT });
        }
        Ok(code)
    }

    /// Sets DLL overrides with `regedit`, winetricks style (`"*name"="mode"`).
    fn set_overrides(&mut self, step: &str, dlls: &[String], mode: &str) -> Result<()> {
        let mut reg = String::from("Windows Registry Editor Version 5.00\r\n\r\n[HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides]\r\n");
        for dll in dlls {
            reg.push_str(&format!("\"*{dll}\"=\"{mode}\"\r\n"));
        }
        let file = self.cache.join(format!(".overrides-{}.reg", std::process::id()));
        std::fs::write(&file, reg).map_err(io(format!("cannot write {}", file.display())))?;
        let wine_path = to_wine_path(&file);
        let code = self.proton(step, "runinprefix", &["regedit", "/S", &wine_path], true);
        let _ = std::fs::remove_file(&file);
        match code? {
            0 => Ok(()),
            code => Err(SetupError::ExitCode { step: step.into(), program: "regedit".into(), code }),
        }
    }
}

/// `/home/x/a b.exe` → `Z:\home\x\a b.exe`.
fn to_wine_path(path: &Path) -> String {
    format!("Z:{}", path.to_string_lossy().replace('/', "\\"))
}

/// The `"name"=value` lines of a registry section in one of Wine's `.reg` files, names and
/// key compared without case. Key as written in regedit (`Software\Wine\DllOverrides`).
pub fn reg_values(reg_file: &str, key: &str) -> Vec<(String, String)> {
    let wanted = format!("[{}]", key.replace('\\', "\\\\")).to_lowercase();
    let mut out = Vec::new();
    let mut inside = false;
    for line in reg_file.lines() {
        if line.starts_with('[') {
            // `[Software\\Wine\\DllOverrides] 1795313173`
            let header = line.split_once("] ").map_or(line.to_owned(), |(h, _)| format!("{h}]"));
            inside = header.to_lowercase() == wanted;
            continue;
        }
        if inside
            && let Some(rest) = line.strip_prefix('"')
            && let Some((name, value)) = rest.split_once("\"=")
        {
            out.push((name.to_owned(), value.to_owned()));
        }
    }
    out
}

fn read_reg(pfx: &Path, file: &str) -> String {
    std::fs::read(pfx.join(file)).map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default()
}

/// The overrides in `dlls` that `user.reg` lacks or has with another mode.
fn missing_overrides(pfx: &Path, dlls: &[String], mode: &str) -> Vec<String> {
    let have = reg_values(&read_reg(pfx, "user.reg"), "Software\\Wine\\DllOverrides");
    let want = format!("\"{mode}\"");
    dlls.iter()
        .filter(|dll| {
            !have.iter().any(|(n, v)| {
                (n.eq_ignore_ascii_case(&format!("*{dll}")) || n.eq_ignore_ascii_case(dll)) && v == &want
            })
        })
        .cloned()
        .collect()
}

/// A DWORD value of a registry key in `system.reg`.
fn reg_dword(pfx: &Path, key: &str, name: &str) -> Option<u32> {
    reg_values(&read_reg(pfx, "system.reg"), key)
        .into_iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .and_then(|(_, v)| u32::from_str_radix(v.strip_prefix("dword:")?, 16).ok())
}

/// A real DLL, not one of Wine's placeholder files (they carry "Wine builtin DLL" in their
/// DOS stub, which is how winetricks tells them apart too).
fn is_real_dll(path: &Path) -> bool {
    let mut head = [0u8; 512];
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    let n = std::io::Read::read(&mut file, &mut head).unwrap_or(0);
    n >= 64 && !head[..n].windows(16).any(|w| w == b"Wine builtin DLL")
}

/// One setup step.
pub trait Step {
    /// A short name for the CLI and the log: `vcrun2022`.
    fn id(&self) -> &'static str;
    /// What it does, for people.
    fn title(&self) -> &'static str;
    /// `None` when done; otherwise what is missing. Reads files only: never starts Wine.
    fn missing(&self, pfx: &Path) -> Option<String>;
    /// Does the work.
    fn apply(&self, ctx: &mut Context<'_>) -> Result<()>;
}

/// Creates the prefix with the chosen Proton (`proton waitforexitandrun wineboot -u`).
pub struct CreatePrefix;

impl Step for CreatePrefix {
    fn id(&self) -> &'static str {
        "prefix"
    }
    fn title(&self) -> &'static str {
        "create the Wine prefix with the chosen Proton"
    }
    fn missing(&self, pfx: &Path) -> Option<String> {
        let ok = pfx.join("system.reg").is_file()
            && pfx.join("user.reg").is_file()
            && pfx.join("drive_c/windows/system32/kernel32.dll").is_file()
            && pfx.join("drive_c/windows/syswow64").is_dir();
        (!ok).then(|| "no prefix yet".to_owned())
    }
    fn apply(&self, ctx: &mut Context<'_>) -> Result<()> {
        std::fs::create_dir_all(ctx.prefix).map_err(io(format!("cannot create {}", ctx.prefix.display())))?;
        let code = ctx.proton(self.id(), "waitforexitandrun", &["wineboot", "-u"], false)?;
        if code != 0 {
            return Err(SetupError::ExitCode { step: self.id().into(), program: "wineboot".into(), code });
        }
        Ok(())
    }
}

/// Visual C++ 2015-2022 (v14) x86 and x64, with its DLLs native-first.
pub struct VcRuntime;

/// The build number of the pinned VC_redist (14.51.36247).
const VC_BUILD: u32 = 36_247;

impl Step for VcRuntime {
    fn id(&self) -> &'static str {
        "vcrun2022"
    }
    fn title(&self) -> &'static str {
        "Visual C++ 2015-2022 runtime (x86 and x64)"
    }
    fn missing(&self, pfx: &Path) -> Option<String> {
        let dlls: Vec<String> = VC_DLLS.iter().map(|s| (*s).to_owned()).collect();
        let mut gaps = Vec::new();
        for (key, arch) in [
            ("Software\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x64", "x64"),
            ("Software\\Wow6432Node\\Microsoft\\VisualStudio\\14.0\\VC\\Runtimes\\x86", "x86"),
        ] {
            let installed = reg_dword(pfx, key, "Installed") == Some(1);
            let build = reg_dword(pfx, key, "Bld").unwrap_or(0);
            if !installed || build < VC_BUILD {
                gaps.push(format!("{arch} runtime (build {build}, want {VC_BUILD})"));
            }
        }
        if !missing_overrides(pfx, &dlls, "native,builtin").is_empty() {
            gaps.push("DLL overrides".into());
        }
        (!gaps.is_empty()).then(|| gaps.join(", "))
    }
    fn apply(&self, ctx: &mut Context<'_>) -> Result<()> {
        let dlls: Vec<String> = VC_DLLS.iter().map(|s| (*s).to_owned()).collect();
        let pfx = ctx.pfx();
        let missing = missing_overrides(&pfx, &dlls, "native,builtin");
        if !missing.is_empty() {
            // Before the installer, as winetricks and Fluorine do.
            ctx.set_overrides(self.id(), &missing, "native,builtin")?;
        }
        for download in [VC_X86, VC_X64] {
            let exe = ctx.fetch(&download)?;
            let wine_path = to_wine_path(&exe);
            let code = ctx.proton(self.id(), "runinprefix", &[&wine_path, "/install", "/quiet", "/norestart"], true)?;
            if !installer_ok(code) {
                return Err(SetupError::ExitCode { step: self.id().into(), program: download.file_name().into(), code });
            }
        }
        Ok(())
    }
}

/// Microsoft installer exit codes that mean success: done, done but reboot wanted, or a newer
/// version is already there.
fn installer_ok(code: i32) -> bool {
    matches!(code, 0 | 1638 | 1641 | 3010)
}

/// DLLs copied into `system32` (64-bit) and `syswow64` (32-bit), then made native.
pub struct NativeDlls {
    pub id: &'static str,
    pub title: &'static str,
}

impl NativeDlls {
    fn names(&self) -> Vec<String> {
        match self.id {
            "d3dcompiler_47" => vec!["d3dcompiler_47".into()],
            _ => d3dx_dlls(),
        }
    }
}

/// The d3dcompiler_47 step.
pub const D3DCOMPILER_47: NativeDlls = NativeDlls { id: "d3dcompiler_47", title: "d3dcompiler_47 (x86 and x64)" };
/// The d3dx9/10/11 step.
pub const D3DX: NativeDlls =
    NativeDlls { id: "d3dx", title: "d3dx9, d3dx10, d3dx11, d3dcompiler_42/43 from DirectX June 2010 (x86 and x64)" };

impl Step for NativeDlls {
    fn id(&self) -> &'static str {
        self.id
    }
    fn title(&self) -> &'static str {
        self.title
    }
    fn missing(&self, pfx: &Path) -> Option<String> {
        let windows = pfx.join("drive_c/windows");
        let names = self.names();
        let mut absent: Vec<String> = Vec::new();
        for name in &names {
            for dir in ["system32", "syswow64"] {
                let path = windows.join(dir).join(format!("{name}.dll"));
                let good = if self.id == "d3dcompiler_47" {
                    let want = if dir == "system32" { D3DCOMPILER_47_64.sha256 } else { D3DCOMPILER_47_32.sha256 };
                    sha256_file(&path).is_ok_and(|h| h == want)
                } else {
                    is_real_dll(&path)
                };
                if !good {
                    absent.push(format!("{dir}/{name}.dll"));
                }
            }
        }
        let overrides = missing_overrides(pfx, &names, "native");
        if absent.is_empty() && overrides.is_empty() {
            return None;
        }
        let mut gaps = Vec::new();
        if !absent.is_empty() {
            gaps.push(format!("{} DLL files ({}...)", absent.len(), absent[0]));
        }
        if !overrides.is_empty() {
            gaps.push(format!("{} DLL overrides", overrides.len()));
        }
        Some(gaps.join(", "))
    }
    fn apply(&self, ctx: &mut Context<'_>) -> Result<()> {
        let windows = ctx.pfx().join("drive_c/windows");
        if self.id == "d3dcompiler_47" {
            for (download, dir) in [(D3DCOMPILER_47_64, "system32"), (D3DCOMPILER_47_32, "syswow64")] {
                let file = ctx.fetch(&download)?;
                copy_dll(&file, &windows.join(dir).join("d3dcompiler_47.dll"))?;
            }
        } else {
            extract_d3dx(ctx, &windows)?;
        }
        let names = self.names();
        let missing = missing_overrides(&ctx.pfx(), &names, "native");
        if !missing.is_empty() {
            ctx.set_overrides(self.id, &missing, "native")?;
        }
        Ok(())
    }
}

/// Replaces `dest` (Wine's placeholder, if any) with `source`, through a temporary file.
fn copy_dll(source: &Path, dest: &Path) -> Result<()> {
    let temp = dest.with_extension("strata-tmp");
    std::fs::copy(source, &temp).map_err(io(format!("cannot copy to {}", temp.display())))?;
    std::fs::rename(&temp, dest).map_err(io(format!("cannot replace {}", dest.display())))?;
    Ok(())
}

/// Takes the d3dx DLLs out of the DirectX redistributable with `7z` (it reads the
/// self-extracting cabinet and the inner cabinets; no Wine, no DXSETUP, no audio cabinets).
fn extract_d3dx(ctx: &mut Context<'_>, windows: &Path) -> Result<()> {
    let redist = ctx.fetch(&DIRECTX_JUN2010)?;
    let work = ctx.cache.join(format!(".directx-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    let sevenzip = |args: &[&std::ffi::OsStr]| -> Result<()> {
        let output = std::process::Command::new("7z")
            .args(args)
            .output()
            .map_err(|e| SetupError::Io { what: "cannot run 7z (install p7zip)".into(), source: e })?;
        if output.status.success() {
            Ok(())
        } else {
            Err(SetupError::ExitCode { step: "d3dx".into(), program: "7z".into(), code: output.status.code().unwrap_or(-1) })
        }
    };
    let out_flag = format!("-o{}", work.display());
    let result = (|| -> Result<()> {
        sevenzip(&[
            "x".as_ref(),
            "-y".as_ref(),
            out_flag.as_ref(),
            redist.as_os_str(),
            "*d3dx*.cab".as_ref(),
            "*D3DCompiler*.cab".as_ref(),
        ])?;
        let cabs = std::fs::read_dir(&work).map_err(io("cannot list the DirectX cabinets"))?;
        let mut cabs: Vec<PathBuf> = cabs.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        cabs.sort();
        let names = d3dx_dlls();
        let mut placed = 0;
        for cab in cabs {
            let cab_name = cab.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            let dir = if cab_name.ends_with("_x64.cab") {
                "system32"
            } else if cab_name.ends_with("_x86.cab") {
                "syswow64"
            } else {
                continue;
            };
            let inner = work.join(format!("{cab_name}.d"));
            let flag = format!("-o{}", inner.display());
            sevenzip(&["x".as_ref(), "-y".as_ref(), flag.as_ref(), cab.as_os_str(), "*.dll".as_ref()])?;
            for dll in std::fs::read_dir(&inner).map_err(io("cannot list a cabinet"))?.filter_map(|e| e.ok()) {
                let file = dll.file_name().to_string_lossy().to_lowercase();
                let stem = file.trim_end_matches(".dll");
                if names.iter().any(|n| n == stem) {
                    copy_dll(&dll.path(), &windows.join(dir).join(&file))?;
                    placed += 1;
                }
            }
        }
        ctx.note(&format!("d3dx: placed {placed} DLL files"));
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// A .NET Desktop Runtime (x64), for the tools that need it (Synthesis, Pandora, ...).
pub struct DotNet {
    pub id: &'static str,
    pub title: &'static str,
    pub download: Download,
    /// `8.0`.
    pub line: &'static str,
    /// The pinned patch release: 34 for 8.0.34.
    pub patch: u32,
}

/// .NET 8 Desktop Runtime.
pub const DOTNET8: DotNet =
    DotNet { id: "dotnet8", title: ".NET 8 Desktop Runtime (x64)", download: DOTNET8_DESKTOP, line: "8.0", patch: 34 };
/// .NET 9 Desktop Runtime.
pub const DOTNET9: DotNet =
    DotNet { id: "dotnet9", title: ".NET 9 Desktop Runtime (x64)", download: DOTNET9_DESKTOP, line: "9.0", patch: 23 };

impl Step for DotNet {
    fn id(&self) -> &'static str {
        self.id
    }
    fn title(&self) -> &'static str {
        self.title
    }
    fn missing(&self, pfx: &Path) -> Option<String> {
        let shared = pfx.join("drive_c/Program Files/dotnet/shared");
        let ok = ["Microsoft.NETCore.App", "Microsoft.WindowsDesktop.App"].iter().all(|framework| {
            std::fs::read_dir(shared.join(framework)).into_iter().flatten().flatten().any(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_prefix(&format!("{}.", self.line))
                    .and_then(|p| p.parse::<u32>().ok())
                    .is_some_and(|p| p >= self.patch)
            })
        });
        (!ok).then(|| format!("no {}.{} or newer", self.line, self.patch))
    }
    fn apply(&self, ctx: &mut Context<'_>) -> Result<()> {
        let exe = ctx.fetch(&self.download)?;
        let wine_path = to_wine_path(&exe);
        let code = ctx.proton(self.id, "runinprefix", &[&wine_path, "/install", "/quiet", "/norestart"], true)?;
        if !installer_ok(code) {
            return Err(SetupError::ExitCode { step: self.id.into(), program: self.download.file_name().into(), code });
        }
        Ok(())
    }
}

/// Skyrim SE's step list, in order.
pub fn skyrim_steps() -> Vec<Box<dyn Step>> {
    vec![
        Box::new(CreatePrefix),
        Box::new(VcRuntime),
        Box::new(D3DCOMPILER_47),
        Box::new(D3DX),
        Box::new(DOTNET8),
        Box::new(DOTNET9),
    ]
}

/// What happened to one step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Its check passed: nothing was done.
    AlreadyDone,
    /// It ran and its check passes now.
    Done,
    /// `check` only: what is missing.
    Missing(String),
}

/// Runs the steps in order (or only checks them). A step that ran must pass its check
/// afterwards; the first failure stops the run. `progress` hears about each step.
pub fn run(
    steps: &[Box<dyn Step>],
    ctx: &mut Context<'_>,
    check_only: bool,
    progress: &mut dyn FnMut(&dyn Step, &str),
) -> Result<Vec<(&'static str, Outcome)>> {
    let pfx = ctx.pfx();
    let mut report = Vec::new();
    for step in steps {
        let outcome = match step.missing(&pfx) {
            None => Outcome::AlreadyDone,
            Some(gap) if check_only => Outcome::Missing(gap),
            Some(gap) => {
                progress(step.as_ref(), &format!("running ({gap})"));
                ctx.note(&format!("=== {}: {} (missing: {gap})", step.id(), step.title()));
                step.apply(ctx)?;
                if let Some(still) = step.missing(&pfx) {
                    return Err(SetupError::NotDone { step: step.id().into(), detail: still });
                }
                Outcome::Done
            }
        };
        ctx.note(&format!("=== {}: {:?}", step.id(), outcome));
        progress(step.as_ref(), match &outcome {
            Outcome::AlreadyDone => "already done",
            Outcome::Done => "done",
            Outcome::Missing(_) => "missing",
        });
        report.push((step.id(), outcome));
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
