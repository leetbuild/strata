//! Qt's escaping of ini keys and values, ported from Qt 6.8 `qsettings.cpp`.
//!
//! Qt works in UTF-16 code units, and so does this module internally: an escape such as
//! `\x2014` is one code unit, and a byte array is one unit (0..=255) per byte.

/// A value as `QSettings` would hand it to the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A plain string. Numbers and booleans are strings too (`true`, `42`).
    String(String),
    /// `@ByteArray(...)`: MO2 stores paths and names this way.
    ByteArray(Vec<u8>),
    /// A comma-separated list (`a, b, "c, d"`).
    List(Vec<String>),
    /// `@Invalid()`: no value, or an empty list.
    Invalid,
    /// `@Variant(...)`, `@DateTime(...)`, `@Rect(...)`, `@Size(...)`, `@Point(...)`: kept as
    /// the decoded text (one char per byte for the binary ones) and written back the same.
    Other(String),
}

impl Value {
    /// The value as text: strings as they are, byte arrays as UTF-8 (lossy), lists joined
    /// with ", ", `Invalid` as "".
    pub fn to_text(&self) -> String {
        match self {
            Value::String(s) | Value::Other(s) => s.clone(),
            Value::ByteArray(b) => String::from_utf8_lossy(b).into_owned(),
            Value::List(items) => items.join(", "),
            Value::Invalid => String::new(),
        }
    }

    /// `true`/`false`/`1`/`0` as Qt's `toBool` reads a string: false for "", "0" and "false"
    /// (any case), true otherwise.
    pub fn to_bool(&self) -> bool {
        let text = self.to_text();
        !(text.is_empty() || text == "0" || text.eq_ignore_ascii_case("false"))
    }
}

const fn hex_upper(n: u32) -> u8 {
    b"0123456789ABCDEF"[(n & 15) as usize]
}

fn hex_value(b: u8) -> Option<u16> {
    (b as char).to_digit(16).map(|d| d as u16)
}

/// `iniEscapedKey`: `/` becomes `\`, anything but `[A-Za-z0-9_.-]` becomes `%XX` or `%UXXXX`.
pub fn escape_key(key: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(key.len() * 3 / 2);
    for unit in key.encode_utf16() {
        let ch = u32::from(unit);
        match ch {
            0x2F => out.push(b'\\'),
            _ if unit < 0x80 && ((unit as u8).is_ascii_alphanumeric() || matches!(unit as u8, b'_' | b'-' | b'.')) => {
                out.push(unit as u8);
            }
            0..=0xFF => out.extend_from_slice(&[b'%', hex_upper(ch >> 4), hex_upper(ch)]),
            _ => {
                out.extend_from_slice(b"%U");
                out.extend_from_slice(&[hex_upper(ch >> 12), hex_upper(ch >> 8), hex_upper(ch >> 4), hex_upper(ch)]);
            }
        }
    }
    out
}

/// `iniUnescapedKey`: the reverse of [`escape_key`]. Bad escapes are kept as they are.
pub fn unescape_key(key: &[u8]) -> String {
    let decoded: Vec<u16> = String::from_utf8_lossy(key).encode_utf16().collect();
    let size = decoded.len();
    let mut out: Vec<u16> = Vec::with_capacity(size);
    let mut i = 0;
    while i < size {
        let ch = decoded[i];
        if ch == u16::from(b'\\') {
            out.push(u16::from(b'/'));
            i += 1;
            continue;
        }
        if ch != u16::from(b'%') || i == size - 1 {
            out.push(ch);
            i += 1;
            continue;
        }
        let (first, digits) = if decoded[i + 1] == u16::from(b'U') { (i + 2, 4) } else { (i + 1, 2) };
        let parsed = decoded.get(first..first + digits).and_then(|units| {
            units.iter().try_fold(0u16, |acc, u| {
                u8::try_from(*u).ok().and_then(hex_value).map(|d| (acc << 4) | d)
            })
        });
        match parsed {
            Some(unit) => {
                out.push(unit);
                i = first + digits;
            }
            None => {
                out.push(u16::from(b'%'));
                i += 1;
            }
        }
    }
    String::from_utf16_lossy(&out)
}

/// What `iniUnescapedStringList` found: one string or a list.
enum Unescaped {
    One(Vec<u16>),
    List(Vec<Vec<u16>>),
}

fn chop_trailing_spaces(s: &mut Vec<u16>, limit: usize) {
    while s.len() > limit && matches!(s.last(), Some(0x20 | 0x09)) {
        s.pop();
    }
}

