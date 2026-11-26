//! `strata vfsd`: the process that holds an instance's mounts, and its socket protocol.
//!
//! The mounts live in their own process so that a crash of the window never takes the
//! game's files away mid-run. One JSON object per line each way, over a Unix socket
//! (`docs/ARCHITECTURE.md`, "vfsd protocol").

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::mount::{Mount, MountSpec};

/// The protocol version, sent as `"v"` in every message.
pub const VERSION: u32 = 1;

/// What a client can ask.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    /// Mount all of these, or none.
    Mount { mounts: Vec<MountSpec> },
    /// What is mounted.
    Status,
    /// Which layer and real file a path of a mount resolves to.
    Resolve {
        path: String,
        /// The mount's id; the first mount when absent.
        #[serde(default)]
        mount: Option<String>,
    },
    /// Show a new layer list in the mount `id` without unmounting (a mod toggled or
    /// moved, another profile).
    Refresh { id: String, layers: Vec<strata_game::Layer> },
    /// Unmount everything and exit. Without `force`, busy mounts are an error and stay.
    Unmount {
        #[serde(default)]
        force: bool,
    },
}

/// The socket of the daemon for an instance: `$XDG_RUNTIME_DIR/strata/<hash>.sock`.
pub fn socket_path(instance: &Path) -> PathBuf {
    // FNV-1a: a stable name for the path, nothing more.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in instance.as_os_str().as_encoded_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    runtime.join("strata").join(format!("{hash:016x}.sock"))
}

/// Sends one request and returns the reply. `Err` when no daemon answers on `socket`; a
/// daemon's own refusal comes back as `Ok` with `"ok": false` and an `"error"` text.
pub fn request(socket: &Path, request: &Request) -> io::Result<Value> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    let mut line = serde_json::to_value(request)?;
    line["v"] = json!(VERSION);
    writeln!(stream, "{line}")?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    serde_json::from_str(&reply).map_err(io::Error::other)
}

/// Whether a daemon answers on `socket`.
pub fn is_running(socket: &Path) -> bool {
    request(socket, &Request::Status).is_ok()
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_signal: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

struct Daemon {
    mounts: Vec<Mount>,
    threads: usize,
    started: Instant,
}

impl Daemon {
    fn handle(&mut self, request: Request) -> (Value, bool) {
        match request {
            Request::Mount { mounts } => {
                if !self.mounts.is_empty() {
                    return (error("already mounted: unmount first"), false);
                }
                let mut made = Vec::new();
                for spec in mounts {
                    let id = spec.id.clone();
                    match Mount::new(spec, self.threads) {
                        Ok(mount) => made.push(mount),
                        Err(e) => {
                            // All or nothing: take down what was mounted so far.
                            for mut mount in made {
                                let _ = mount.unmount(true);
                            }
                            return (error(&format!("cannot mount \"{id}\": {e}")), false);
                        }
                    }
                }
                self.mounts = made;
                (self.status(), false)
            }
            Request::Status => (self.status(), false),
            Request::Resolve { path, mount } => {
                let found = match &mount {
                    Some(id) => self.mounts.iter().find(|m| m.id == *id),
                    None => self.mounts.first(),
                };
                let Some(mount) = found else { return (error("no such mount"), false) };
                let tree = mount.tree.read().expect("tree");
                match tree.resolve(path.trim_matches('/')) {
                    Some(id) => {
                        let node = tree.node(id).expect("resolved node");
                        (
                            json!({
                                "v": VERSION, "ok": true, "found": true,
                                "path": tree.path(id),
                                "kind": if node.is_dir() { "dir" } else { "file" },
                                "origin": tree.layer_of(id).map(|l| l.id.clone()),
                                "real": tree.real_path(id),
                            }),
                            false,
                        )
                    }
                    None => (json!({ "v": VERSION, "ok": true, "found": false }), false),
                }
            }
            Request::Refresh { id, layers } => {
                let Some(mount) = self.mounts.iter().find(|m| m.id == id) else {
                    return (error(&format!("no mount \"{id}\"")), false);
                };
                match mount.refresh(layers) {
                    Ok(report) => (
                        json!({
                            "v": VERSION, "ok": true, "id": id, "nodes": report.nodes,
                            "changed": report.changed, "entries": report.entries,
                        }),
                        false,
                    ),
                    Err(e) => (error(&format!("cannot refresh \"{id}\": {e}")), false),
                }
            }
            Request::Unmount { force } => {
                // Highest mount last in, first out; stop at the first that is busy.
                while let Some(mount) = self.mounts.last_mut() {
                    if let Err(e) = mount.unmount(force) {
                        return (error(&format!("{e}. Close the programs using it, or force the unmount.")), false);
                    }
                    self.mounts.pop();
                }
                (json!({ "v": VERSION, "ok": true }), true)
            }
        }
    }

    fn status(&self) -> Value {
        let mounts: Vec<Value> = self
            .mounts
            .iter()
            .map(|m| {
                let tree = m.tree.read().expect("tree");
                json!({ "id": m.id, "mountpoint": m.mountpoint, "label": m.label, "layers": tree.layers().len(), "nodes": tree.len() })
            })
            .collect();
        json!({
            "v": VERSION, "ok": true, "pid": std::process::id(),
            "uptime_secs": self.started.elapsed().as_secs(), "mounts": mounts,
        })
    }
}

fn error(text: &str) -> Value {
    json!({ "v": VERSION, "ok": false, "error": text })
}

fn serve_client(daemon: &mut Daemon, stream: UnixStream) -> io::Result<bool> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut writer = stream.try_clone()?;
    for line in BufReader::new(stream).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let (reply, stop) = match serde_json::from_str::<Request>(&line) {
            Ok(request) => daemon.handle(request),
            Err(e) => (error(&format!("bad request: {e}")), false),
        };
        writeln!(writer, "{reply}")?;
        if stop {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Runs the daemon on `socket` until it is told to unmount, or gets SIGTERM or SIGINT
/// (then it unmounts, lazily if it must, and exits).
pub fn serve(socket: &Path, threads: usize) -> io::Result<()> {
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if socket.exists() {
        if is_running(socket) {
            return Err(io::Error::other(format!("a daemon is already serving {}", socket.display())));
        }
        std::fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    listener.set_nonblocking(true)?;
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
    }

    let mut daemon = Daemon { mounts: Vec::new(), threads, started: Instant::now() };
    eprintln!("vfsd: serving {}", socket.display());
    let result = loop {
        if STOP.load(Ordering::SeqCst) {
            eprintln!("vfsd: signal, unmounting");
            break Ok(());
        }
        match listener.accept() {
            Ok((stream, _)) => match serve_client(&mut daemon, stream) {
                Ok(true) => break Ok(()),
                Ok(false) => {}
                Err(e) => eprintln!("vfsd: client error: {e}"),
            },
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => break Err(e),
        }
    };
    for mut mount in daemon.mounts.drain(..).rev() {
        if let Err(e) = mount.unmount(true) {
            eprintln!("vfsd: {e}");
        }
    }
    let _ = std::fs::remove_file(socket);
    eprintln!("vfsd: exit");
    result
}
