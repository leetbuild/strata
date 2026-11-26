//! The lossless document: Qt's line splitting, with every byte kept.

use crate::codec::{self, Value};

/// One piece of the file. Concatenating every piece's `raw` gives the file back.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    /// Blank space, line ends, comments, lines Qt ignores.
    Trivia(Vec<u8>),
    /// `[Name]`. `name` is the decoded group ("" for `[General]`).
    Section { raw: Vec<u8>, name: String },
    /// `key=value` (without its line end, which is trivia).
    Entry { raw: Vec<u8>, eq: usize, key: String },
}

impl Piece {
    fn raw(&self) -> &[u8] {
        match self {
            Piece::Trivia(raw) | Piece::Section { raw, .. } | Piece::Entry { raw, .. } => raw,
        }
    }
}

/// A `key=value` line as seen by a reader of the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry<'a> {
    /// Decoded group name, "" for `[General]` and for keys before any section.
    pub section: &'a str,
    /// Decoded key, with `/` between the parts of a nested key (`1/binary`).
    pub key: &'a str,
    /// The key as written in the file (escaped).
    pub raw_key: &'a [u8],
    /// The value as written in the file (escaped, untrimmed).
    pub raw_value: &'a [u8],
}

impl Entry<'_> {
    /// The decoded value.
    pub fn value(&self) -> Value {
        codec::decode_value(self.raw_value)
    }
}

/// An ini file, kept so that an unchanged document writes back byte for byte.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    pieces: Vec<Piece>,
}

const fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// Qt's `readIniLine`: the next logical line in `data` from `pos`.
/// Returns `(line_start, line_end, equals_pos)`; `line_start == line_end` at the end of the data.
fn read_line(data: &[u8], pos: usize) -> (usize, usize, Option<usize>) {
    let len = data.len();
    let mut in_quotes = false;
    let mut equals = None;
    let mut start = pos;
    while start < len && is_space(data[start]) {
        start += 1;
    }
    let mut i = start;
    while i < len {
        let ch = data[i];
        i += 1;
        match ch {
            b'=' => {
                if !in_quotes && equals.is_none() {
                    equals = Some(i - 1);
                }
            }
            b'\n' | b'\r' => {
                if i == start + 1 {
                    start += 1;
                } else if !in_quotes {
                    i -= 1;
                    break;
                }
            }
            b'\\' => {
                if i < len {
                    let next = data[i];
                    i += 1;
                    if i < len {
                        let after = data[i];
                        if (next == b'\n' && after == b'\r') || (next == b'\r' && after == b'\n') {
                            i += 1;
                        }
                    }
                }
            }
            b'"' => in_quotes = !in_quotes,
            b';' => {
                if i == start + 1 {
                    // A comment line: skip it and the space after it.
                    while i < len && data[i] != b'\n' && data[i] != b'\r' {
                        i += 1;
                    }
                    while i < len && is_space(data[i]) {
                        i += 1;
                    }
                    start = i;
                } else if !in_quotes {
                    i -= 1;
                    break;
                }
            }
            _ => {}
        }
    }
    (start, i, equals)
}

fn trim(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|b| !is_space(*b)).unwrap_or(bytes.len());
    let end = bytes.iter().rposition(|b| !is_space(*b)).map_or(start, |p| p + 1);
    &bytes[start..end]
}

impl Document {
    /// Parses a file. Never fails: what Qt would ignore is kept as it is.
    pub fn parse(data: &[u8]) -> Self {
        let mut pieces = Vec::new();
        // Qt skips a UTF-8 byte order mark; it stays in the leading trivia.
        let mut pos = if data.starts_with(b"\xef\xbb\xbf") { 3 } else { 0 };
        let mut trivia_from = 0;
        loop {
            let (start, end, equals) = read_line(data, pos);
            if start == end {
                break;
            }
            pos = end;
            let line = &data[start..end];
            let piece = if line[0] == b'[' {
                // Qt: the name runs to the first ']', anything after it is ignored.
                let close = line.iter().position(|b| *b == b']').unwrap_or(line.len());
                let name = trim(&line[1..close]);
                let name = if name.eq_ignore_ascii_case(b"general") {
                    String::new()
                } else {
                    codec::unescape_key(name)
                };
                Piece::Section { raw: line.to_vec(), name }
            } else if let Some(eq) = equals {
                let eq = eq - start;
                Piece::Entry { raw: line.to_vec(), eq, key: codec::unescape_key(trim(&line[..eq])) }
            } else {
                // A line without '=': Qt drops it. Keep it as trivia.
                continue;
            };
            if trivia_from < start {
                pieces.push(Piece::Trivia(data[trivia_from..start].to_vec()));
            }
            pieces.push(piece);
            trivia_from = end;
        }
        if trivia_from < data.len() {
            pieces.push(Piece::Trivia(data[trivia_from..].to_vec()));
        }
        Self { pieces }
    }

