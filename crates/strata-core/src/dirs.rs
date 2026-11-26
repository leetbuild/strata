//! The user's base folders (freedesktop.org XDG Base Directory specification): every crate
//! asks here instead of reading the environment itself.

use std::path::PathBuf;

/// `$<var>` when it is set to an absolute path (the specification says to ignore a relative
/// one), else `$HOME/<fallback>`; `None` without either.
fn base(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
}

/// `$XDG_DATA_HOME`, by default `~/.local/share` (the trash, LOOT's data, stylesheets,
/// desktop entries).
pub fn data_home() -> Option<PathBuf> {
    base("XDG_DATA_HOME", ".local/share")
}

/// `$XDG_CONFIG_HOME`, by default `~/.config` (Strata's own settings).
pub fn config_home() -> Option<PathBuf> {
    base("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_CACHE_HOME`, by default `~/.cache` (prefix setup downloads).
pub fn cache_home() -> Option<PathBuf> {
    base("XDG_CACHE_HOME", ".cache")
}

/// The running program's path (`std::env::current_exe`). When the file was replaced while it
/// runs (a rebuild, a package update) Linux names it `<path> (deleted)`: that suffix is
/// dropped, so the path names the program now there (a desktop entry written with the suffix
/// pointed at nothing, and `xdg-mime` fell back to another handler).
pub fn own_exe() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(stripped) => PathBuf::from(stripped),
        None => exe,
    })
}
