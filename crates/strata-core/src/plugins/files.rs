//! `lockedorder.txt` and the safe write of the plugin list files.

use std::path::Path;

use crate::{Error, Result};

/// One line of `lockedorder.txt`: a plugin held at a load order position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedOrder {
    pub name: String,
    pub position: i64,
}

/// Reads `lockedorder.txt` as `PluginList::readLockedOrderFrom` does: lines end at LF, a
/// line starting with `#` is skipped, the rest must be `name|position`. The name is taken as
/// it is (not trimmed); the position is trimmed and a negative one drops the line. A
/// position that is not a number counts as 0 (`QByteArray::toInt`).
pub fn parse_locked_order(data: &[u8]) -> Vec<LockedOrder> {
    let mut out = Vec::new();
    for line in data.split_inclusive(|b| *b == b'\n') {
        if line.first().is_none_or(|b| *b == b'#') {
            continue;
        }
        let fields: Vec<&[u8]> = line.split(|b| *b == b'|').collect();
        let [name, position] = fields.as_slice() else { continue };
        let position = std::str::from_utf8(position.trim_ascii()).ok().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
        if position < 0 {
            continue;
        }
        out.push(LockedOrder { name: String::from_utf8_lossy(name).into_owned(), position });
    }
    out
}

/// Writes `data` to `path` through a temporary file in the same folder and a rename, but only
/// when the file's content differs. Returns whether it wrote.
///
/// The `modlist` area has the same rule for its files; this small copy keeps the plugin list
/// independent until the two are merged into one helper.
pub(crate) fn write_if_changed(path: &Path, data: &[u8]) -> Result<bool> {
    if std::fs::read(path).is_ok_and(|old| old == data) {
        return Ok(false);
    }
    let error = |source| Error::Write { path: path.to_owned(), source };
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = dir.join(format!(".{name}.strata-tmp"));
    std::fs::write(&temp, data).map_err(error)?;
    if let Err(e) = std::fs::File::open(&temp).and_then(|f| f.sync_all()) {
        let _ = std::fs::remove_file(&temp);
        return Err(error(e));
    }
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        error(e)
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_order_lines_read_as_mo2_reads_them() {
        let data = b"# header\r\nA.esp|3\r\n\r\nB.esp|-1\r\nC.esp| 7 \r\nbad\r\nD.esp|x\r\nE|1|2\r\n";
        let got = parse_locked_order(data);
        assert_eq!(
            got,
            [
                LockedOrder { name: "A.esp".into(), position: 3 },
                LockedOrder { name: "C.esp".into(), position: 7 },
                LockedOrder { name: "D.esp".into(), position: 0 },
            ]
        );
    }

    #[test]
    fn writes_only_when_the_content_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.txt");
        assert!(write_if_changed(&path, b"a").unwrap());
        assert!(!write_if_changed(&path, b"a").unwrap());
        assert!(write_if_changed(&path, b"b").unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"b");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temporary file left");
    }
}
