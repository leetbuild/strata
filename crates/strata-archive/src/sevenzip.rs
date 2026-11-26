//! Reading and extracting 7z / zip / rar (and every other format 7-Zip knows) through the installed
//! `7z` program.
//!
//! The counterpart of MO2's `archive` library (`libs/archive` in Fluorine: `ArchiveImpl::open`,
//! `resetFileList`, `extract`, `cancel`), which loads 7-Zip's `7z.so` itself. Strata runs the `7z`
//! binary instead: the same engine, rar5 included, and
//! no C++ in the crate.
//!
//! The model is MO2's: open lists the entries, the caller (the installer) decides where each
//! entry goes, extraction writes them there. An entry can go to several places, or nowhere.
//!
//! Extraction runs `7z x` on the selected entries into a staging folder inside the output folder,
//! then moves each file to its output path (a rename: same file system). The staging folder is
//! removed at the end, also on error or cancel.

use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// What went wrong reading or extracting an archive.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("7-Zip is not installed (no `7z` or `7zz` in PATH): install the `7zip` package")]
    NotInstalled,
    #[error("archive not found: {0}")]
    NotFound(PathBuf),
    #[error("{0} is encrypted: give its password")]
    PasswordNeeded(PathBuf),
    #[error("wrong password for {0}")]
    WrongPassword(PathBuf),
    #[error("{0} is not an archive 7-Zip can open (or it is damaged): {1}")]
    CannotOpen(PathBuf, String),
    #[error(
        "{archive} holds an unsafe path ({path:?}: absolute, a drive letter or `..`): refusing it"
    )]
    UnsafePath { archive: PathBuf, path: String },
    #[error("entry {0} is out of range")]
    NoSuchEntry(usize),
    #[error("unsafe output path {0:?}: it must be relative, without `..`")]
    UnsafeOutput(String),
    #[error("7-Zip failed on {archive}: {message}")]
    Failed { archive: PathBuf, message: String },
    #[error("7-Zip did not write {path:?} of {archive}: the archive may be damaged")]
    Missing { archive: PathBuf, path: String },
    #[error("extraction cancelled")]
    Cancelled,
    #[error("{0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One entry of an archive, as `7z l -slt` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The path inside the archive, `/`-separated, without `.` or empty parts. Backslashes are
    /// separators too (zips made on Windows), as Fluorine's `safeRelativeOutputPath` treats them.
    pub path: String,
    /// The name exactly as 7-Zip reports it: what selects the entry and where it lands in staging.
    raw: String,
    /// Uncompressed size in bytes.
    pub size: u64,
    pub crc: Option<u32>,
    pub is_dir: bool,
    pub encrypted: bool,
}

/// Extraction progress, from 7-Zip's own percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub percent: u8,
}

/// An opened archive: its path, password and entry list. Nothing stays open between calls.
#[derive(Debug, Clone)]
pub struct Archive {
    path: PathBuf,
    password: Option<String>,
    entries: Vec<Entry>,
    program: PathBuf,
}

