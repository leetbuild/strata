use super::*;

/// A DDS header for `width` x `height` with the given pixel format part (flags, fourcc, bits,
/// masks), followed by `body`.
fn dds(width: u32, height: u32, pf_flags: u32, fourcc: &[u8; 4], bits: u32, masks: [u32; 4], body: &[u8]) -> Vec<u8> {
    let mut d = b"DDS ".to_vec();
    let mut header = [0u32; 31];
    header[0] = 124;
    header[2] = height;
    header[3] = width;
    header[18] = 32; // pixel format size
    header[19] = pf_flags;
    header[20] = u32::from_le_bytes(*fourcc);
    header[21] = bits;
    header[22..26].copy_from_slice(&masks);
    for v in header {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(body);
    d
}

fn pixel(img: &Image, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * img.width + x) * 4) as usize;
    let b = &img.bgra[i..i + 4];
    [b[2], b[1], b[0], b[3]] // as R, G, B, A
}

#[test]
fn bc1_block() {
    // color0 = pure red (565: 0xF800), color1 = pure blue (0x001F); indices: row 0 all 0 (red),
    // row 1 all 1 (blue), rows 2-3 index 2 (2/3 red + 1/3 blue).
    let block = [0x00, 0xF8, 0x1F, 0x00, 0b0000_0000, 0b0101_0101, 0b1010_1010, 0b1010_1010];
    let img = decode_dds(&dds(4, 4, DDPF_FOURCC, b"DXT1", 0, [0; 4], &block)).unwrap();
    assert_eq!((img.width, img.height, img.format.as_str()), (4, 4, "DXT1"));
    assert_eq!(pixel(&img, 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&img, 3, 1), [0, 0, 255, 255]);
    let mid = pixel(&img, 0, 2);
    assert!(mid[0] > 150 && mid[2] > 70 && mid[2] < 100, "{mid:?}");
}

#[test]
fn uncompressed_masks_and_dx10() {
    // 2x1 B8G8R8A8: a red opaque pixel and a half-transparent green one.
    let body = [0, 0, 255, 255, 0, 255, 0, 128];
    let legacy = dds(2, 1, DDPF_RGB | DDPF_ALPHAPIXELS, &[0; 4], 32, [0xff_0000, 0xff00, 0xff, 0xff00_0000], &body);
    let img = decode_dds(&legacy).unwrap();
    assert_eq!(pixel(&img, 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&img, 1, 0), [0, 255, 0, 128]);

    // The same pixels in a DX10 file (DXGI_FORMAT_B8G8R8A8_UNORM = 87).
    let mut dx10 = dds(2, 1, DDPF_FOURCC, b"DX10", 0, [0; 4], &[]);
    for v in [87u32, 3, 0, 1, 0] {
        dx10.extend_from_slice(&v.to_le_bytes());
    }
    dx10.extend_from_slice(&body);
    let img2 = decode_dds(&dx10).unwrap();
    assert_eq!((img2.bgra.clone(), img2.format.as_str()), (img.bgra.clone(), "B8G8R8A8"));

    // 5-bit masks scale to 8 bits: R5G6B5 white.
    let white = decode_dds(&dds(1, 1, DDPF_RGB, &[0; 4], 16, [0xF800, 0x07E0, 0x001F, 0], &[0xff, 0xff])).unwrap();
    assert_eq!(pixel(&white, 0, 0), [255, 255, 255, 255]);
}

