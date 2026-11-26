//! MO2's `VersionInfo` (uibase `versioninfo.cpp`): enough of it to write a version the way
//! `ModInfoRegular::saveMeta` does (`canonicalString`), e.g. `d2026.11.9` → `d2026.11.9.0`,
//! `1.2` → `1.2.0.0`, `1.05` → `f1.05`, `2.0 beta3` → `2.0.0.0b3`.

use std::cmp::Ordering;
use std::sync::LazyLock;

use regex::Regex;

static VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d+)(\.(\d+))?(\.(\d+))?(\.(\d+))?").expect("valid regex"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scheme {
    Regular,
    DecimalMark,
    NumbersAndLetters,
    Date,
    Literal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Release {
    PreAlpha,
    Alpha,
    Beta,
    Candidate,
    Final,
}

/// A parsed version (`VersionInfo(QString)`, scheme discovered from the string).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    valid: bool,
    scheme: Scheme,
    release: Release,
    parts: [i64; 4],
    decimal_positions: usize,
    rest: String,
}

/// `QString::toInt`: 0 when it does not fit.
fn to_int(s: &str) -> i64 {
    s.parse::<i32>().map_or(0, i64::from)
}

impl Version {
    /// `VersionInfo::parse(versionString, SCHEME_DISCOVER, false)`.
    pub fn parse(text: &str) -> Self {
        let mut v = Version {
            valid: false,
            scheme: Scheme::Regular,
            release: Release::Final,
            parts: [0; 4],
            decimal_positions: 0,
            rest: String::new(),
        };
        if text.is_empty() {
            return v;
        }
        if text.eq_ignore_ascii_case("final") {
            v.parts[0] = 1;
            v.valid = true;
            return v;
        }
        let mut temp = text;
        if let Some(t) = temp.strip_prefix('f') {
            v.scheme = Scheme::DecimalMark;
            temp = t;
        } else if let Some(t) = temp.strip_prefix('n') {
            v.scheme = Scheme::NumbersAndLetters;
            temp = t;
        } else if let Some(t) = temp.strip_prefix('d') {
            v.scheme = Scheme::Date;
            temp = t;
        }
        if temp.starts_with(['v', 'V']) {
            temp = &temp[1..];
        }
        let mut temp = temp.to_owned();
        if let Some(m) = VERSION.captures(&temp) {
            let cap = |i: usize| m.get(i).map_or("", |c| c.as_str());
            v.parts[0] = to_int(cap(1));
            v.parts[1] = to_int(cap(3));
            let (sub, subsub, minor) = (cap(5).to_owned(), cap(7).to_owned(), cap(3).to_owned());
            if !sub.is_empty() && v.scheme == Scheme::DecimalMark {
                // two dots: not a decimal mark
                v.scheme = Scheme::Regular;
            }
            if v.scheme != Scheme::DecimalMark {
                v.parts[2] = to_int(&sub);
                v.parts[3] = to_int(&subsub);
            }
            if sub.is_empty() && minor.len() > 1 && minor.starts_with('0') {
                v.scheme = Scheme::DecimalMark;
                v.decimal_positions = minor.len();
            }
            let end = m.get(0).map_or(0, |c| c.end());
            temp.replace_range(..end, "");
        } else {
            v.scheme = Scheme::Literal;
        }
        if v.scheme == Scheme::Regular {
            temp = v.parse_release_type(&temp);
        }
        if v.scheme == Scheme::Date && v.parts[0] < 1900 {
            v.scheme = Scheme::Regular;
        }
        v.rest = temp.trim().to_owned();
        v.valid = true;
        v
    }

    /// `VersionInfo::parseReleaseType`: the first of `alpha`, `beta`, `prealpha`, `rc` (the
    /// keys of a `std::map`, so in that order) found anywhere, without case, is cut out; a
    /// leading `a` / `b` counts too.
    fn parse_release_type(&mut self, text: &str) -> String {
        const TYPES: [(&str, Release); 4] = [
            ("alpha", Release::Alpha),
            ("beta", Release::Beta),
            ("prealpha", Release::PreAlpha),
            ("rc", Release::Candidate),
        ];
        self.release = Release::Final;
        let lower = text.to_ascii_lowercase();
        let mut cut = TYPES.iter().find_map(|(word, release)| lower.find(word).map(|at| (at, word.len(), *release)));
        if cut.is_none() {
            if text.starts_with('a') {
                cut = Some((0, 1, Release::Alpha));
            } else if text.starts_with('b') {
                cut = Some((0, 1, Release::Beta));
            }
        }
        let mut out = text.to_owned();
        if let Some((at, len, release)) = cut {
            self.release = release;
            out.replace_range(at..at + len, "");
        }
        out.trim().to_owned()
    }

    /// `VersionInfo::canonicalString`: empty for an invalid (empty) version.
    pub fn canonical(&self) -> String {
        if !self.valid {
            return String::new();
        }
        let [a, b, c, d] = self.parts;
        let mut out = match self.scheme {
            Scheme::Regular => format!("{a}.{b}.{c}.{d}"),
            Scheme::DecimalMark => format!("f{a}.{b:0>width$}", width = self.decimal_positions),
            Scheme::NumbersAndLetters => format!("n{a}.{b}.{c}.{d}"),
            Scheme::Date => format!("d{a}.{b}.{c}.{d}"),
            Scheme::Literal => String::new(),
        };
        out.push_str(match self.release {
            Release::PreAlpha => " pre-alpha",
            Release::Alpha => "a",
            Release::Beta => "b",
            Release::Candidate => "rc",
            Release::Final => "",
        });
        out.push_str(&self.rest);
        out
    }
}

