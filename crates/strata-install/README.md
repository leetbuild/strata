# strata-install

MO2's `InstallationManager` and its installer plugins, without dialogs: an archive becomes a mod folder with
its `meta.ini`, and the mod is added to the profile.

**Must not:** do UI. Every question is a call on `install::InstallUi`, answered by the shell's dialogs or by the
CLI's options (`strata install`).

| Module | What it holds (MO2 source it ports) |
|---|---|
| `install` | the pipeline: `InstallationManager::install`, `doInstall`, name clash (merge / replace / rename) |
| `instance` | `install_into`: what the CLI and the window share (the mod added to the list, auto-enable) |
| `tree` | uibase `IFileTree` without Qt |
| `checker` | `ModDataChecker` (does a folder look like Data), with Leet's Root rule |
| `rootsorter` | Leet change 4: game-folder files sorted into `Root/` |
| `installers` | quick, BAIN, bundle, FOMOD's tree, the manual installer's model |
| `fomod/` | `ModuleConfig.xml` / `info.xml` parsing (`mod.rs`), the dialog's logic (`session.rs`) |
| `meta` | the download's `.meta`, the new mod's `meta.ini` keys |
| `name` | `GuessedValue`, `fixDirectoryName`, Nexus file names, ModPub names |
| `version` | enough of uibase `VersionInfo`: canonical strings and comparison |
| `profile` | the new mod's place in the list, Install to Separator |
| `downloads` | the Downloads tab's actions |
| `bsa` | "BSA Extractor" on install |
| `settings` | the install settings in `ModOrganizer.ini` |
| `archive` | the `Archives` trait over `strata_archive::Archive` (tests replace it) |

Tests: `cargo test -p strata-install` (unit tests; `tests/golden.rs` oracle check 5 with archives built by `7z`
in the test; `tests/fomod_real.rs`). `tools/oracle/check-install.sh <instance>` replays a real instance's
downloads in a scratch instance.
