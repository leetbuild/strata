# 04 — Proton, prefix, launching

Fluorine's best part: **it's streamlined.** Pick a Proton, get a working prefix, press Run.
Strata keeps that flow and drops Fluorine's mistakes.

## What Fluorine (0.4.1) does, and where it goes wrong
- The game folder can be a downgraded copy outside Steam (a "Game Root"); the prefix lives next to it; ENB
  setups want GE-Proton.
- Steam must be running for SteamStub DRM; Fluorine starts it (`[fluorine] steam_drm=true`).
- Proton is wrapped in a Steam Linux Runtime container. Fluorine always picks **SLR 4**, but GE-Proton10 is
  built for **sniper (SLR 3)**: inside SLR 4 `libvpx.so.6` is missing, GStreamer's libav fails, xWMA voices are
  silent.
- Fluorine installs its own FAudio into the prefix, which spams errors; Proton's builtin xaudio2 works.
- Fluorine writes `dxvk.conf` with `dxvk.enableGraphicsPipelineLibrary = False` for ENB.
- A global `~/.config/fluorine/config.json` `prefix_path` silently overrides the instance's prefix.

## Strata's rules
1. **Per-instance settings only.** Prefix, Proton and runtime live in the instance (`ModOrganizer.ini`, reading
   Fluorine's `[fluorine]` keys when present so existing instances just work). No global override.
2. **The runtime matches the Proton.** Read the Proton's `toolmanifest.vdf` (`require_tool_appid`) and use that
   SLR (sniper for GE 10, SLR 4 only for a Proton that asks for it), found through Steam's `libraryfolders.vdf`.
   A missing runtime is an error naming the app to install, never a fallback to another runtime. Strata builds
   the launch command itself (`<SLR>/_v2-entry-point --verb=waitforexitandrun -- <proton>/proton
   waitforexitandrun <exe>`); **umu-launcher is not used**: Proton skips its `steam.exe` bridge when `UMU_ID` is
   set, which SteamStub DRM needs. The backend is a small `Runner` trait so umu can
   be added later for games without Steam DRM.
3. **No custom audio stack.** The prefix uses Proton's builtin xaudio2/FAudio. Never install DLL overrides the
   user didn't ask for.
4. **Prefix setup is explicit and idempotent.** A setup step list (create prefix with the chosen Proton, then
   dependencies: vcrun 2015-2022, d3dcompiler_47, d3dx9/10/11 from DirectX Jun 2010, .NET runtimes for tools,
   corefonts if needed). Each step checks before it acts, so rerunning never breaks a working prefix (Fluorine's
   "Set up now" did). Verify downloads by sha256 (Fluorine's `prefixsetuprunner.cpp` has the URLs and hashes;
   re-check them). Log every step.
5. **Steam**: detect Steam (native and Flatpak), start it when the game needs SteamStub DRM, set
   `SteamAppId`/`SteamGameId`/`STEAM_COMPAT_*` the way Fluorine's `protonlauncher.cpp` does (read it: it records
   several hard lessons, e.g. `STEAM_COMPAT_DATA_PATH` must be the compatdata folder, not `pfx`).
6. **ENB**: write `dxvk.conf` next to the game exe (in the overlay's Root layer, not the real folder) when ENB is
   detected.
7. **Tools** (xEdit, BodySlide, Pandora, DynDOLOD, LOOT GUI, CK): run in the same prefix through the overlay,
   like the game. MO2's "run from the mod folder" tools must keep working (Leet: never Root-sort them).
8. **Logs**: every launch writes the exact command line and environment to `<instance>/logs/`.

## Where the game's files land (for tests and diagnostics)
`<prefix>/drive_c/users/steamuser/Documents/My Games/Skyrim Special Edition/` (inis, `SKSE/skse64.log`, saves),
`.../AppData/Local/Skyrim Special Edition/` (`plugins.txt`, `loadorder.txt`).

## Crate
`strata-launch`: Steam detection, Proton discovery (`compatibilitytools.d`, Steam libraries), runtime choice,
prefix setup steps, process launch and wait (the game's process tree inside the container), cleanup.
