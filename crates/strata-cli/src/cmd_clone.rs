//! `strata clone-instance <source> <target>`: a safe test copy of a portable instance.
//!
//! 1. Copies the instance as `cp --reflink=auto -a` does, and says whether it was a reflink
//!    copy (instant, shares the blocks) or a full one. A full copy bigger than the free space
//!    or than 5 GB needs `--allow-full-copy`.
//! 2. Rewrites every value of the copy's `ModOrganizer.ini` that points into the source so it
//!    points into the copy: generically over the lossless ini document (decode, replace the
//!    source folder in each of its spellings, encode back), so keys Strata does not know are
//!    covered too. The spellings: `/home/x/I`, `Z:\home\x\I`, `Z:/home/x/I` (any drive letter
//!    case), inside `@ByteArray(...)`, lists and quoted arguments alike.
//! 3. A game folder or prefix outside the source would still be used, and written to, by the
//!    copy: the command refuses unless `--game-copy` / `--prefix-copy` copy them into the target
//!    too (then their paths are rewritten the same way). Other paths outside are warnings.
//! 4. Symlinks in the copy that point into the source (Fluorine links the profile's inis into
//!    the prefix's `My Games`) are repointed into the copy.
//! 5. Prints `strata info` of the copy and fails if one of its folders lies outside the target.
//!
//! The source is only read. `STRATA_CLONE_FULL_COPY_LIMIT` (bytes) replaces the 5 GB limit
//! (tests).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::json;
use strata_ini::{Document, Value};

use crate::Global;
use crate::commands::print_json;

/// The CLI's options.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub(crate) struct CloneArgs {
    /// The instance to copy (the folder with ModOrganizer.ini). Only read.
    pub source: PathBuf,
    /// The new folder (must not exist).
    pub target: PathBuf,
    /// Allow a full (non-reflink) copy bigger than 5 GB or than the free space.
    #[arg(long)]
    pub allow_full_copy: bool,
    /// The prefix lies outside the source: copy it into <target>/Prefix and use that.
    #[arg(long)]
    pub prefix_copy: bool,
    /// The game folder lies outside the source: copy it into <target>/Game Root and use that.
    #[arg(long)]
    pub game_copy: bool,
}

const FULL_COPY_LIMIT: u64 = 5 * 1024 * 1024 * 1024;

/// One folder to copy and the folder it becomes.
struct Copy {
    what: &'static str,
    from: PathBuf,
    to: PathBuf,
}

fn inside(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

/// The first regular file under `dir` (for the reflink probe).
fn some_file(dir: &Path) -> Option<PathBuf> {
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&d).ok()?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let Ok(t) = e.file_type() else { continue };
            if t.is_file() {
                return Some(e.path());
            }
            if t.is_dir() {
                stack.push(e.path());
            }
        }
    }
    None
}

/// Whether a file of `from` can be reflinked into `dest_dir` (same btrfs / XFS file system).
fn can_reflink(from: &Path, dest_dir: &Path) -> bool {
    let Some(file) = some_file(from) else { return true };
    let probe = dest_dir.join(format!(".strata-reflink-probe-{}", std::process::id()));
    let ok = Command::new("cp")
        .arg("--reflink=always")
        .arg(&file)
        .arg(&probe)
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let _ = std::fs::remove_file(&probe);
    ok
}

/// `du -sb`: the bytes a full copy would write.
fn size_of(dir: &Path) -> Result<u64> {
    let out = Command::new("du").arg("-sb").arg(dir).output().context("cannot run du")?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().next().and_then(|n| n.parse().ok()).with_context(|| format!("du gave no size for {}", dir.display()))
}

/// `df`: the free bytes where `dir` lives.
fn free_space(dir: &Path) -> Result<u64> {
    let out = Command::new("df").args(["-B1", "--output=avail"]).arg(dir).output().context("cannot run df")?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().nth(1).and_then(|l| l.trim().parse().ok()).with_context(|| format!("df gave no free space for {}", dir.display()))
}