impl Version {
    /// Whether the string held a version (`VersionInfo::isValid`: not empty).
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// The decimal value MO2 compares decimal-mark versions by (`QString("%1.%2").toFloat()`).
    fn decimal(&self) -> f32 {
        format!("{}.{:0>width$}", self.parts[0], self.parts[1], width = self.decimal_positions).parse().unwrap_or(0.0)
    }

    /// uibase's `operator<(VersionInfo, VersionInfo)`: invalid below valid, dates below the
    /// rest, decimal comparison when either is a decimal-mark version, else part by part; then
    /// the release type, then the rest (numerically when both are integers).
    pub fn less(&self, other: &Self) -> bool {
        if self.valid != other.valid {
            return !self.valid;
        }
        let (ld, rd) = (self.scheme == Scheme::Date, other.scheme == Scheme::Date);
        if ld != rd {
            return ld;
        }
        if !ld && (self.scheme == Scheme::DecimalMark || other.scheme == Scheme::DecimalMark) {
            let (l, r) = (self.decimal(), other.decimal());
            if (l - r).abs() > 0.001 {
                return l < r;
            }
        } else if self.parts != other.parts {
            return self.parts < other.parts;
        }
        if self.release != other.release {
            return (self.release as u8) < (other.release as u8);
        }
        match (self.rest.parse::<i32>(), other.rest.parse::<i32>()) {
            (Ok(l), Ok(r)) => l < r,
            _ => self.rest < other.rest,
        }
    }
}

/// `ModInfoRegular::updateAvailable` / `downgradeAvailable` for a mod's `version`,
/// `newestVersion`, `ignoredVersion` and `nexusFileStatus` (4 old, 6 removed: always an
/// update). `None` when there is no newest version to compare with.
pub fn update_state(version: &str, newest: &str, ignored: &str, file_status: i64) -> Option<Ordering> {
    let (v, n, i) = (Version::parse(version), Version::parse(newest), Version::parse(ignored));
    let same = |a: &Version, b: &Version| !a.less(b) && !b.less(a);
    if i.valid && same(&i, &n) {
        return n.valid.then_some(Ordering::Equal);
    }
    if file_status == 4 || file_status == 6 {
        return Some(Ordering::Less);
    }
    if !n.valid {
        return None;
    }
    Some(if v.less(&n) {
        Ordering::Less
    } else if n.less(&v) {
        Ordering::Greater
    } else {
        Ordering::Equal
    })
}

/// The canonical form of a version string (what `saveMeta` writes back).
pub fn canonical(text: &str) -> String {
    Version::parse(text).canonical()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_mo2_does() {
        let lt = |a: &str, b: &str| Version::parse(a).less(&Version::parse(b));
        assert!(lt("1.2", "1.10"));
        assert!(lt("1.0rc1", "1.0"));
        assert!(lt("1.0beta", "1.0rc"));
        assert!(lt("1.05", "1.5"));
        assert!(!lt("1.50", "1.5"));
        assert!(lt("d2026.1.1", "0.1"), "dates sort below regular versions");
        assert!(lt("", "0.1"));
        assert!(lt("1.0 2", "1.0 10"));
        assert!(!lt("2.0", "2.0.0.0"));
        assert_eq!(update_state("1.0", "1.1", "", 1), Some(Ordering::Less));
        assert_eq!(update_state("1.2", "1.1", "", 1), Some(Ordering::Greater));
        assert_eq!(update_state("1.1", "1.1.0", "", 1), Some(Ordering::Equal));
        assert_eq!(update_state("1.0", "1.1", "1.1", 1), Some(Ordering::Equal), "the user ignored that version");
        assert_eq!(update_state("1.0", "", "", 4), Some(Ordering::Less), "marked old on Nexus");
        assert_eq!(update_state("1.0", "", "", 1), None);
    }

    #[test]
    fn canonical_strings_as_mo2_writes_them() {
        for (input, want) in [
            ("", ""),
            ("d2026.11.9", "d2026.11.9.0"),
            ("d2026.11.9.0", "d2026.11.9.0"),
            ("1.2", "1.2.0.0"),
            ("1.12.3.0", "1.12.3.0"),
            ("v2.0", "2.0.0.0"),
            ("1.05", "f1.05"),
            ("f1.5", "f1.5"),
            ("2.0 beta3", "2.0.0.0b3"),
            ("1.0rc1", "1.0.0.0rc1"),
            ("1.0a", "1.0.0.0a"),
            ("1.0 prealpha", "1.0.0.0apre"), // "alpha" is found first
            ("dev", "ev"),
            ("1.4b", "1.4.0.0b"),
            ("1AE", "1.0.0.0AE"),
            ("final", "1.0.0.0"),
            ("abc", "abc"),
            ("d1.5", "1.5.0.0"),
            ("n1.2.3", "n1.2.3.0"),
            ("0.2.1", "0.2.1.0"),
        ] {
            assert_eq!(canonical(input), want, "{input:?}");
        }
    }
}
