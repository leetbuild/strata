//! Archives: 7z / zip / rar extraction for installs (through the `7z` program), BSA/BA2 reading.
//!
//! See `docs/ARCHITECTURE.md`.

pub mod bsa;
pub mod pack;
pub mod sevenzip;

pub use bsa::{ArchivedFile, Format, GameArchive};
pub use sevenzip::{Archive, Entry, Error, Progress};
