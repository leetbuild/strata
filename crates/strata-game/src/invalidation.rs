//! Archive invalidation for the older games (`GamebryoBSAInvalidation`, `DummyBSA` in Fluorine
//! `libs/game_bethesda/src/gamebryo`), and the Windows-style ini editing it needs
//! (`GetPrivateProfileString` / `WritePrivateProfileString`, which MO2 uses on the game's ini).

/// The bytes of MO2's dummy archive: one empty folder holding an empty `dummy.dds`
/// (`DummyBSA::write`, byte for byte, including its fixed offsets).
pub fn dummy_bsa(version: u32) -> Vec<u8> {
    let folder = "";
    let file = "dummy.dds";
    let total_file_name_length = file.len() as u32 + 1;
    let mut out = Vec::new();
    let u32le = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    // header
    out.extend_from_slice(b"BSA\0");
    u32le(&mut out, version);
    u32le(&mut out, 0x24); // offset to the folder records
    u32le(&mut out, 0x01 | 0x02); // has directories, has files
    u32le(&mut out, 1); // folder count
    u32le(&mut out, 1); // file count
    u32le(&mut out, folder.len() as u32 + 1); // total folder name length
    u32le(&mut out, total_file_name_length);
    u32le(&mut out, 2); // file flags: has dds
    // folder record
    out.extend_from_slice(&gen_hash(folder).to_le_bytes());
    u32le(&mut out, 1);
    u32le(&mut out, 0x34 + total_file_name_length);
    // file record block: folder name, then the file record
    out.extend_from_slice(folder.as_bytes());
    out.push(0);
    out.extend_from_slice(&gen_hash(file).to_le_bytes());
    u32le(&mut out, 0);
    u32le(&mut out, 0x44 + file.len() as u32 + 1 + 4);
    // file names, then the file's 4-byte size
    out.extend_from_slice(file.as_bytes());
    out.push(0);
    u32le(&mut out, 0);
    out
}

/// `genHashInt`.
fn gen_hash_int(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |h, b| h.wrapping_mul(0x1003f).wrapping_add(u32::from(*b)))
}

/// `genHash`: the BSA name hash of MO2's dummy archive.
fn gen_hash(name: &str) -> u64 {
    let lower: Vec<u8> = name.bytes().map(|b| if b == b'\\' { b'/' } else { b.to_ascii_lowercase() }).collect();
    let ext_at = lower.iter().rposition(|b| *b == b'.').unwrap_or(lower.len());
    let (stem, ext) = lower.split_at(ext_at);
    let length = stem.len() as u64;
    let mut hash: u64 = 0;
    if length > 0 {
        let last = u64::from(stem[stem.len() - 1]);
        let second_last = if length > 2 { u64::from(stem[stem.len() - 2]) } else { 0 };
        hash = last | (second_last << 8) | (length << 16) | (u64::from(stem[0]) << 24);
    }
    if !ext.is_empty() {
        match &ext[1..] {
            b"kf" => hash |= 0x80,
            b"nif" => hash |= 0x8000,
            b"dds" => hash |= 0x8080,
            b"wav" => hash |= 0x8000_0000,
            _ => {}
        }
        // genHashInt(fileNameLowerU + 1, extU - 2): the stem without its first and last two
        // characters (empty when the stem is that short).
        let middle = if stem.len() > 3 { &stem[1..stem.len() - 2] } else { &[][..] };
        let temp = u64::from(gen_hash_int(middle)) + u64::from(gen_hash_int(ext));
        hash |= (temp & 0xFFFF_FFFF) << 32;
    }
    hash
}

/// A value of a Windows ini as `GetPrivateProfileString` reads it: section and key without
/// case, the value trimmed (and one pair of surrounding quotes removed). `None` when absent.
pub fn ini_get(data: &[u8], section: &str, key: &str) -> Option<String> {
    let text = String::from_utf8_lossy(data);
    let mut current = String::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('[') {
            current = rest.split(']').next().unwrap_or("").trim().to_owned();
        } else if current.eq_ignore_ascii_case(section)
            && let Some((k, v)) = t.split_once('=')
            && k.trim().eq_ignore_ascii_case(key)
        {
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(v);
            return Some(v.to_owned());
        }
    }
    None
}

