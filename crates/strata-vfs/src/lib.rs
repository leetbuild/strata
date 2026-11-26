//! The overlay: layer list to merged tree, the FUSE filesystem, copy-on-write to overwrite.
//!
//! This crate knows nothing about profiles: it gets a resolved layer list
//! (`strata_game::Layer`, lowest priority first).

pub mod conflicts;
pub mod daemon;
pub mod fs;
pub mod mount;
pub mod tree;

pub use conflicts::ConflictIndex;
pub use mount::{Mount, MountSpec};
pub use tree::{NodeId, Options, Tree};
