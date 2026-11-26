# strata-diag

The problems list (MO2's Problems dialog): each check is the counterpart of an MO2 diagnose plugin and takes
plain inputs (a plugin list, a folder, log text, the mods' sources), so the caller decides where they come
from (`strata_ui::modlist::problems::collect` gathers them for the CLI and the window).

**Must not:** read the overlay or the prefix itself.

One module (`lib.rs`): `Problem`, `BasicSettings` (the `diagnose_basic` switches), `missing_masters`, `form43`,
`script_extender` (the SKSE log), `overwrite`, `nitpick`, `profile_tweaks`, `invalid_font_config`,
`alternate_game_sources`, `executables_in_root`.

Tests: `cargo test -p strata-diag` (`src/tests.rs`; one reads the SKSE log of the last `tools/e2e.sh` run).
