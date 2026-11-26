//! The instance manager and the new-instance wizard (`strata_ui::instances`).

use std::path::{Path, PathBuf};

use strata_ui::instances;

use crate::{guarded, quiet};

#[cxx::bridge(namespace = "strata::instances")]
mod ffi {
    /// One row of the instance manager (`instances::InstanceInfo`).
    struct InstanceRow {
        name: String,
        root: String,
        base_dir: String,
        game_name: String,
        game_dir: String,
        prefix: String,
        proton: String,
        supported: bool,
    }

    /// One thing "Delete instance" offers (`instances::DeletionCandidate`).
    struct DeletionRow {
        path: String,
        mandatory: bool,
    }

    /// A game of the wizard's game page (`instances::GameChoice`).
    struct GameRow {
        name: String,
        binary: String,
        steam_dir: String,
    }

    /// A Proton of the wizard (`instances::protons`); `note` says what it lacks.
    struct ProtonRow {
        name: String,
        path: String,
        note: String,
    }

    /// What the wizard creates (`instances::NewInstance`); `prefix` empty: `<folder>/Prefix`.
    struct NewInstanceSpec {
        folder: String,
        game_name: String,
        game_dir: String,
        proton: String,
        prefix: String,
        local_inis: bool,
        local_saves: bool,
    }

    extern "Rust" {
        /// Every instance, the open one (`current`, may be empty) included.
        fn list(current: &str) -> Vec<InstanceRow>;
        /// The instances root (Leet change 1).
        fn instances_root() -> String;
        fn set_instances_root(root: &str) -> Result<()>;
        /// "" when the name is a good folder name, else why not.
        fn name_problem(name: &str) -> String;
        /// "" when the instance may be renamed or deleted now, else why not.
        fn busy(root: &str) -> String;
        /// Renames the instance folder and the paths into it; returns the new folder.
        fn rename(root: &str, new_name: &str) -> Result<String>;
        fn deletion_candidates(root: &str) -> Vec<DeletionRow>;
        fn delete_paths(root: &str, paths: Vec<String>, trash: bool) -> Result<()>;
        fn games() -> Vec<GameRow>;
        fn protons() -> Vec<ProtonRow>;
        /// Creates the instance; returns its folder.
        fn create(spec: NewInstanceSpec) -> Result<String>;
    }
}

use ffi::*;

fn config() -> Option<PathBuf> {
    strata_ui::config::config_file()
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn list(current: &str) -> Vec<InstanceRow> {
    quiet(|| {
        let current = (!current.is_empty()).then(|| Path::new(current));
        instances::list(config().as_deref(), current)
            .into_iter()
            .map(|i| InstanceRow {
                name: i.name,
                root: text(&i.root),
                base_dir: text(&i.base_dir),
                game_name: i.game_name,
                game_dir: i.game_dir,
                prefix: i.prefix,
                proton: i.proton,
                supported: i.supported,
            })
            .collect()
    })
}

fn instances_root() -> String {
    quiet(|| text(&instances::instances_root(config().as_deref())))
}

fn set_instances_root(root: &str) -> Result<(), String> {
    guarded(|| {
        let config = config().ok_or("neither XDG_CONFIG_HOME nor HOME is set")?;
        instances::set_instances_root(&config, Path::new(root)).map_err(|e| e.to_string())
    })
}

fn name_problem(name: &str) -> String {
    quiet(|| instances::name_problem(name))
}

fn busy(root: &str) -> String {
    quiet(|| instances::busy(Path::new(root)))
}

fn rename(root: &str, new_name: &str) -> Result<String, String> {
    guarded(|| {
        instances::rename(config().as_deref(), Path::new(root), new_name).map(|p| text(&p)).map_err(|e| e.to_string())
    })
}

fn deletion_candidates(root: &str) -> Vec<DeletionRow> {
    quiet(|| {
        instances::deletion_candidates(Path::new(root))
            .into_iter()
            .map(|c| DeletionRow { path: text(&c.path), mandatory: c.mandatory })
            .collect()
    })
}

fn delete_paths(root: &str, paths: Vec<String>, trash: bool) -> Result<(), String> {
    guarded(|| {
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        instances::delete(config().as_deref(), Path::new(root), &paths, trash).map_err(|e| e.to_string())
    })
}

fn games() -> Vec<GameRow> {
    quiet(|| {
        instances::games()
            .into_iter()
            .map(|g| GameRow { name: g.name, binary: g.binary, steam_dir: g.steam_dir })
            .collect()
    })
}

fn protons() -> Vec<ProtonRow> {
    quiet(|| {
        instances::protons()
            .into_iter()
            .map(|(name, path, note)| ProtonRow { name, path: text(&path), note })
            .collect()
    })
}

fn create(spec: NewInstanceSpec) -> Result<String, String> {
    guarded(|| {
        let new = instances::NewInstance {
            folder: PathBuf::from(&spec.folder),
            game_name: spec.game_name,
            game_dir: PathBuf::from(&spec.game_dir),
            proton: PathBuf::from(&spec.proton),
            prefix: (!spec.prefix.is_empty()).then(|| PathBuf::from(&spec.prefix)),
            local_inis: spec.local_inis,
            local_saves: spec.local_saves,
        };
        let folder = instances::create(&new).map_err(|e| e.to_string())?;
        if let Some(config) = config() {
            // a new instance outside the root is listed from now on
            let _ = instances::remember(&config, &folder);
        }
        Ok(text(&folder))
    })
}
