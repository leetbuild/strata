//! What the window stores in the instance's `ModOrganizer.ini`: window geometry, splitter,
//! dock and header states, the selected tab... under the keys MO2 uses
//! (`GeometrySettings`, `WidgetSettings` in `settings.cpp`; names from
//! `settingsutilities.cpp` `widgetNameWithTopLevel`), so MO2 and Strata restore each
//! other's layout.
//!
//! Every store goes through the core's `strata_core::settings::store_ini`: the file is
//! re-read, only the given keys change and it is written only when the bytes changed, so keys
//! written by others in the meantime survive and an unchanged layout never touches the file.

use std::path::Path;

use strata_core::settings::{Missing, store_ini};
use strata_ini::{Document, Value};

use crate::Result;

/// One value to store: `[section] key = value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub section: String,
    pub key: String,
    pub value: Value,
}

/// Sets `settings` in the ini file `path` (created when missing). Returns whether the file
/// changed.
pub fn store(path: &Path, settings: &[Setting]) -> Result<bool> {
    let values: Vec<(&str, &str, Option<Value>)> =
        settings.iter().map(|s| (s.section.as_str(), s.key.as_str(), Some(s.value.clone()))).collect();
    Ok(store_ini(path, &values, Missing::Create)?.0)
}

/// Reads `[section] key` from the ini file `path`, as raw bytes for byte arrays (window
/// states) and as UTF-8 text for anything else; `None` when absent.
pub fn load(path: &Path, section: &str, key: &str) -> Option<Vec<u8>> {
    let doc = Document::parse(&std::fs::read(path).ok()?);
    match doc.value(section, key)? {
        Value::ByteArray(bytes) => Some(bytes),
        Value::Invalid => None,
        other => Some(other.to_text().into_bytes()),
    }
}

/// A list of strings as `QSettings` writes a `QVariantList` of strings
/// (`WidgetSettings::saveTreeExpandState`): `@Invalid()` when empty, `@Variant(...)` for
/// one item (the plain form would be read back as a string), `a, b` for more.
pub fn string_list_value(items: &[String]) -> Value {
    match items {
        [] => Value::Invalid,
        [one] => {
            // QDataStream Qt_4_0: QVariantList (type 9), count, then each a QVariant
            // QString (type 10): byte length and UTF-16 big-endian.
            let mut bytes = vec![0, 0, 0, 9];
            bytes.extend_from_slice(&1u32.to_be_bytes());
            bytes.extend_from_slice(&[0, 0, 0, 10]);
            let units: Vec<u16> = one.encode_utf16().collect();
            bytes.extend_from_slice(&(u32::try_from(units.len() * 2).unwrap_or(u32::MAX)).to_be_bytes());
            for u in units {
                bytes.extend_from_slice(&u.to_be_bytes());
            }
            Value::Other(format!("@Variant({})", bytes.iter().map(|b| char::from(*b)).collect::<String>()))
        }
        many => Value::List(many.to_vec()),
    }
}

/// Reads a list written as [`string_list_value`] writes it (and a plain string as a list of
/// one). `None` for a value that is none of these.
pub fn string_list(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Invalid => Some(Vec::new()),
        Value::List(items) => Some(items.clone()),
        Value::String(s) => Some(vec![s.clone()]),
        Value::Other(text) => {
            let inner = text.strip_prefix("@Variant(")?.strip_suffix(')')?;
            let bytes: Vec<u8> = inner.chars().map(|c| u8::try_from(u32::from(c)).ok()).collect::<Option<_>>()?;
            let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
            let kind = u32_at(0)?;
            let count = u32_at(4)? as usize;
            let mut at = 8;
            let mut items = Vec::new();
            for _ in 0..count {
                // a QVariantList holds QVariants (each with its type); a QStringList (11)
                // holds bare QStrings
                if kind == 9 {
                    if u32_at(at)? != 10 {
                        return None;
                    }
                    at += 4;
                } else if kind != 11 {
                    return None;
                }
                let len = u32_at(at)?;
                at += 4;
                if len == u32::MAX {
                    items.push(String::new()); // a null QString
                    continue;
                }
                let raw = bytes.get(at..at + len as usize)?;
                at += len as usize;
                let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                items.push(String::from_utf16_lossy(&units));
            }
            Some(items)
        }
        Value::ByteArray(_) => None,
    }
}

