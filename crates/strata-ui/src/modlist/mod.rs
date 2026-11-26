//! Views for the `modlist` area's dialogs (rounds 2 and 3): profiles, executables, settings, the ini editor, the
//! delete confirmation. Each is plain Rust over `strata_core`; the dialogs
//! in `shell/src/modlist/` render them through `strata-ffi`'s `bridge_modlist.rs`.

pub mod executables;
pub mod inieditor;
pub mod problems;
pub mod profiles;
pub mod settings;
