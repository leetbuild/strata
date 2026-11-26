//! QSettings-compatible ini reading and writing with a byte-exact round trip.
//!
//! Two layers:
//! - [`Document`]: the file as Qt's reader splits it (sections, `key=value` entries, and
//!   everything else kept verbatim), so writing an unchanged document gives the same bytes.
//! - [`codec`]: Qt's escaping of keys and values (`@ByteArray(...)`, `\x..`, lists, `%20`).
//!
//! This crate knows nothing about MO2. The rules come from Qt 6's `qsettings.cpp`
//! (`readIniLine`, `iniEscapedKey`, `iniEscapedString`, `iniUnescapedStringList`,
//! `variantToString`, `stringToVariant`).

pub mod codec;
mod document;

pub use codec::Value;
pub use document::{Document, Entry};