    /// The file's bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.pieces.iter().flat_map(|p| p.raw().iter().copied()).collect()
    }

    /// Every `key=value` line, in file order.
    pub fn entries(&self) -> impl Iterator<Item = Entry<'_>> {
        let mut section = "";
        self.pieces.iter().filter_map(move |piece| match piece {
            Piece::Section { name, .. } => {
                section = name;
                None
            }
            Piece::Entry { raw, eq, key } => Some(Entry {
                section,
                key,
                raw_key: trim(&raw[..*eq]),
                raw_value: &raw[eq + 1..],
            }),
            Piece::Trivia(_) => None,
        })
    }

    /// Section names in file order, each once ("" = `[General]`).
    pub fn sections(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for piece in &self.pieces {
            if let Piece::Section { name, .. } = piece
                && !seen.contains(&name.as_str())
            {
                seen.push(name);
            }
        }
        seen
    }

    /// The entry for `key` in `section` ("" = `[General]`). Section and key names compare
    /// exactly, as in Qt on Linux. When a key is repeated the last one wins, as in Qt.
    pub fn get(&self, section: &str, key: &str) -> Option<Entry<'_>> {
        self.entries().filter(|e| e.section == section && e.key == key).last()
    }

    /// The decoded value of `key` in `section`.
    pub fn value(&self, section: &str, key: &str) -> Option<Value> {
        self.get(section, key).map(|e| e.value())
    }

    /// The value as text: strings as they are, byte arrays as UTF-8, lists joined with ", ".
    pub fn string(&self, section: &str, key: &str) -> Option<String> {
        self.value(section, key).map(|v| v.to_text())
    }

    /// The line end used by the file (`\r\n` when the first line end is one, else `\n`).
    fn line_end(&self) -> &'static [u8] {
        let bytes = self.to_bytes();
        match bytes.iter().position(|b| *b == b'\n') {
            Some(p) if p > 0 && bytes[p - 1] == b'\r' => b"\r\n",
            _ => b"\n",
        }
    }

    /// Sets `key` in `section` to `value`. An existing line keeps its place and its key's
    /// spelling; a new key goes to the end of its section; a new section goes to the end
    /// of the file. Returns whether the document changed.
    pub fn set(&mut self, section: &str, key: &str, value: &Value) -> bool {
        let encoded = codec::encode_value(value);
        let mut current = "";
        let mut found = None;
        let mut section_end = None;
        for (i, piece) in self.pieces.iter().enumerate() {
            match piece {
                Piece::Section { name, .. } => current = name,
                Piece::Entry { key: k, .. } if current == section => {
                    if k == key {
                        found = Some(i);
                    }
                    section_end = Some(i + 1);
                }
                _ => {}
            }
            if current == section && matches!(piece, Piece::Section { .. }) {
                section_end = Some(i + 1);
            }
        }

        if let Some(i) = found {
            let Piece::Entry { raw, eq, .. } = &mut self.pieces[i] else { unreachable!() };
            if raw[*eq + 1..] == encoded[..] {
                return false;
            }
            raw.truncate(*eq + 1);
            raw.extend_from_slice(&encoded);
            return true;
        }

        let eol = self.line_end();
        let mut raw = codec::escape_key(key);
        let eq = raw.len();
        raw.push(b'=');
        raw.extend_from_slice(&encoded);
        let entry = Piece::Entry { raw, eq, key: key.to_owned() };

        match section_end {
            Some(at) => {
                // After the section's last entry (or its header): line end first, then the entry.
                self.pieces.insert(at, entry);
                self.pieces.insert(at, Piece::Trivia(eol.to_vec()));
            }
            None => {
                let ends_with_newline = self.to_bytes().last().is_none_or(|b| *b == b'\n');
                if !ends_with_newline {
                    self.pieces.push(Piece::Trivia(eol.to_vec()));
                }
                if !(section.is_empty() && self.pieces.is_empty()) {
                    let name = if section.is_empty() { b"General".to_vec() } else { codec::escape_key(section) };
                    let mut header = vec![b'['];
                    header.extend_from_slice(&name);
                    header.push(b']');
                    self.pieces.push(Piece::Section { raw: header, name: section.to_owned() });
                    self.pieces.push(Piece::Trivia(eol.to_vec()));
                }
                self.pieces.push(entry);
                self.pieces.push(Piece::Trivia(eol.to_vec()));
            }
        }
        true
    }

    /// Removes `key` from `section` (every occurrence, with its line end). Returns whether
    /// anything was removed.
    pub fn remove(&mut self, section: &str, key: &str) -> bool {
        let mut current = String::new();
        let mut doomed = Vec::new();
        for (i, piece) in self.pieces.iter().enumerate() {
            match piece {
                Piece::Section { name, .. } => current.clone_from(name),
                Piece::Entry { key: k, .. } if current == section && k == key => doomed.push(i),
                _ => {}
            }
        }
        for &i in doomed.iter().rev() {
            self.pieces.remove(i);
            // Drop the line end that followed the entry.
            if let Some(Piece::Trivia(t)) = self.pieces.get_mut(i) {
                let cut = if t.starts_with(b"\r\n") { 2 } else { usize::from(t.starts_with(b"\n")) };
                t.drain(..cut);
                if t.is_empty() {
                    self.pieces.remove(i);
                }
            }
        }
        !doomed.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"[General]\r\ngameName=Skyrim Special Edition\r\nselected_profile=@ByteArray(red)\r\n\
        gamePath=@ByteArray(Z:\\\\home\\\\user\\\\Game Root)\r\n\r\n; a comment\r\n[customExecutables]\r\nsize=1\r\n\
        1\\title=SKSE\r\n1\\arguments=-D:\\\"Z:\\\\x y\\\" -q\r\n[Plugin%20Settings]\r\nSub-Separator\\nest=2\r\n";

    #[test]
    fn unchanged_document_is_byte_identical() {
        for data in [SAMPLE, b"", b"\n\n", b"no equals here\nkey=1", b"; only a comment", b"\xef\xbb\xbf[a]\nb=1\n"] {
            assert_eq!(Document::parse(data).to_bytes(), data);
        }
    }

    #[test]
    fn reads_sections_keys_and_values() {
        let doc = Document::parse(SAMPLE);
        assert_eq!(doc.sections(), ["", "customExecutables", "Plugin Settings"]);
        assert_eq!(doc.string("", "gameName").unwrap(), "Skyrim Special Edition");
        assert_eq!(doc.value("", "selected_profile").unwrap(), Value::ByteArray(b"red".to_vec()));
        assert_eq!(doc.string("", "gamePath").unwrap(), "Z:\\home\\user\\Game Root");
        assert_eq!(doc.string("customExecutables", "1/title").unwrap(), "SKSE");
        assert_eq!(doc.string("customExecutables", "1/arguments").unwrap(), "-D:\"Z:\\x y\" -q");
        assert_eq!(doc.string("Plugin Settings", "Sub-Separator/nest").unwrap(), "2");
        assert!(doc.get("", "missing").is_none());
    }

    #[test]
    fn set_replaces_in_place_and_reports_no_change() {
        let mut doc = Document::parse(SAMPLE);
        assert!(!doc.set("", "gameName", &Value::String("Skyrim Special Edition".into())));
        assert_eq!(doc.to_bytes(), SAMPLE);
        assert!(doc.set("", "gameName", &Value::String("Oblivion".into())));
        let text = String::from_utf8(doc.to_bytes()).unwrap();
        assert!(text.starts_with("[General]\r\ngameName=Oblivion\r\nselected_profile="), "{text}");
        assert_eq!(text.len(), SAMPLE.len() - "Skyrim Special Edition".len() + "Oblivion".len());
    }

    #[test]
    fn set_adds_to_the_end_of_the_section_or_a_new_section() {
        let mut doc = Document::parse(b"[General]\na=1\n\n[B]\nb=2\n");
        doc.set("", "c d", &Value::String("x; y".into()));
        doc.set("New", "n/m", &Value::String("v".into()));
        assert_eq!(
            String::from_utf8(doc.to_bytes()).unwrap(),
            "[General]\na=1\nc%20d=\"x; y\"\n\n[B]\nb=2\n[New]\nn\\m=v\n"
        );
        assert_eq!(doc.string("", "c d").unwrap(), "x; y");
    }

    #[test]
    fn remove_takes_the_line_away() {
        let mut doc = Document::parse(b"[A]\r\nx=1\r\ny=2\r\n");
        assert!(doc.remove("A", "x"));
        assert!(!doc.remove("A", "x"));
        assert_eq!(doc.to_bytes(), b"[A]\r\ny=2\r\n");
    }

    #[test]
    fn a_quoted_value_may_span_lines_and_hold_a_semicolon() {
        let doc = Document::parse(b"a=\"x;\ny\"\nb=1 ; trailing\n");
        assert_eq!(doc.string("", "a").unwrap(), "x;\ny");
        assert_eq!(doc.string("", "b").unwrap(), "1");
    }
}
