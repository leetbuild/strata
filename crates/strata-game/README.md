# strata-game

The `Game` trait (MO2's `IPluginGame` and its game features) with one implementation per game, and the layer
model the core hands to the overlay. Plugin headers and save games are read here.

**Must not:** depend on the core's internals.

| Module | What it holds |
|---|---|
| `lib` | `Game` (paths, ini files, primary plugins, plugin list format, script extender, executables, mod mappings), `by_name` |
| `skyrimse`, `skyrimle`, `oblivion` | the games, each from its MO2 `game_*` plugin |
| `layer` | `Layer`, `LayerKind`, `Mapping`: the layer model (`docs/ARCHITECTURE.md`) |
| `plugin` | the TES4 header of `.esm/.esp/.esl` (flags, form version, masters); `build_plugin` for tests |
| `save` | `.ess` save headers (screenshot, plugin lists; LZ4 / zlib) |
| `invalidation` | archive invalidation for the older games (`GamebryoBSAInvalidation`, the dummy BSA) |

Tests: `cargo test -p strata-game` (some read real plugins and saves with `STRATA_REAL_GAME` / `STRATA_REAL_INSTANCES`, read only).
