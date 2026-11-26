//! The filter box under the mod list (`ModListSortProxy::filterMatchesMod`, the text part).

/// What a mod offers to the filter. A field is only searched while its column is shown, as
/// in MO2 (`m_EnabledColumns`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Fields<'a> {
    /// The internal name (MO2 matches `info->name()`, so a separator matches `_separator`).
    pub name: &'a str,
    /// `None` while the Notes column is hidden.
    pub comments: Option<&'a str>,
    /// `None` while the Category column is hidden.
    pub categories: Option<&'a [&'a str]>,
    /// `None` while the Nexus ID column is hidden or the mod has no id.
    pub nexus_id: Option<i64>,
}

fn contains(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Whether a mod passes `filter`: `||`, `OR` and `|` split it into alternatives (`;` too),
/// each alternative is words that must all be found, each in the name, the notes, a category,
/// or as the leading digits of the Nexus ID. An empty filter passes everything.
pub fn matches(filter: &str, mod_: Fields<'_>) -> bool {
    let filter = filter.replace("||", ";").replace("OR", ";").replace('|', ";");
    let alternatives: Vec<&str> = filter.split(';').filter(|s| !s.is_empty()).collect();
    if alternatives.is_empty() {
        return true;
    }
    alternatives.iter().any(|alternative| {
        alternative.split(' ').filter(|w| !w.is_empty()).all(|word| {
            contains(mod_.name, word)
                || mod_.comments.is_some_and(|c| contains(c, word))
                || mod_.categories.is_some_and(|cats| cats.iter().any(|c| contains(c, word)))
                || id_matches(mod_.nexus_id, word)
        })
    })
}

/// MO2 compares the number with the id, then the id without its last digit, and so on.
fn id_matches(id: Option<i64>, word: &str) -> bool {
    let (Some(mut id), Ok(wanted)) = (id, word.parse::<i64>()) else { return false };
    while id > 0 {
        if id == wanted {
            return true;
        }
        id /= 10;
    }
    false
}

/// `CategoryFactory::SpecialCategories`: the filter panel's `<...>` entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Special {
    Checked = 10000,
    UpdateAvailable,
    HasCategory,
    Conflict,
    HasHiddenFiles,
    Endorsed,
    Backup,
    Managed,
    HasGameData,
    HasNexusId,
    Tracked,
}

impl Special {
    /// In the panel's order (`FilterList::refresh`).
    pub const ALL: [Special; 11] = [
        Special::Checked,
        Special::UpdateAvailable,
        Special::Backup,
        Special::Managed,
        Special::HasCategory,
        Special::Conflict,
        Special::HasHiddenFiles,
        Special::Endorsed,
        Special::Tracked,
        Special::HasNexusId,
        Special::HasGameData,
    ];

    /// `CategoryFactory::getSpecialCategoryName`.
    pub fn name(self) -> &'static str {
        match self {
            Special::Checked => "<Active>",
            Special::UpdateAvailable => "<Update available>",
            Special::HasCategory => "<Has category>",
            Special::Conflict => "<Conflicted>",
            Special::HasHiddenFiles => "<Has hidden files>",
            Special::Endorsed => "<Endorsed>",
            Special::Backup => "<Has backup>",
            Special::Managed => "<Managed>",
            Special::HasGameData => "<Has valid game data>",
            Special::HasNexusId => "<Has Nexus ID>",
            Special::Tracked => "<Tracked on Nexus>",
        }
    }

    pub fn from_id(id: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|s| *s as i32 == id)
    }
}

/// What an entry of the filter panel is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    Special,
    Content,
    Category,
}

/// One entry of the filter panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelEntry {
    pub kind: PanelKind,
    pub id: i32,
    pub name: String,
}

/// One ticked entry of the filter panel (`ModListSortProxy::Criteria`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criterion {
    /// A `Special` id, or a category id (`TypeSpecial` and `TypeCategory` share the check).
    Category { id: i32, inverse: bool },
    /// A `content::Content` id.
    Content { id: u8, inverse: bool },
}

/// What the criteria look at for one mod.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub enabled: bool,
    /// Foreign mods and Overwrite (`alwaysEnabled`).
    pub always_enabled: bool,
    pub separator: bool,
    pub backup: bool,
    pub foreign: bool,
    pub overwrite: bool,
    pub categories: Vec<i32>,
    pub conflicted: bool,
    pub hidden_files: bool,
    pub endorsed: bool,
    pub tracked: bool,
    pub valid: bool,
    pub nexus_id: i64,
    pub contents: Vec<u8>,
    /// `updateAvailable() || downgradeAvailable()`.
    pub update_available: bool,
}

/// The panel's separator choice (`ModListSortProxy::SeparatorsMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separators {
    Filter,
    Show,
    Hide,
}

