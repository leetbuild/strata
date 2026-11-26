# 01 — Scope

Strata = **MO2 2.5.2 behaviour** + **MO2 Leet Edition features** + **Fluorine's Proton/prefix flow**, Linux only.

## References (read the source when in doubt; never guess MO2 behaviour)
| What | Where |
|---|---|
| MO2 2.5.2 (C++) | [ModOrganizer2/modorganizer](https://github.com/ModOrganizer2/modorganizer), tag `v2.5.2`, and its plugin repositories |
| MO2 Leet Edition | the MO2 fork whose 12 change sets ("Leet change N") Strata ports |
| Fluorine Manager (MO2 Linux port, every MO2 plugin ported) | [SulfurNitride/Fluorine-Manager](https://github.com/SulfurNitride/Fluorine-Manager): `libs/` = uibase, game plugins, installers, previews, tools; `src/src/vfs/` + `fuseconnector.*` = FUSE; `protonlauncher.*`, `wineprefix.*`, `prefixsetuprunner.*`, `slrmanager.*`, `steamdetection.*` = Proton |
| Upstream MO2 (GitHub, ModOrganizer2 org) | for anything the local repos lack (e.g. `modorganizer-game_bethesda`, `modorganizer-installer_fomod`) |

## In
### MO2 2.5.2 core
- Instances: portable (instance = folder) and global (named, under one root; Leet's `instances_root` rule).
- Profiles: mod list and order, plugin list and order, local saves, local game inis, profile archive invalidation.
- Mod list: enable/disable, priority by drag, separators (with colours), categories, filters, groups, notes,
  flags (conflicts, redundant, not endorsed, update available, no valid game data, ...), content icons, the
  Overwrite pseudo-mod, foreign/unmanaged mods (DLC, game files), backups of the mod list.
- Plugin list: order, enable, lock, masters and missing-master flags, light/ESL flags, LOOT sort, groups.
- Archives (BSA/BA2), Data tab (merged tree with origin), Saves tab (save info: name, level, location, missing
  plugins), Downloads tab.
- Conflicts: per-file winners and losers, mod info dialog's Conflicts tabs (general and advanced).
- Mod info dialog: text files, ini files, images, optional esps, conflicts, categories, Nexus info, notes, filetree.
- Installing: archives (7z/zip/rar), installers (quick, manual, BAIN incl. wizard, FOMOD XML, bundle), replace /
  merge / rename on name clash.
- Downloads: Nexus API (key or SSO), nxm:// handler, download queue, pause/resume, query info, hide.
- Executables list: title, binary, arguments, working dir, steam app id, hide, force libraries, ... ; run.
- Settings dialog (General, Theme, Nexus, Plugins, Workarounds, Diagnostics, Paths, Steam) as far as it applies
  on Linux.
- Problems/diagnostics (the built-in checks, `spec/05-builtins.md`).
- Translations: out (English only).

### Leet Edition (all 12 change sets)
instances root (1), header auto-save (2), Root Builder (3-6: Root folders, sorter, Root tab + Root conflicts,
deploy), mod mappings (OBRE, 7), Senjay picks (8: theme defaults, toolbar, delete permanently vs trash, `modl://`
links), Sub-Separator (9: nested separators, same meta.ini keys), clean restart (10), auto-enable installed mods
(11), and the lessons of the optimization review (12). The numbers are that file's; "Leet change N" in the spec
always means them. Plus the cherry-picked upstream fixes.

### Fluorine pieces worth keeping
Prefix creation and dependency setup, Proton selection, Steam DRM handling, SLR wrapping, dxvk.conf for ENB,
the Library-free instance creation from the CLI (`create-portable`), and its hard-won FUSE lessons
(`spec/03-vfs.md`). Everything else of Fluorine's UI is out.

## Out
- Windows, macOS, Wine for the manager, usvfs.
- The Python plugin API. MO2's own Python plugins and the useful third-party ones are **built in** instead. A plugin nobody uses is dropped, not ported.
- MO2's plugin DLL interface (uibase `IPlugin*`): Strata has internal Rust traits, not a public plugin ABI.
- Wabbajack/CLF3 install queues, OpenMW, Fluorine's Library page, Steam cloud sync.
- Translations.

## Games, in order
1. **Skyrim Special Edition 1.5.97** (a downgraded "Game Root" copy outside Steam). Everything is proven here first.
2. Skyrim LE (archive invalidation, `plugins.txt` without `*`, SKSE 1.7).
3. Classic Oblivion 1.2.0.416 (xOBSE, BSA invalidation, `Oblivion.ini` in My Games).
4. Fallout 4 / Fallout NV / Oblivion Remastered (mod mappings) on demand.

A game is a Rust implementation of the `Game` trait (`docs/ARCHITECTURE.md`), the counterpart of MO2's
`game_bethesda` plugins and their game features (script extender, data archives, local saves, save game info,
plugin list format, BSA invalidation, mod data checker/content).
