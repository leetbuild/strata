//! Bethesda's game archives, read only: BSA (Oblivion v103, Skyrim LE / Fallout 3 / New Vegas
//! v104, Skyrim SE v105) and BA2 (Fallout 4 / 76 / Starfield: general and texture archives).
//!
//! MO2 reads BSAs through bsatk (`BSA::Archive::read`, used by `DirectoryEntry::addFromBSA` for
//! the experimental archive parsing) and Fluorine through the `ba2` crate (`libs/bsa_ffi`). This is
//! a small reader of our own, checked against `ba2` in the tests. Layouts: UESP's "BSA File Format"
//! and "BA2 File Format" pages, the same fields `ba2` reads.
//!
//! Opening reads only the directory part (header, records, names); file data is read on demand.
//! Names are decoded as UTF-8 as MO2 does (`ToWString(name, true)`), `\` becomes `/`.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// What went wrong reading a game archive.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
    #[error("{0} is not a BSA or BA2 archive")]
    NotAnArchive(PathBuf),
    #[error("{0}: unsupported archive version {1}")]
    Unsupported(PathBuf, String),
    #[error("{0} is damaged: {1}")]
    Damaged(PathBuf, String),
    #[error("{0}: {1} can't be extracted yet (texture archives need DDS headers rebuilt)")]
    TextureArchive(PathBuf, String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Which kind of game archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// TES4-style BSA: 103 (Oblivion), 104 (Skyrim LE, Fallout 3, New Vegas), 105 (Skyrim SE).
    Bsa(u32),
    /// BA2: version (1, 7, 8 Fallout 4; 2, 3 Starfield) and whether it holds textures (`DX10`).
    Ba2 { version: u32, textures: bool },
}

/// One file of a game archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedFile {
    /// `folder/name`, `/`-separated, as stored (game archives store lower case).
    pub path: String,
    /// Bytes stored in the archive (compressed size when compressed).
    pub stored_size: u64,
    pub compressed: bool,
    offset: u64,
    /// BA2 only: the size after decompression.
    unpacked_size: u64,
}

/// An opened game archive: its file list. Data is read when asked for.
#[derive(Debug, Clone)]
pub struct GameArchive {
    path: PathBuf,
    format: Format,
    files: Vec<ArchivedFile>,
    /// BSA v104/v105 with flag 0x100: every file's data starts with its full path.
    embedded_names: bool,
    /// BA2 v3: 3 = LZ4 blocks, otherwise zlib.
    ba2_codec: u32,
}

const BSA_DIR_NAMES: u32 = 0x1;
const BSA_FILE_NAMES: u32 = 0x2;
const BSA_COMPRESSED: u32 = 0x4;
const BSA_EMBEDDED_NAMES: u32 = 0x100;
/// In a BSA file record's size: compression is the opposite of the archive's default.
const BSA_TOGGLE_COMPRESSION: u32 = 0x4000_0000;
const BSA_SIZE_MASK: u32 = 0x3FFF_FFFF;

struct Input {
    path: PathBuf,
    r: BufReader<File>,
}

impl Input {
    fn io(&self, e: std::io::Error) -> Error {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            Error::Damaged(self.path.clone(), "it ends too early".into())
        } else {
            Error::Io(self.path.clone(), e)
        }
    }
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut b = [0u8; N];
        self.r.read_exact(&mut b).map_err(|e| self.io(e))?;
        Ok(b)
    }
    fn vec(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut b = vec![0u8; n];
        self.r.read_exact(&mut b).map_err(|e| self.io(e))?;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }
    fn seek(&mut self, to: u64) -> Result<()> {
        self.r.seek(SeekFrom::Start(to)).map(|_| ()).map_err(|e| self.io(e))
    }
}

fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).replace('\\', "/")
}

