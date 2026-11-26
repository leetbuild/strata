//! Plugin files (`.esm`, `.esp`, `.esl`): the TES4 header record.
//!
//! Read as MO2's `esptk` reads it (`ESP::File::init`, `ESP::Record::readFrom`,
//! `ESP::SubRecord::readFrom` in Fluorine `libs/esptk`): the first record must be `TES4`; its
//! fields are `HEDR` (version, record count), `MAST` (masters), `CNAM` (author) and `SNAM`
//! (description); `XXXX` gives the size of the next field when it is over 64 KiB. Only the
//! header record is read, never the rest of the file.

use std::io::Read;
use std::path::Path;

/// The master flag of the TES4 record (`Record::FLAG_MASTER`).
pub const FLAG_MASTER: u32 = 0x0000_0001;
/// The light flag of Skyrim SE and Fallout 4 (`Record::FLAG_LIGHT`, the FE/FF space).
pub const FLAG_LIGHT: u32 = 0x0000_0200;

/// Why a plugin header could not be read. MO2 treats such a plugin as having no flags and
/// no masters (`PluginList::ESPInfo::ESPInfo` catches the exception and logs it).
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("cannot read {path}: {source}")]
    Read { path: std::path::PathBuf, source: std::io::Error },
    #[error("not a plugin: it does not start with a TES4 record")]
    NotTes4,
    #[error("the plugin is cut short ({0}). Reinstall the mod it comes from.")]
    Truncated(&'static str),
}

/// What the TES4 header of a plugin says.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PluginHeader {
    /// The record flags (master, light, ...).
    pub flags: u32,
    /// The form version: 43 for Skyrim LE, 44 for Skyrim SE. `None` for Oblivion-style
    /// plugins, whose header has no such field.
    pub form_version: Option<u16>,
    /// `HEDR` version (1.7 for Skyrim SE). 0 when the field is missing or the wrong size.
    pub version: f32,
    /// `HEDR` record count. 0 means a dummy plugin (MO2's `isDummy`, "no records").
    pub num_records: i32,
    /// The masters in the order the file lists them (the game resolves form ids with this
    /// order). MO2 keeps them in a sorted set; the order only matters to the game.
    pub masters: Vec<String>,
    /// `CNAM`, read as Latin-1 like MO2.
    pub author: String,
    /// `SNAM`, read as Latin-1 like MO2.
    pub description: String,
}

/// Size of a TES4 record header: type, data size, flags, id, revision (Oblivion-style);
/// Skyrim adds form version and an unknown u16.
const RECORD_HEADER: usize = 20;

fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

/// A zero-terminated string: everything up to the first NUL, as `esptk` reads it.
fn c_bytes(data: &[u8]) -> &[u8] {
    data.iter().position(|b| *b == 0).map_or(data, |end| &data[..end])
}

fn latin1(data: &[u8]) -> String {
    c_bytes(data).iter().map(|b| char::from(*b)).collect()
}

impl PluginHeader {
    /// Reads the header of the plugin file at `path` (only the bytes of the first record).
    pub fn read(path: &Path) -> Result<Self, PluginError> {
        let io = |source| PluginError::Read { path: path.to_owned(), source };
        let mut file = std::fs::File::open(path).map_err(io)?;
        let mut head = vec![0u8; RECORD_HEADER + 4];
        let got = read_up_to(&mut file, &mut head).map_err(io)?;
        head.truncate(got);
        if head.len() < 4 || &head[..4] != b"TES4" {
            return Err(PluginError::NotTes4);
        }
        if head.len() < RECORD_HEADER + 4 {
            return Err(PluginError::Truncated("record header"));
        }
        let size = u32_at(&head, 4) as usize;
        let mut rest = vec![0u8; size];
        let got = read_up_to(&mut file, &mut rest).map_err(io)?;
        head.extend_from_slice(&rest[..got]);
        Self::parse(&head)
    }

