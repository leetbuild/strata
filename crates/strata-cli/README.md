# strata-cli

The program's entry point. `run(args, shell)` parses the command line (clap) and dispatches: a subcommand runs
here; no subcommand opens the window by calling `shell` (the C++ shell's entry, handed in by
`shell/src/main.cpp` through `strata-ffi`). `vfsd` is the same binary started again (`/proc/self/exe vfsd`).

`strata-headless` (`main.rs`) is a cargo-only binary of the same library without a shell, so `cargo build` and
`cargo test` need no Qt or CMake. It is a development target and is not installed.

**Must not:** link Qt.

| Module | Commands |
|---|---|
| `lib` | the `Command` enum (one block per area), global options, `Action` |
| `commands` | dispatch; `info`, `list`, `tree`; shared `open`, `print_json`, `build_tree` |
| `deploy` | `deploy`, `undeploy`, `status`, `resolve`, hidden `vfsd` |
| `run` | `run` (mount, launch through Proton, wait, unmount) |
| `oracle` | hidden `oracle roundtrip` (check 1) |
| `cmd_modlist` | `enable`, `disable`, `move`, `mod`, `separator`, `profile`, `backup`, `overwrite`, `category`, `executable`, `link`, `register-links` |
| `cmd_builtins` | `ini`, `origins`, `ls`, `vfs-export`, `analyze`, `refresh`, `separator-colors`, `mohidden`, `changelog` |
| `cmd_plugins` | `plugins`, `sort`, `problems`, `list plugins`, `list saves` |
| `cmd_conflicts` | `conflicts`, `archive`, `pack-bsa`, `extract-bsa`, hidden `preview`, `oracle conflicts` |
| `cmd_install` | `install`, `downloads` |
| `cmd_nexus` | `nexus`, `nxm` |
| `cmd_prefix` | `prefix`, `proton`, `create-portable` |

Tests: `cargo test -p strata-cli` (unit tests; `tests/automation.rs` runs `strata-headless --json` per MO2 ApI
Bridge route on a built instance; `tests/nexus_e2e.rs` against the fake Nexus, needs `7z`).
