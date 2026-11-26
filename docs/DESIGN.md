# Design

How Strata works and why: the rules the code follows. `docs/ARCHITECTURE.md` maps the code itself (crates,
the CLI, the shell); comments in the code point at the sections here.

Strata is **MO2 2.5.2's behaviour** plus the **MO2 Leet Edition** additions, on Linux, with games run under
Proton. When in doubt about what MO2 does, its source (`ModOrganizer2/modorganizer`, tag `v2.5.2`, and the
plugin repositories) is the answer, not a guess.

## Instance files

Strata's files are MO2's files. MO2 (on Windows), other MO2-compatible managers on Linux (Fluorine Manager)
and Strata can take turns on one instance with nothing lost.

### Rules for every file
- **Round trip**: reading and writing a file without changes gives the same bytes: line endings, BOM, key
  order, comments, unknown sections and keys. Tests check this per file type.
- **Atomic writes** (temp file + rename), and only when the content changed.
- **Qt `QSettings` ini quirks** are reproduced exactly: `@ByteArray(...)`, `@Variant(...)`, `@Size(w h)`,
  escaped `\\` and `%U` sequences, array sections (`size=`, `1\key=`), keys with `/` stored as `\`.
- **Paths** written on Windows (`C:\...`, `Z:\...`) and on Linux both occur. They are normalised on read and
  written back in the form found.

### The instance folder
```
<instance>/
  ModOrganizer.ini        [General], [Settings] (folders, %BASE_DIR%, style), [customExecutables],
                          [Plugins] (per-plugin settings), [fluorine] (prefix, Proton: read, kept)
  mods/<Name>/            files + meta.ini
  profiles/<Profile>/     modlist.txt, plugins.txt, loadorder.txt, archives.txt (LE), settings.ini,
                          local inis, saves/ (local saves)
  overwrite/              files written by programs at run time
  downloads/              archives + <archive>.meta
  strata/                 Strata's own files (whiteouts.txt, journals, scan cache); other managers ignore it