/// `categoryMatchesMod` / `contentMatchesMod`, with the criterion's inversion.
fn criterion_matches(c: Criterion, m: &Facts) -> bool {
    let (b, inverse) = match c {
        Criterion::Content { id, inverse } => (m.contents.contains(&id), inverse),
        Criterion::Category { id, inverse } => {
            let b = match Special::from_id(id) {
                Some(Special::Checked) => m.enabled || m.always_enabled,
                Some(Special::UpdateAvailable) => m.update_available,
                Some(Special::HasCategory) => !m.categories.is_empty(),
                Some(Special::Conflict) => m.conflicted,
                Some(Special::HasHiddenFiles) => m.hidden_files,
                Some(Special::Endorsed) => m.endorsed,
                Some(Special::Backup) => m.backup,
                Some(Special::Managed) => !m.foreign,
                Some(Special::HasGameData) => m.valid,
                // "never show these", whatever the inversion
                Some(Special::HasNexusId) if m.foreign || m.backup || m.overwrite => return false,
                Some(Special::HasNexusId) => m.nexus_id > 0,
                Some(Special::Tracked) => m.tracked,
                None => m.categories.contains(&id),
            };
            (b, inverse)
        }
    };
    b != inverse
}

/// The panel's part of `filterMatchesMod`: separators as chosen, then every criterion (And)
/// or any (Or; no criteria = everything).
pub fn matches_criteria(criteria: &[Criterion], and: bool, separators: Separators, m: &Facts) -> bool {
    if criteria.is_empty() && separators == Separators::Filter {
        return true;
    }
    if m.separator {
        match separators {
            Separators::Show => return true,
            Separators::Hide => return false,
            Separators::Filter => {}
        }
    }
    if and {
        criteria.iter().all(|c| criterion_matches(*c, m))
    } else {
        criteria.is_empty() || criteria.iter().any(|c| criterion_matches(*c, m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn criteria_and_or_inverse_and_separators_as_mo2() {
        let active = Facts { enabled: true, categories: vec![3], contents: vec![0], nexus_id: 5, valid: true, ..Facts::default() };
        let off = Facts { valid: true, ..Facts::default() };
        let sep = Facts { separator: true, ..Facts::default() };
        let checked = Criterion::Category { id: Special::Checked as i32, inverse: false };
        let unchecked = Criterion::Category { id: Special::Checked as i32, inverse: true };
        let cat3 = Criterion::Category { id: 3, inverse: false };
        let plugins = Criterion::Content { id: 0, inverse: false };
        assert!(matches_criteria(&[checked], true, Separators::Filter, &active));
        assert!(!matches_criteria(&[checked], true, Separators::Filter, &off));
        assert!(matches_criteria(&[unchecked], true, Separators::Filter, &off));
        assert!(matches_criteria(&[cat3, plugins], true, Separators::Filter, &active));
        assert!(!matches_criteria(&[cat3, unchecked], true, Separators::Filter, &active));
        assert!(matches_criteria(&[cat3, unchecked], false, Separators::Filter, &active), "Or");
        assert!(!matches_criteria(&[checked], true, Separators::Filter, &sep));
        assert!(matches_criteria(&[checked], true, Separators::Show, &sep));
        assert!(!matches_criteria(&[], true, Separators::Hide, &sep));
        assert!(matches_criteria(&[], true, Separators::Filter, &sep));
        // Has Nexus ID never shows foreign mods, inverted or not
        let foreign = Facts { foreign: true, always_enabled: true, ..Facts::default() };
        let no_id_inverted = Criterion::Category { id: Special::HasNexusId as i32, inverse: true };
        assert!(!matches_criteria(&[no_id_inverted], true, Separators::Filter, &foreign));
        assert!(matches_criteria(&[checked], true, Separators::Filter, &foreign), "always enabled");
    }

    fn named(name: &str) -> Fields<'_> {
        Fields { name, ..Fields::default() }
    }

    #[test]
    fn words_must_all_match_and_alternatives_any() {
        assert!(matches("", named("SkyUI")));
        assert!(matches("sky ui", named("SkyUI")));
        assert!(!matches("sky foo", named("SkyUI")));
        assert!(matches("foo || ui", named("SkyUI")));
        assert!(matches("fooORui", named("SkyUI")));
        assert!(matches("foo|ui", named("SkyUI")));
        assert!(matches("separator", named("Tools_separator")), "the internal name is searched");
    }

    #[test]
    fn notes_categories_and_ids_count_only_while_their_column_shows() {
        let cats = ["Armour", "Weapons"];
        let full = Fields { name: "X", comments: Some("todo later"), categories: Some(&cats), nexus_id: Some(12345) };
        assert!(matches("later", full));
        assert!(matches("weap", full));
        assert!(matches("123", full), "leading digits of the id");
        assert!(!matches("345", full));
        assert!(!matches("later", named("X")));
        assert!(!matches("123", named("X")));
    }
}