/// Copies `c.from` to `c.to` (`cp --reflink=auto -a`); returns "reflink" or "full".
fn copy(c: &Copy, allow_full: bool) -> Result<&'static str> {
    let dest_dir = c.to.parent().context("the target has no parent folder")?;
    let reflink = can_reflink(&c.from, dest_dir);
    if !reflink {
        let size = size_of(&c.from)?;
        let free = free_space(dest_dir)?;
        let limit = std::env::var("STRATA_CLONE_FULL_COPY_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(FULL_COPY_LIMIT);
        if !allow_full && (size > free || size > limit) {
            bail!(
                "copying the {} ({}) would be a full copy of {} MB ({} MB free, limit {} MB): the target is not on \
                 the same btrfs/XFS file system. Pick a target there, or add --allow-full-copy",
                c.what,
                c.from.display(),
                size / 1_000_000,
                free / 1_000_000,
                limit / 1_000_000
            );
        }
        if size > free {
            bail!("the {} needs {} MB, only {} MB are free at {}", c.what, size / 1_000_000, free / 1_000_000, dest_dir.display());
        }
    }
    let status = Command::new("cp")
        .args(["--reflink=auto", "-a", "--no-target-directory"])
        .arg(&c.from)
        .arg(&c.to)
        .status()
        .context("cannot run cp")?;
    if !status.success() {
        bail!("cp failed copying {} to {}", c.from.display(), c.to.display());
    }
    Ok(if reflink { "reflink" } else { "full" })
}

/// The spellings of a host folder in MO2's ini values: `(before, after)` pairs, Wine forms first.
fn spellings(from: &Path, to: &Path) -> Vec<(String, String)> {
    let (f, t) = (from.to_string_lossy().into_owned(), to.to_string_lossy().into_owned());
    let back = |s: &str| s.replace('/', "\\");
    let mut out = Vec::new();
    for drive in ["Z:", "z:"] {
        out.push((format!("{drive}{}", back(&f)), format!("{drive}{}", back(&t))));
        out.push((format!("{drive}{f}"), format!("{drive}{t}")));
    }
    out.push((f, t));
    out
}

/// What may follow a folder name in a value: the end, a separator, a quote or a list comma.
fn boundary(next: Option<char>) -> bool {
    next.is_none_or(|c| matches!(c, '/' | '\\' | '"' | '\'' | ';' | ',' | ')'))
}

/// Replaces each listed folder in `text`, in every spelling, where it is a whole path prefix.
fn replace_roots(text: &str, roots: &[(PathBuf, PathBuf)]) -> String {
    let pairs: Vec<(String, String)> = roots.iter().flat_map(|(f, t)| spellings(f, t)).collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'scan: while i < text.len() {
        for (from, to) in &pairs {
            if text[i..].starts_with(from.as_str()) {
                let bare = from.starts_with('/');
                // a bare /path right after a drive letter belongs to a Z: form matched above
                if bare && text[..i].ends_with(':') {
                    continue;
                }
                if boundary(text[i + from.len()..].chars().next()) {
                    out.push_str(to);
                    i += from.len();
                    continue 'scan;
                }
            }
        }
        let c = text[i..].chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// Host paths a value names (Wine `X:\...` / `X:/...` forms and absolute Linux paths), as far as
/// a quote or the end.
fn paths_in(text: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let wine = i + 2 < b.len() && b[i].is_ascii_alphabetic() && b[i + 1] == b':' && matches!(b[i + 2], b'\\' | b'/')
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric());
        // an absolute Linux path: the whole value, or a /home path inside an argument
        let linux = b[i] == b'/' && (i == 0 || (matches!(b[i - 1], b'"' | b' ' | b'=') && text[i..].starts_with("/home")));
        if wine || linux {
            let start = if wine { i + 2 } else { i };
            let end = text[start..].find(['"', ';']).map_or(text.len(), |n| start + n);
            let p = text[start..end].replace('\\', "/");
            if wine && !b[i].eq_ignore_ascii_case(&b'z') {
                // a drive inside the prefix (C:): it moves with the prefix
            } else {
                found.push(PathBuf::from(p.trim_end()));
            }
            i = end;
        } else {
            i += 1;
        }
    }
    found
}