/// The ini with `key` set in `section`, as `WritePrivateProfileString` does it: the existing
/// line is replaced (its key spelling kept), or the key goes after the section's last line,
/// or a new section goes at the end. Every other byte stays as it was; line ends follow the
/// file (CRLF when it has any).
pub fn ini_set(data: &[u8], section: &str, key: &str, value: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(data).into_owned();
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
    let mut current = String::new();
    let mut section_end: Option<usize> = None; // index after the section's last non-blank line
    for i in 0..lines.len() {
        let t = lines[i].trim().to_owned();
        if let Some(rest) = t.strip_prefix('[') {
            current = rest.split(']').next().unwrap_or("").trim().to_owned();
            if current.eq_ignore_ascii_case(section) {
                section_end = Some(i + 1);
            }
            continue;
        }
        if !current.eq_ignore_ascii_case(section) {
            continue;
        }
        if let Some((k, _)) = t.split_once('=')
            && k.trim().eq_ignore_ascii_case(key)
        {
            let ending = if lines[i].ends_with("\r\n") { "\r\n" } else if lines[i].ends_with('\n') { "\n" } else { "" };
            lines[i] = format!("{}={value}{ending}", k.trim());
            return lines.concat().into_bytes();
        }
        if !t.is_empty() {
            section_end = Some(i + 1);
        }
    }
    let new_line = format!("{key}={value}{eol}");
    match section_end {
        Some(at) => {
            if at > 0 && !lines[at - 1].ends_with('\n') {
                lines[at - 1].push_str(eol);
            }
            lines.insert(at, new_line);
        }
        None => {
            if lines.last().is_some_and(|l| !l.ends_with('\n')) {
                lines.push(eol.to_owned());
            }
            lines.push(format!("[{section}]{eol}"));
            lines.push(new_line);
        }
    }
    lines.concat().into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dummy_bsa_is_mo2s_byte_for_byte() {
        // What Fluorine's DummyBSA(0x68) wrote into the LE game folder (od -t x1 of the
        // real `Skyrim - Invalidation.bsa` of an LE instance, read only).
        let want: [u8; 83] = [
            0x42, 0x53, 0x41, 0x00, 0x68, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3e, 0x00, 0x00, 0x00, 0x00, 0xf9,
            0xed, 0x05, 0x64, 0xfd, 0xc6, 0x50, 0x8e, 0x00, 0x00, 0x00, 0x00, 0x52, 0x00, 0x00, 0x00, 0x64, 0x75, 0x6d,
            0x6d, 0x79, 0x2e, 0x64, 0x64, 0x73, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(dummy_bsa(0x68), want);
    }

    #[test]
    fn ini_values_are_read_and_written_like_windows_does() {
        let ini = b"[General]\r\nsLanguage=ENGLISH\r\n\r\n[archive]\r\nSInvalidationFile=ArchiveInvalidation.txt\r\nbInvalidateOlderFiles=0\r\n\r\n[Audio]\r\nx=1\r\n";
        assert_eq!(ini_get(ini, "Archive", "binvalidateolderfiles").as_deref(), Some("0"));
        assert_eq!(ini_get(ini, "Archive", "Missing"), None);
        let set = ini_set(ini, "Archive", "bInvalidateOlderFiles", "1");
        assert_eq!(set, b"[General]\r\nsLanguage=ENGLISH\r\n\r\n[archive]\r\nSInvalidationFile=ArchiveInvalidation.txt\r\nbInvalidateOlderFiles=1\r\n\r\n[Audio]\r\nx=1\r\n");
        let added = ini_set(&set, "Archive", "SResourceArchiveList", "a.bsa, b.bsa");
        assert_eq!(ini_get(&added, "Archive", "SResourceArchiveList").as_deref(), Some("a.bsa, b.bsa"));
        assert!(String::from_utf8_lossy(&added).contains("bInvalidateOlderFiles=1\r\nSResourceArchiveList=a.bsa, b.bsa\r\n\r\n[Audio]"));
        let new_section = ini_set(b"[General]\nx=1", "Archive", "k", "v");
        assert_eq!(new_section, b"[General]\nx=1\n[Archive]\nk=v\n");
        assert_eq!(ini_get(b"[A]\nk = \"quoted\" \n", "a", "K").as_deref(), Some("quoted"));
    }
}