/// `iniUnescapedStringList`, as the state machine it is in Qt.
fn unescape_string_list(raw: &[u8]) -> Unescaped {
    #[derive(Clone, Copy)]
    enum State {
        SkipSpaces,
        Normal,
        Hex,
        Oct,
    }
    const ESCAPES: [(u8, u16); 11] = [
        (b'a', 7), (b'b', 8), (b'f', 12), (b'n', 10), (b'r', 13), (b't', 9), (b'v', 11),
        (b'"', 34), (b'?', 63), (b'\'', 39), (b'\\', 92),
    ];
    let oct = |b: u8| (b'0'..=b'7').contains(&b).then(|| u16::from(b - b'0'));

    let mut list: Option<Vec<Vec<u16>>> = None;
    let mut cur: Vec<u16> = Vec::new();
    let mut in_quotes = false;
    let mut quoted = false;
    let mut escape: u16 = 0;
    let mut i = 0;
    let mut state = State::SkipSpaces;

    'machine: loop {
        match state {
            State::SkipSpaces => {
                while i < raw.len() && matches!(raw[i], b' ' | b'\t') {
                    i += 1;
                }
                state = State::Normal;
            }
            State::Normal => {
                let mut chop_limit = cur.len();
                while i < raw.len() {
                    match raw[i] {
                        b'\\' => {
                            i += 1;
                            if i >= raw.len() {
                                break 'machine;
                            }
                            let ch = raw[i];
                            i += 1;
                            if let Some((_, unit)) = ESCAPES.iter().find(|(c, _)| *c == ch) {
                                cur.push(*unit);
                                continue 'machine; // back to Normal: resets the chop limit
                            }
                            if ch == b'x' {
                                escape = 0;
                                if i >= raw.len() {
                                    break 'machine;
                                }
                                if raw[i].is_ascii_hexdigit() {
                                    state = State::Hex;
                                    continue 'machine;
                                }
                            } else if let Some(o) = oct(ch) {
                                escape = o;
                                state = State::Oct;
                                continue 'machine;
                            } else if (ch == b'\n' || ch == b'\r') && i < raw.len() {
                                let ch2 = raw[i];
                                if (ch2 == b'\n' || ch2 == b'\r') && ch2 != ch {
                                    i += 1;
                                }
                            }
                            // any other character after a backslash is skipped
                            chop_limit = cur.len();
                        }
                        b'"' => {
                            i += 1;
                            quoted = true;
                            in_quotes = !in_quotes;
                            if !in_quotes {
                                state = State::SkipSpaces;
                                continue 'machine;
                            }
                        }
                        b',' if !in_quotes => {
                            if !quoted {
                                chop_trailing_spaces(&mut cur, chop_limit);
                            }
                            list.get_or_insert_with(Vec::new).push(std::mem::take(&mut cur));
                            quoted = false;
                            i += 1;
                            state = State::SkipSpaces;
                            continue 'machine;
                        }
                        _ => {
                            let mut j = i + 1;
                            while j < raw.len() && !matches!(raw[j], b'\\' | b'"' | b',') {
                                j += 1;
                            }
                            cur.extend(String::from_utf8_lossy(&raw[i..j]).encode_utf16());
                            i = j;
                        }
                    }
                }
                if !quoted {
                    chop_trailing_spaces(&mut cur, chop_limit);
                }
                break 'machine;
            }
            State::Hex | State::Oct => {
                let digit = raw.get(i).and_then(|b| match state {
                    State::Hex => hex_value(*b),
                    _ => oct(*b),
                });
                match digit {
                    Some(d) => {
                        let shift = if matches!(state, State::Hex) { 4 } else { 3 };
                        escape = (escape << shift).wrapping_add(d);
                        i += 1;
                    }
                    None => {
                        cur.push(escape);
                        if i >= raw.len() {
                            break 'machine;
                        }
                        state = State::Normal;
                    }
                }
            }
        }
    }

    match list {
        Some(mut items) => {
            items.push(cur);
            Unescaped::List(items)
        }
        None => Unescaped::One(cur),
    }
}

