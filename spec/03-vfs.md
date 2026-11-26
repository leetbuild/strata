# 03 — The overlay (FUSE)

`strata deploy` mounts the active profile's layers; `strata run` mounts, launches, waits, unmounts.

## What the game must see
- **Data**: the game's own `Data/` at the bottom, then mods by priority (low → high), then `overwrite/` on top.
  For each path the highest layer that has it wins. Directories merge.
- **Game Root** (Leet's Root Builder, which on Linux becomes part of the overlay): every mod's top-level `Root/`
  folder layered over the game folder itself, same priority rules. SKSE's loader, ENB's dlls, `enbseries.ini`
  live in `Root/`. The real game folder never changes.
  **One mount, in place over the Game Root** (Root layers at the top level, Data layers under `Data/`). vfsd
  opens the real folder before mounting and reads the bottom layer through that handle (`openat`), so tools see
  the same `Z:\...\Game Root\Data` paths as under MO2 and Fluorine. Not Fluorine's way (mount `Data`, reflink
  Root files into the game folder): that writes to the real folder.
- **Mod mappings** (OBRE, Leet change 7): a game can map `<mod>/<key>` to other targets. Design the layer model
  so a game supplies a list of (source subfolder → target folder) rules; SSE uses the default.
- Hidden files: `*.mohidden` are invisible.
- BSA/BA2 archives are files like any other; the overlay never unpacks them.

## Behaviour
- **Case-insensitive, case-preserving** lookups (Windows semantics). The first spelling seen wins. Wine checks
  `FS_IOC_GETFLAGS` for `FS_CASEFOLD_FL` on directories to skip its own slow case search: answer it (Fluorine
  `vfs/mo2filesystem.h` does this; it is a big speed-up).
- **Writes go to `overwrite/`**: creating a file creates it there; modifying a file from a mod or the game
  copies it up to `overwrite/` first (copy-on-write), unless a Leet rule says otherwise (Root files edited at run
  time, e.g. ENB's `enbseries.ini`, go back to their mod: Leet change 6). Deletes of lower-layer files are
  whiteouts in **`<instance>/strata/whiteouts.txt`**: one mount-relative path per line, case preserved, written
  atomically. No marker files inside `overwrite/` (MO2 and Fluorine would list them). A whiteout exists only
  while the path is absent from `overwrite/`: creating the path again removes it, so "delete, then recreate"
  leaves just the new file in overwrite. MO2 and Fluorine ignore the list (the file reappears for them; nothing
  is lost). The window lists the entries on Overwrite with a restore action.
- **Crash safety**: a journal before any copy-up promotion (Fluorine `vfs/stagingpromotion.*`); a stale mount from
  a crash is detected and cleaned on the next start (`fusermount3 -u`, lazy unmount fallback).
- **Profile files in the prefix are overlaid, not copied.** vfsd puts small extra mounts over
  `<prefix>/drive_c/users/steamuser/AppData/Local/Skyrim Special Edition/` (`plugins.txt`, `loadorder.txt`) and
  `.../Documents/My Games/Skyrim Special Edition/` (the profile's local inis; `Saves/` when the profile has local
  saves). Bottom layer = the prefix's real folder, top layer and write target = the profile folder, limited to
  the files the `Game` trait maps (so `modlist.txt` never shows up there). The game writes straight to the
  profile: no copy-back to lose in a crash, no symlinks or bind mounts, and the prefix's own files stay as they
  were. Everything else the game writes in those folders (SKSE logs, ...) lands in the prefix as usual.
  Checked in M1 (oracle check 7): the SLR container sees these mounts, so no copying is needed.

## Performance (1,000+ mods)
- Build the merged index once from a scan of all enabled layers, in parallel (rayon), with a persistent scan
  cache keyed by (path, mtime, size) so a restart doesn't rescan unchanged mods (Fluorine `vfs/scancache.*`).
- Lookups O(1) on the case-folded key; inode table stable for the mount's lifetime.
- Large reads: FUSE **passthrough** (kernel 6.9+) lets the kernel read backing files directly. It needs
  CAP_SYS_ADMIN; Fluorine ships a tiny setuid-free helper with that capability (`docker/fluorine-fuse-helper.c`,
  `vfs/fusepassthrough.h`). Plan it for after M1; measure first.
- Kernel caching: entry/attr timeouts and `keep_cache` on for files that can't change during the run.
- Target: a 1,300-mod list mounts in seconds with a warm cache; a game load is no slower than under Fluorine.

## Crate
`strata-vfs`, on the `fuser` crate. The mount runs in its own process (`strata vfsd`) so a UI crash never takes
the game's filesystem down mid-run, and the UI talks to it over a Unix socket (status, refresh, unmount).

## Lessons from Fluorine (read its code before designing)
`src/src/vfs/`: `mo2filesystem.*` (the FUSE ops, casefold ioctl, caches), `vfstree.*`/`vfsindex.*`/`vfscatalog.*`
(index), `runtimeindex.*` (immutable base + small mutable overlay + negative cache), `overwritemanager.*`,
`stagingpromotion.*`, `rootlocatordeployment.*` (Fluorine reflinks Root files into Game Root instead of
overlaying), `archiveindex.*`, `gamesavemigration.*`, `inodetable.*`. `fuseconnector.cpp` (mount/unmount, stale
mounts). Take the ideas, not the code. Starfield needed phantom directories for missing path components: note it
for later games.
