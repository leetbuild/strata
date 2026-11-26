//! Skyrim save games (`.ess`): what MO2's Saves tab shows.
//!
//! Read as MO2's `SkyrimSESaveGame` reads them (Fluorine `libs/game_bethesda`:
//! `skyrimsesavegame.cpp` `fetchInformationFields` / `fetchDataFields`, and
//! `gamebryosavegame.cpp` `FileWrapper`): the `TESV_SAVEGAME` magic, the header (save number,
//! player name, level, location, time of day, race, the save's file time), the screenshot,
//! then the compressed body (Skyrim SE: LZ4 or zlib; Skyrim LE: none) whose start lists the
//! plugins and, from form version 78, the light plugins.

use std::path::Path;

/// Why a save could not be read. MO2 leaves such a file out of the list and logs it
/// (`GameGamebryo::listSaves`).
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("cannot read {path}: {source}")]
    Read { path: std::path::PathBuf, source: std::io::Error },
    #[error("not a Skyrim save: it does not start with TESV_SAVEGAME")]
    WrongMagic,
    #[error("the save is cut short ({0})")]
    Truncated(&'static str),
    #[error("the save's compressed data is damaged ({0})")]
    Compressed(String),
    #[error("unknown compression type {0} in the save")]
    UnknownCompression(u16),
}

/// The screenshot stored in a save: raw pixels, rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    /// RGBA (premultiplied, Skyrim SE) or RGB (Skyrim LE), `width * height * bpp` bytes.
    pub pixels: Vec<u8>,
    pub alpha: bool,
}

/// A save's header and plugin lists.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SaveGame {
    /// 12 for Skyrim SE, 9 for Skyrim LE.
    pub version: u32,
    pub save_number: u32,
    pub player_name: String,
    pub player_level: u16,
    pub location: String,
    pub time_of_day: String,
    pub race: String,
    /// When the game wrote the save, Unix seconds (UTC), as MO2 computes it: the save's file
    /// time, minus six hours for Skyrim SE ("for some reason, the file time is off by about 6
    /// hours", `SkyrimSESaveGame`); Skyrim LE's as it is (`SkyrimSaveGame`).
    pub created: i64,
    pub screenshot: Screenshot,
    /// The form version at the start of the body (78 and up list light plugins).
    pub form_version: u8,
    pub plugins: Vec<String>,
    pub light_plugins: Vec<String>,
}

const MAGIC: &[u8] = b"TESV_SAVEGAME";
const OBLIVION_MAGIC: &[u8] = b"TES4SAVEGAME";

/// Days from 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
const COMPRESSION_NONE: u16 = 0;
const COMPRESSION_ZLIB: u16 = 1;
const COMPRESSION_LZ4: u16 = 2;

/// A cursor over bytes; every read checks the length.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], SaveError> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.data.len()).ok_or(SaveError::Truncated(what))?;
        let out = &self.data[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self, what: &'static str) -> Result<u8, SaveError> {
        Ok(self.take(1, what)?[0])
    }

    fn u16(&mut self, what: &'static str) -> Result<u16, SaveError> {
        let b = self.take(2, what)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self, what: &'static str) -> Result<u32, SaveError> {
        let b = self.take(4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self, what: &'static str) -> Result<u64, SaveError> {
        let lo = u64::from(self.u32(what)?);
        let hi = u64::from(self.u32(what)?);
        Ok(lo | hi << 32)
    }

    /// A `bstring` (Oblivion, `TYPE_BSTRING`): u8 length, then the bytes, read as UTF-8 up to
    /// the first NUL.
    fn bstring(&mut self, what: &'static str) -> Result<String, SaveError> {
        let len = usize::from(self.u8(what)?);
        let bytes = self.take(len, what)?;
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }

    /// A `wstring`: u16 length, then the bytes, read as UTF-8 up to the first NUL
    /// (`FileWrapper::read<QString>`, `StringFormat::UTF8`).
    fn string(&mut self, what: &'static str) -> Result<String, SaveError> {
        let len = usize::from(self.u16(what)?);
        let bytes = self.take(len, what)?;
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }
}

