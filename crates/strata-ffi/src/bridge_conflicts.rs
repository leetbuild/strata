//! The `conflicts` area's bridge: the mod info dialog and the previews. Logic lives in
//! `strata_ui::conflicts`; this file only converts, and every function catches panics.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

#[cxx::bridge(namespace = "strata::conflicts")]
mod ffi {
    struct PluginItem {
        path: String,
        active: bool,
    }

    /// A row of the general conflicts tab.
    struct GeneralRow {
        shown: String,
        others: String,
        /// The file on disk; empty when it is in an archive.
        real: String,
        /// The other mods of the row ("go to").
        mods: Vec<String>,
    }

    struct AdvancedRow {
        shown: String,
        before_all: String,
        before_nearest: String,
        after_all: String,
        after_nearest: String,
        real: String,
        mods: Vec<String>,
    }

    struct CategoryItem {
        id: i32,
        name: String,
        parent: i32,
        checked: bool,
    }

    struct NexusInfo {
        mod_id: i64,
        version: String,
        newest_version: String,
        url: String,
        custom_url: bool,
        description: String,
        endorsed: String,
        tracked: bool,
        game_name: String,
        last_query: String,
    }

    /// Decoded DDS pixels: B, G, R, A bytes (`QImage::Format_ARGB32`).
    struct DdsImage {
        width: u32,
        height: u32,
        bgra: Vec<u8>,
        format: String,
    }


    /// A file of a provider, for the preview's variants.
    struct Variant {
        layer: String,
        real: String,
    }

    extern "Rust" {
        type ModInfoView;
        /// Reads the mod information of `name` in `profile` ("" = the selected one).
        fn open_mod_info(root: &str, profile: &str, name: &str) -> Result<Box<ModInfoView>>;
        fn name(self: &ModInfoView) -> String;
        fn dir(self: &ModInfoView) -> String;
        fn is_separator(self: &ModInfoView) -> bool;
        fn text_files(self: &ModInfoView) -> Vec<String>;
        fn ini_files(self: &ModInfoView) -> Vec<String>;
        fn images(self: &ModInfoView) -> Vec<String>;
        fn plugins(self: &ModInfoView) -> Vec<PluginItem>;
        fn overwrites(self: &ModInfoView) -> Vec<GeneralRow>;
        fn overwritten(self: &ModInfoView) -> Vec<GeneralRow>;
        fn no_conflict(self: &ModInfoView) -> Vec<GeneralRow>;
        fn advanced(self: &ModInfoView) -> Vec<AdvancedRow>;
        fn categories(self: &ModInfoView) -> Vec<CategoryItem>;
        fn primary_category(self: &ModInfoView) -> i32;
        fn nexus(self: &ModInfoView) -> NexusInfo;
        fn comments(self: &ModInfoView) -> String;
        fn notes(self: &ModInfoView) -> String;

        /// Edits, each through `strata_ui::conflicts::modinfo`.
        fn save_mod_file(dir: &str, rel: &str, content: &[u8]) -> Result<()>;
        fn save_mod_notes(dir: &str, comments: &str, notes: &str) -> Result<bool>;
        fn save_mod_categories(dir: &str, primary: i32, checked: Vec<i32>) -> Result<bool>;
        fn activate_plugin(dir: &str, rel: &str, new_name: &str) -> Result<String>;
        fn deactivate_plugin(dir: &str, rel: &str, back_to: &str) -> Result<String>;

        /// Previews.
        fn preview_kind(name: &str) -> String;
        fn read_text(path: &str) -> Result<String>;
        fn decode_dds(path: &str) -> Result<DdsImage>;
        /// A thumbnail: the smallest mip level at least `size` pixels on its longer side (reads
        /// only that level). Any thread.
        fn decode_dds_thumbnail(path: &str, size: u32) -> Result<DdsImage>;
        /// The loose copies of a merged file, lowest first; `root` for `/Root/...` paths.
        fn file_providers(root_dir: &str, profile: &str, root: bool, path: &str) -> Result<Vec<Variant>>;
    }
}

use strata_ui::conflicts::{modinfo, preview};
use crate::{guarded, quiet};

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn open(root: &str, profile: &str) -> Result<strata_ui::Session, String> {
    strata_ui::Session::open(Path::new(root), (!profile.is_empty()).then_some(profile)).map_err(|e| e.to_string())
}

/// The mod information the dialog shows (`strata_ui::conflicts::modinfo::ModInfo`).
pub struct ModInfoView(modinfo::ModInfo);

fn open_mod_info(root: &str, profile: &str, name: &str) -> Result<Box<ModInfoView>, String> {
    guarded(|| {
        let session = open(root, profile)?;
        modinfo::ModInfo::open(session.instance(), session.profile(), name)
            .map(|m| Box::new(ModInfoView(m)))
            .map_err(|e| e.to_string())
    })
}

fn general(rows: &[modinfo::GeneralRow]) -> Vec<ffi::GeneralRow> {
    rows.iter()
        .map(|r| ffi::GeneralRow {
            shown: r.shown.clone(),
            others: r.others.clone(),
            real: r.real.as_deref().map(text).unwrap_or_default(),
            mods: r.mods.clone(),
        })
        .collect()
}

