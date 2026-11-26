//! "Query info" by checksum (MO2's `DownloadManager::queryInfoMd5` and
//! `nxmFileInfoFromMd5Available`): the archive's MD5 is looked up on Nexus, one hit is
//! chosen, and its mod and file go into the download's `.meta`.

use std::io::Read;
use std::path::Path;

use md5::{Digest, Md5};
use strata_ini::{Document, Value};

use crate::api::Md5Hit;
use crate::{Error, Result};

/// Nexus file categories MO2 skips when choosing among several hits.
const REMOVED: i64 = 6;
const ARCHIVED: i64 = 7;

/// The archive's MD5, lower-case hex.
pub fn file_md5(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).map_err(|e| Error::Io(path.to_owned(), e))?;
    let mut hasher = Md5::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).map_err(|e| Error::Io(path.to_owned(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// `nxmFileInfoFromMd5Available`'s choice: the only hit; else the only one whose file name
/// is the archive's (any case); else the only one not removed or archived. `None` when it
/// stays ambiguous (MO2 then falls back to the file-name query).
pub fn choose<'a>(hits: &'a [Md5Hit], archive_name: &str) -> Option<&'a Md5Hit> {
    if hits.len() == 1 {
        return hits.first();
    }
    let only = |pred: &dyn Fn(&Md5Hit) -> bool| {
        let mut it = hits.iter().filter(|h| pred(h));
        match (it.next(), it.next()) {
            (Some(h), None) => Some(h),
            _ => None,
        }
    };
    only(&|h| h.file_details.file_name.eq_ignore_ascii_case(archive_name))
        .or_else(|| only(&|h| !matches!(h.file_details.category_id, REMOVED | ARCHIVED)))
}

/// Writes the hit into `<archive>.meta`: `name`, `fileID`, `version` (the file's, or the
/// mod version when empty), `fileCategory`, `modID`, `modName`, `category`, `gameName` and
/// `repository=Nexus`. Other keys stay.
pub fn apply(archive: &Path, hit: &Md5Hit, game_short_name: &str) -> Result<()> {
    let mut meta = archive.as_os_str().to_owned();
    meta.push(".meta");
    let meta = std::path::PathBuf::from(meta);
    let mut doc = match std::fs::read(&meta) {
        Ok(data) => Document::parse(&data),
        Err(_) => Document::parse(b"[General]\n"),
    };
    let f = &hit.file_details;
    let m = &hit.mod_info;
    let version = if f.version.is_empty() { &f.mod_version } else { &f.version };
    for (key, value) in [
        ("name", f.name.clone()),
        ("fileID", f.file_id.to_string()),
        ("version", version.clone()),
        ("fileCategory", f.category_id.to_string()),
        ("modID", m.mod_id.to_string()),
        ("modName", m.name.clone()),
        ("category", m.category_id.to_string()),
        ("gameName", game_short_name.to_owned()),
        ("repository", "Nexus".to_owned()),
    ] {
        doc.set("", key, &Value::String(value));
    }
    crate::write_meta_file(&meta, &doc.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(file: &str, category: i64, mod_id: i64) -> Md5Hit {
        let json = serde_json::json!({
            "mod": { "mod_id": mod_id, "name": "SkyUI", "category_id": 42 },
            "file_details": { "file_id": 1, "name": "Main", "version": "", "mod_version": "5.2SE", "category_id": category, "file_name": file },
        });
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn md5_of_a_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a");
        std::fs::write(&p, "hello").unwrap();
        assert_eq!(file_md5(&p).unwrap(), "5d41402abc4b2a76b9719d911017c592");
    }

    #[test]
    fn choice_follows_mo2() {
        let hits = [hit("A.7z", 1, 1), hit("B.7z", 1, 2)];
        assert_eq!(choose(&hits, "b.7Z").unwrap().mod_info.mod_id, 2);
        let hits = [hit("A.7z", REMOVED, 1), hit("A.7z", 1, 2)];
        assert_eq!(choose(&hits, "x.7z").unwrap().mod_info.mod_id, 2, "the only active one");
        let hits = [hit("A.7z", 1, 1), hit("A.7z", 1, 2)];
        assert!(choose(&hits, "A.7z").is_none(), "ambiguous");
    }

    #[test]
    fn writes_the_meta() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("SkyUI.7z");
        std::fs::write(format!("{}.meta", a.display()), "[General]\ninstalled=true\n").unwrap();
        apply(&a, &hit("SkyUI.7z", 1, 12604), "SkyrimSE").unwrap();
        let text = std::fs::read_to_string(format!("{}.meta", a.display())).unwrap();
        assert!(text.starts_with("[General]\ninstalled=true\nname=Main\nfileID=1\nversion=5.2SE\n"), "{text}");
        assert!(text.contains("modID=12604\nmodName=SkyUI\ncategory=42\ngameName=SkyrimSE\nrepository=Nexus\n"));
    }
}
