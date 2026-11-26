# strata-ffi

Everything that crosses the Rust/C++ boundary, and nothing else. CMake links this crate (a static library,
through Corrosion) into `build/strata` with the C++ shell.

**Must not:** hold state or logic of its own; let a panic unwind into C++.

| File | What it holds |
|---|---|
| `lib.rs` | `strata_run`: the C entry `shell/src/main.cpp` calls; it hands the arguments to `strata_cli::run`. `guarded` / `quiet`: the panic catchers every bridge function goes through |
| `bridge.rs` | the main window (shell area): `Session`, rows, tabs, settings, live deploy |
| `bridge_modlist.rs` | profiles, executables, settings dialogs, delete confirmation (`strata::modlist`) |
| `bridge_builtins.rs` | ini editor, Problems dialog, the built-in plugins' switches (`strata::builtins`) |
| `bridge_conflicts.rs` | mod info dialog, previews (conflicts area) |
| `bridge_install.rs` | installer dialogs, downloads (install area) |

Each file is listed in `corrosion_add_cxxbridge` in the root `CMakeLists.txt`; Corrosion generates
`strata_bridge/<file>.h`. Every function catches panics (a `Result` function returns an error, the others an
empty value). No cargo tests: the logic it converts is tested in `strata-ui`; the shell's tests drive it
(`tools/shell-test.sh`, `tools/shell-drive.sh`).
