# strata-vfs

The overlay: layer list → merged tree, the FUSE filesystem (`fuser`), copy-on-write into overwrite, whiteouts,
the journal, the vfsd daemon and its socket protocol, and the conflict index.

**Must not:** know about profiles: it gets a resolved layer list.

| Module | What it holds |
|---|---|
| `tree` | `Tree`: every layer's files as one folder, case-insensitive; writes, renames, deletions (whiteouts) |
| `tree/journal` | the overlay's journal for multi-step changes on disk |
| `tree/rebase` | `refresh`: a new tree that keeps the old node ids (inodes) |
| `fs` | the FUSE filesystem over a `Tree` |
| `mount` | mounting in place over a folder, stale mounts from a crash cleaned first |
| `daemon` | `strata vfsd`: holds an instance's mounts; JSON lines over a Unix socket (`docs/ARCHITECTURE.md`) |
| `conflicts` | `ConflictIndex`: every file's providers (loose and archived), MO2's per-mod conflict flags, incremental |

Tests: `cargo test -p strata-vfs`. `tests/mount.rs`, `tests/model.rs`, `tests/crash.rs` mount FUSE in a temp
folder (they print `SKIPPED` without `/dev/fuse` or `fusermount3`); the integration tests are built with the
`crash-hooks` feature, which lets `tests/crash.rs` stop vfsd at a chosen step. `tests/tree.rs` needs no FUSE.
Oracle: check 2a (`check-tree.sh`), check 3 (conflicts), check 6 (`check-crash.sh`).
