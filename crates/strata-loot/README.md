# strata-loot

LOOT sorting through the `libloot` crate, as MO2's Sort button does through lootcli; the masterlist download.

**Must not:** mount anything itself. The caller gives it a game folder: `strata_ui::loot_view` passes the profile
through Strata's case-insensitive overlay (the live deploy, or a private mount for the sort), or, without FUSE, the
mods as additional data folders.

`lib.rs`: `sort` (the new order), `sort_with_report` (the order and lootcli's report, steps and libloot's log
lines to an `Observer`, cancellable between steps), `data_dir` (`$XDG_DATA_HOME/strata/loot/<game>`), masterlist
and prelude updates. `report.rs`: the report (`LOOTWorker::createJsonReport`'s JSON names).

Tests: `cargo test -p strata-loot`; `strata sort --dry-run` on an instance.