impl Archive {
    /// Lists the archive. With `password: None` an archive whose entry names are encrypted fails
    /// with [`Error::PasswordNeeded`]; one that only encrypts the data opens, and extraction then
    /// fails the same way (MO2 asks for a rar's password at extraction too). Ask the user and open
    /// again with the password.
    pub fn open(path: &Path, password: Option<&str>) -> Result<Self> {
        let program = find_7z()?;
        let path = std::path::absolute(path).map_err(|e| Error::Io(path.to_owned(), e))?;
        if !path.is_file() {
            return Err(Error::NotFound(path));
        }
        let out = command(&program, password)
            .args(["l", "-slt"])
            .arg(&path)
            .output()
            .map_err(|e| Error::Io(program.clone(), e))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() {
            return Err(classify_failure(&path, password, &format!("{stderr}\n{}", listing_errors(&stdout))));
        }
        let entries = parse_listing(&stdout);
        if let Some(e) = entries.iter().find(|e| e.path.is_empty()) {
            return Err(Error::UnsafePath { archive: path, path: e.raw.clone() });
        }
        Ok(Self { path, password: password.map(str::to_owned), entries, program })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Extracts every entry to its own path under `out_dir` (folders too, empty ones included).
    pub fn extract_all(
        &self,
        out_dir: &Path,
        progress: &mut dyn FnMut(Progress),
        cancel: &AtomicBool,
    ) -> Result<()> {
        let plan: Vec<(usize, String)> =
            self.entries.iter().enumerate().map(|(i, e)| (i, e.path.clone())).collect();
        self.extract(out_dir, &plan, progress, cancel)
    }

    /// Extracts the entries of `plan`, each `(entry index, output path relative to out_dir)`.
    /// An entry may appear more than once (it is copied), a folder entry makes an empty folder.
    /// Entries not in the plan are not decompressed. Existing files are replaced, as MO2's
    /// `CArchiveExtractCallback::GetStream` does.
    pub fn extract(
        &self,
        out_dir: &Path,
        plan: &[(usize, String)],
        progress: &mut dyn FnMut(Progress),
        cancel: &AtomicBool,
    ) -> Result<()> {
        let mut outputs: HashMap<usize, Vec<String>> = HashMap::new();
        for (index, to) in plan {
            if *index >= self.entries.len() {
                return Err(Error::NoSuchEntry(*index));
            }
            let to = safe_relative(to).ok_or_else(|| Error::UnsafeOutput(to.clone()))?;
            outputs.entry(*index).or_default().push(to);
        }
        let io = |p: &Path| {
            let p = p.to_owned();
            move |e| Error::Io(p, e)
        };
        std::fs::create_dir_all(out_dir).map_err(io(out_dir))?;
        let staging = tempfile::Builder::new()
            .prefix(".strata-extract-")
            .tempdir_in(out_dir)
            .map_err(io(out_dir))?;

        let mut files: Vec<usize> =
            outputs.keys().copied().filter(|i| !self.entries[*i].is_dir).collect();
        files.sort_unstable();
        if !files.is_empty() {
            let list = staging.path().join(".list");
            let mut text = String::new();
            for i in &files {
                text.push_str(&self.entries[*i].raw);
                text.push('\n');
            }
            std::fs::write(&list, text).map_err(io(&list))?;
            let unpacked = staging.path().join("x");
            self.run_extract(&unpacked, &list, progress, cancel)?;
        }

        // Folders first (an entry's folder may also be an explicit folder entry), then files,
        // grouped by name: an archive may hold one name twice, and 7-Zip writes it once.
        let mut indices: Vec<usize> = outputs.keys().copied().collect();
        indices.sort_unstable();
        let mut by_name: Vec<(&Entry, Vec<&String>)> = Vec::new();
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for i in indices {
            let entry = &self.entries[i];
            if entry.is_dir {
                for to in &outputs[&i] {
                    let dest = out_dir.join(to);
                    std::fs::create_dir_all(&dest).map_err(io(&dest))?;
                }
                continue;
            }
            let slot = *seen.entry(&entry.raw).or_insert_with(|| {
                by_name.push((entry, Vec::new()));
                by_name.len() - 1
            });
            by_name[slot].1.extend(&outputs[&i]);
        }
        for (entry, targets) in by_name {
            let source = staging.path().join("x").join(&entry.raw);
            if std::fs::symlink_metadata(&source).is_err() {
                return Err(Error::Missing { archive: self.path.clone(), path: entry.path.clone() });
            }
            for (n, to) in targets.iter().enumerate() {
                let dest = out_dir.join(to);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent).map_err(io(parent))?;
                }
                if n + 1 == targets.len() {
                    std::fs::rename(&source, &dest).map_err(io(&dest))?;
                } else {
                    std::fs::copy(&source, &dest).map_err(io(&dest))?;
                }
            }
        }
        staging.close().map_err(io(out_dir))
    }

    /// Runs `7z x` for the names in `list` into `dest`, reporting progress, killing 7-Zip when
    /// `cancel` is set.
    fn run_extract(
        &self,
        dest: &Path,
        list: &Path,
        progress: &mut dyn FnMut(Progress),
        cancel: &AtomicBool,
    ) -> Result<()> {
        let mut child = command(&self.program, self.password.as_deref())
            // -spd: names are literal (no wildcards); -aoa: overwrite; -bsp1: progress on stdout.
            .args(["x", "-spd", "-aoa", "-bso0", "-bsp1", "-bse2"])
            .arg(format!("-o{}", dest.display()))
            .arg(&self.path)
            .arg(format!("@{}", list.display()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Io(self.program.clone(), e))?;
        let result = watch(&mut child, progress, cancel);
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let (status, stderr) = result?;
        // Exit code 1 is a warning (7-Zip's "non fatal error"); missing files are caught later.
        if status.code().is_some_and(|c| c <= 1) {
            return Ok(());
        }
        Err(classify_failure(&self.path, self.password.as_deref(), &stderr))
    }
}

/// Reads 7-Zip's progress until it exits; returns its status and stderr.
fn watch(
    child: &mut Child,
    progress: &mut dyn FnMut(Progress),
    cancel: &AtomicBool,
) -> Result<(std::process::ExitStatus, String)> {
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let err_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = BufReader::new(&mut stderr).read_to_string(&mut text);
        text
    });
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let out_reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut pending = Vec::new();
    let mut last = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                pending.extend_from_slice(&chunk);
                // 7-Zip redraws its progress with backspaces; take the newest complete number.
                while let Some(end) = pending.iter().position(|b| matches!(b, b'\x08' | b'\r' | b'\n')) {
                    let piece: Vec<u8> = pending.drain(..=end).collect();
                    if let Some(percent) = parse_percent(&piece)
                        && last != Some(percent)
                    {
                        last = Some(percent);
                        progress(Progress { percent });
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = out_reader.join();
    let status = child.wait().map_err(|e| Error::Io(PathBuf::from("7z"), e))?;
    let stderr = err_reader.join().unwrap_or_default();
    if last != Some(100) && status.success() {
        progress(Progress { percent: 100 });
    }
    Ok((status, stderr))
}

/// Packs everything in `dir` (hidden files and empty folders too) into a new `archive`, its type
/// from the extension (`.7z`, `.zip`), with 7-Zip's default compression: `7z a <archive> .`
/// run in `dir` (a repack). `archive` must not exist.
pub fn create(archive: &Path, dir: &Path) -> Result<()> {
    let program = find_7z()?;
    let archive = std::path::absolute(archive).map_err(|e| Error::Io(archive.to_owned(), e))?;
    let kind = match archive.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("zip") => "-tzip",
        _ => "-t7z",
    };
    // not command(): a bare -p makes `7z a` ask for a password
    let out = Command::new(&program)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .current_dir(dir)
        .args(["a", kind, "-y", "-scsUTF-8"])
        .arg(&archive)
        .arg(".")
        .output()
        .map_err(|e| Error::Io(program.clone(), e))?;
    if !out.status.success() {
        let message = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(Error::Failed { archive, message });
    }
    Ok(())
}