fn units_to_string(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// One char per byte, as Qt's `toLatin1` (units above 255 become `?`).
fn units_to_latin1(units: &[u16]) -> Vec<u8> {
    units.iter().map(|u| u8::try_from(*u).unwrap_or(b'?')).collect()
}

/// Reads a value as written after the `=` (`iniUnescapedStringList` + `stringToVariant`).
pub fn decode_value(raw: &[u8]) -> Value {
    let units = match unescape_string_list(raw) {
        Unescaped::List(items) => return Value::List(items.iter().map(|u| units_to_string(u)).collect()),
        Unescaped::One(units) => units,
    };
    let starts = |prefix: &str| units.len() >= prefix.len() && prefix.bytes().zip(&units).all(|(b, u)| u16::from(b) == *u);
    if starts("@") {
        if units.last() == Some(&u16::from(b')')) {
            if starts("@ByteArray(") {
                return Value::ByteArray(units_to_latin1(&units[11..units.len() - 1]));
            }
            if starts("@String(") {
                return Value::String(units_to_string(&units[8..units.len() - 1]));
            }
            if starts("@Variant(") || starts("@DateTime(") {
                // Binary: keep one char per byte so it writes back unchanged.
                return Value::Other(units_to_latin1(&units).iter().map(|b| char::from(*b)).collect());
            }
            if starts("@Rect(") || starts("@Size(") || starts("@Point(") {
                return Value::Other(units_to_string(&units));
            }
            if units_to_string(&units) == "@Invalid()" {
                return Value::Invalid;
            }
        }
        if starts("@@") {
            return Value::String(units_to_string(&units[1..]));
        }
    }
    Value::String(units_to_string(&units))
}

/// `iniEscapedString` for one string (already in its `variantToString` form).
fn escape_units(units: &[u16], out: &mut Vec<u8>) {
    let is_prefix = |prefix: &str| units.len() >= prefix.len() && prefix.bytes().zip(units).all(|(b, u)| u16::from(b) == *u);
    let use_codec = !(is_prefix("@ByteArray(") || is_prefix("@Variant(") || is_prefix("@DateTime("));
    let start = out.len();
    let mut needs_quotes = false;
    let mut escape_next_if_digit = false;

    // With the codec, units are written as UTF-8: decode pairs so surrogates join up.
    let chars: Vec<(u32, Option<char>)> = if use_codec {
        char::decode_utf16(units.iter().copied())
            .map(|r| r.map_or((0xFFFD, Some('\u{FFFD}')), |c| (c as u32, Some(c))))
            .collect()
    } else {
        units.iter().map(|u| (u32::from(*u), None)).collect()
    };

    for (ch, as_char) in chars {
        if matches!(ch, 0x3B | 0x2C | 0x3D) {
            needs_quotes = true;
        }
        let is_hex = ch < 0x80 && (ch as u8).is_ascii_hexdigit();
        if escape_next_if_digit && is_hex {
            out.extend_from_slice(format!("\\x{ch:x}").as_bytes());
            continue;
        }
        escape_next_if_digit = false;
        match ch {
            0 => {
                out.extend_from_slice(b"\\0");
                escape_next_if_digit = true;
            }
            7 => out.extend_from_slice(b"\\a"),
            8 => out.extend_from_slice(b"\\b"),
            12 => out.extend_from_slice(b"\\f"),
            10 => out.extend_from_slice(b"\\n"),
            13 => out.extend_from_slice(b"\\r"),
            9 => out.extend_from_slice(b"\\t"),
            11 => out.extend_from_slice(b"\\v"),
            0x22 | 0x5C => out.extend_from_slice(&[b'\\', ch as u8]),
            _ if ch <= 0x1F || (ch >= 0x7F && !use_codec) => {
                out.extend_from_slice(format!("\\x{ch:x}").as_bytes());
                escape_next_if_digit = true;
            }
            _ => match as_char {
                Some(c) => out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
                None => out.push(ch as u8),
            },
        }
    }

    if needs_quotes || (start < out.len() && (out[start] == b' ' || out[out.len() - 1] == b' ')) {
        out.insert(start, b'"');
        out.push(b'"');
    }
}

fn string_units(s: &str) -> Vec<u16> {
    // variantToString: a string holding NUL is wrapped, one starting with '@' gets a second '@'.
    let mut units: Vec<u16> = s.encode_utf16().collect();
    if units.contains(&0) {
        let mut wrapped: Vec<u16> = "@String(".encode_utf16().collect();
        wrapped.append(&mut units);
        wrapped.push(u16::from(b')'));
        wrapped
    } else {
        if units.first() == Some(&u16::from(b'@')) {
            units.insert(0, u16::from(b'@'));
        }
        units
    }
}

/// Writes a value the way `QSettings` does (`variantToString` + `iniEscapedString`).
pub fn encode_value(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    match value {
        Value::String(s) => escape_units(&string_units(s), &mut out),
        Value::ByteArray(bytes) => {
            let mut units: Vec<u16> = "@ByteArray(".encode_utf16().collect();
            units.extend(bytes.iter().map(|b| u16::from(*b)));
            units.push(u16::from(b')'));
            escape_units(&units, &mut out);
        }
        Value::List(items) if items.is_empty() => out.extend_from_slice(b"@Invalid()"),
        Value::List(items) => {
            for (i, item) in items.iter().enumerate() {
                if i != 0 {
                    out.extend_from_slice(b", ");
                }
                // Qt escapes list items as they are (no '@' doubling: they are not variants).
                escape_units(&item.encode_utf16().collect::<Vec<_>>(), &mut out);
            }
        }
        Value::Invalid => out.extend_from_slice(b"@Invalid()"),
        Value::Other(text) => escape_units(&text.encode_utf16().collect::<Vec<_>>(), &mut out),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> Value {
        Value::String(text.to_owned())
    }

    #[test]
    fn keys_escape_and_unescape() {
        for (plain, escaped) in [
            ("1/binary", "1\\binary"),
            ("Sub-Separator/nest", "Sub-Separator\\nest"),
            ("Python Proxy/tryInit", "Python%20Proxy\\tryInit"),
            ("a.b_c-d", "a.b_c-d"),
            ("naïve — key", "na%EFve%20%U2014%20key"),
        ] {
            assert_eq!(escape_key(plain), escaped.as_bytes(), "{plain}");
            assert_eq!(unescape_key(escaped.as_bytes()), plain, "{escaped}");
        }
        assert_eq!(unescape_key(b"50%"), "50%");
        assert_eq!(unescape_key(b"%zz%2"), "%zz%2");
    }

    #[test]
    fn values_decode_as_qt_does() {
        assert_eq!(decode_value(b"Skyrim Special Edition"), s("Skyrim Special Edition"));
        assert_eq!(decode_value(b"  padded  "), s("padded"));
        assert_eq!(decode_value(b"\" kept \""), s(" kept "));
        assert_eq!(decode_value(b"@ByteArray(Z:\\\\home\\\\x y)"), Value::ByteArray(b"Z:\\home\\x y".to_vec()));
        assert_eq!(decode_value(b"@ByteArray(caf\\xe9)"), Value::ByteArray(vec![b'c', b'a', b'f', 0xE9]));
        assert_eq!(decode_value(b"a, b, \"c, d\""), Value::List(vec!["a".into(), "b".into(), "c, d".into()]));
        assert_eq!(decode_value(b"@Invalid()"), Value::Invalid);
        assert_eq!(decode_value(b"@@home"), s("@home"));
        assert_eq!(decode_value(b"@Size(800 600)"), Value::Other("@Size(800 600)".into()));
        assert_eq!(decode_value(b"line\\nnext\\ttab"), s("line\nnext\ttab"));
        assert_eq!(decode_value(b"\\x41\\x2014\\101"), s("A\u{2014}A"));
        assert_eq!(decode_value("caf\u{e9} \u{1F600}".as_bytes()), s("caf\u{e9} \u{1F600}"));
        assert_eq!(decode_value(b""), s(""));
    }

    #[test]
    fn values_encode_as_qt_does() {
        assert_eq!(encode_value(&s("plain text")), b"plain text");
        assert_eq!(encode_value(&s("a=b")), b"\"a=b\"");
        assert_eq!(encode_value(&s("x; y")), b"\"x; y\"");
        assert_eq!(encode_value(&s(" lead")), b"\" lead\"");
        assert_eq!(encode_value(&s("-D:\"Z:\\x\"")), b"-D:\\\"Z:\\\\x\\\"");
        assert_eq!(encode_value(&s("@home")), b"@@home");
        assert_eq!(encode_value(&s("caf\u{e9} \u{1F600}")), "caf\u{e9} \u{1F600}".as_bytes());
        assert_eq!(encode_value(&s("a\u{1}b")), b"a\\x1\\x62");
        assert_eq!(encode_value(&Value::ByteArray(b"Z:\\home".to_vec())), b"@ByteArray(Z:\\\\home)");
        assert_eq!(encode_value(&Value::ByteArray(vec![0xE9, b'1', b'z'])), b"@ByteArray(\\xe9\\x31z)");
        assert_eq!(encode_value(&Value::List(vec!["a".into(), "b c".into(), "d,e".into()])), b"a, b c, \"d,e\"");
        assert_eq!(encode_value(&Value::List(vec![])), b"@Invalid()");
    }

    #[test]
    fn binary_variants_survive() {
        let raw: &[u8] = b"@Variant(\\0\\0\\0\\x43\\x1\\xff\\xff--??FF\\0\\0)";
        let value = decode_value(raw);
        assert!(matches!(value, Value::Other(_)));
        assert_eq!(encode_value(&value), raw);
    }

    #[test]
    fn decode_then_encode_is_stable() {
        for raw in [
            &b"Skyrim Special Edition"[..], b"@ByteArray(Z:\\\\home\\\\user\\\\Games\\\\Skyrim SE\\\\Game Root)",
            b"-D:\\\"Z:\\\\a b\\\" -IKnowWhatImDoing", b"false", b"", b"a, b", b"\"x, y\"", b"@Size(1 2)", b"@Invalid()",
        ] {
            assert_eq!(encode_value(&decode_value(raw)), raw, "{}", String::from_utf8_lossy(raw));
        }
    }
}