/// Rewrites the copy's `ModOrganizer.ini`. Returns the changed keys (`section/key`) and the
/// warnings (values still naming paths outside the target).
fn rewrite_ini(ini: &Path, roots: &[(PathBuf, PathBuf)], target: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let data = std::fs::read(ini).with_context(|| format!("cannot read {}", ini.display()))?;
    let mut doc = Document::parse(&data);
    let mut changes = Vec::new();
    let mut updates = Vec::new();
    for e in doc.entries() {
        let value = e.value();
        let new = match &value {
            Value::String(s) => Value::String(replace_roots(s, roots)),
            Value::ByteArray(b) => match std::str::from_utf8(b) {
                Ok(s) => Value::ByteArray(replace_roots(s, roots).into_bytes()),
                Err(_) => continue,
            },
            Value::List(items) => Value::List(items.iter().map(|s| replace_roots(s, roots)).collect()),
            _ => continue,
        };
        if new != value {
            updates.push((e.section.to_owned(), e.key.to_owned(), new));
        }
    }
    for (section, key, value) in &updates {
        doc.set(section, key, value);
        changes.push(if section.is_empty() { key.clone() } else { format!("[{section}] {key}") });
    }
    let mut warnings = Vec::new();
    for e in doc.entries() {
        let text = match e.value() {
            Value::String(s) => s,
            Value::ByteArray(b) => String::from_utf8_lossy(&b).into_owned(),
            Value::List(items) => items.join(";"),
            _ => continue,
        };
        for p in paths_in(&text) {
            if !p.starts_with(target) {
                let name = if e.section.is_empty() { e.key.to_owned() } else { format!("[{}] {}", e.section, e.key) };
                warnings.push(format!("{name}: {}", p.display()));
            }
        }
    }
    strata_core::write::write_if_changed(ini, &doc.to_bytes())?;
    Ok((changes, warnings))
}

/// What [`relink`] did.
#[derive(Default)]
struct Links {
    /// Into the source (or a copied folder): now into the copy.
    moved: Vec<String>,
    /// A prefix ini linked to another instance's profile (a shared prefix): now to this
    /// copy's profile file of the same name.
    from_other: Vec<String>,
    /// Still outside the copy (a warning).
    outside: Vec<String>,
    /// A `My Games` link leading outside with no file of that name in the copy's profile: the
    /// game would write through it into another folder, so the command fails over these.
    dangerous: Vec<String>,
    /// Wine's own links (drive letters, Proton's fonts and libraries): fine, only counted.
    wine: usize,
}

/// Whether a link outside the copy is one Wine / Proton makes in every prefix.
fn wine_link(path: &Path, link: &Path, proton: Option<&Path>) -> bool {
    let text = link.to_string_lossy();
    path.components().any(|c| c.as_os_str() == "dosdevices")
        || proton.is_some_and(|p| link.starts_with(p))
        || text.contains("/compatibilitytools.d/")
        || text.contains("/steamapps/common/Proton")
}

/// The file of `dir` named `name` without regard to case (Fluorine's links use the game's
/// spelling, `Skyrim.ini`; the profile may hold `skyrim.ini`).
fn file_in(dir: &Path, name: &std::ffi::OsStr) -> Option<PathBuf> {
    let want = name.to_string_lossy().to_lowercase();
    std::fs::read_dir(dir).ok()?.flatten().find(|e| e.file_name().to_string_lossy().to_lowercase() == want).map(|e| e.path())
}