impl ModInfoView {
    fn name(&self) -> String {
        quiet(|| self.0.name.clone())
    }
    fn dir(&self) -> String {
        quiet(|| text(&self.0.dir))
    }
    fn is_separator(&self) -> bool {
        quiet(|| self.0.is_separator)
    }
    fn text_files(&self) -> Vec<String> {
        quiet(|| self.0.files.text.clone())
    }
    fn ini_files(&self) -> Vec<String> {
        quiet(|| self.0.files.ini.clone())
    }
    fn images(&self) -> Vec<String> {
        quiet(|| self.0.files.images.clone())
    }
    fn plugins(&self) -> Vec<ffi::PluginItem> {
        quiet(|| self.0.files.plugins.iter().map(|p| ffi::PluginItem { path: p.path.clone(), active: p.active }).collect())
    }
    fn overwrites(&self) -> Vec<ffi::GeneralRow> {
        quiet(|| general(&self.0.conflicts.overwrites))
    }
    fn overwritten(&self) -> Vec<ffi::GeneralRow> {
        quiet(|| general(&self.0.conflicts.overwritten))
    }
    fn no_conflict(&self) -> Vec<ffi::GeneralRow> {
        quiet(|| general(&self.0.conflicts.no_conflict))
    }
    fn advanced(&self) -> Vec<ffi::AdvancedRow> {
        quiet(|| {
            self.0
                .conflicts
                .advanced
                .iter()
                .map(|r| ffi::AdvancedRow {
                    shown: r.shown.clone(),
                    before_all: r.before_all.clone(),
                    before_nearest: r.before_nearest.clone(),
                    after_all: r.after_all.clone(),
                    after_nearest: r.after_nearest.clone(),
                    real: r.real.as_deref().map(text).unwrap_or_default(),
                    mods: r.mods.clone(),
                })
                .collect()
        })
    }
    fn categories(&self) -> Vec<ffi::CategoryItem> {
        quiet(|| {
            self.0
                .categories
                .iter()
                .map(|c| ffi::CategoryItem { id: c.id, name: c.name.clone(), parent: c.parent, checked: c.checked })
                .collect()
        })
    }
    fn primary_category(&self) -> i32 {
        quiet(|| self.0.primary_category)
    }
    fn nexus(&self) -> ffi::NexusInfo {
        catch_unwind(AssertUnwindSafe(|| {
            let n = &self.0.nexus;
            ffi::NexusInfo {
                mod_id: n.mod_id,
                version: n.version.clone(),
                newest_version: n.newest_version.clone(),
                url: n.url.clone(),
                custom_url: n.custom_url,
                description: n.description.clone(),
                endorsed: n.endorsed.clone(),
                tracked: n.tracked,
                game_name: n.game_name.clone(),
                last_query: n.last_query.clone(),
            }
        }))
        .unwrap_or(ffi::NexusInfo {
            mod_id: -1,
            version: String::new(),
            newest_version: String::new(),
            url: String::new(),
            custom_url: false,
            description: String::new(),
            endorsed: String::new(),
            tracked: false,
            game_name: String::new(),
            last_query: String::new(),
        })
    }
    fn comments(&self) -> String {
        quiet(|| self.0.comments.clone())
    }
    fn notes(&self) -> String {
        quiet(|| self.0.notes.clone())
    }
}

fn save_mod_file(dir: &str, rel: &str, content: &[u8]) -> Result<(), String> {
    guarded(|| modinfo::save_file(Path::new(dir), rel, content).map_err(|e| e.to_string()))
}

fn save_mod_notes(dir: &str, comments: &str, notes: &str) -> Result<bool, String> {
    guarded(|| modinfo::save_notes(Path::new(dir), comments, notes).map_err(|e| e.to_string()))
}

fn save_mod_categories(dir: &str, primary: i32, checked: Vec<i32>) -> Result<bool, String> {
    guarded(|| modinfo::save_categories(Path::new(dir), primary, &checked).map_err(|e| e.to_string()))
}

fn activate_plugin(dir: &str, rel: &str, new_name: &str) -> Result<String, String> {
    guarded(|| modinfo::activate_plugin(Path::new(dir), rel, new_name).map_err(|e| e.to_string()))
}

fn deactivate_plugin(dir: &str, rel: &str, back_to: &str) -> Result<String, String> {
    guarded(|| {
        modinfo::deactivate_plugin(Path::new(dir), rel, (!back_to.is_empty()).then_some(back_to)).map_err(|e| e.to_string())
    })
}

fn preview_kind(name: &str) -> String {
    quiet(|| preview::kind(name).unwrap_or_default().to_owned())
}

fn read_text(path: &str) -> Result<String, String> {
    guarded(|| {
        std::fs::read(path).map(|d| preview::decode_text(&d)).map_err(|e| format!("cannot read {path}: {e}"))
    })
}

fn decode_dds(path: &str) -> Result<ffi::DdsImage, String> {
    guarded(|| {
        let data = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        let img = preview::decode_dds(&data).map_err(|e| e.to_string())?;
        Ok(ffi::DdsImage { width: img.width, height: img.height, bgra: img.bgra, format: img.format })
    })
}

fn decode_dds_thumbnail(path: &str, size: u32) -> Result<ffi::DdsImage, String> {
    guarded(|| {
        let img = preview::decode_dds_thumbnail(std::path::Path::new(path), size).map_err(|e| e.to_string())?;
        Ok(ffi::DdsImage { width: img.width, height: img.height, bgra: img.bgra, format: img.format })
    })
}


fn file_providers(root_dir: &str, profile: &str, root: bool, path: &str) -> Result<Vec<ffi::Variant>, String> {
    guarded(|| {
        let session = open(root_dir, profile)?;
        strata_ui::conflicts::file_providers(session.instance(), session.profile(), root, path)
            .map(|list| list.into_iter().map(|v| ffi::Variant { layer: v.layer, real: text(&v.real) }).collect())
            .map_err(|e| e.to_string())
    })
}