impl GameArchive {
    /// Reads the file list of a BSA or BA2.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| Error::Io(path.to_owned(), e))?;
        let mut input = Input { path: path.to_owned(), r: BufReader::new(file) };
        let magic: [u8; 4] = input.bytes().map_err(|_| Error::NotAnArchive(path.to_owned()))?;
        match &magic {
            b"BSA\0" => Self::open_bsa(input),
            b"BTDX" => Self::open_ba2(input),
            _ => Err(Error::NotAnArchive(path.to_owned())),
        }
    }

    fn open_bsa(mut input: Input) -> Result<Self> {
        let version = input.u32()?;
        if !matches!(version, 103..=105) {
            return Err(Error::Unsupported(input.path, format!("BSA {version}")));
        }
        let folder_offset = input.u32()?;
        let flags = input.u32()?;
        let folder_count = input.u32()? as usize;
        let file_count = input.u32()? as usize;
        let _folder_names_len = input.u32()?;
        let file_names_len = input.u32()? as usize;
        input.seek(u64::from(folder_offset))?;
        let mut counts = Vec::with_capacity(folder_count.min(1 << 20));
        for _ in 0..folder_count {
            let _hash = input.u64()?;
            counts.push(input.u32()? as usize);
            if version == 105 {
                let _pad = input.u32()?;
                let _offset = input.u64()?;
            } else {
                let _offset = input.u32()?;
            }
        }
        if counts.iter().sum::<usize>() != file_count {
            return Err(Error::Damaged(input.path, "the folders do not add up to the file count".into()));
        }
        // The file records, each folder's block led by its name.
        let default_compressed = flags & BSA_COMPRESSED != 0;
        let mut files = Vec::with_capacity(file_count.min(1 << 22));
        for &count in &counts {
            let folder = if flags & BSA_DIR_NAMES != 0 {
                let len = input.u8()? as usize;
                text(&input.vec(len)?)
            } else {
                String::new()
            };
            for _ in 0..count {
                let _hash = input.u64()?;
                let size = input.u32()?;
                let offset = input.u32()?;
                files.push(ArchivedFile {
                    path: String::new(),
                    stored_size: u64::from(size & BSA_SIZE_MASK),
                    compressed: default_compressed != (size & BSA_TOGGLE_COMPRESSION != 0),
                    offset: u64::from(offset),
                    unpacked_size: 0,
                });
            }
            // The folder for now; the file name is appended from the name block below.
            let start = files.len() - count;
            for f in &mut files[start..] {
                f.path.clone_from(&folder);
            }
        }
        if flags & BSA_FILE_NAMES != 0 {
            let names = input.vec(file_names_len)?;
            let mut parts = names.split(|b| *b == 0);
            for f in &mut files {
                let name = parts
                    .next()
                    .ok_or_else(|| Error::Damaged(input.path.clone(), "fewer names than files".into()))?;
                let name = text(name);
                f.path = match f.path.trim_matches('/') {
                    "" | "." => name,
                    folder => format!("{folder}/{name}"),
                };
            }
        }
        Ok(Self {
            path: input.path,
            format: Format::Bsa(version),
            files,
            embedded_names: version >= 104 && flags & BSA_EMBEDDED_NAMES != 0,
            ba2_codec: 0,
        })
    }

    fn open_ba2(mut input: Input) -> Result<Self> {
        let version = input.u32()?;
        if !matches!(version, 1..=3 | 7 | 8) {
            return Err(Error::Unsupported(input.path, format!("BA2 {version}")));
        }
        let kind: [u8; 4] = input.bytes()?;
        let textures = match &kind {
            b"GNRL" => false,
            b"DX10" => true,
            other => return Err(Error::Unsupported(input.path, format!("BA2 type {}", String::from_utf8_lossy(other)))),
        };
        let count = input.u32()? as usize;
        let names_offset = input.u64()?;
        let mut ba2_codec = 0;
        if matches!(version, 2 | 3) {
            let _unknown = input.u64()?;
        }
        if version == 3 {
            ba2_codec = input.u32()?;
        }
        let mut files = Vec::with_capacity(count.min(1 << 22));
        for _ in 0..count {
            let _name_hash = input.u32()?;
            let _ext: [u8; 4] = input.bytes()?;
            let _dir_hash = input.u32()?;
            if textures {
                let _unknown = input.u8()?;
                let chunks = input.u8()?;
                let _chunk_header = input.u16()?;
                let _dims: [u8; 8] = input.bytes()?; // height, width, mips, format, cubemap, tile mode
                let (mut stored, mut unpacked, mut first) = (0u64, 0u64, None);
                for _ in 0..chunks {
                    let offset = input.u64()?;
                    let packed = input.u32()?;
                    let size = input.u32()?;
                    let _mips: [u8; 4] = input.bytes()?;
                    let _sentinel = input.u32()?;
                    first.get_or_insert(offset);
                    stored += u64::from(if packed == 0 { size } else { packed });
                    unpacked += u64::from(size);
                }
                files.push(ArchivedFile {
                    path: String::new(),
                    stored_size: stored,
                    compressed: stored != unpacked,
                    offset: first.unwrap_or(0),
                    unpacked_size: unpacked,
                });
            } else {
                let _flags = input.u32()?;
                let offset = input.u64()?;
                let packed = input.u32()?;
                let size = input.u32()?;
                let _sentinel = input.u32()?;
                files.push(ArchivedFile {
                    path: String::new(),
                    stored_size: u64::from(if packed == 0 { size } else { packed }),
                    compressed: packed != 0,
                    offset,
                    unpacked_size: u64::from(size),
                });
            }
        }
        if names_offset != 0 {
            input.seek(names_offset)?;
            for f in &mut files {
                let len = input.u16()? as usize;
                f.path = text(&input.vec(len)?);
            }
        }
        Ok(Self { path: input.path, format: Format::Ba2 { version, textures }, files, embedded_names: false, ba2_codec })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn format(&self) -> Format {
        self.format
    }

    pub fn files(&self) -> &[ArchivedFile] {
        &self.files
    }

    /// The content of file `index`, decompressed.
    pub fn read(&self, index: usize) -> Result<Vec<u8>> {
        let damaged = |why: String| Error::Damaged(self.path.clone(), why);
        let f = self.files.get(index).ok_or_else(|| damaged(format!("no file {index}")))?;
        if let Format::Ba2 { textures: true, .. } = self.format {
            return Err(Error::TextureArchive(self.path.clone(), f.path.clone()));
        }
        let file = File::open(&self.path).map_err(|e| Error::Io(self.path.clone(), e))?;
        let mut input = Input { path: self.path.clone(), r: BufReader::new(file) };
        input.seek(f.offset)?;
        let mut remaining = f.stored_size;
        if self.embedded_names {
            let len = input.u8()?;
            input.vec(usize::from(len))?;
            remaining = remaining.checked_sub(1 + u64::from(len)).ok_or_else(|| damaged(format!("{}: size", f.path)))?;
        }
        let (expected, codec_lz4_frame) = match self.format {
            Format::Bsa(version) if f.compressed => {
                let original = u64::from(input.u32()?);
                remaining = remaining.checked_sub(4).ok_or_else(|| damaged(format!("{}: size", f.path)))?;
                (original, version == 105)
            }
            Format::Bsa(_) => (remaining, false),
            Format::Ba2 { .. } => (f.unpacked_size, false),
        };
        let data = input.vec(usize::try_from(remaining).map_err(|_| damaged(format!("{}: size", f.path)))?)?;
        if !f.compressed {
            return Ok(data);
        }
        let mut out = Vec::with_capacity(usize::try_from(expected).unwrap_or(0));
        let result = if codec_lz4_frame {
            lz4_flex::frame::FrameDecoder::new(&data[..]).read_to_end(&mut out).map(|_| ())
        } else if matches!(self.format, Format::Ba2 { version: 3, .. }) && self.ba2_codec == 3 {
            lz4_flex::block::decompress(&data, usize::try_from(expected).unwrap_or(0))
                .map(|d| out = d)
                .map_err(std::io::Error::other)
        } else {
            inflate(&data, &mut out)
        };
        result.map_err(|e| damaged(format!("{}: {e}", f.path)))?;
        if out.len() as u64 != expected {
            return Err(damaged(format!("{}: {} bytes after decompression, {expected} expected", f.path, out.len())));
        }
        Ok(out)
    }

    /// Writes every file under `out_dir` (folders made as needed). Refuses unsafe paths.
    pub fn extract_all(&self, out_dir: &Path) -> Result<()> {
        for (i, f) in self.files.iter().enumerate() {
            let rel = crate::sevenzip::safe_relative(&f.path)
                .ok_or_else(|| Error::Damaged(self.path.clone(), format!("unsafe path {:?}", f.path)))?;
            let dest = out_dir.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::Io(parent.to_owned(), e))?;
            }
            std::fs::write(&dest, self.read(i)?).map_err(|e| Error::Io(dest.clone(), e))?;
        }
        Ok(())
    }
}

/// zlib, tolerating a stream cut short after the last byte of output: Bethesda's own tools wrote
/// some (Skyrim LE's `HighResTexturePack02.bsa`, `textures/clutter/placeholder.txt`: "Temp" with
/// no end block). The caller checks the length.
fn inflate(data: &[u8], out: &mut Vec<u8>) -> std::io::Result<()> {
    let mut z = flate2::Decompress::new(true);
    loop {
        let before = (z.total_in(), z.total_out());
        out.reserve(64 * 1024);
        let status = z
            .decompress_vec(&data[z.total_in() as usize..], out, flate2::FlushDecompress::None)
            .map_err(std::io::Error::other)?;
        if status == flate2::Status::StreamEnd || (z.total_in(), z.total_out()) == before {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests;
