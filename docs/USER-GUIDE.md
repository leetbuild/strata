# Strata user guide

Strata behaves like Mod Organizer 2. This guide covers what is different on Linux: instances, Proton, the
overlay and the command line. For the mod list, plugins, installers and dialogs, MO2's own documentation
applies.

## Before you start
- **Steam** with the game installed, and a **Proton**. GE-Proton (from ProtonUp-Qt or the
  `compatibilitytools.d` folder) works well. Strata lists every Proton Steam knows (`strata proton list`).
- The **Steam Linux Runtime** your Proton asks for. GE-Proton and Proton 8+ want *sniper*:
  `steam steam://install/1628350`. Strata never swaps in another runtime, because a wrong runtime means
  missing libraries (no voices, for example).
- `fuse3` and `7zip` installed (the Arch package pulls them in).
- **Skyrim SE 1.5.97 users**: keep a copy of the downgraded game folder apart from Steam's (a "Game Root"),
  so a Steam update can't touch it, and point the instance at that copy.

## Instances
An instance is one folder: `ModOrganizer.ini`, `mods/`, `profiles/`, `downloads/`, `overwrite/`, and, for
instances Strata creates, the prefix (`Prefix/`). The **instance manager** (File > Change Game, and at the
first start) lists:
- every instance folder under the **instances folder** (default `~/Games`; the button at the bottom of the
  dialog changes it), and
- every instance you opened from somewhere else.

From there: switch (Strata restarts on the other instance), rename (paths inside the folder in
`ModOrganizer.ini` are fixed up), delete (to the trash or for good; a prefix outside the instance folder is
never deleted), open the folder or `ModOrganizer.ini`.

### A new instance
**Create new instance** in the instance manager:
1. **Game**: Strata finds the supported games in your Steam libraries. Point it at another folder if you keep
   a Game Root copy.
2. **Name and location**: by default `~/Games/<name>`.
3. **Proton and prefix**: a new prefix inside the instance folder (recommended), or an existing prefix shared
   with other instances. Profile-local inis and saves are on, as in MO2's defaults.
4. **Review**, then **Create**. Leave *Set up the prefix now* ticked: Proton creates the prefix, then VC++
   2015-2022, d3dcompiler_47, d3dx9/10/11 and .NET Desktop are installed. This downloads a few hundred MB and
   takes some minutes. Each step checks first, so `strata prefix setup -i <instance>` can be run again any
   time.

The same from a terminal:
```
strata create-portable ~/Games/MySkyrim --game "/path/to/Skyrim Special Edition" \
       --proton ~/.local/share/Steam/compatibilitytools.d/GE-Proton10-14
strata prefix setup -i ~/Games/MySkyrim
```

### An instance from MO2 on Windows, or from Fluorine
Copy the instance folder (portable instances are the easiest; a global one lives in
`%LOCALAPPDATA%\ModOrganizer\<name>`). Then open it: `strata -i <folder>`, or set its parent folder as the
instances folder in the instance manager.

Strata reads Windows paths (`C:\...`, `Z:\...`) and keeps them as they are when it writes. Two things need
setting on Linux, in `ModOrganizer.ini` (instance manager > *Open INI*):
```
[General]
gamePath=@ByteArray(/home/you/Games/Skyrim Special Edition/Game Root)

[fluorine]
prefix_path=/home/you/Games/MySkyrim/Prefix
proton_path=/home/you/.local/share/Steam/compatibilitytools.d/GE-Proton10-14
```
`prefix_path` is the folder that holds (or will hold) `pfx/`. Fluorine instances already have these keys.
Then run `strata prefix setup -i <folder>` once. Executables whose paths point at Windows drives need their
paths changed in *Modify Executables*.

## Running the game
Pick an executable (SKSE, the game, xEdit, BodySlide, ...) and press **Run**. Strata mounts the profile's
mods over the game folder, starts the program through Proton in the instance's prefix, waits for it and for
anything it started, then unmounts. Files the game or a tool writes go to **Overwrite**. Root files (a mod's
`Root/` folder: SKSE's loader, ENB, DLL loaders) appear next to the game's exe while it runs.

- **ENB**: when the merged game folder holds ENB and no `dxvk.conf`, Strata writes the one ENB needs under
  DXVK into Overwrite's Root.
- **Environment variables** per instance: Settings > Leet > *Environment variables* (for example
  `PROTON_NO_NTSYNC=1`, `DXVK_HUD=fps`). They are set last, so they win over Strata's own.
- **Tools outside Strata**: `strata deploy -i <instance>` keeps the mount up until `strata undeploy`, so any
  program sees the merged folder.

## Nexus
Settings > Nexus > *Connect to Nexus* logs in through the browser; the key is kept in your desktop's keyring
(`libsecret`). **Associate** makes Strata the handler of *Download with Manager* links (`nxm://`). A link
clicked while Strata runs goes to the open window; otherwise Strata starts on the last instance.

## Where things are
| What | Where |
|---|---|
| Strata's own settings (last instance, instances folder) | `~/.config/strata/strata.ini` |
| Your stylesheets | `~/.local/share/strata/stylesheets/` (or `<instance>/stylesheets/`) |
| The game's inis and saves (profile-local) | `<instance>/profiles/<profile>/` |
| SKSE logs, crash logs | in the prefix: `pfx/drive_c/users/steamuser/Documents/My Games/<game>/SKSE/` |
| Strata's journal and caches for an instance | `<instance>/strata/` (MO2 and Fluorine ignore it) |

## The command line
`strata --help` lists every command; `strata <command> --help` explains one. Most take `-i <instance>`
(default: the current folder) and `-p <profile>`, and print JSON with `--json`. A few useful ones:
```
strata list mods                  # the mod list by priority
strata conflicts "Some Mod"       # who wins each of a mod's files
strata origins meshes/x.nif       # every mod that provides a file
strata install archive.7z         # install an archive (installer questions as options)
strata sort                       # sort the plugins with LOOT
strata problems                   # MO2's Problems checks
strata clone-instance ...         # a cheap test copy of an instance (reflinks)
```

## When something goes wrong
- The log at the bottom of the window shows what Strata did; `strata info -i <instance>` shows its view of
  the instance, prefix and Proton.
- A mount left behind after a crash: `strata undeploy -i <instance> --force`.
- Please report bugs with that output and the steps that led there.
