//! Waiting for a run to be over. The command Strata starts returns when the first program
//! exits (the SKSE loader, say); the game it started lives on in the prefix.

use std::path::Path;
use std::time::{Duration, Instant};

/// The processes running in the prefix whose `pfx` folder is `pfx`: `(pid, name)`.
/// Found by their `WINEPREFIX`; processes of other users cannot be read and are left out.
///
/// Proton sets `WINEPREFIX` with a trailing slash (`.../pfx/`), so trailing slashes don't
/// count. (Before this was known, a timed-out run left the whole prefix running.)
pub fn prefix_processes(pfx: &Path) -> Vec<(u32, String)> {
    processes_in(Path::new("/proc"), pfx)
}

fn processes_in(proc_root: &Path, pfx: &Path) -> Vec<(u32, String)> {
    let pfx = pfx.to_string_lossy();
    let wanted = pfx.trim_end_matches('/').as_bytes();
    let matches = |var: &[u8]| {
        var.strip_prefix(b"WINEPREFIX=").is_some_and(|value| {
            let end = value.iter().rposition(|b| *b != b'/').map_or(0, |i| i + 1);
            &value[..end] == wanted
        })
    };
    let Ok(entries) = std::fs::read_dir(proc_root) else { return Vec::new() };
    let mut found = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        if pid == std::process::id() {
            continue;
        }
        let Ok(environ) = std::fs::read(entry.path().join("environ")) else { continue };
        if environ.split(|b| *b == 0).any(matches) {
            let name = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
            found.push((pid, name.trim().to_owned()));
        }
    }
    found.sort();
    found
}

/// Blocks until no process runs in the prefix, checking every `interval`. Returns false
/// when `timeout` passed first.
pub fn wait_for_prefix(pfx: &Path, interval: Duration, timeout: Option<Duration>) -> bool {
    let started = Instant::now();
    loop {
        if prefix_processes(pfx).is_empty() {
            return true;
        }
        if timeout.is_some_and(|t| started.elapsed() > t) {
            return false;
        }
        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_slash_in_winepreix_still_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let process = |pid: &str, env: &str, comm: &str| {
            let dir = tmp.path().join(pid);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("environ"), env.replace(' ', "\0")).unwrap();
            std::fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
        };
        process("10", "A=1 WINEPREFIX=/p/pfx/", "wineserver");
        process("11", "WINEPREFIX=/p/pfx", "explorer.exe");
        process("12", "WINEPREFIX=/p/pfx2/", "other");
        process("13", "HOME=/p", "bash");
        let found = processes_in(tmp.path(), Path::new("/p/pfx"));
        assert_eq!(found, [(10, "wineserver".to_owned()), (11, "explorer.exe".to_owned())]);
        assert_eq!(processes_in(tmp.path(), Path::new("/p/pfx/")).len(), 2);
    }
}