/// Repoints symlinks under `dir` whose absolute target lies in one of `roots` (into its
/// copy), and the prefix's `My Games` links to another instance's profile (into `profile`,
/// the copy's selected profile, as Fluorine links them when it starts this instance).
fn relink(dir: &Path, roots: &[(PathBuf, PathBuf)], target: &Path, profile: &Path, proton: Option<&Path>) -> Result<Links> {
    let mut out = Links::default();
    let mut stack = vec![dir.to_owned()];
    let repoint = |path: &Path, new: &Path| -> Result<()> {
        std::fs::remove_file(path)?;
        std::os::unix::fs::symlink(new, path)?;
        Ok(())
    };
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).with_context(|| format!("cannot read {}", d.display()))?.flatten() {
            let path = e.path();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                stack.push(path);
                continue;
            }
            if !t.is_symlink() {
                continue;
            }
            let link = std::fs::read_link(&path)?;
            if !link.is_absolute() || link.starts_with(target) {
                continue;
            }
            if let Some((from, to)) = roots.iter().find(|(from, _)| link.starts_with(from)) {
                let new = to.join(link.strip_prefix(from).unwrap_or(Path::new("")));
                repoint(&path, &new)?;
                out.moved.push(format!("{} -> {}", path.display(), new.display()));
            } else if wine_link(&path, &link, proton) {
                out.wine += 1;
            } else if path.components().any(|c| c.as_os_str() == "My Games") {
                match path.file_name().and_then(|n| file_in(profile, n)) {
                    Some(new) => {
                        repoint(&path, &new)?;
                        out.from_other.push(format!("{} -> {} (was {})", path.display(), new.display(), link.display()));
                    }
                    None => out.dangerous.push(format!("{} -> {}", path.display(), link.display())),
                }
            } else {
                out.outside.push(format!("{} -> {}", path.display(), link.display()));
            }
        }
    }
    for v in [&mut out.moved, &mut out.from_other, &mut out.outside, &mut out.dangerous] {
        v.sort();
    }
    Ok(out)
}