/// 100-ns intervals between 1601-01-01 and 1970-01-01.
const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;
/// Six hours in 100-ns intervals (`SkyrimSESaveGame::SkyrimSESaveGame`).
const SIX_HOURS: u64 = 216_000_000_000;

impl SaveGame {
    /// Reads a save file. `plugins`: also decompress the body for the plugin lists (MO2 does
    /// this lazily, when the Saves tab shows a save's tooltip).
    pub fn read(path: &Path, plugins: bool) -> Result<Self, SaveError> {
        let data = std::fs::read(path).map_err(|source| SaveError::Read { path: path.to_owned(), source })?;
        Self::parse(&data, plugins)
    }

    /// Parses a save's bytes: Skyrim (`TESV_SAVEGAME`) or Oblivion (`TES4SAVEGAME`).
    pub fn parse(data: &[u8], plugins: bool) -> Result<Self, SaveError> {
        if data.starts_with(OBLIVION_MAGIC) {
            return Self::parse_oblivion(data, plugins);
        }
        let mut r = Reader { data, at: 0 };
        if r.take(MAGIC.len(), "magic").ok() != Some(MAGIC) {
            return Err(SaveError::WrongMagic);
        }
        let mut save = Self::default();
        let _header_size = r.u32("header")?;
        save.version = r.u32("header")?;
        save.save_number = r.u32("header")?;
        save.player_name = r.string("player name")?;
        save.player_level = r.u32("player level")? as u16;
        save.location = r.string("location")?;
        save.time_of_day = r.string("time of day")?;
        save.race = r.string("race")?;
        r.take(2 + 4 + 4, "header")?; // gender, experience gathered, experience required
        // SkyrimSESaveGame takes six hours off; Skyrim LE's SkyrimSaveGame does not.
        let raw_time = r.u64("file time")?;
        let filetime = if save.version == 12 { raw_time.wrapping_sub(SIX_HOURS) } else { raw_time };
        save.created = (filetime as i64 - FILETIME_UNIX_EPOCH as i64).div_euclid(10_000_000);

        let width = r.u32("screenshot")?;
        let height = r.u32("screenshot")?;
        // Skyrim SE (version 12) adds the compression type and an alpha channel.
        let (compression, alpha) = if save.version == 12 { (r.u16("compression")?, true) } else { (COMPRESSION_NONE, false) };
        let bpp = if alpha { 4 } else { 3 };
        let size = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(bpp)).ok_or(SaveError::Truncated("screenshot"))?;
        save.screenshot = Screenshot { width, height, pixels: r.take(size, "screenshot")?.to_vec(), alpha };
        if !plugins {
            return Ok(save);
        }

