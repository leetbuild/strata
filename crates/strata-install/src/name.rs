//! The mod name of an install: MO2's `GuessedValue<QString>` (uibase `guessedvalue.h`), the
//! `fixDirectoryName` filter (uibase `filesystemutilities.cpp`) and the Nexus file name
//! guess (`NexusInterface::interpretNexusFileName`).

use std::sync::LazyLock;

use regex::Regex;

/// How sure a guess is (`EGuessQuality`), weakest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quality {
    Invalid,
    /// A very basic guess (the archive's file name).
    Fallback,
    /// A good guess (the Nexus file name pattern).
    Good,
    /// From metadata, usually what the author meant (`.meta` `modName`, FOMOD `info.xml`).
    Meta,
    /// From a previous install of the same data.
    Preset,
    /// The user's choice; always wins.
    User,
}

/// A mod name with its quality and every variant seen (the dialogs offer the variants).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuessedName {
    value: String,
    quality: Quality,
    variants: Vec<String>,
}

impl Default for GuessedName {
    fn default() -> Self {
        Self { value: String::new(), quality: Quality::Invalid, variants: Vec::new() }
    }
}

impl GuessedName {
    /// `GuessedValue::update(value, quality)` with the `fixDirectoryName` filter: a name the
    /// filter rejects is ignored; otherwise it becomes a variant, and the value when its
    /// quality is at least the current one.
    pub fn update(&mut self, value: &str, quality: Quality) {
        let Some(fixed) = fix_directory_name(value) else { return };
        if let Err(at) = self.variants.binary_search(&fixed) {
            self.variants.insert(at, fixed.clone());
        }
        if quality >= self.quality {
            self.value = fixed;
            self.quality = quality;
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    /// Every name seen, sorted (`std::set<QString>` order).
    pub fn variants(&self) -> &[String] {
        &self.variants
    }
}

/// `fixDirectoryName`: makes a name usable as a folder name, or `None` when nothing is left
/// (the core's, shared with the mod list).
pub fn fix_directory_name(name: &str) -> Option<String> {
    strata_core::fix_directory_name(name)
}

static COMPLEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"^([a-zA-Z0-9_'"\-.() ]*?)([-_ ][VvRr]+[0-9]+(?:(?:[\.][0-9]+){0,2}|(?:[_][0-9]+){0,2}|(?:[-.][0-9]+){0,2})?[ab]?)??-([1-9][0-9]+)?-.*?\.(zip|rar|7z)"#,
    )
    .expect("valid regex")
});
static SIMPLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[^a-zA-Z]*([a-zA-Z_ ]+)").expect("valid regex"));

/// `interpretNexusFileName(fileName, modName, modID, query = false)`: the mod name and the
/// Nexus mod id a Nexus file name carries (`SkyUI_5_2_SE-12604-5-2SE.7z` → `SkyUI_5_2_SE`,
/// 12604). The id is `None` when the name has none.
pub fn interpret_nexus_file_name(file_name: &str) -> (String, Option<i64>) {
    if let Some(c) = COMPLEX.captures(file_name) {
        let id = c.get(3).and_then(|m| m.as_str().parse().ok());
        return (c.get(1).map_or("", |m| m.as_str()).to_owned(), id);
    }
    if let Some(m) = SIMPLE.find(file_name) {
        return (m.as_str().to_owned(), None);
    }
    (String::new(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_or_equal_quality_wins_and_filter_applies() {
        let mut n = GuessedName::default();
        n.update("archive.name", Quality::Fallback);
        n.update("Better", Quality::Good);
        n.update("Worse", Quality::Fallback);
        assert_eq!(n.value(), "Better");
        n.update("  Meta:  Name.. ", Quality::Meta);
        assert_eq!(n.value(), "Meta Name");
        n.update("CON", Quality::User);
        assert_eq!(n.value(), "Meta Name", "rejected by the filter");
        assert_eq!(n.variants(), ["Better", "Meta Name", "Worse", "archive.name"]);
    }

    #[test]
    fn nexus_file_names() {
        assert_eq!(
            interpret_nexus_file_name("AVG Main File-84743-2-3-4-0-1756567990.7z"),
            ("AVG Main File".to_owned(), Some(84743))
        );
        assert_eq!(interpret_nexus_file_name("SkyUI_5_2_SE-12604-5-2SE.7z"), ("SkyUI_5_2_SE".to_owned(), Some(12604)));
        assert_eq!(interpret_nexus_file_name("My Mod v1.2-1234-1-2.zip"), ("My Mod".to_owned(), Some(1234)));
        assert_eq!(interpret_nexus_file_name("12 cool_mod.7z"), ("12 cool_mod".to_owned(), None));
        assert_eq!(interpret_nexus_file_name("123.7z"), ("123.7z".to_owned(), None), "the letters part starts at the z");
        assert_eq!(interpret_nexus_file_name("1234"), (String::new(), None));
    }
}
