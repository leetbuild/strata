//! Handing nxm:// links from the browser to a running window (MO2's `nxmhandler.exe` and its
//! single-instance check: a link goes to the MO2 that runs, or starts one).
//!
//! Each window listens on a Unix socket `<runtime dir>/strata/window-<pid>.sock`. The browser
//! runs `strata link <url>` (the `strata-links.desktop` entry), which offers the link to each
//! socket: one line with the link; the window answers one line, `ok` when it queued the
//! download or `no <why>` (another game's link, for one). A socket nobody listens on is left
//! from a window that died: it is removed. When no window takes the link, `strata link`
//! starts the window on the last instance (when that is for the link's game) and offers the
//! link again once the window listens.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use strata_nexus::links::{self, Link};

/// The folder of the windows' sockets: `$XDG_RUNTIME_DIR/strata` (only the user can read
/// it), else a folder of the user's own in the temporary folder.
pub fn socket_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        Some(run) => PathBuf::from(run).join("strata"),
        None => {
            let user = std::env::var("USER").unwrap_or_else(|_| "user".to_owned());
            std::env::temp_dir().join(format!("strata-{user}"))
        }
    }
}

/// The socket of the window of process `pid`; creates the folder (mode 700).
pub fn socket_path(pid: u32) -> std::io::Result<PathBuf> {
    let dir = socket_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    Ok(dir.join(format!("window-{pid}.sock")))
}

/// The Nexus game of a link to a file (`skyrimspecialedition`), `None` for anything else.
pub fn link_game(link: &str) -> Option<String> {
    match links::parse(link).ok()? {
        Link::NexusFile { game, .. } => Some(game),
        _ => None,
    }
}

/// Whether a window of an instance of `nexus_name` takes `link`: `Err` says why not.
pub fn accepts(nexus_name: &str, link: &str) -> Result<(), String> {
    match link_game(link) {
        Some(game) if game.eq_ignore_ascii_case(nexus_name) => Ok(()),
        Some(game) => Err(format!("the link is for {game}, this window manages {nexus_name}")),
        None => Err("only nxm:// links to a file can be downloaded".to_owned()),
    }
}

/// What offering a link to the running windows gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offered {
    /// A window queued it (its socket).
    Taken(PathBuf),
    /// No window took it; the windows' reasons (empty when none runs).
    NotTaken(Vec<String>),
}

/// Offers `link` to one socket: `Ok(None)` when it took it, `Ok(Some(why))` when it refused,
/// `Err` when nobody listens there.
fn offer_to(socket: &Path, link: &str) -> std::io::Result<Option<String>> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(format!("{}\n", link.trim()).as_bytes())?;
    let mut answer = String::new();
    BufReader::new(&stream).read_line(&mut answer)?;
    let answer = answer.trim();
    Ok(if answer == "ok" { None } else { Some(answer.strip_prefix("no ").unwrap_or(answer).to_owned()) })
}

/// Offers `link` to every running window, newest socket first; removes sockets nobody listens
/// on.
pub fn offer(link: &str) -> Offered {
    let mut sockets: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(socket_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sock"))
        .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
        .collect();
    sockets.sort_by(|a, b| b.0.cmp(&a.0));
    let mut reasons = Vec::new();
    for (_, socket) in sockets {
        match offer_to(&socket, link) {
            Ok(None) => return Offered::Taken(socket),
            Ok(Some(why)) => reasons.push(why),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound) => {
                let _ = std::fs::remove_file(&socket);
            }
            Err(e) => reasons.push(format!("{}: {e}", socket.display())),
        }
    }
    Offered::NotTaken(reasons)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    const LINK: &str = "nxm://skyrimspecialedition/mods/5/files/77?key=k&expires=1900000000&user_id=9";

    #[test]
    fn a_window_takes_its_game_and_refuses_others() {
        assert_eq!(accepts("skyrimspecialedition", LINK), Ok(()));
        assert!(accepts("skyrim", LINK).unwrap_err().contains("skyrimspecialedition"));
        assert!(accepts("skyrim", "nxm://skyrim/collections/abc").is_err());
        assert_eq!(link_game(LINK).as_deref(), Some("skyrimspecialedition"));
    }

    /// A fake window answering `answer`; returns what it was sent.
    fn window(socket: &Path, answer: &'static str) -> std::thread::JoinHandle<String> {
        let listener = UnixListener::bind(socket).unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            (&stream).write_all(answer.as_bytes()).unwrap();
            line
        })
    }

    #[test]
    fn offers_go_to_each_socket_and_stale_ones_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        // the environment is the process's: this is the only test that sets it
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", dir.path()) };
        let sockets = socket_dir();
        std::fs::create_dir_all(&sockets).unwrap();
        assert_eq!(offer(LINK), Offered::NotTaken(vec![]), "no window runs");

        // a socket left by a dead window: removed
        drop(UnixListener::bind(sockets.join("window-1.sock")).unwrap());
        let refusing = window(&sockets.join("window-2.sock"), "no another game\n");
        assert_eq!(offer(LINK), Offered::NotTaken(vec!["another game".into()]));
        assert_eq!(refusing.join().unwrap().trim(), LINK);
        assert!(!sockets.join("window-1.sock").exists());

        std::fs::remove_file(sockets.join("window-2.sock")).unwrap();
        let taking = window(&sockets.join("window-3.sock"), "ok\n");
        assert_eq!(offer(LINK), Offered::Taken(sockets.join("window-3.sock")));
        assert_eq!(taking.join().unwrap().trim(), LINK);
        assert_eq!(socket_path(42).unwrap(), sockets.join("window-42.sock"));
    }
}
