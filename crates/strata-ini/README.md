# strata-ini

QSettings-compatible ini reading and writing with a byte-exact round trip: MO2's `ModOrganizer.ini`, `meta.ini`,
`settings.ini` come back unchanged unless a value changes, and a changed value is written as Qt writes it.

**Must not:** know about MO2.

| Module | What it holds |
|---|---|
| `document` | `Document`: the lossless document (Qt's line splitting, every byte kept), get / set / remove one key |
| `codec` | Qt's escaping of keys and values (`iniEscapedKey`, `iniEscapedString`, `@Variant`, `@ByteArray`), from Qt 6.8 `qsettings.cpp` |

Tests: `cargo test -p strata-ini` (unit tests incl. property tests of the codec); oracle check 1
(`tools/oracle/check-roundtrip.sh <instance>`) round-trips every ini of a real instance.
