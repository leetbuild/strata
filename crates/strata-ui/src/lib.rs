//! What the window shows, as plain Rust: the shell (`shell/`, C++) renders these views and
//! owns no state of its own (`docs/DESIGN.md`, "The shell"). Nothing here knows Qt; `strata-ffi` carries
//! it across the `cxx` bridge.
//!
//! Some of this reads instance files the core does not model yet (a mod's `meta.ini` keys
//! the mod list shows, download `.meta` files, the saves folder). Each such reader says so and
//! names the MO2 function it follows; it moves into `strata-core` when the core grows the
//! typed view (the `modlist` and `plugins` areas).

pub mod config;
pub mod conflicts;
pub mod conflict_flags;
pub mod content;
pub mod crashlogs;
pub mod data;
pub mod downloads;
mod error;
pub mod filter;
pub mod handover;
pub mod html;
pub mod instances;
pub mod install;
pub mod links;
pub mod listtools;
pub mod live;
pub mod modlist;
pub mod mods;
pub mod nexus;
pub mod loot_view;
pub mod plugins;
pub mod profile_tree;
pub mod saves;
mod session;
pub mod settings;
pub mod style;
#[cfg(test)]
mod test_instance;

pub use error::{Error, Result};
pub use session::Session;
