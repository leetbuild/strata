//! A mod's `meta.ini`: a typed view over the lossless document.
//!
//! Every getter reads the key as `ModInfoRegular::readMeta` does, with its default; every
//! setter changes just that key and leaves the rest of the file as it is (unknown keys,
//! order, comments). MO2's `saveMeta` rewrites all its keys whenever one changes; Strata
//! changes only the line it must, so a file MO2 wrote stays byte-identical apart from it.

use std::path::{Path, PathBuf};

use strata_ini::{Document, Value};

use crate::{Error, Result, write::write_if_changed};

/// `[General]` in the document.
const GENERAL: &str = "";

/// The Sub-Separator plugin's name in `[Plugins]` (Leet change 9, `subseparator.h`).
pub const SUB_SEPARATOR: &str = "Sub-Separator";

/// `EndorsedState` (uibase `imodinterface.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endorsed {
    False,
    True,
    Unknown,
    Never,
}

/// A colour as MO2 stores it for separators (`QColor`, 8 bits per channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// `#rrggbb` or `#aarrggbb`, as Qt's `QColor::name` writes them.
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#')?;
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        match hex.len() {
            6 => Some(Self { r: byte(0)?, g: byte(2)?, b: byte(4)?, a: 255 }),
            8 => Some(Self { a: byte(0)?, r: byte(2)?, g: byte(4)?, b: byte(6)? }),
            _ => None,
        }
    }

    /// `#rrggbb` when opaque, else `#aarrggbb`.
    pub fn name(&self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.a, self.r, self.g, self.b)
        }
    }
}

/// `QVariant(QColor)` as `QSettings` stores it: `@Variant(` + the type id (67, four bytes),
/// then `QColor`'s stream form (spec, then alpha, red, green, blue and padding as 16-bit
/// values). `QSettings` streams with `QDataStream::Qt_4_0`, so there is no null flag.
const COLOR_TYPE: [u8; 4] = [0, 0, 0, 0x43];

/// A colour stored as `QSettings` stores a `QColor` (separator colours, and the conflict
/// highlight colours in `ModOrganizer.ini`); `None` for an invalid colour or another value.
pub fn color_from_value(value: &Value) -> Option<Color> {
    decode_color(value)
}

fn decode_color(value: &Value) -> Option<Color> {
    let Value::Other(text) = value else { return None };
    let bytes: Vec<u8> = text.strip_prefix("@Variant(")?.strip_suffix(')')?.chars().map(|c| c as u32 as u8).collect();
    let data = bytes.strip_prefix(&COLOR_TYPE[..])?;
    // spec 1 = Rgb; 0 = invalid (no colour); other specs are not written for separators.
    if data.len() != 11 || data[0] != 1 {
        return None;
    }
    let channel = |i: usize| data[i]; // the high byte of a 16-bit channel (v * 0x101)
    Some(Color { a: channel(1), r: channel(3), g: channel(5), b: channel(7) })
}

fn encode_color(color: Option<Color>) -> Value {
    let mut bytes = COLOR_TYPE.to_vec();
    match color {
        Some(c) => {
            bytes.push(1);
            for v in [c.a, c.r, c.g, c.b, 0] {
                bytes.extend_from_slice(&(u16::from(v) * 0x101).to_be_bytes());
            }
        }
        // QColor() streams as spec 0, alpha 0xffff, the rest 0.
        None => bytes.extend_from_slice(&[0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0]),
    }
    let mut text = String::from("@Variant(");
    text.extend(bytes.iter().map(|b| char::from(*b)));
    text.push(')');
    Value::Other(text)
}

/// Qt's `QString::toInt`: the whole (trimmed) text must be a number, else 0.
fn to_int(value: &Value) -> Option<i64> {
    value.to_text().trim().parse().ok()
}

/// An opened `meta.ini`.
#[derive(Debug, Clone)]
pub struct ModMeta {
    path: PathBuf,
    doc: Document,
}

impl ModMeta {
    /// Reads `<mod folder>/meta.ini`. A missing file reads as empty (every key at its
    /// default), as `QSettings` does.
    pub fn open(mod_dir: &Path) -> Result<Self> {
        let path = mod_dir.join("meta.ini");
        let doc = match std::fs::read(&path) {
            Ok(data) => Document::parse(&data),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Document::default(),
            Err(source) => return Err(Error::Read { path, source }),
        };
        Ok(Self { path, doc })
    }

