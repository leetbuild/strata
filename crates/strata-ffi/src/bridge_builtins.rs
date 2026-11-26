//! The `modlist` area's bridge for the built-ins: the ini editor, the Problems dialog and
//! the built-in plugins' switches. Each function opens the instance in `root` itself, does one thing through `strata_core` /
//! `strata_ui::modlist`, and catches panics (a panic becomes "internal error: ...").

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use strata_core::Instance;
use strata_ui::modlist::{inieditor, problems};
use crate::guarded;

#[cxx::bridge(namespace = "strata::builtins")]
mod ffi {
    struct IniTab {
        name: String,
        path: String,
        text: String,
        read_only: bool,
        /// Why the file gets no tab ("missing", "too big"), or empty.
        skipped: String,
    }

    struct IniEditorView {
        description: String,
        tabs: Vec<IniTab>,
    }

    struct ProblemRow {
        /// A stable name (`missing-masters`, ...), given back to `problem_fix`.
        id: String,
        summary: String,
        /// The description pane (HTML).
        description: String,
        /// The Fix button's tooltip; empty: no fix.
        fix: String,
    }


    extern "Rust" {
        /// The ini editor's tabs for `profile` ("" = the selected one).
        fn ini_editor_view(root: &str, profile: &str) -> Result<IniEditorView>;
        /// Saves one ini. `read_only`: 0 refuse a read-only file, 1 clear its flag, 2 write
        /// once and keep it read-only. Returns whether the file changed.
        fn ini_save(root: &str, profile: &str, name: &str, text: &str, read_only: u8) -> Result<bool>;

        /// Every problem of `profile` ("" = the selected one). Builds the merged tree: call
        /// it off the UI thread.
        fn problems_collect(root: &str, profile: &str) -> Result<Vec<ProblemRow>>;
        /// Runs a problem's fix; returns what it did.
        fn problem_fix(root: &str, profile: &str, id: &str) -> Result<String>;

        /// Whether the window offers a built-in plugin (`[PluginPersistance] <name>\enabled`).
        fn builtin_enabled(root: &str, name: &str) -> bool;
    }
}

fn open(root: &str) -> Result<Instance, String> {
    Instance::open(Path::new(root)).map_err(|e| e.to_string())
}

fn text<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn ini_editor_view(root: &str, profile: &str) -> Result<ffi::IniEditorView, String> {
    guarded(|| {
        let view = inieditor::view(&open(root)?, profile).map_err(text)?;
        Ok(ffi::IniEditorView {
            description: view.description,
            tabs: view
                .tabs
                .into_iter()
                .map(|t| ffi::IniTab { name: t.name, path: t.path, text: t.text, read_only: t.read_only, skipped: t.skipped })
                .collect(),
        })
    })
}

fn ini_save(root: &str, profile: &str, name: &str, content: &str, read_only: u8) -> Result<bool, String> {
    guarded(|| inieditor::save(&open(root)?, profile, name, content, read_only).map_err(text))
}

fn open_profile(root: &str, profile: &str) -> Result<(Instance, strata_core::Profile), String> {
    let instance = open(root)?;
    let profile = instance.profile((!profile.is_empty()).then_some(profile)).map_err(text)?;
    Ok((instance, profile))
}

fn problems_collect(root: &str, profile: &str) -> Result<Vec<ffi::ProblemRow>, String> {
    guarded(|| {
        let (instance, profile) = open_profile(root, profile)?;
        let rows = problems::collect(&instance, &profile).map_err(text)?;
        Ok(rows
            .into_iter()
            .map(|r| ffi::ProblemRow {
                description: r.description_html(),
                fix: r.fix.unwrap_or_default().to_owned(),
                id: r.problem.id.to_owned(),
                summary: r.problem.summary,
            })
            .collect())
    })
}

fn problem_fix(root: &str, profile: &str, id: &str) -> Result<String, String> {
    guarded(|| {
        let (instance, profile) = open_profile(root, profile)?;
        let trash = strata_core::Trash::home().map_err(text)?;
        problems::fix(&instance, &profile, id, &trash).map_err(text)
    })
}

fn builtin_enabled(root: &str, name: &str) -> bool {
    catch_unwind(AssertUnwindSafe(|| open(root).is_ok_and(|i| i.builtin_enabled(name)))).unwrap_or(false)
}

