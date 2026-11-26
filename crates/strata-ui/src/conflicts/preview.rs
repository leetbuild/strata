//! File previews (MO2's `preview_base` and `preview_dds` plugins): DDS textures decoded to
//! pixels, text files decoded to text. The shell turns the pixels into a `QImage`; every other
//! image format is Qt's own (`QImageReader`, as `preview_base` does).
//!
//! DDS: the top mip level of the first surface, in BC1 (DXT1), BC2 (DXT3), BC3 (DXT5), BC4,
//! BC5 (ATI2), BC7 and uncompressed RGB(A) with any channel masks (the legacy header) or the
//! common DX10 formats. Block decoding is the `texture2ddecoder` crate's.

use crate::{Error, Result};

/// Decoded pixels: `bgra` holds `width * height` pixels as B, G, R, A bytes (Qt's
/// `QImage::Format_ARGB32` on a little-endian machine).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    /// What the file holds, for the preview's caption: `BC7`, `DXT1`, `B8G8R8A8`...
    pub format: String,
}

/// How the pixel data is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Bc1,
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Bc7,
    /// Uncompressed: bytes per pixel and the R, G, B, A masks (0 = channel absent).
    Masks { bytes: u32, r: u32, g: u32, b: u32, a: u32 },
}

const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x20000;

