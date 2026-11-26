# Credits

Strata is Copyright (C) 2026 leet and licensed under the GNU General Public License, version 3 only
(`LICENSE`). It stands on these projects:

- **[Mod Organizer 2](https://github.com/ModOrganizer2/modorganizer)** (GPL-3.0), by Tannin and the MO2
  team. Strata's window is built from MO2 2.5.2's Qt Designer forms (`shell/forms/`), uses its icons
  (`shell/resources/`) and reproduces its behaviour, file formats and many of its texts. The installers,
  game support, previews and checks follow MO2's plugins (`modorganizer-installer_*`, `game_bethesda`,
  `preview_*`, `diagnose_basic`, `bsa_extractor`, `bsapacker`, ...).
- **MO2 Leet Edition**: the MO2 fork whose additions Strata ports (instances root, header auto-save, Root
  Builder folders and sorter, Sub-Separators, auto-enable, trash deletes, `modl://` links).
- **[Fluorine Manager](https://github.com/SulfurNitride/Fluorine-Manager)** (GPL-3.0), by SulfurNitride: the
  MO2 port to Linux whose Proton and prefix flow and FUSE lessons Strata follows; Strata reads its
  `[fluorine]` instance keys.
- **Icons** in `shell/resources/`, as MO2 ships them: the [Tango Icon Theme](http://tango.freedesktop.org)
  (public domain), the RRZE Icon Set (CC BY-SA 3.0), icons by Lorc, Delapouite and sbed from
  [game-icons.net](https://game-icons.net) (CC BY 3.0), and the checkbox icons by [Icons8](https://icons8.com).
  Strata's own icon (`data/icons/`) is part of Strata (GPL-3.0).
- **Stylesheets** bundled in `shell/stylesheets/`: *vs15 Dark* and its colours by chintsu_kun, holt59 and
  the MO2 team; *Skyrim* by chintsu_kun; *Night Eyes* by Ciathyza; *Parchment* by Bob; *dark* and
  *dracula* from MO2's bundled set.
- **Built-in versions of open-source MO2 plugins** (their behaviour, written again in Rust and C++):
  - **Crash Log Tools** (Crash Log Labeler / Viewer) by Parapets, updated by Miss Corruption
    ([Exit-9B/ModOrganizer-CrashLogTools](https://github.com/Exit-9B/ModOrganizer-CrashLogTools), MIT,
    Copyright (c) 2022 Parapets). The names database it labels with (meh321's Address Library database) is
    downloaded when asked for, not shipped.
  - **Install to Separator** by gabriel-andreescu
    ([modorganizer-install_to_separator](https://github.com/gabriel-andreescu/modorganizer-install_to_separator),
    GPL-3.0).
- **Compatible with** Kezyma's Root Builder (a mod's `Root/` folder holds files for the game's own folder)
  and Sub-Separator (nested separators, `Sub-Separator\nest` in `meta.ini`): Strata reads and writes the
  same layout and keys with its own implementation.
- **[LOOT](https://loot.github.io)** through the `libloot` crate (GPL-3.0), for plugin sorting.
- **[7-Zip](https://www.7-zip.org)**, called as a program, for archives.
- Rust crates listed in `Cargo.lock` (MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0 and similar permissive
  licences, all compatible with the GPL), and **Qt 6** (LGPL-3.0).
- Test fixtures: `crates/strata-install/tests/fixtures/` holds the FOMOD `ModuleConfig.xml` of powerofthree's
  Papyrus Extender, used to test the FOMOD parser.