#[test]
fn refuses_what_it_cannot_show() {
    assert!(decode_dds(b"not a dds").is_err());
    assert!(decode_dds(&dds(4, 4, DDPF_FOURCC, b"DXT1", 0, [0; 4], &[0; 4])).is_err(), "data cut short");
    assert!(decode_dds(&dds(4, 4, DDPF_FOURCC, b"ETC1", 0, [0; 4], &[0; 64])).is_err());
    let mut bc6 = dds(4, 4, DDPF_FOURCC, b"DX10", 0, [0; 4], &[]);
    bc6.extend_from_slice(&[95, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
    assert!(decode_dds(&bc6).is_err(), "BC6H is not offered");
}

#[test]
fn text_encodings_and_kinds() {
    assert_eq!(decode_text(b"\xEF\xBB\xBFhi"), "hi");
    assert_eq!(decode_text(&[0xFF, 0xFE, b'h', 0, b'i', 0]), "hi");
    assert_eq!(decode_text(&[0xFE, 0xFF, 0, b'h', 0, b'i']), "hi");
    assert_eq!(decode_text(b"caf\xe9"), "café", "Latin-1 when not UTF-8");
    assert_eq!(kind("Textures/X.DDS"), Some("dds"));
    assert_eq!(kind("a.png"), Some("image"));
    assert_eq!(kind("readme.TXT"), Some("text"));
    assert_eq!(kind("plugin.esp"), None);
    assert_eq!(kind("noext"), None);
}

/// Real textures of a real instance's mods (the first of `STRATA_REAL_INSTANCES`, read only),
/// one per format found: each decodes to its header's size. Skipped when it isn't set.
#[test]
fn real_textures_decode() {
    let instances = std::env::var("STRATA_REAL_INSTANCES").unwrap_or_default();
    let Some(first) = instances.split(':').find(|s| !s.is_empty()) else {
        return eprintln!("SKIPPED: real_textures_decode: STRATA_REAL_INSTANCES is not set (real data, read only)");
    };
    let mods = std::path::Path::new(first).join("mods");
    if !mods.is_dir() {
        eprintln!("SKIPPED: real_textures_decode: no {} (real data, read only)", mods.display());
        return;
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut stack = vec![mods];
    let mut checked = 0;
    let mut read = 0;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dds"))
                && p.metadata().is_ok_and(|m| m.len() < 256 << 10)
            {
                read += 1;
                let data = std::fs::read(&p).unwrap();
                let Ok(img) = decode_dds(&data) else { continue };
                if seen.insert(img.format.clone()) {
                    assert_eq!(img.bgra.len(), (img.width * img.height * 4) as usize, "{}", p.display());
                    // a texture whose blocks are nearly all zero bytes may well be black
                    let busy = data.iter().skip(148).filter(|b| **b != 0).count() * 4 > data.len().saturating_sub(148);
                    assert!(!busy || img.bgra.iter().any(|b| *b != 0), "{}: all black", p.display());
                    checked += 1;
                }
            }
        }
        if checked >= 6 || read > 400 {
            break;
        }
    }
    eprintln!("decoded formats: {seen:?}");
    assert!(checked > 0);
}

/// A 256x128 B8G8R8A8 texture with four mip levels, each one colour (level n: red = 10 * n):
/// the 64-pixel thumbnail is level 2 (64x32), read from its own offset.
#[test]
fn the_thumbnail_reads_the_smallest_level_still_big_enough() {
    let mut body = Vec::new();
    let (mut w, mut h) = (256u32, 128u32);
    for level in 0..4u8 {
        for _ in 0..w * h {
            body.extend_from_slice(&[0, 0, level * 10, 255]);
        }
        (w, h) = (w / 2, h / 2);
    }
    let mut data = dds(256, 128, DDPF_RGB | DDPF_ALPHAPIXELS, &[0; 4], 32, [0xff_0000, 0xff00, 0xff, 0xff00_0000], &body);
    data[28..32].copy_from_slice(&4u32.to_le_bytes()); // dwMipMapCount
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.dds");
    std::fs::write(&path, &data).unwrap();

    let thumb = decode_dds_thumbnail(&path, 64).unwrap();
    assert_eq!((thumb.width, thumb.height), (64, 32));
    assert_eq!(pixel(&thumb, 5, 5), [20, 0, 0, 255]);
    // bigger than any small level: the top one
    let top = decode_dds_thumbnail(&path, 512).unwrap();
    assert_eq!((top.width, top.height, pixel(&top, 0, 0)), (256, 128, [0, 0, 0, 255]));
    // the full decode is unchanged
    assert_eq!(decode_dds(&data).unwrap().width, 256);
}