/// Reads `[section] key` of the ini file `path` as a list of strings.
pub fn load_list(path: &Path, section: &str, key: &str) -> Option<Vec<String>> {
    let doc = Document::parse(&std::fs::read(path).ok()?);
    string_list(&doc.value(section, key)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_separators_round_trip_as_qt_writes_them() {
        // Real values from MO2 instances: one item, two items, none.
        let doc = Document::parse(
            b"[Widgets]\nMainWindow_modList_index=@Variant(\\0\\0\\0\\t\\0\\0\\0\\x1\\0\\0\\0\\n\\0\\0\\0\\x1c\\0\\x44\\0L\\0L\\0 \\0G\\0\\x61\\0m\\0\\x65\\0 \\0\\x46\\0i\\0x\\0\\x65\\0s)\nb=Base Game Master Fixes, My Mods\nc=@Invalid()\n",
        );
        let one = doc.value("Widgets", "MainWindow_modList_index").unwrap();
        assert_eq!(string_list(&one), Some(vec!["DLL Game Fixes".to_owned()]));
        assert_eq!(string_list_value(&["DLL Game Fixes".to_owned()]), one, "written back the same");
        let two = doc.value("Widgets", "b").unwrap();
        assert_eq!(string_list(&two), Some(vec!["Base Game Master Fixes".to_owned(), "My Mods".to_owned()]));
        assert_eq!(string_list(&doc.value("Widgets", "c").unwrap()), Some(Vec::new()));
        assert_eq!(string_list_value(&[]), Value::Invalid);

        let tmp = tempfile::tempdir().unwrap();
        let ini = tmp.path().join("ModOrganizer.ini");
        for items in [vec![], vec!["Ünï, \"x\"".to_owned()], vec!["a".to_owned(), "b, c".to_owned()]] {
            store(&ini, &[setting("Widgets", "k", string_list_value(&items))]).unwrap();
            assert_eq!(load_list(&ini, "Widgets", "k"), Some(items));
        }
    }

    fn setting(section: &str, key: &str, value: Value) -> Setting {
        Setting { section: section.into(), key: key.into(), value }
    }

    #[test]
    fn a_window_state_round_trips_as_mo2_writes_it_and_other_keys_survive() {
        let tmp = tempfile::tempdir().unwrap();
        let ini = tmp.path().join("ModOrganizer.ini");
        std::fs::write(&ini, "[General]\r\ngameName=SkyrimSE\r\n\r\n[Geometry]\r\nother=1\r\n").unwrap();
        let state = vec![0u8, 0, 0, 0xff, 1, b'@', b'\\'];
        assert!(store(&ini, &[setting("Geometry", "MainWindow_state", Value::ByteArray(state.clone()))]).unwrap());
        assert_eq!(load(&ini, "Geometry", "MainWindow_state"), Some(state.clone()));
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(text.starts_with("[General]\r\ngameName=SkyrimSE\r\n\r\n[Geometry]\r\nother=1\r\nMainWindow_state=@ByteArray("), "{text}");
        // the same value again: the file is not written
        let before = std::fs::metadata(&ini).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!store(&ini, &[setting("Geometry", "MainWindow_state", Value::ByteArray(state))]).unwrap());
        assert_eq!(std::fs::metadata(&ini).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn a_change_made_by_someone_else_in_between_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let ini = tmp.path().join("ModOrganizer.ini");
        std::fs::write(&ini, "[Widgets]\na=1\n").unwrap();
        std::fs::write(&ini, "[Widgets]\na=1\nb=2\n").unwrap();
        store(&ini, &[setting("Widgets", "MainWindow_tabWidget_index", Value::String("2".into()))]).unwrap();
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "[Widgets]\na=1\nb=2\nMainWindow_tabWidget_index=2\n");
        assert_eq!(load(&ini, "Widgets", "MainWindow_tabWidget_index"), Some(b"2".to_vec()));
        assert_eq!(load(&ini, "Widgets", "missing"), None);
    }
}