    /// Parses a plugin's bytes (at least the first record).
    pub fn parse(data: &[u8]) -> Result<Self, PluginError> {
        if data.len() < 4 || &data[..4] != b"TES4" {
            return Err(PluginError::NotTes4);
        }
        if data.len() < RECORD_HEADER + 4 {
            return Err(PluginError::Truncated("record header"));
        }
        let size = u32_at(data, 4) as usize;
        let flags = u32_at(data, 8);
        // Record::readFrom: if the 4 bytes after the 20-byte header are "HEDR", the plugin
        // is Oblivion-style; otherwise they are the form version and an unknown u16.
        let (form_version, start) = if &data[RECORD_HEADER..RECORD_HEADER + 4] == b"HEDR" {
            (None, RECORD_HEADER)
        } else {
            (Some(u16_at(data, RECORD_HEADER)), RECORD_HEADER + 4)
        };
        let fields = data.get(start..start + size).ok_or(PluginError::Truncated("TES4 record"))?;

        let mut header = Self { flags, form_version, ..Self::default() };
        let mut at = 0;
        let mut size_override: Option<usize> = None;
        while at < fields.len() {
            let head = fields.get(at..at + 6).ok_or(PluginError::Truncated("field header"))?;
            let kind = &head[..4];
            let mut len = usize::from(u16_at(head, 4));
            at += 6;
            if let Some(big) = size_override.take() {
                len = big;
            }
            let body = fields.get(at..at + len).ok_or(PluginError::Truncated("field data"))?;
            at += len;
            match kind {
                b"XXXX" => {
                    if len != 4 {
                        return Err(PluginError::Truncated("XXXX field is not 4 bytes"));
                    }
                    size_override = Some(u32_at(body, 0) as usize);
                }
                // ESP::File::onHEDR: a wrong size gives version 0 and one record.
                b"HEDR" if body.len() == 12 => {
                    header.version = f32::from_le_bytes([body[0], body[1], body[2], body[3]]);
                    header.num_records = u32_at(body, 4) as i32;
                }
                b"HEDR" => {
                    header.version = 0.0;
                    header.num_records = 1;
                }
                b"MAST" if !body.is_empty() => {
                    header.masters.push(String::from_utf8_lossy(c_bytes(body)).into_owned());
                }
                b"CNAM" if !body.is_empty() => header.author = latin1(body),
                b"SNAM" if !body.is_empty() => header.description = latin1(body),
                _ => {}
            }
        }
        Ok(header)
    }

    /// The master flag is set (`ESP::File::isMaster`).
    pub fn is_master_flagged(&self) -> bool {
        self.flags & FLAG_MASTER != 0
    }

    /// The light flag is set (`ESP::File::isLight(false)`, Skyrim SE's flag).
    pub fn is_light_flagged(&self) -> bool {
        self.flags & FLAG_LIGHT != 0
    }

    /// The plugin has no records (`ESP::File::isDummy`).
    pub fn is_dummy(&self) -> bool {
        self.num_records == 0
    }
}

fn read_up_to(file: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match file.read(&mut buf[got..])? {
            0 => break,
            n => got += n,
        }
    }
    Ok(got)
}

/// The name ends in `.esm`, any case (`PluginList::ESPInfo`: `hasMasterExtension`).
pub fn has_master_extension(name: &str) -> bool {
    extension_is(name, "esm")
}

/// The name ends in `.esl`, any case (`hasLightExtension`).
pub fn has_light_extension(name: &str) -> bool {
    extension_is(name, "esl")
}

/// The name is a plugin's: `.esp`, `.esm` or `.esl`, any case (`PluginList::refresh`).
pub fn is_plugin_name(name: &str) -> bool {
    ["esp", "esm", "esl"].iter().any(|ext| extension_is(name, ext))
}

fn extension_is(name: &str, ext: &str) -> bool {
    name.len() > 4
        && name.is_char_boundary(name.len() - 4)
        && name[name.len() - 4..].eq_ignore_ascii_case(&format!(".{ext}"))
}