    /// A `meta.ini` from bytes (tests, installers).
    pub fn from_bytes(path: PathBuf, data: &[u8]) -> Self {
        Self { path, doc: Document::parse(data) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The whole document, for keys this view doesn't name.
    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.doc.to_bytes()
    }

    /// Writes the file if its content changed. Returns whether it was written. The mod's
    /// folder must exist (MO2 writes no `meta.ini` for a mod that is gone).
    pub fn save(&self) -> Result<bool> {
        if self.path.parent().is_some_and(|dir| !dir.is_dir()) {
            return Err(Error::Write {
                path: self.path.clone(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "the mod's folder is gone"),
            });
        }
        write_if_changed(&self.path, &self.doc.to_bytes())
    }

    fn get(&self, key: &str) -> Option<Value> {
        self.doc.value(GENERAL, key)
    }

    fn text(&self, key: &str, default: &str) -> String {
        self.get(key).map_or_else(|| default.to_owned(), |v| v.to_text())
    }

    fn int(&self, key: &str, default: i64) -> i64 {
        self.get(key).map_or(default, |v| to_int(&v).unwrap_or(0))
    }

    fn flag(&self, key: &str) -> bool {
        self.get(key).is_some_and(|v| v.to_bool())
    }

    /// Sets a `[General]` string key. Returns whether the document changed.
    pub fn set_text(&mut self, key: &str, value: &str) -> bool {
        self.doc.set(GENERAL, key, &Value::String(value.to_owned()))
    }

    fn set_int(&mut self, key: &str, value: i64) -> bool {
        self.set_text(key, &value.to_string())
    }

    fn set_flag(&mut self, key: &str, value: bool) -> bool {
        self.set_text(key, if value { "true" } else { "false" })
    }

    // [General], in readMeta's order.

    pub fn comments(&self) -> String {
        self.text("comments", "")
    }
    pub fn set_comments(&mut self, text: &str) -> bool {
        self.set_text("comments", text)
    }
    pub fn notes(&self) -> String {
        self.text("notes", "")
    }
    pub fn set_notes(&mut self, text: &str) -> bool {
        self.set_text("notes", text)
    }
    /// Empty when not set (MO2 then keeps the managed game's short name).
    pub fn game_name(&self) -> String {
        self.text("gameName", "")
    }
    pub fn set_game_name(&mut self, name: &str) -> bool {
        self.set_text("gameName", name)
    }
    /// The Nexus mod id; -1 when not set.
    pub fn mod_id(&self) -> i64 {
        self.int("modid", -1)
    }
    pub fn set_mod_id(&mut self, id: i64) -> bool {
        self.set_int("modid", id)
    }
    pub fn version(&self) -> String {
        self.text("version", "")
    }
    pub fn set_version(&mut self, version: &str) -> bool {
        self.set_text("version", version)
    }
    pub fn newest_version(&self) -> String {
        self.text("newestVersion", "")
    }
    pub fn set_newest_version(&mut self, version: &str) -> bool {
        self.set_text("newestVersion", version)
    }
    pub fn ignored_version(&self) -> String {
        self.text("ignoredVersion", "")
    }
    pub fn installation_file(&self) -> String {
        self.text("installationFile", "")
    }
    pub fn set_installation_file(&mut self, file: &str) -> bool {
        self.set_text("installationFile", file)
    }
    pub fn nexus_description(&self) -> String {
        self.text("nexusDescription", "")
    }
    pub fn nexus_file_status(&self) -> i64 {
        self.int("nexusFileStatus", 1)
    }
    pub fn nexus_category(&self) -> i64 {
        self.int("nexusCategory", 0)
    }
    pub fn repository(&self) -> String {
        self.text("repository", "Nexus")
    }
    pub fn set_repository(&mut self, repository: &str) -> bool {
        self.set_text("repository", repository)
    }
    pub fn converted(&self) -> bool {
        self.flag("converted")
    }
    pub fn validated(&self) -> bool {
        self.flag("validated")
    }
    pub fn url(&self) -> String {
        self.text("url", "")
    }
    pub fn set_url(&mut self, url: &str) -> bool {
        self.set_text("url", url)
    }

    /// `hasCustomURL`, or for files written before 2.2.0 the rule of `readMeta`: a custom
    /// URL when the mod id is not valid and a URL is set.
    pub fn has_custom_url(&self) -> bool {
        match self.get("hasCustomURL") {
            Some(v) => v.to_bool(),
            None => self.mod_id() <= 0 && !self.url().is_empty(),
        }
    }
    pub fn set_has_custom_url(&mut self, on: bool) -> bool {
        self.set_flag("hasCustomURL", on)
    }

    /// ISO dates as stored (`lastNexusQuery`, `lastNexusUpdate`, `nexusLastModified`); empty
    /// when not set.
    pub fn last_nexus_query(&self) -> String {
        self.text("lastNexusQuery", "")
    }
    pub fn last_nexus_update(&self) -> String {
        self.text("lastNexusUpdate", "")
    }
    pub fn nexus_last_modified(&self) -> String {
        self.text("nexusLastModified", "")
    }

    /// The separator colour; `None` when not set or invalid.
    pub fn color(&self) -> Option<Color> {
        self.get("color").as_ref().and_then(decode_color)
    }
    /// Sets the colour as `QSettings` writes a `QColor`; `None` writes an invalid colour,
    /// as MO2 does when the colour is reset.
    pub fn set_color(&mut self, color: Option<Color>) -> bool {
        if self.color() == color && self.get("color").is_some() {
            return false;
        }
        self.doc.set(GENERAL, "color", &encode_color(color))
    }

    pub fn tracked(&self) -> bool {
        self.flag("tracked")
    }

    /// `endorsed` as `readMeta` reads it: a number (0 false, 1 true, 3 never, anything else
    /// unknown; text that is not a number counts as 0, as `QVariant::toInt` gives).
    pub fn endorsed(&self) -> Endorsed {
        match self.get("endorsed") {
            None => Endorsed::Unknown,
            Some(v) => match to_int(&v).unwrap_or(0) {
                0 => Endorsed::False,
                1 => Endorsed::True,
                3 => Endorsed::Never,
                _ => Endorsed::Unknown,
            },
        }
    }

    /// The category ids in the order stored, as `readMeta` takes them from `category`
    /// (`"5,"`, `"5,3,7"`, `"-1,7"`): numbers only, without 0, negatives and repeats.
    /// Whether each category exists is for the caller to check (`Categories`).
    pub fn categories(&self) -> Vec<i32> {
        let mut ids = Vec::new();
        for id in self.category_tokens().into_iter().flatten() {
            if id > 0 && !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    }

    /// The primary category: `readMeta` makes only the *first* entry primary, and only
    /// when it is a valid id, so `"-1,7"` (what `saveMeta` writes without a primary) has
    /// none.
    pub fn primary_category(&self) -> Option<i32> {
        self.category_tokens().first().copied().flatten().filter(|id| *id > 0)
    }

    /// `category` split as `QString::split(',', SkipEmptyParts)`, each token as a number.
    fn category_tokens(&self) -> Vec<Option<i32>> {
        let text = match self.get("category") {
            Some(Value::List(items)) => items.join(","),
            Some(v) => v.to_text(),
            None => String::new(),
        };
        text.split(',').filter(|p| !p.is_empty()).map(|p| p.trim().parse::<i32>().ok()).collect()
    }

    /// Writes `category` as `saveMeta` does: the primary id (or -1), a comma, then the
    /// others ascending.
    pub fn set_categories(&mut self, primary: Option<i32>, others: &[i32]) -> bool {
        let primary = primary.unwrap_or(-1);
        let mut rest: Vec<i32> = others.iter().copied().filter(|id| *id != primary).collect();
        rest.sort_unstable();
        rest.dedup();
        let rest: Vec<String> = rest.iter().map(i32::to_string).collect();
        self.set_text("category", &format!("{primary},{}", rest.join(",")))
    }

    /// `[installedFiles]`: (mod id, file id) pairs.
    pub fn installed_files(&self) -> Vec<(i64, i64)> {
        let size = self.doc.value("installedFiles", "size").and_then(|v| to_int(&v)).unwrap_or(0);
        (1..=size)
            .map(|i| {
                let get = |key: &str| self.doc.value("installedFiles", &format!("{i}/{key}")).and_then(|v| to_int(&v)).unwrap_or(0);
                (get("modid"), get("fileid"))
            })
            .collect()
    }

    /// A plugin's setting in `[Plugins]` (`Sub-Separator\nest` is plugin `Sub-Separator`,
    /// key `nest`).
    pub fn plugin_setting(&self, plugin: &str, key: &str) -> Option<Value> {
        self.doc.value("Plugins", &format!("{plugin}/{key}"))
    }

    pub fn set_plugin_setting(&mut self, plugin: &str, key: &str, value: &Value) -> bool {
        self.doc.set("Plugins", &format!("{plugin}/{key}"), value)
    }

    /// A separator's level (`SubSeparator::nest`): 0 when missing, never negative.
    pub fn nest(&self) -> u32 {
        self.plugin_setting(SUB_SEPARATOR, "nest").and_then(|v| to_int(&v)).map_or(0, |n| n.clamp(0, i64::from(u32::MAX)) as u32)
    }

    /// Sets the level (`SubSeparator::setNest`): nothing when it is the same already.
    pub fn set_nest(&mut self, nest: u32) -> bool {
        if self.nest() == nest {
            return false;
        }
        self.set_plugin_setting(SUB_SEPARATOR, "nest", &Value::String(nest.to_string()))
    }
}

/// The `meta.ini` MO2 writes for a mod it creates (`OrganizerCore::createMod`): what
/// `QSettings` produces from those five values and an empty `installedFiles` array.
pub fn new_mod_meta() -> &'static [u8] {
    b"[General]\nmodid=0\nversion=\nnewestVersion=\ncategory=0\ninstallationFile=\n\n[installedFiles]\nsize=0\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A separator's meta.ini from a real list, as MO2 wrote it.
    const SEPARATOR: &[u8] = b"[General]\r\nmodid=0\r\nversion=\r\nignoredVersion=\r\nnewestVersion=\r\ncategory=\"-1,\"\r\n\
        installationFile=\r\nrepository=Nexus\r\ngameName=SkyrimSE\r\ncomments=\r\nnotes=\r\nnexusDescription=\r\nurl=\r\n\
        hasCustomURL=false\r\nnexusFileStatus=1\r\nlastNexusQuery=\r\nlastNexusUpdate=\r\nnexusLastModified=2026-09-15T22:46:47Z\r\n\
        nexusCategory=0\r\nconverted=false\r\nvalidated=false\r\ncolor=@Variant(\\0\\0\\0\\x43\\x1\\xff\\xff&&\\x88\\x88\\x88\\x88\\0\\0)\r\n\
        tracked=0\r\n\r\n[installedFiles]\r\nsize=0\r\n\r\n[Plugins]\r\nSub-Separator\\maxSize=14\r\nSub-Separator\\nest=1\r\n";

    fn meta(data: &[u8]) -> ModMeta {
        ModMeta::from_bytes(PathBuf::from("meta.ini"), data)
    }

    #[test]
    fn reads_what_mo2_wrote() {
        let m = meta(SEPARATOR);
        assert_eq!(m.mod_id(), 0);
        assert_eq!(m.game_name(), "SkyrimSE");
        assert_eq!(m.repository(), "Nexus");
        assert!(m.categories().is_empty());
        assert_eq!(m.color(), Some(Color { r: 0x26, g: 0x88, b: 0x88, a: 255 }));
        assert_eq!(m.nest(), 1);
        assert_eq!(m.endorsed(), Endorsed::Unknown);
        assert!(!m.has_custom_url());
        assert_eq!(m.nexus_last_modified(), "2026-09-15T22:46:47Z");
        assert_eq!(m.to_bytes(), SEPARATOR);
    }

    #[test]
    fn defaults_are_readmetas() {
        let m = meta(b"");
        assert_eq!((m.mod_id(), m.nexus_file_status(), m.repository().as_str()), (-1, 1, "Nexus"));
        assert_eq!((m.nest(), m.color(), m.endorsed()), (0, None, Endorsed::Unknown));
        let m = meta(b"[General]\nmodid=0\nurl=https://x\n");
        assert!(m.has_custom_url(), "an old file with a URL and no mod id has a custom URL");
        let m = meta(b"[General]\nmodid=12\nurl=https://x\n");
        assert!(!m.has_custom_url());
        assert_eq!(meta(b"[General]\nendorsed=true\n").endorsed(), Endorsed::False, "QVariant::toInt of text is 0");
        assert_eq!(meta(b"[General]\nendorsed=3\n").endorsed(), Endorsed::Never);
        assert_eq!(meta(b"[Plugins]\nSub-Separator\\nest=-2\n").nest(), 0);
    }

    #[test]
    fn categories_read_and_write_as_mo2() {
        assert_eq!(meta(b"[General]\ncategory=\"5,\"\n").categories(), [5]);
        assert_eq!(meta(b"[General]\ncategory=\"7,3,0,-1,x,3\"\n").categories(), [7, 3]);
        assert_eq!(meta(b"[General]\ncategory=\"7,3\"\n").primary_category(), Some(7));
        // "-1,7": 7 is a category but no primary (readMeta only takes the first entry)
        let m = meta(b"[General]\ncategory=\"-1,7,3\"\n");
        assert_eq!((m.categories(), m.primary_category()), (vec![7, 3], None));
        assert_eq!(meta(b"[General]\ncategory=\"-1,\"\n").primary_category(), None);
        // unquoted, Qt reads a list
        assert_eq!(meta(b"[General]\ncategory=7, 3\n").categories(), [7, 3]);
        let mut m = meta(b"[General]\ncategory=\"-1,\"\n");
        assert!(m.set_categories(Some(9), &[4, 9, 2]));
        assert_eq!(m.to_bytes(), b"[General]\ncategory=\"9,2,4\"\n");
        m.set_categories(Some(9), &[]);
        assert_eq!(m.to_bytes(), b"[General]\ncategory=\"9,\"\n");
        m.set_categories(None, &[]);
        assert_eq!(m.to_bytes(), b"[General]\ncategory=\"-1,\"\n");
    }

    #[test]
    fn a_change_touches_only_its_line() {
        let mut m = meta(SEPARATOR);
        assert!(!m.set_nest(1));
        assert!(m.set_nest(3));
        assert!(m.set_notes("look; here"));
        let text = String::from_utf8(m.to_bytes()).unwrap();
        let original = String::from_utf8(SEPARATOR.to_vec()).unwrap();
        let expected = original.replace("nest=1", "nest=3").replace("notes=\r\n", "notes=\"look; here\"\r\n");
        assert_eq!(text, expected);
    }

    #[test]
    fn colors_encode_as_qt_and_round_trip() {
        let mut m = meta(SEPARATOR);
        // the same colour: no change, the original bytes stay
        assert!(!m.set_color(Some(Color { r: 0x26, g: 0x88, b: 0x88, a: 255 })));
        let red = Color { r: 0xad, g: 0x37, b: 0x3b, a: 255 };
        assert!(m.set_color(Some(red)));
        // as written by MO2 for this colour (a real list, 6 separators); Qt quotes it for the ';'
        let text = String::from_utf8(m.to_bytes()).unwrap();
        assert!(text.contains("color=\"@Variant(\\0\\0\\0\\x43\\x1\\xff\\xff\\xad\\xad\\x37\\x37;;\\0\\0)\"\r\n"), "{text}");
        assert_eq!(m.color(), Some(red));
        assert!(m.set_color(None));
        assert!(String::from_utf8(m.to_bytes()).unwrap().contains("color=@Variant(\\0\\0\\0\\x43\\0\\xff\\xff\\0\\0\\0\\0\\0\\0\\0\\0)"));
        assert_eq!(m.color(), None);
        assert_eq!(Color::parse("#268888"), Some(Color { r: 0x26, g: 0x88, b: 0x88, a: 255 }));
        assert_eq!(Color::parse("#80268888").unwrap().name(), "#80268888");
    }

    #[test]
    fn installed_files_and_plugin_settings() {
        let m = meta(b"[installedFiles]\nsize=2\n1\\modid=10\n1\\fileid=20\n2\\modid=11\n2\\fileid=21\n");
        assert_eq!(m.installed_files(), [(10, 20), (11, 21)]);
        let m = meta(SEPARATOR);
        assert_eq!(m.plugin_setting(SUB_SEPARATOR, "maxSize"), Some(Value::String("14".into())));
    }
}