```
- `modlist.txt`: `+Name` enabled, `-Name` disabled, `*Name` foreign (DLC, game files); the top line has the
  highest priority; separators are mods named `<Name>_separator`; Overwrite is not listed.
- `meta.ini`: every key is preserved, including `Sub-Separator\nest` on nested separators.
- `plugins.txt` / `loadorder.txt`: SSE marks enabled plugins with `*` and leaves out the base masters; LE lists
  only enabled plugins. Both live in the profile and reach the prefix through the overlay (below).
- Scale: lists with well over 1,000 mods, hundreds of GB and dozens of nested separators are normal.

## The overlay

`strata deploy` mounts the active profile's layers (FUSE); `strata run` mounts, launches, waits, unmounts.

### What the game sees
- **Data**: the game's own `Data/` at the bottom, then mods by priority (low to high), then `overwrite/` on top.
  For each path the highest layer that has it wins; folders merge.
- **Root**: every mod's top-level `Root/` folder over the game folder itself (SKSE's loader, ENB's DLLs), with
  the same rules. **One mount, in place over the game folder**: vfsd opens the real folder before mounting and
  reads the bottom layer through that handle, so programs see the usual `Z:\...\Data` paths and the real game
  folder never changes.
- **Mod mappings**: a game can map `<mod>/<subfolder>` to another target (Oblivion Remastered); SSE uses the
  default.
- `*.mohidden` files are invisible. BSA/BA2 archives are files like any other.

### Behaviour
- **Case-insensitive, case-preserving** lookups, as on Windows; the first spelling seen wins. Wine asks
  directories for `FS_CASEFOLD_FL` (`FS_IOC_GETFLAGS`) to skip its own slow case search; vfsd answers it.
- **Writes go to `overwrite/`**: new files are made there; changing a file of a mod or the game copies it up
  first. Root files edited at run time (ENB's ini) go back to their mod.
- **Whiteouts**: deleting a lower-layer file records its path in `<instance>/strata/whiteouts.txt` (one path
  per line, written atomically); no marker files in `overwrite/`, where other managers would list them.
  Creating the path again removes the whiteout. Other managers ignore the list (the file just reappears).
  The window lists the entries on Overwrite with a restore action.

### Crash safety
A journal is written before any copy-up is promoted, and finished at the next mount if vfsd died in the
middle. A stale mount left by a crash is found and cleaned at the next start (`fusermount3 -u`, then a lazy
unmount).

### Profile files in the prefix are overlaid, not copied
vfsd puts small extra mounts over the prefix's `AppData/Local/<Game>/` (`plugins.txt`, `loadorder.txt`) and
`Documents/My Games/<Game>/` (the profile's local inis; `Saves/` with local saves). The bottom layer is the
prefix's real folder; the top layer and write target is the profile folder, limited to the files the game's
definition maps. The game writes straight into the profile: nothing to copy back, nothing lost in a crash, no
symlinks. Everything else written there (SKSE logs, ...) lands in the prefix as usual.

### Speed
The merged index is built from a parallel scan with a persistent cache keyed by (path, mtime, size), so a
restart doesn't rescan unchanged mods. Lookups are O(1) on the case-folded key; inodes are stable for the
mount's lifetime; entry, attribute and page caching stay on for the run. The mount runs in its own process
(`strata vfsd`), so a window crash never takes the game's files away mid-run.

## Proton and launching

Pick a Proton, get a working prefix, press Run.

1. **Per-instance settings only.** Prefix, Proton and runtime live in the instance's `ModOrganizer.ini`
   (the `[fluorine]` keys, so instances made by Fluorine Manager work as they are). No global override.
2. **The runtime matches the Proton.** The Proton's `toolmanifest.vdf` (`require_tool_appid`) names its Steam
   Linux Runtime (sniper for GE-Proton 10), found through Steam's `libraryfolders.vdf`. A missing runtime is an
   error naming what to install, never a fallback to another one: GE-Proton 10 inside SLR 4 lacks
   `libvpx.so.6`, and xWMA voices go silent. Strata builds the launch command itself
   (`<SLR>/_v2-entry-point --verb=waitforexitandrun -- <proton>/proton waitforexitandrun <exe>`). umu-launcher
   is not used: Proton skips its `steam.exe` bridge when `UMU_ID` is set, and SteamStub DRM needs it.
3. **No custom audio stack.** The prefix uses Proton's own xaudio2/FAudio; no DLL overrides nobody asked for.
4. **Prefix setup is explicit and idempotent.** A list of steps (create the prefix with the chosen Proton, then
   VC++ 2015-2022, d3dcompiler_47, the DirectX June 2010 DLLs, .NET runtimes for tools, fonts if needed). Each
   step checks before it acts, so running it again never breaks a working prefix. Downloads are pinned by
   sha256. Every step is logged.
5. **Steam**: native and Flatpak are found; Steam is started when the game needs SteamStub DRM;
   `SteamAppId`, `SteamGameId` and `STEAM_COMPAT_*` are set (`STEAM_COMPAT_DATA_PATH` is the compatdata
   folder, not `pfx`).
6. **ENB**: with ENB in the merged game folder, `dxvk.conf` goes next to the game's exe in the overlay, not in
   the real folder.
7. **Tools** (xEdit, BodySlide, Pandora, DynDOLOD, LOOT, the CK) run in the same prefix through the overlay,
   like the game. Tools that run from their mod folder keep working.
8. **Logs**: every launch writes its exact command line and environment to `<instance>/logs/`.

## The shell

The window is C++ Qt 6 Widgets, because `.qss` stylesheets apply only to Widgets and Rust has no good Widgets
bindings. It owns no state: models, actions and settings live in the Rust crates and cross the `cxx` bridge.
It starts from MO2's own Designer forms and aims to look like MO2 2.5.2.

### Style
- Sheets are found in `<instance>/stylesheets/`, `~/.local/share/strata/stylesheets/` and the bundled set
  (MO2's own sheets). Symlinks are followed: desktop themes link their rendered sheets in.
- **Hot reload**: the active sheet is watched (edits in place and replace by rename); a change re-applies it
  within a second. The sheet is applied to the whole application only, never per widget, so nothing overrides
  it.

### Behaviour
- The UI thread never waits on disk: scans, installs, downloads and overlay refreshes run in workers and report
  progress.
- Header state, splitters and window geometry are saved on change and restored per instance.
- Every action is reachable from the menu bar too.

## Oracle checks

The machine proves MO2 parity, not a person clicking around. The reference answers come from MO2's behaviour
on real instances and from small reference implementations written straight from MO2's source
(`tools/oracle/*.py`). Runs use **copies** of instances (`_stage/oracle/<name>/`, reflink copies where the
filesystem allows) and never write to a real instance, game folder or prefix. Prefixes may be shared by several
instances, so a test that launches anything copies the prefix and runs with `--prefix <copy>`.

| Corpus | Use |
|---|---|
| Synthetic (`tools/gen-instance`) | every edge case on purpose; runs everywhere, in `tools/test.sh` and CI |
| Real instances (`STRATA_REAL_INSTANCES`) | real-world files, read only; the launch corpus for check 7 |
| A large real list (1,000+ mods), as a copy | scale: round trip, index, mount, tree parity, speed; never launched |

1. **Format round trip**: every `ModOrganizer.ini`, `modlist.txt`, `meta.ini`, `plugins.txt`,
   `loadorder.txt` and `*.meta` parsed and written back byte for byte.
2. **Merged tree parity**: Strata's merged tree (path to winning layer) against (a) a small Python reference
   written from MO2's `DirectoryEntry` rules (`tools/oracle/merge.py`) and (b) the tree another MO2 VFS shows
   for the same instance; differences are explained or fixed.
3. **Conflicts parity**: winners and losers per file and the per-mod flags.
4. **Plugin list parity**: active plugins, order, masters, missing masters, light flags; LOOT's sort equals
   libloot's on the same input.
5. **Install golden tests**: archives built in the test (simple, wrapped, Data + Root, BAIN, FOMOD with
   conditions, bundle) against expected trees.
6. **Mount behaviour**: case-insensitive open and stat, folder merges, copy-up, create / rename / delete,
   whiteouts, unicode, the casefold ioctl, concurrent readers, recovery after `kill -9` of vfsd.
7. **End to end in the game**: `strata run SKSE` on a copy; a tiny SKSE plugin (`tools/probe/`) writes
   `strata-probe.json` at `kDataLoaded` and quits the game. Passing means the file lands in the copy's
   `overwrite/` with the expected plugin list: overlay, Root, Proton and SKSE proven without a person.
8. **Speed**: index time cold and warm, mount time, `find` over the mount, a game load to `kDataLoaded`.

What the machine can't judge: the look next to MO2, the feel of the lists, a real play session.