/// Builds the bytes of a minimal plugin: a TES4 record with `HEDR`, `CNAM`, optional
/// `SNAM` and the masters (each `MAST` followed by its `DATA`, as the Creation Kit writes
/// them). For tests and generated instances.
pub fn build_plugin(flags: u32, form_version: u16, masters: &[&str], author: &str, num_records: u32) -> Vec<u8> {
    fn field(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
        out.extend_from_slice(kind);
        out.extend_from_slice(&(body.len() as u16).to_le_bytes());
        out.extend_from_slice(body);
    }
    let mut fields = Vec::new();
    let mut hedr = 1.71f32.to_le_bytes().to_vec();
    hedr.extend_from_slice(&num_records.to_le_bytes());
    hedr.extend_from_slice(&0x800u32.to_le_bytes());
    field(&mut fields, b"HEDR", &hedr);
    field(&mut fields, b"CNAM", &[author.as_bytes(), b"\0"].concat());
    for master in masters {
        field(&mut fields, b"MAST", &[master.as_bytes(), b"\0"].concat());
        field(&mut fields, b"DATA", &0u64.to_le_bytes());
    }
    let mut out = b"TES4".to_vec();
    out.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // form id
    out.extend_from_slice(&0u32.to_le_bytes()); // revision
    out.extend_from_slice(&form_version.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&fields);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_plugin_reads_back() {
        let data = build_plugin(FLAG_MASTER | FLAG_LIGHT, 44, &["Skyrim.esm", "Update.esm"], "me", 3);
        let h = PluginHeader::parse(&data).unwrap();
        assert_eq!(h.form_version, Some(44));
        assert!(h.is_master_flagged() && h.is_light_flagged() && !h.is_dummy());
        assert_eq!(h.masters, ["Skyrim.esm", "Update.esm"]);
        assert_eq!(h.author, "me");
        assert!((h.version - 1.71).abs() < 1e-6);
    }

    #[test]
    fn trailing_records_are_not_needed_and_a_short_file_is_an_error() {
        let mut data = build_plugin(0, 43, &[], "", 0);
        let whole = PluginHeader::parse(&data).unwrap();
        assert!(whole.is_dummy());
        assert_eq!(whole.form_version, Some(43));
        data.extend_from_slice(b"GRUP....garbage");
        assert_eq!(PluginHeader::parse(&data).unwrap(), whole);
        let cut = build_plugin(0, 44, &["Skyrim.esm"], "", 1);
        assert!(matches!(PluginHeader::parse(&cut[..cut.len() - 3]), Err(PluginError::Truncated(_))));
        assert!(matches!(PluginHeader::parse(b"TES3xxxxxxxxxxxxxxxxxxxxxxx"), Err(PluginError::NotTes4)));
    }

    #[test]
    fn oblivion_style_headers_have_no_form_version() {
        // A 20-byte record header followed straight by HEDR.
        let mut fields = Vec::new();
        fields.extend_from_slice(b"HEDR");
        fields.extend_from_slice(&12u16.to_le_bytes());
        fields.extend_from_slice(&1.0f32.to_le_bytes());
        fields.extend_from_slice(&5u32.to_le_bytes());
        fields.extend_from_slice(&0u32.to_le_bytes());
        fields.extend_from_slice(b"MAST\x0e\x00Oblivion.esm\0\0");
        let mut data = b"TES4".to_vec();
        data.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        data.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        data.extend_from_slice(&fields);
        let h = PluginHeader::parse(&data).unwrap();
        assert_eq!(h.form_version, None);
        assert_eq!(h.num_records, 5);
        assert!(h.is_master_flagged());
        assert_eq!(h.masters, ["Oblivion.esm"]);
    }

    #[test]
    fn xxxx_sets_the_size_of_the_next_field() {
        let mut fields = Vec::new();
        fields.extend_from_slice(b"XXXX\x04\x00");
        fields.extend_from_slice(&5u32.to_le_bytes());
        fields.extend_from_slice(b"CNAM\x00\x00abcd\0");
        let mut data = b"TES4".to_vec();
        data.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        data.extend_from_slice(&[0; 16]);
        data.extend_from_slice(&fields);
        assert_eq!(PluginHeader::parse(&data).unwrap().author, "abcd");
    }

    #[test]
    fn extensions_compare_without_case() {
        assert!(has_master_extension("Skyrim.ESM"));
        assert!(has_light_extension("x.Esl"));
        assert!(is_plugin_name("a b.esp"));
        assert!(!is_plugin_name(".esp") && !is_plugin_name("x.esp.mohidden") && !is_plugin_name("é.bsa"));
    }

    #[test]
    fn real_plugins_read() {
        // Read only: the base game's masters of a real game folder (`STRATA_REAL_GAME`).
        let Some(game) = std::env::var_os("STRATA_REAL_GAME") else {
            return eprintln!("SKIPPED: real_plugins_read: STRATA_REAL_GAME is not set (real data, read only)");
        };
        let data = std::path::PathBuf::from(game).join("Data");
        let Ok(skyrim) = PluginHeader::read(&data.join("Skyrim.esm")) else {
            eprintln!("SKIPPED: real_plugins_read: no {} (real data, read only)", data.display());
            return;
        };
        assert!(skyrim.is_master_flagged());
        assert_eq!(skyrim.form_version, Some(44));
        assert!(skyrim.masters.is_empty());
        let update = PluginHeader::read(&data.join("Update.esm")).unwrap();
        assert_eq!(update.masters, ["Skyrim.esm"]);
    }
}