        let body;
        let mut b = match compression {
            COMPRESSION_NONE => Reader { data: &data[r.at..], at: 0 },
            COMPRESSION_ZLIB | COMPRESSION_LZ4 => {
                let uncompressed = r.u32("compressed size")? as usize;
                let compressed = r.u32("compressed size")? as usize;
                let input = r.take(compressed, "compressed data")?;
                body = if compression == COMPRESSION_LZ4 {
                    lz4_flex::block::decompress(input, uncompressed).map_err(|e| SaveError::Compressed(e.to_string()))?
                } else {
                    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(input, uncompressed)
                        .map_err(|e| SaveError::Compressed(format!("{e:?}")))?
                };
                if body.len() != uncompressed {
                    return Err(SaveError::Compressed(format!("{} bytes instead of {uncompressed}", body.len())));
                }
                Reader { data: &body, at: 0 }
            }
            other => return Err(SaveError::UnknownCompression(other)),
        };
        save.form_version = b.u8("form version")?;
        b.take(4, "plugin info size")?;
        let count = b.u8("plugin count")?;
        save.plugins = (0..count).map(|_| b.string("plugin name")).collect::<Result<_, _>>()?;
        if save.form_version >= 78 {
            let count = b.u16("light plugin count")?;
            save.light_plugins = (0..count).map(|_| b.string("light plugin name")).collect::<Result<_, _>>()?;
        }
        Ok(save)
    }

    /// An Oblivion save, as `OblivionSaveGame` reads it: versions, the exe's time, header
    /// version and size, save number, name and location as byte-length strings, level (u16),
    /// game days and ticks, the save's own time (a `SYSTEMTIME` MO2 takes as UTC), the
    /// screenshot (size, then width, height, RGB), then the plugins (u8 count, byte-length
    /// strings). Nothing is compressed.
    fn parse_oblivion(data: &[u8], plugins: bool) -> Result<Self, SaveError> {
        let mut r = Reader { data, at: OBLIVION_MAGIC.len() };
        let mut save = Self::default();
        let major = r.u8("version")?;
        let _minor = r.u8("version")?;
        save.version = u32::from(major);
        r.take(16, "exe time")?;
        let _header_version = r.u32("header")?;
        let _header_size = r.u32("header")?;
        save.save_number = r.u32("save number")?;
        save.player_name = r.bstring("player name")?;
        save.player_level = r.u16("player level")?;
        save.location = r.bstring("location")?;
        r.take(4 + 4, "game time")?; // game days, game ticks
        let mut st = [0u16; 8];
        for v in &mut st {
            *v = r.u16("save time")?;
        }
        // wYear, wMonth, wDayOfWeek, wDay, wHour, wMinute, wSecond, wMilliseconds
        save.created = days_from_civil(i64::from(st[0]), i64::from(st[1]), i64::from(st[3])) * 86_400
            + i64::from(st[4]) * 3600
            + i64::from(st[5]) * 60
            + i64::from(st[6]);
        let image_size = r.u32("screenshot")?;
        if image_size > 0 {
            let width = r.u32("screenshot")?;
            let height = r.u32("screenshot")?;
            let size = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(3)).ok_or(SaveError::Truncated("screenshot"))?;
            save.screenshot = Screenshot { width, height, pixels: r.take(size, "screenshot")?.to_vec(), alpha: false };
        }
        if plugins {
            let count = r.u8("plugin count")?;
            save.plugins = (0..count).map(|_| r.bstring("plugin name")).collect::<Result<_, _>>()?;
        }
        Ok(save)
    }

    /// The name MO2's Saves tab shows: `name, #number, Level level, location`
    /// (`GamebryoSaveGame::getName`).
    pub fn display_name(&self) -> String {
        format!("{}, #{}, Level {}, {}", self.player_name, self.save_number, self.player_level, self.location)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wstring(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u16).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }

    /// A Skyrim SE save with a 2x1 screenshot and an LZ4 body.
    fn build(plugins: &[&str], light: &[&str]) -> Vec<u8> {
        let mut header = Vec::new();
        for v in [12u32, 7] {
            header.extend_from_slice(&v.to_le_bytes());
        }
        wstring(&mut header, "Prisoner");
        header.extend_from_slice(&3u32.to_le_bytes());
        wstring(&mut header, "Helgen Keep");
        wstring(&mut header, "Morndas");
        wstring(&mut header, "NordRace");
        header.extend_from_slice(&[0; 10]);
        // 2026-11-02 01:30:24 UTC + 6 h, as a file time
        let unix = 1_793_583_024u64;
        header.extend_from_slice(&((unix * 10_000_000) + FILETIME_UNIX_EPOCH + SIX_HOURS).to_le_bytes());
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&(header.len() as u32).to_le_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&COMPRESSION_LZ4.to_le_bytes());
        out.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let mut body = vec![78u8, 0, 0, 0, 0, plugins.len() as u8];
        for p in plugins {
            wstring(&mut body, p);
        }
        body.extend_from_slice(&(light.len() as u16).to_le_bytes());
        for p in light {
            wstring(&mut body, p);
        }
        body.extend_from_slice(&[0; 64]);
        let packed = lz4_flex::block::compress(&body);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        out.extend_from_slice(&packed);
        out
    }

    #[test]
    fn a_built_save_reads_back() {
        let data = build(&["Skyrim.esm", "Update.esm"], &["a.esl"]);
        let s = SaveGame::parse(&data, true).unwrap();
        assert_eq!(s.display_name(), "Prisoner, #7, Level 3, Helgen Keep");
        assert_eq!((s.race.as_str(), s.time_of_day.as_str()), ("NordRace", "Morndas"));
        assert_eq!(s.created, 1_793_583_024);
        assert_eq!((s.screenshot.width, s.screenshot.height, s.screenshot.pixels.len()), (2, 1, 8));
        assert_eq!(s.plugins, ["Skyrim.esm", "Update.esm"]);
        assert_eq!(s.light_plugins, ["a.esl"]);
        let header_only = SaveGame::parse(&data, false).unwrap();
        assert!(header_only.plugins.is_empty());
        assert!(matches!(SaveGame::parse(&data[..data.len() - 10], true), Err(SaveError::Truncated(_))));
        assert!(matches!(SaveGame::parse(b"TES4", false), Err(SaveError::WrongMagic)));
    }

    #[test]
    fn real_saves_read() {
        // Read only: the profiles' saves of the instances in `STRATA_REAL_INSTANCES`.
        let instances = std::env::var("STRATA_REAL_INSTANCES").unwrap_or_default();
        let mut dirs = Vec::new();
        for inst in instances.split(':').filter(|s| !s.is_empty()) {
            let Ok(profiles) = std::fs::read_dir(Path::new(inst).join("profiles")) else { continue };
            dirs.extend(profiles.flatten().map(|p| p.path().join("saves")));
        }
        let mut seen = 0;
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "ess")) {
                let s = SaveGame::read(&e.path(), true).unwrap_or_else(|err| panic!("{}: {err}", e.path().display()));
                assert!(matches!(s.version, 9 | 12), "{}: version {}", e.path().display(), s.version); // LE, SE
                assert!(!s.player_name.is_empty());
                assert!(s.plugins.iter().any(|p| p.eq_ignore_ascii_case("Skyrim.esm")), "{:?}", s.plugins);
                seen += 1;
            }
        }
        if seen == 0 {
            eprintln!("SKIPPED: real_saves_read: no saves in STRATA_REAL_INSTANCES (real data, read only)");
        }
    }
    /// An Oblivion save built from `OblivionSaveGame`'s layout.
    #[test]
    fn an_oblivion_save_reads_back() {
        let bstring = |out: &mut Vec<u8>, s: &str| {
            out.push(s.len() as u8 + 1);
            out.extend_from_slice(s.as_bytes());
            out.push(0);
        };
        let mut d = OBLIVION_MAGIC.to_vec();
        d.extend_from_slice(&[0, 125]); // version
        d.extend_from_slice(&[0; 16]); // exe time
        d.extend_from_slice(&[0; 8]); // header version, size
        d.extend_from_slice(&42u32.to_le_bytes());
        bstring(&mut d, "Hero");
        d.extend_from_slice(&7u16.to_le_bytes());
        bstring(&mut d, "Imperial City");
        d.extend_from_slice(&[0; 8]); // game days, ticks
        for v in [2026u16, 11, 3, 25, 6, 30, 15, 500] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&(8 + 6u32).to_le_bytes());
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        d.push(2);
        bstring(&mut d, "Oblivion.esm");
        bstring(&mut d, "Knights.esp");
        let s = SaveGame::parse(&d, true).unwrap();
        assert_eq!(s.display_name(), "Hero, #42, Level 7, Imperial City");
        assert_eq!(s.created, 1_795_588_215, "2026-11-25 06:30:15 as UTC");
        assert_eq!((s.screenshot.width, s.screenshot.height, s.screenshot.alpha), (2, 1, false));
        assert_eq!(s.plugins, ["Oblivion.esm", "Knights.esp"]);
        assert!(matches!(SaveGame::parse(&d[..d.len() - 4], true), Err(SaveError::Truncated(_))));
    }

}
