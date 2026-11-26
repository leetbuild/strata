//! The model: instances, profiles, the mod list. See `docs/ARCHITECTURE.md`.
//!
//! Everything here follows MO2 2.5.2's source (`profile.cpp`, `modinfo.cpp`, `settings.cpp`);
//! the function docs name the MO2 function they mirror.

pub mod backups;
pub mod dirs;
mod categories;
mod error;
mod events;
pub mod executables;
pub mod inis;
mod instance;
mod layers;
mod meta;
mod modlist;
mod overwrite;
pub mod plugins;
mod profile;
pub mod profiles;
pub mod separators;
pub mod settings;
mod time;
pub mod trash;
pub mod winepath;
pub mod write;

pub use categories::{Categories, Category};
pub use error::{Error, Result};
pub use events::{ModListEvent, ProfileEvent};
pub use executables::Executable;
pub use instance::{Instance, InstancePaths, ProtonSettings};
pub use settings::DeleteChoice;
pub use layers::{ProfileMount, profile_mounts, resolve_layers};
pub use meta::{Color, Endorsed, ModMeta, SUB_SEPARATOR, color_from_value, new_mod_meta};
pub use modlist::{ModEntry, ModKind, ModList, fix_directory_name, is_backup_name, is_separator_name, rename_in_list};
pub use time::LocalTime;
pub use trash::Trash;
pub use profile::{ListedMod, ListedState, MODLIST_HEADER, ModListFile, PluginListFile, Profile};
