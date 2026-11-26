//! The Nexus API key in the freedesktop secret service (GNOME Keyring, KWallet, KeePassXC),
//! through libsecret's `secret-tool`. Never in a file: MO2 keeps it in the Windows
//! credential store (`ModOrganizer2_APIKEY`) for the same reason.
//!
//! The item's attributes are `application=strata`, `service=nexus`, `kind=api-key`.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::{Error, Result};

const ATTRIBUTES: [&str; 6] = ["application", "strata", "service", "nexus", "kind", "api-key"];

/// The secret service, through a `secret-tool` program.
#[derive(Debug, Clone)]
pub struct SecretTool {
    program: PathBuf,
}

impl Default for SecretTool {
    /// `secret-tool` from the PATH, or the program `STRATA_SECRET_TOOL` names (tests use a
    /// fake that keeps the secret in memory of their own).
    fn default() -> Self {
        let program = std::env::var_os("STRATA_SECRET_TOOL").map_or_else(|| PathBuf::from("secret-tool"), PathBuf::from);
        Self { program }
    }
}

impl SecretTool {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self { program: program.into() }
    }

    fn command(&self, verb: &str) -> Command {
        let mut c = Command::new(&self.program);
        c.arg(verb);
        c
    }

    fn missing(&self, e: std::io::Error) -> Error {
        Error::Keyring(format!(
            "cannot run {} ({e}): install libsecret (it has secret-tool) and a secret service such as gnome-keyring",
            self.program.display()
        ))
    }

    /// Stores the key (replacing an older one).
    pub fn store(&self, key: &str) -> Result<()> {
        let mut child = self
            .command("store")
            .args(["--label", "Strata: Nexus Mods API key"])
            .args(ATTRIBUTES)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| self.missing(e))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(key.as_bytes()).map_err(|e| self.missing(e))?;
        }
        let out = child.wait_with_output().map_err(|e| self.missing(e))?;
        if !out.status.success() {
            let why = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            return Err(Error::Keyring(format!("the secret service refused to store the key: {why}")));
        }
        Ok(())
    }

    /// The stored key, `None` when there is none.
    pub fn lookup(&self) -> Result<Option<String>> {
        let out = self.command("lookup").args(ATTRIBUTES).stderr(Stdio::piped()).output().map_err(|e| self.missing(e))?;
        if !out.status.success() {
            // secret-tool exits 1 with no output when nothing matches
            let why = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            return if why.is_empty() { Ok(None) } else { Err(Error::Keyring(why)) };
        }
        let key = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        Ok((!key.is_empty()).then_some(key))
    }

    /// Removes the stored key ("log out").
    pub fn clear(&self) -> Result<()> {
        let out = self.command("clear").args(ATTRIBUTES).stderr(Stdio::piped()).output().map_err(|e| self.missing(e))?;
        if !out.status.success() {
            let why = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            if !why.is_empty() {
                return Err(Error::Keyring(why));
            }
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
/// A stand-in `secret-tool` for tests: a shell script keeping the secret in `<dir>/secret`
/// and checking it gets exactly Strata's attributes.
pub fn fake_secret_tool(dir: &std::path::Path) -> std::io::Result<PathBuf> {
    let script = dir.join("secret-tool");
    let store = dir.join("secret");
    let text = format!(
        "#!/bin/sh\nverb=$1; shift\n[ \"$verb\" = store ] && {{ [ \"$1\" = --label ] || exit 9; shift 2; }}\n\
         [ \"$*\" = 'application strata service nexus kind api-key' ] || {{ echo \"bad attributes: $*\" >&2; exit 8; }}\n\
         case $verb in\n  store) cat > '{0}' ;;\n  lookup) [ -f '{0}' ] && cat '{0}' || exit 1 ;;\n  clear) rm -f '{0}' ;;\nesac\n",
        store.display()
    );
    std::fs::write(&script, text)?;
    let mut perm = std::fs::metadata(&script)?.permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
    std::fs::set_permissions(&script, perm)?;
    // A program another thread starts in this moment inherits our write handle until it execs:
    // running the script then fails with "Text file busy" (it made a test flaky). Wait until it
    // runs.
    for _ in 0..200 {
        match std::process::Command::new(&script).arg("probe").stdout(Stdio::null()).stderr(Stdio::null()).status() {
            Err(e) if e.raw_os_error() == Some(26) => std::thread::sleep(std::time::Duration::from_millis(5)),
            _ => break,
        }
    }
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_lookup_clear_through_the_program() {
        let dir = tempfile::tempdir().unwrap();
        let tool = SecretTool::new(fake_secret_tool(dir.path()).unwrap());
        assert_eq!(tool.lookup().unwrap(), None);
        tool.store("KEY-1").unwrap();
        assert_eq!(tool.lookup().unwrap().as_deref(), Some("KEY-1"));
        assert_eq!(std::fs::read_to_string(dir.path().join("secret")).unwrap(), "KEY-1", "sent on stdin, no newline");
        tool.store("KEY-2").unwrap();
        assert_eq!(tool.lookup().unwrap().as_deref(), Some("KEY-2"));
        tool.clear().unwrap();
        assert_eq!(tool.lookup().unwrap(), None);
    }

    #[test]
    fn a_missing_program_says_what_to_install() {
        let err = SecretTool::new("/nonexistent/secret-tool").lookup().unwrap_err().to_string();
        assert!(err.contains("libsecret"), "{err}");
    }

    /// The real secret service of this session: it stores a test key and puts the user's own
    /// key back, so it runs only when asked (`STRATA_TEST_KEYRING=1`).
    #[test]
    fn real_secret_service() {
        if std::env::var_os("STRATA_TEST_KEYRING").is_none() {
            return eprintln!("SKIPPED: real_secret_service: touches the desktop's keyring, runs only with STRATA_TEST_KEYRING=1");
        }
        let tool = SecretTool::default();
        let had = tool.lookup().unwrap();
        tool.store("strata-test").unwrap();
        assert_eq!(tool.lookup().unwrap().as_deref(), Some("strata-test"));
        match had {
            Some(k) => tool.store(&k).unwrap(),
            None => tool.clear().unwrap(),
        }
    }
}