pub(crate) fn clone_instance(global: &Global, args: &CloneArgs) -> Result<()> {
    let source = args.source.canonicalize().with_context(|| format!("cannot find {}", args.source.display()))?;
    if !source.join("ModOrganizer.ini").is_file() {
        bail!("{} is not a portable instance (no ModOrganizer.ini)", source.display());
    }
    if args.target.exists() || args.target.symlink_metadata().is_ok() {
        bail!("{} exists already: give a new folder", args.target.display());
    }
    let name = args.target.file_name().context("the target needs a folder name")?;
    let parent = match args.target.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(p) => p.canonicalize().with_context(|| format!("the folder {} does not exist", p.display()))?,
        None => std::env::current_dir()?,
    };
    let target = parent.join(name);
    if inside(&target, &source) || inside(&source, &target) {
        bail!("the target must not be inside the source or hold it");
    }
    let instance = strata_core::Instance::open(&source)?;
    let game = instance.game_path.canonicalize().unwrap_or_else(|_| instance.game_path.clone());
    let prefix = instance.proton.prefix.as_ref().map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()));

    let mut copies = vec![Copy { what: "instance", from: source.clone(), to: target.clone() }];
    let mut roots = vec![(source.clone(), target.clone())];
    if let Some(p) = prefix.as_ref().filter(|p| !inside(p, &source)) {
        if !args.prefix_copy {
            bail!(
                "the prefix [fluorine] prefix_path = {} lies outside the instance: the copy would run the game in \
                 (and write to) that real prefix. Add --prefix-copy to copy it into the target",
                p.display()
            );
        }
        let to = target.join("Prefix");
        copies.push(Copy { what: "prefix", from: p.clone(), to: to.clone() });
        roots.push((p.clone(), to));
    }
    // a game folder inside a prefix that is copied moves with it
    let in_copied_prefix = roots.iter().skip(1).any(|(from, _)| inside(&game, from));
    if !inside(&game, &source) && !in_copied_prefix {
        if !args.game_copy {
            bail!(
                "the game folder gamePath = {} lies outside the instance: a deploy of the copy would change that \
                 real folder. Add --game-copy to copy it into the target",
                game.display()
            );
        }
        let to = target.join("Game Root");
        copies.push(Copy { what: "game folder", from: game.clone(), to: to.clone() });
        roots.push((game.clone(), to));
    }

    let mut kinds = Vec::new();
    for c in &copies {
        if c.to != target && c.to.exists() {
            bail!("{} exists in the source's copy already; cannot copy the {} there", c.to.display(), c.what);
        }
        let kind = copy(c, args.allow_full_copy)?;
        kinds.push(json!({ "what": c.what, "from": c.from, "to": c.to, "copy": kind }));
        if !global.json {
            println!("copied the {} {} -> {} ({kind} copy)", c.what, c.from.display(), c.to.display());
        }
    }
    let (changed, warnings) = rewrite_ini(&target.join("ModOrganizer.ini"), &roots, &target)?;
    let copy = strata_core::Instance::open(&target)?;
    let profile = copy.profile(None)?;
    let links = relink(&target, &roots, &target, &profile.dir, copy.proton.proton.as_deref())?;

    // the copy must stand on its own
    let mut escaped = Vec::new();
    let mut check = |what: &str, p: &Path| {
        if !p.starts_with(&target) {
            escaped.push(format!("{what} = {}", p.display()));
        }
    };
    check("game_path", &copy.game_path);
    check("profile_dir", &profile.dir);
    check("mods", &copy.paths.mods);
    check("overwrite", &copy.paths.overwrite);
    check("downloads", &copy.paths.downloads);
    if let Some(p) = &copy.proton.prefix {
        check("prefix", p);
    }

    if global.json {
        print_json(&json!({
            "copies": kinds, "changed": changed, "warnings": warnings, "relinked": links.moved,
            "relinked_from_other_instance": links.from_other, "links_outside": links.outside,
            "links_into_other_folders": links.dangerous, "wine_links": links.wine, "outside": escaped,
        }))?;
    } else {
        println!("ModOrganizer.ini: {} keys rewritten", changed.len());
        for c in &changed {
            println!("  {c}");
        }
        for l in &links.moved {
            println!("symlink repointed: {l}");
        }
        for l in &links.from_other {
            println!("symlink repointed from another instance's profile: {l}");
        }
        for w in &warnings {
            println!("WARNING: points outside the copy: {w}");
        }
        for l in &links.outside {
            println!("WARNING: symlink points outside the copy: {l}");
        }
        for l in &links.dangerous {
            println!("ERROR: the game would write through this symlink outside the copy: {l}");
        }
        if links.wine > 0 {
            println!("({} Wine / Proton links to drives and Proton's own files left as they are)", links.wine);
        }
        println!();
        let info = Global { instance: Some(target.clone()), profile: None, ..global.clone() };
        crate::commands::info(&info)?;
    }
    if !links.dangerous.is_empty() {
        bail!(
            "{} symlink(s) in the copy lead outside it (listed above): remove or repoint them before using the copy",
            links.dangerous.len()
        );
    }
    if !escaped.is_empty() {
        bail!("the copy still uses folders outside {}: {}", target.display(), escaped.join(", "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_of_the_source_is_replaced_and_only_whole_folders() {
        let roots = vec![(PathBuf::from("/home/u/Games/I"), PathBuf::from("/home/u/Games/I - Copy"))];
        for (before, after) in [
            ("Z:\\home\\u\\Games\\I\\Game Root", "Z:\\home\\u\\Games\\I - Copy\\Game Root"),
            ("z:/home/u/Games/I/Game Root", "z:/home/u/Games/I - Copy/Game Root"),
            ("/home/u/Games/I/Prefix", "/home/u/Games/I - Copy/Prefix"),
            ("/home/u/Games/I", "/home/u/Games/I - Copy"),
            ("-D:\"Z:\\home\\u\\Games\\I\\Game Root\\Data\" -IKnowWhatImDoing", "-D:\"Z:\\home\\u\\Games\\I - Copy\\Game Root\\Data\" -IKnowWhatImDoing"),
            ("/home/u/Games/I2/x", "/home/u/Games/I2/x"),
            ("/home/u/Games/I - Other/x", "/home/u/Games/I - Other/x"),
            ("Z:\\home\\u\\Games\\Prefix", "Z:\\home\\u\\Games\\Prefix"),
        ] {
            assert_eq!(replace_roots(before, &roots), after, "{before}");
        }
    }

    #[test]
    fn paths_named_in_values() {
        assert_eq!(paths_in("Z:\\home\\u\\x"), [PathBuf::from("/home/u/x")]);
        assert_eq!(paths_in("-D:\"Z:\\home\\u\\G\\Data\" -q"), [PathBuf::from("/home/u/G/Data")]);
        assert_eq!(paths_in("/home/u/Prefix"), [PathBuf::from("/home/u/Prefix")]);
        assert_eq!(paths_in("/opt/proton"), [PathBuf::from("/opt/proton")]);
        assert!(paths_in("a/b/c").is_empty());
        assert!(paths_in("C:\\users\\steamuser").is_empty(), "inside the prefix");
        assert!(paths_in("vs15 Dark.qss").is_empty());
    }
}