fn u32_at(data: &[u8], at: usize) -> u32 {
    data.get(at..at + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn bad(why: &str) -> Error {
    Error::Invalid(format!("not a DDS texture Strata can show: {why}"))
}

/// The DX10 header's `DXGI_FORMAT`: the formats Bethesda games use.
fn dxgi(format: u32) -> Option<(Encoding, &'static str)> {
    let masks = |r, g, b, a| Encoding::Masks { bytes: 4, r, g, b, a };
    Some(match format {
        70..=72 => (Encoding::Bc1, "BC1"),
        73..=75 => (Encoding::Bc2, "BC2"),
        76..=78 => (Encoding::Bc3, "BC3"),
        79..=81 => (Encoding::Bc4, "BC4"),
        82..=84 => (Encoding::Bc5, "BC5"),
        97..=99 => (Encoding::Bc7, "BC7"),
        27..=29 => (masks(0xff, 0xff00, 0xff_0000, 0xff00_0000), "R8G8B8A8"),
        87 | 90 | 91 => (masks(0xff_0000, 0xff00, 0xff, 0xff00_0000), "B8G8R8A8"),
        88 | 92 | 93 => (masks(0xff_0000, 0xff00, 0xff, 0), "B8G8R8X8"),
        _ => return None,
    })
}

/// What a DDS header says: how the pixels are stored, the top level's size, where the data
/// starts, and how many mip levels follow one another.
struct Header {
    encoding: Encoding,
    format: String,
    width: u32,
    height: u32,
    offset: usize,
    mips: u32,
}

/// The bytes the DX10 header ends at: enough to parse any header.
const HEADER_LEN: usize = 148;

fn header(data: &[u8]) -> Result<Header> {
    if data.len() < 128 || &data[..4] != b"DDS " || u32_at(data, 4) != 124 {
        return Err(bad("no DDS header"));
    }
    let height = u32_at(data, 12);
    let width = u32_at(data, 16);
    if width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(bad(&format!("size {width}x{height}")));
    }
    let mips = u32_at(data, 28).max(1);
    let pf_flags = u32_at(data, 80);
    let fourcc = &data[84..88];
    let bits = u32_at(data, 88);
    let mut offset = 128usize;
    let (encoding, format) = if pf_flags & DDPF_FOURCC != 0 {
        match fourcc {
            b"DXT1" => (Encoding::Bc1, "DXT1".to_owned()),
            b"DXT2" | b"DXT3" => (Encoding::Bc2, "DXT3".to_owned()),
            b"DXT4" | b"DXT5" => (Encoding::Bc3, "DXT5".to_owned()),
            b"ATI1" | b"BC4U" => (Encoding::Bc4, "BC4".to_owned()),
            b"ATI2" | b"BC5U" => (Encoding::Bc5, "BC5".to_owned()),
            b"DX10" => {
                offset = HEADER_LEN;
                let code = u32_at(data, 128);
                let (e, name) = dxgi(code).ok_or_else(|| bad(&format!("DXGI format {code}")))?;
                (e, name.to_owned())
            }
            other => return Err(bad(&format!("compression {:?}", String::from_utf8_lossy(other)))),
        }
    } else if pf_flags & (DDPF_RGB | DDPF_LUMINANCE) != 0 && matches!(bits, 8 | 16 | 24 | 32) {
        let a = if pf_flags & DDPF_ALPHAPIXELS != 0 { u32_at(data, 104) } else { 0 };
        let (r, g, b) = (u32_at(data, 92), u32_at(data, 96), u32_at(data, 100));
        // Luminance: the one mask stands for grey.
        let (g, b) = if pf_flags & DDPF_LUMINANCE != 0 { (r, r) } else { (g, b) };
        (Encoding::Masks { bytes: bits / 8, r, g, b, a }, format!("{bits}-bit RGB"))
    } else {
        return Err(bad("unknown pixel format"));
    };
    Ok(Header { encoding, format, width, height, offset, mips })
}

/// The bytes one level of `width` x `height` takes.
fn level_len(encoding: Encoding, width: u32, height: u32) -> usize {
    let blocks = (width.div_ceil(4) * height.div_ceil(4)) as usize;
    match encoding {
        Encoding::Bc1 | Encoding::Bc4 => blocks * 8,
        Encoding::Bc2 | Encoding::Bc3 | Encoding::Bc5 | Encoding::Bc7 => blocks * 16,
        Encoding::Masks { bytes, .. } => (width * height * bytes) as usize,
    }
}

/// Decodes one level whose data starts `body`.
fn decode_level(body: &[u8], encoding: Encoding, width: u32, height: u32) -> Result<Vec<u8>> {
    let pixels = (width * height) as usize;
    let mut out = vec![0u32; pixels];
    let (w, h) = (width as usize, height as usize);
    let result = match encoding {
        Encoding::Bc1 => texture2ddecoder::decode_bc1a(body, w, h, &mut out),
        Encoding::Bc2 => texture2ddecoder::decode_bc2(body, w, h, &mut out),
        Encoding::Bc3 => texture2ddecoder::decode_bc3(body, w, h, &mut out),
        Encoding::Bc4 => texture2ddecoder::decode_bc4(body, w, h, &mut out),
        Encoding::Bc5 => texture2ddecoder::decode_bc5(body, w, h, &mut out),
        Encoding::Bc7 => texture2ddecoder::decode_bc7(body, w, h, &mut out),
        Encoding::Masks { bytes, r, g, b, a } => decode_masks(body, pixels, bytes as usize, [r, g, b, a], &mut out),
    };
    result.map_err(bad)?;
    Ok(out.iter().flat_map(|p| p.to_le_bytes()).collect())
}

/// Decodes the top mip level of a DDS file.
pub fn decode_dds(data: &[u8]) -> Result<Image> {
    let h = header(data)?;
    let bgra = decode_level(&data[h.offset.min(data.len())..], h.encoding, h.width, h.height)?;
    Ok(Image { width: h.width, height: h.height, bgra, format: h.format })
}

/// A thumbnail of the DDS file at `path`: the smallest mip level that is still at least
/// `size` pixels on its longer side (the top level when the file has no smaller ones). Reads
/// the header and that one level, not the file: a 4K texture's 64-pixel level is a few KB.
pub fn decode_dds_thumbnail(path: &std::path::Path, size: u32) -> Result<Image> {
    use std::io::{Read, Seek, SeekFrom};
    let io = |e| Error::Read { path: path.to_owned(), source: e };
    let mut file = std::fs::File::open(path).map_err(io)?;
    let mut head = Vec::with_capacity(HEADER_LEN);
    file.by_ref().take(HEADER_LEN as u64).read_to_end(&mut head).map_err(io)?;
    let h = header(&head)?;
    let (mut w, mut hh, mut at) = (h.width, h.height, h.offset);
    for _ in 1..h.mips {
        let (nw, nh) = ((w / 2).max(1), (hh / 2).max(1));
        // block formats keep whole 4x4 blocks: stop before a level smaller than one
        if nw.max(nh) < size || nw.min(nh) < 4 {
            break;
        }
        at += level_len(h.encoding, w, hh);
        (w, hh) = (nw, nh);
    }
    file.seek(SeekFrom::Start(at as u64)).map_err(io)?;
    let mut body = vec![0u8; level_len(h.encoding, w, hh)];
    file.read_exact(&mut body).map_err(io)?;
    let bgra = decode_level(&body, h.encoding, w, hh)?;
    Ok(Image { width: w, height: hh, bgra, format: h.format })
}

/// One channel from a pixel by its mask, scaled to 8 bits.
fn channel(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let value = (pixel & mask) >> mask.trailing_zeros();
    let max = mask >> mask.trailing_zeros();
    ((u64::from(value) * 255 + u64::from(max) / 2) / u64::from(max)) as u8
}

fn decode_masks(body: &[u8], pixels: usize, bytes: usize, masks: [u32; 4], out: &mut [u32]) -> std::result::Result<(), &'static str> {
    if body.len() < pixels * bytes {
        return Err("Not enough data to decode image!");
    }
    for (i, px) in body.chunks_exact(bytes).take(pixels).enumerate() {
        let mut raw = [0u8; 4];
        raw[..bytes].copy_from_slice(px);
        let p = u32::from_le_bytes(raw);
        let [r, g, b, a] = masks;
        let alpha = if a == 0 { 255 } else { channel(p, a) };
        out[i] = u32::from_le_bytes([channel(p, b), channel(p, g), channel(p, r), alpha]);
    }
    Ok(())
}

/// A text file for the text preview (`PreviewBase` shows text files in a read-only editor):
/// UTF-8, or UTF-16 with a byte order mark; anything else as Windows' Latin-1 would show it.
pub fn decode_text(data: &[u8]) -> String {
    if let Some(rest) = data.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |rest: &[u8], le: bool| {
        let units: Vec<u16> =
            rest.as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) }).collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = data.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = data.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    match std::str::from_utf8(data) {
        Ok(text) => text.to_owned(),
        Err(_) => data.iter().map(|b| char::from(*b)).collect(),
    }
}

/// What kind of preview a file gets, by its name: `dds`, `image` (Qt reads it), `text`, or
/// nothing (`None`). MO2 offers "Preview" only for the extensions its preview plugins name.
pub fn kind(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e)?;
    match ext {
        "dds" => Some("dds"),
        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "tga" | "tif" | "tiff" | "webp" | "ico" | "svg" | "xpm" | "xbm" | "pbm"
        | "pgm" | "ppm" => Some("image"),
        "txt" | "ini" | "json" | "cfg" | "log" | "toml" | "xml" | "md" | "psc" | "yaml" | "yml" | "csv" => Some("text"),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
