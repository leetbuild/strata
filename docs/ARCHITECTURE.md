# Architecture (the contract between agents)

Change this file first when an interface changes, then the code.

## Shape
```
                      ┌───────────────── strata (one binary) ─────────────────┐
  user ── Qt window ──┤ shell/ (C++ Qt6 Widgets: views, delegates, dialogs)   │
                      │      │ cxx bridge (strata-ffi)                        │
  user ── CLI ────────┤ strata-cli ── strata-core ── strata-game (Game trait)  │
                      │                 │     │         strata-install       │
                      │                 │     │         strata-nexus         │
                      │                 │     └──────── strata-launch ───────┼── Steam, Proton, prefix
                      └─────────────────┼───────────────────────────────────── ┘
                                        │ Unix socket (JSON lines)
                              strata vfsd (same binary, own process) ── FUSE ── the game in Proton
```

## Crates (Cargo workspace `crates/`)
Each crate's `README.md` lists its modules and how to run its tests.

| Crate | Owns | Must not |
|---|---|---|
| `strata-ini` | QSettings-compatible ini read/write with a byte-exact round trip (`document`, `codec`) | know about MO2 |
| `strata-core` | Instance, Profile(s), ModList, `meta.ini` (`ModMeta`), Sub-Separator nesting, categories, the plugin list and saves (`plugins/`), executables, settings, the profile's game inis (`inis`), overwrite, backups, trash, the layer list (`layers`), change events. Writes go through `write.rs` (atomic, only when changed; `Transaction` + journal in `<instance>/strata/journal` for several files) | touch FUSE, Qt, network |
| `strata-game` | `Game` trait + Skyrim SE, Skyrim LE, Oblivion: paths, ini files, plugin list format, script extender, executables, archive invalidation, mod mappings; plugin headers (`plugin`), save headers (`save`), the layer types (`layer`) | depend on core internals |
| `strata-archive` | BSA/BA2 reading (`bsa`), BSA writing, packer and extractor (`pack`), 7z/zip/rar through the `7z` program (`sevenzip`) | |
| `strata-vfs` | layer list → merged tree (`tree`, journal, `refresh` rebase), the FUSE filesystem (`fs`), mounting (`mount`), vfsd and its protocol (`daemon`), the conflict index (`conflicts`: MO2's Data and Root structures, archive contents, per-mod flags, incremental) | know about profiles (it gets a resolved layer list) |
| `strata-install` | installers (quick, manual model, BAIN, FOMOD, bundle), Root sorter, the install pipeline (`install::install`: name guessing, name clash, extraction, `meta.ini`, the download's `.meta`); `instance::install_into` (what CLI and window share) adds the mod through the core's `ModList` with Install to Separator; `downloads`: the Downloads tab's actions | do UI (every question is a method of `install::InstallUi`, answered by the CLI's options or the window's dialogs through `strata_ui::install`) |
| `strata-launch` | Steam detection and start, Proton discovery (`compatibilitytools.d` and the libraries), the Proton's runtime (direct SLR), the launch plan under the `Runner` trait, waiting for the prefix to go quiet, prefix setup (`setup`: steps that check before they act, pinned sha256 downloads) | run anything but through `ProtonCall` / `Runner` |
| `strata-nexus` | link parsing (`links`), the v1 API behind a `Transport` (`api`), SSO (`sso`), the key in the secret service (`keystore`), nxm → queued download (`nxm`), the queue with pause / resume and MO2's `.meta` (`queue`), query by MD5 (`query`), a fake server for tests (`fake`) | refresh or store anyone's OAuth token; print a key |
| `strata-loot` | LOOT sorting over the `libloot` crate, lootcli's report (messages, dirty info, missing masters / requirements, incompatibilities), masterlist download | mount anything itself (it gets a game folder: `strata_ui::loot_view` gives it the profile through the overlay, case-insensitive, or the mods as additional data folders without FUSE) |
| `strata-diag` | the problems list: checks over plain inputs (plugin list, folders, log text, mod sources) | read the overlay or the prefix itself (`strata_ui::modlist::problems` gathers the inputs) |
| `strata-ui` | what the window shows and does, as plain Rust (see "The shell" below); also `crashlogs` (Crash Log Labeler / Viewer), shared with the CLI | know Qt; hold state the shell can't see |
| `strata-ffi` | everything that crosses the Rust/C++ boundary: `strata_run` and the `cxx` bridges. It is **the static library** CMake links, so it depends on every crate the program needs | hold state or logic of its own (it has no cargo tests) |
| `strata-cli` | library: `run(args, shell_entry)` = the CLI subcommands, `vfsd`, the shell when there is no subcommand; plus the cargo-only `strata-headless` binary | link Qt |

The shell (`shell/`, C++) is linked into the same binary through Corrosion; `strata` with no arguments opens the
window. One binary, one install.

## Entry point
- CMake links `build/strata`; the Rust crates come in as one static library, `strata-ffi` (Corrosion).
- `shell/src/main.cpp` is a trampoline: it calls `strata_run(argc, argv, shellEntry)` (C ABI, in `strata-ffi`),
  which calls `strata_cli::run(args, shell)` and returns its exit code. Rust parses the arguments (`-i` and `-p`
  reach the shell as `shellEntry(argc, argv, instance, profile)`, empty when not given). Only with no subcommand
  does it call `shell_entry`, which creates the `QApplication` and the window. CLI and `vfsd` paths never
  initialise Qt.
- `strata-headless`: a cargo-only binary of the same library with no shell (without a subcommand it prints that
  it was built without the window). `cargo build` / `cargo test` need no Qt or CMake; `tools/test.sh` uses it.
  It is a development target and is not installed.
- `vfsd` is started by executing `/proc/self/exe vfsd`, never by `fork()` from a process that has Qt running.
- Every `cxx` bridge function catches panics; nothing unwinds into C++.

## Core rules
- **One source of truth**: the core owns state; the shell renders snapshots and sends commands.
- **Events**: mod list and profile operations return typed events (`strata_core::ModListEvent`: `StateChanged`,
  `Moved {from, to}` (one per single move, in order), `NestChanged`, `Renamed`, `Removed`, `Added`,
  `MetaChanged`, `OverwriteChanged`; `ProfileEvent`: `Created`, `Renamed`, `Removed`, `Selected`,
  `SettingsChanged`, `BackupChanged`). The bridge hands them to the shell (`ModEvent`), which updates its models
  incrementally (no full reset on a toggle); the CLI prints them with `--json`.
- **Threads**: the UI thread does no long I/O: tree scans, conflict indexes, problem checks and installs run on
  worker threads (the shell's `QThread`s; rayon inside `strata-vfs` for scans).
- **Errors**: `thiserror` per library crate, `anyhow` only in the CLI. Every error shown to the user says what to do.
- **Writes**: atomic, only when content changed, journaled where several files change together. `ModOrganizer.ini`
  is changed only through `strata_core::settings::store_instance_ini`; the XDG folders come from `strata_core::dirs`.
- **Logs**: each run writes its own log into `<instance>/logs/`: `strata-vfsd.log` (the daemon's stderr),
  `strata-run-<time>.log` (the exact command and environment of a launch), `strata-prefix-setup-<time>.log`.
  There is no general application log yet (no `tracing`).

## The layer model (core → vfs)
The core resolves the active profile into an ordered list (`strata_core::resolve_layers`):
`Layer { id, kind: Game|Mod|Overwrite|Passthrough, source: PathBuf, rules: Vec<Mapping> }`, low → high priority,
where a `Mapping { from, to, exclude, include, write_back }` comes from the game (SSE: `"" → Data/` without
`Root` and `meta.ini`; `Root/ → ./` without `Data`, `Data Files`, `Saves` and the game's inis, `write_back`).
The types live in `strata-game` (`layer.rs`), so that neither `strata-core` nor `strata-vfs` depends on the
other. vfsd builds the merged tree from the list (`strata-vfs/src/tree.rs`).

Writes: a new entry goes to the highest **writable** layer (`Overwrite`, `Passthrough`) whose rules take its
path; changing a file of a read-only layer copies it there first (through a temp file, so a crash leaves the
copy whole or absent), unless its rule has `write_back`; deleting a read-only layer's file records a whiteout. A change of several steps
on disk (a folder rename, a moved mod file and its whiteout) is written to `<instance>/strata/overlay-journal.json`
first and finished by the next mount if vfsd dies in the middle.
`Passthrough` is the prefix folder under a profile mount: everything the profile doesn't claim is written
straight through. The main mount sits in place over the Game Root. The game also names the **profile mounts**
(`strata_core::profile_mounts`): prefix folders overlaid with the profile's files (`plugins.txt`,
`loadorder.txt`, local inis, `saves/`), each one a mountpoint with its own short layer list and a file filter
(`spec/03-vfs.md`). Whiteouts live in `<instance>/strata/whiteouts.txt`. A mod list change sends a new list
over the socket (`refresh`); vfsd swaps the tree between FUSE requests. A profile switch unmounts and mounts
again (`strata_cli::select_profile`).

## vfsd protocol (JSON lines over `$XDG_RUNTIME_DIR/strata/<instance-hash>.sock`)
Requests are `{"v":1,"cmd":...}` (`strata_vfs::daemon::Request`), replies `{"v":1,"ok":true,...}` or
`{"v":1,"ok":false,"error":"..."}`. The daemon answers these five requests:
- `mount {mounts: [{id, layers, mountpoint, whiteouts?, label?}]}`: the game mount plus the profile mounts, all
  or nothing. A mountpoint that is one of the layers' own folders is mounted in place. `label` is free text
  given back by `status` (the CLI puts the profile's name there).
- `status` → `pid`, `uptime_secs`, and per mount: `id`, `mountpoint`, `label`, `layers` (count), `nodes`.
- `resolve {path, mount?}` → `found`, shown `path`, `kind`, `origin` (layer id), `real` file.
- `refresh {id, layers}` → `id`, `nodes`, `changed`, `entries`: the mount shows a new layer list without
  unmounting. The tree is rebuilt while FUSE requests wait; unchanged paths keep their node ids (= inodes); the
  kernel is told to forget the files and folder entries that changed. Open files read on.
- `unmount {force?}`: unmounts everything and the daemon exits; busy mounts are an error unless forced.

It sends no events of its own (the `written` / `error` events of `spec/03-vfs.md` are not built). The daemon
unmounts on SIGTERM and SIGINT. A mount it left behind when killed is cleaned by the next mount.

## CLI (`--json` everywhere)
Global options: `-i/--instance <path>` (default: the current folder), `-p/--profile <name>` (default: the
selected one), `--json`, `--prefix <path>` (this run only, never stored; for tests on a copied prefix).
Commands that change what the game sees refresh a live deploy of the same profile. The list below is generated
from `strata --help` and each subcommand's help by `tools/cli-help.py` (2026-11-25):

- `strata list [mods|profiles|executables|plugins|saves]`: List the instance's mods (in priority order, lowest first), profiles, executables, plugins or saves
- `strata info`: Show the instance's game, folders, profile, prefix and Proton
- `strata tree` [--origin]: Print the merged game folder: every file the game would see
- `strata deploy`: Mount the profile's layers over the game folder (and its files into the prefix)
- `strata run [TITLE] [-- <ARGS>...]` [--timeout --dry-run]: Mount, run one of the instance's executables through Proton, wait, unmount
- `strata undeploy` [--force]: Unmount what `deploy` mounted
- `strata status`: Show what is mounted for this instance
- `strata resolve <PATH>`: Show which mod a path of the mounted game folder comes from, and its real file
- `strata enable <MODS>...`: Enable mods (names in any case). Refreshes a live deploy of the profile
- `strata disable <MODS>...`: Disable mods. Refreshes a live deploy of the profile
- `strata move <MODS>...` [--to --by]: Move mods to a priority (as a drop on that row in MO2), or by an offset
- `strata mod`: Create, rename and edit mods (meta.ini)
  - `strata mod create <NAME>`: Create an empty mod (on top, or just above --at; at the end of --at when it is a separator)
  - `strata mod rename <OLD> <NEW>`: Rename a mod: its folder and its line in every profile's modlist.txt
  - `strata mod remove <NAMES>...`: Remove mods: their folders go to the trash (or with --permanent are deleted)
  - `strata mod info <NAME>`: Show a mod's meta.ini as MO2 reads it
  - `strata mod notes <NAME> [TEXT]`: Show or set a mod's notes
  - `strata mod color <NAME> [COLOR]`: Show or set a separator's colour: #rrggbb, #aarrggbb, or "none"
  - `strata mod categories <NAME> [IDS]`: Show or set a mod's categories: ids, comma-separated, primary first (`none` clears)
- `strata separator`: Separators and Sub-Separator levels
  - `strata separator add <NAME>`: Add a separator (name without "_separator"). --at a separator: at its end, one level deeper; --at a mod: just above it, at its separator's level
  - `strata separator nest <NAME> <LEVEL>`: Set a separator's Sub-Separator level
  - `strata separator tree`: Print the separator tree as the window shows it in ascending priority
- `strata profile`: Create, copy, rename and delete profiles; local saves and inis
  - `strata profile create <NAME>`: Create a profile (inis and plugins.txt from the prefix)
  - `strata profile copy <FROM> <NAME>`: Copy a profile, saves and all
  - `strata profile select <NAME>`: Make a profile the instance's selected one (a live deploy is unmounted and mounted again)
  - `strata profile rename <OLD> <NEW>`: Rename a profile (not the selected one)
  - `strata profile delete <NAME>`: Delete a profile with its local saves (not the selected one). To the trash by default
  - `strata profile local-saves <STATE>`: Profile-specific saves on or off (--profile or the selected one)
  - `strata profile local-inis <STATE>`: Profile-specific game inis on or off
- `strata backup`: Backups of the mod list and the load order
  - `strata backup create [WHAT]`: Back up the mod list or the load order, as MO2's backup buttons do
  - `strata backup list [WHAT]`: List the backups, newest first
  - `strata backup restore <SUFFIX> [WHAT]`: Restore a backup by its suffix (as `backup list` shows it)
- `strata overwrite`: Clear overwrite or move its content into a mod
  - `strata overwrite clear`: Delete everything in overwrite (to the trash by default), and Strata's whiteouts
  - `strata overwrite move <NAME>`: Move overwrite's content into a mod (merging; overwrite's files win)
- `strata category`: The instance's mod categories
  - `strata category list`: List the categories (categories.dat)
  - `strata category add <NAME>`: Add a category; prints its id
  - `strata category rename <ID> <NAME>`: Rename a category
  - `strata category remove <ID>`: Remove a category from categories.dat
- `strata executable`: The executables list: add, change, remove, reorder
  - `strata executable list`: List the executables with all their fields
  - `strata executable set <TITLE>`: Add an executable, or change the one with this title
  - `strata executable remove <TITLE>`: Remove an executable
  - `strata executable move <TITLE> <TO>`: Move an executable to a place in the list (0 = first)
- `strata link <URL>`: The browser's handler of nxm:// links: hands the link to the open window, which downloads it (starts the window on the last instance when none is open)
- `strata register-links` [--make-default]: Write Strata's desktop entry for nxm:// and modl:// links (asked for, never silent)
- `strata plugins`: Change the plugin list: enable, disable, move, lock, unlock
  - `strata plugins enable <NAMES>...`: Enable plugins (primary plugins are always on)
  - `strata plugins disable <NAMES>...`: Disable plugins
  - `strata plugins move --to <TO> <NAMES>...`: Send plugins to a priority (0 = first), keeping masters above plugins and plugins below their masters, as MO2's "Send to priority" does
  - `strata plugins lock <NAME>`: Lock a plugin at its load order position (lockedorder.txt)
  - `strata plugins unlock <NAME>`: Unlock a plugin
- `strata sort` [--dry-run --offline]: Sort the plugins with LOOT (libloot), as MO2's Sort button does
- `strata problems` [--fix]: List problems, as MO2's Problems dialog: missing masters, form 43 plugins, script extender plugins that failed to load, files in overwrite (when enabled), ...
- `strata conflicts [MOD]`: Conflicts: every mod's flag, or one mod's files with who wins them (the mod info dialog's Conflicts tabs)
- `strata archive <FILE>` [--extract]: List the files of an archive: BSA / BA2, or 7z / zip / rar (through 7-Zip)
- `strata pack-bsa <MOD>` [--plugin --no-compress --hide-loose]: Pack a mod's loose files into a BSA named after its plugin (MO2's BSA packer)
- `strata extract-bsa <MOD>` [--remove]: Unpack a mod's BSA/BA2 archives into the mod folder (MO2's BSA extractor)
- `strata install [ARCHIVE]` [--reinstall --password --name --choices --merge --replace --rename --backup --no-root-sort --no-enable --extract-bsa --remove-bsa --under --plan]: Install a mod from an archive, answering the installer's questions from options
- `strata prefix`: The instance's Wine prefix: `strata prefix setup [--check]`
  - `strata prefix setup`: Make the instance's prefix fit for the game and its tools: create it with the instance's Proton, then VC++ 2015-2022, d3dcompiler_47, d3dx9/10/11, .NET 8 and 9 Desktop. Each step checks first; a working prefix is left as it is
- `strata proton`: The Protons on this machine: `strata proton list`
  - `strata proton list`: The Protons Steam knows: compatibilitytools.d and the Steam libraries
- `strata create-portable --game <PATH> --proton <PATH> <FOLDER>` [--game --proton --wine-prefix --game-name]: Create a new portable instance from scratch (Fluorine's CLI of the same name)
- `strata downloads <ACTION> [ARCHIVES]...` [--permanently --under]: Act on downloaded archives (the Downloads tab's actions)
- `strata ini`: The profile's game inis (MO2's ini editor): list, show, save from standard input
  - `strata ini list`: The game's inis of the profile and where each one is
  - `strata ini show <NAME>`: Print one ini (`Skyrim.ini`, any case)
  - `strata ini save <NAME>`: Replace one ini with the text read from standard input (written with CRLF, as MO2's ini editor writes; nothing written when unchanged)
- `strata origins <PATHS>...`: Every mod that provides a file, the winner first (loose files and loaded archives). Paths relative to Data (`Root/...` for the Root folder); `-` reads paths from standard input, one per line
- `strata ls [PATH]` [--filter --recursive --dirs]: The entries of a folder of the merged game folder, with their origin
- `strata vfs-export <FILE>`: Write every file of the merged game folder as CSV (path,origin,real); `-` = stdout
- `strata analyze <MOD>` [--limit]: One mod: its place, plugins, conflicts and files per conflict status
- `strata refresh`: Tell a live deploy that mods changed on disk (a mod extracted by hand)
- `strata crashlogs`: Crash Logger's logs: list them, label their call stacks with function names, update the names database (Crash Log Labeler / Viewer)
  - `strata crashlogs list`: The crash logs, newest first, and their folder (the Crash Log Viewer)
  - `strata crashlogs label [FILES]...`: Label the crash logs' call stacks with the names database (all logs, or the given files); a changed log's old text goes to the trash
  - `strata crashlogs update`: Download the names database (meh321's AddressLibraryDatabase, skyrimae.rename)
- `strata nexus`: The Nexus account: login (SSO), logout, status, check-updates
  - `strata nexus login`: Log in through the website (Nexus SSO: the browser opens, you allow Strata) and keep the API key in the secret service. --key-stdin stores a personal API key instead
  - `strata nexus logout`: Forget the stored API key
  - `strata nexus status`: Who the stored key belongs to, and the API's rate limits (the key is never shown)
  - `strata nexus check-updates`: Ask Nexus which of the instance's mods have newer versions; writes `newestVersion`
- `strata nxm <URL>` [--install]: Download the file of an nxm:// link into the downloads folder (and install it)

Hidden (tests and internals): `strata vfsd --socket <path>`, `strata oracle roundtrip|conflicts`,
`strata preview <dds> --out <pam>`.

## The shell (`shell/`, C++ Qt 6 Widgets)
The window renders `strata-ui` views through the bridges and holds no state of its own. One area = its own
files:

| Area | C++ (`shell/src/`) | Forms (`shell/forms/`, MO2's `.ui`) | Bridge (`crates/strata-ffi/src/`) | Views (`crates/strata-ui/src/`) |
|---|---|---|---|---|
| main window (shell) | `main.cpp`, `mainwindow.*`, `modlist.*`, `pluginstab.*`, `datatab.*`, `filterpanel.*`, `layoutstate.*`, `loglist.cpp`, `style.*`, `downloadproxy.h`, `widgets/` | `mainwindow.ui` | `bridge.rs` (`Session`) | `session`, `mods`, `filter`, `content`, `conflict_flags`, `plugins`, `saves`, `data`, `downloads`, `live`, `settings`, `style`, `config` |
| modlist | `modlist/`: profiles, executables, settings, delete confirm, ini editor, Problems, Crash Log Viewer (`dialogs.h` = the entry points) | `profilesdialog.ui`, `profileinputdialog.ui`, `editexecutablesdialog.ui`, `settingsdialog.ui`, `textviewer.ui`, `problemsdialog.ui` | `bridge_modlist.rs`, `bridge_builtins.rs`, `bridge_crashlogs.rs` | `modlist/` |
| conflicts | `conflicts/`: mod info dialog, previews | `modinfodialog.ui`, `previewdialog.ui` | `bridge_conflicts.rs` | `conflicts/` |
| install | `install/`: installer host and dialogs, FOMOD dialog, download actions and queue | `install/*.ui` | `bridge_install.rs` | `install`, `nexus`, `downloads` |
| loot | `loot/`: the Sort button's LOOT dialog (progress, log, Cancel, report) and the report's markdown, plugin icons and tooltips | `lootdialog.ui` | `bridge_loot.rs` (`LootHost`) | `plugins` (`sort_with_report`, `SortOutcome`) |
| instances | `instances/`: the instance manager (File > Change Game, and at start when there is no instance) and the new-instance wizard | `instancemanagerdialog.ui` | `bridge_instances.rs` | `instances` |

Other targets of `shell/CMakeLists.txt`: `strata-dialog-shot` (one installer dialog to a picture,
`tools/shell-test-install.sh`) and `strata-shell-test` (Qt Test driving the real window offscreen,
`shell/tests/`, `tools/shell-drive.sh`). Each bridge file is one line of `corrosion_add_cxxbridge` in the root
`CMakeLists.txt`; Corrosion generates `strata_bridge/<file>.h`.

## Testing
- `cargo test --workspace`: unit tests per crate (property tests for parsers), integration tests in each crate's
  `tests/` (FUSE tests mount in a temp folder and print `SKIPPED` without `/dev/fuse`), golden installer tests,
  the CLI end to end (`strata-cli/tests/automation.rs`, `nexus_e2e.rs`).
- Oracle checks (`spec/07-oracle.md`) as `tools/oracle/check-*.sh`, each exiting non-zero on mismatch, with
  Python references beside them (`modlist.py`, `conflicts.py`, `plugins.py`, `bsa_reference.py`, ...).
- The shell: `tools/shell-test.sh` (window pictures), `tools/shell-dialogs.sh` (dialog pictures),
  `tools/shell-drive.sh` (driven by Qt Test), `tools/shell-test-install.sh`, `tools/conflicts-shell-test.sh`.
- `tools/test.sh` runs everything that needs no game; `tools/e2e.sh` runs check 7 (starts the game: ask first).

## Layout
```
crates/<crate>/      Rust (each with README.md)
shell/               C++ Qt Widgets (CMake): src/<area>/, forms/ = MO2 .ui files, tests/, stylesheets/, resources
tools/               build.sh, test.sh, e2e.sh, gen-instance, oracle/, probe/ (SKSE probe plugin), shell-*.sh,
                     bench-overlay.sh, fake-nexus.py
- Unit tests per crate, property tests for parsers (`proptest`), golden tests for installers and formats.
- FUSE tests mount in a temp dir (needs `/dev/fuse`; skip with a clear message when absent).
- Oracle checks (`spec/07-oracle.md`) as `tools/oracle/*.sh`, each exiting non-zero on mismatch.
- `build/strata-shell-test` (`shell/tests/`, Qt Test): the same shell objects (CMake object library
  `strata_shell`) driven offscreen by clicks, keys, drops and answered dialogs on generated instances; it goes
  through `strata_run` too, so a deploy it starts finds `vfsd` in its own executable. `tools/shell-drive.sh`.
- `tools/test.sh` runs all of the above that need no game; `tools/e2e.sh` runs check 7.

## Layout
```
crates/<crate>/      Rust
shell/               C++ Qt Widgets (CMake), forms/ = MO2 .ui files, resources/, tests/ = strata-shell-test
tools/               build.sh, test.sh, e2e.sh, oracle/, gen-instance/, probe/ (SKSE probe plugin)
spec/  docs/         design notes, the user guide, the architecture
_stage/              scratch, oracle copies, logs (git-ignored)
```
