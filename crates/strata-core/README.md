# strata-core

The model of an MO2 instance: instance, profiles, the mod list, `meta.ini`, separators, categories, the plugin
list, executables, overwrite, backups, settings, and the change events every operation returns. Every write
goes through `write.rs` (atomic, only when the bytes change; a journaled `Transaction` when several files change
together).

**Must not:** touch FUSE, Qt or the network.

| Module | What it holds |
|---|---|
| `instance` | `Instance::open`: `ModOrganizer.ini`, the game, the resolved paths, the `[fluorine]` Proton settings |
| `profile` | `Profile`, `ModListFile` (`modlist.txt`), `PluginListFile` (`plugins.txt`, `loadorder.txt`) |
| `profiles` | create, copy, rename, delete, select profiles; local saves and local inis on and off |
| `modlist` | `ModList`: the mods on disk ordered by `modlist.txt`; enable, move, create, rename, remove, save |
| `separators` | Sub-Separator nesting (Leet change 9) as pure functions: tree, drop rules, roll-ups |
| `meta` | `ModMeta`: a typed view over a mod's `meta.ini`; `Color` as `QSettings` stores a `QColor` |
| `categories` | `categories.dat` |
| `plugins/` | the plugin list (`mod.rs`: order, flags, masters, locks), `files.rs` (`lockedorder.txt`), `saves.rs` (the Saves tab) |
| `executables` | `[customExecutables]`: read, store as MO2 stores it, Root paths rewritten to the game folder |
| `settings` | the settings MO2's settings dialog writes that apply on Linux (table with defaults), the remembered delete choice; `store_ini` / `store_instance_ini`: the one writer of `ModOrganizer.ini` every area calls |
| `inis` | the profile's game inis for the ini editor: where each is, saving it as MO2's `TextViewer` does |
| `overwrite` | clear overwrite, move it into a mod |
| `backups` | `modlist.txt` / load order backups named as MO2 names them, restore |
| `layers` | profile → the overlay's layer list and profile mounts |
| `events` | `ModListEvent`, `ProfileEvent` |
| `write` | `write_if_changed`, `Transaction` + journal, `recover` |
| `trash` | the freedesktop trash |
| `dirs` | the XDG base folders (`data_home`, `config_home`, `cache_home`) |
| `winepath` | Windows paths in the ini ↔ host paths (`Z:\\`, drives inside the prefix) |
| `time`, `error` | local time for backup names; the crate's `Error` |

Tests: `cargo test -p strata-core` (unit tests; `tests/edit.rs` writing operations on an instance built on
disk, `tests/instance.rs`, `tests/plugins_saves.rs`). Oracle: `tools/oracle/check-modlist.sh <instance>`
(MO2's reading rules after random operations), check 1 (`check-roundtrip.sh`).