/// `"  45% 3 - name"` → 45.
fn parse_percent(piece: &[u8]) -> Option<u8> {
    let text = String::from_utf8_lossy(piece);
    let (number, _) = text.trim_start().split_once('%')?;
    number.trim().parse().ok().filter(|p| *p <= 100)
}

/// The `7z` (p7zip / 7-Zip) or `7zz` (7-Zip's own Linux build) in PATH.
fn find_7z() -> Result<PathBuf> {
    let path = std::env::var_os("PATH").ok_or(Error::NotInstalled)?;
    for name in ["7z", "7zz"] {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(Error::NotInstalled)
}

/// A `7z` command that never prompts: stdin is closed and a password is always given (an empty
/// `-p` means "no password"), names are UTF-8.
fn command(program: &Path, password: Option<&str>) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .arg(format!("-p{}", password.unwrap_or("")))
        .arg("-scsUTF-8");
    cmd
}

fn classify_failure(archive: &Path, password: Option<&str>, stderr: &str) -> Error {
    let archive = archive.to_owned();
    if stderr.contains("Wrong password") || stderr.contains("Cannot open encrypted archive") {
        return match password {
            None | Some("") => Error::PasswordNeeded(archive),
            Some(_) => Error::WrongPassword(archive),
        };
    }
    let message = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    // "Cannot open the file as archive", "... as [7z] archive", "Is not archive".
    if stderr.contains("Cannot open the file as") || stderr.contains("Is not archive") {
        return Error::CannotOpen(archive, message);
    }
    Error::Failed { archive, message }
}

/// The lines 7-Zip's listing prints under `ERRORS:` on stdout (a damaged archive: "Unexpected end
/// of archive"); its stderr is empty then.
fn listing_errors(stdout: &str) -> String {
    let mut lines = stdout.lines().skip_while(|l| l.trim() != "ERRORS:").skip(1);
    let errors: Vec<&str> = lines.by_ref().take_while(|l| !l.trim().is_empty() && !l.contains(" = ")).collect();
    errors.join("\n")
}

/// Parses `7z l -slt`: blocks of `Key = Value` lines after the `----------` line, one per entry.
fn parse_listing(text: &str) -> Vec<Entry> {
    let Some((_, body)) = text.split_once("\n----------\n") else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    let mut fields: HashMap<&str, &str> = HashMap::new();
    let mut flush = |fields: &mut HashMap<&str, &str>| {
        if let Some(raw) = fields.get("Path") {
            // `Folder` (zip, rar) or the `D` Windows attribute (7z).
            let is_dir = match fields.get("Folder") {
                Some(f) => *f == "+",
                None => fields
                    .get("Attributes")
                    .and_then(|a| a.split(' ').next())
                    .is_some_and(|a| a.contains('D')),
            };
            entries.push(Entry {
                path: safe_relative(raw).unwrap_or_default(),
                raw: (*raw).to_owned(),
                size: fields.get("Size").and_then(|s| s.parse().ok()).unwrap_or(0),
                crc: fields.get("CRC").and_then(|c| u32::from_str_radix(c, 16).ok()),
                is_dir,
                encrypted: fields.get("Encrypted") == Some(&"+"),
            });
        }
        fields.clear();
    };
    for line in body.lines() {
        if line.is_empty() {
            flush(&mut fields);
        } else if let Some((key, value)) = line.split_once(" = ") {
            fields.insert(key, value);
        } else if let Some(key) = line.strip_suffix(" =") {
            fields.insert(key, "");
        }
    }
    flush(&mut fields);
    entries
}

/// Fluorine's `safeRelativeOutputPath` (`libs/archive/src/extractcallback.cpp`): `\` → `/`,
/// `.` and empty parts dropped; `None` for an absolute path, a drive letter, `..` or nothing.
pub fn safe_relative(path: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    if path.starts_with('/') {
        return None;
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            p => {
                let b = p.as_bytes();
                if parts.is_empty() && b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
                    return None;
                }
                parts.push(p);
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(test)]
mod tests;
