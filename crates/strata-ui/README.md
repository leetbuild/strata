# strata-ui

What the window shows and does, as plain Rust. The C++ shell (`shell/`) owns no state: it holds a `Session`
and the dialogs' views through the `cxx` bridge (`strata-ffi`) and renders rows. Everything here is tested with
`cargo test`, no Qt needed.

**Must not:** know Qt.

| Module | What it holds |
|---|---|
| `session` | `Session`: an opened instance and profile, the core's `ModList` and its operations with their events |
| `mods` | the mod list rows (flags, nesting by `SeparatorTree`) |
| `filter` | the filter box and the filter panel's criteria |
| `content` | the Content column |
| `conflict_flags` | the Conflicts column and the conflict highlight, from `strata_vfs::ConflictIndex` |
| `plugins` | the Plugins and Archives tabs over `strata_core::plugins`; LOOT sort |
| `saves` | the Saves tab |
| `data` | the Data and Root tabs over the merged tree |
| `downloads`, `nexus` | the Downloads tab and its queue |
| `install` | the installer dialogs: `strata_install`'s questions as JSON messages |
| `live` | refreshing a live deploy after a change; `deployed_profile` |
| `settings` | the window's layout stored in `ModOrganizer.ini` under MO2's keys |
| `style`, `config` | stylesheet search and the default sheet (`riki` when found, else `vs15 Dark`); Strata's own config (the last instance) |
| `conflicts/` | the conflicts area: the mod info dialog (`modinfo`), previews (`preview`: DDS, images, text) |
| `modlist/` | the modlist area's dialogs: profiles, executables, settings, ini editor, problems, built-in plugins |
| `crashlogs` | Crash Log Labeler / Viewer (the third-party `crashlogtools`): the log folder, the names database, labelling |
| `test_instance` | (tests only) a small instance on disk |

Tests: `cargo test -p strata-ui`.
