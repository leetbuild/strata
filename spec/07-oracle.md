# 07 — The oracle: how the machine proves parity

The machine proves parity, not a person clicking around:
**MO2's behaviour on real instances is the reference answer**, captured from Fluorine (MO2 2.5.x on Linux) and
from small reference implementations written straight from the MO2 source.

All oracle runs use **copies** of instances (`_stage/oracle/<name>/`, made with `cp --reflink=auto -a`, nearly
free on btrfs or XFS) and never write to a real instance, Game Root or prefix.

**Prefixes may be shared** by several instances. Tests never launch anything in a real one: they reflink-copy it
to `_stage/oracle/prefix/` and run with `--prefix <copy>` (a one-run flag, logged with the launch; not a stored
or global setting).

## Corpora
| Instance | Use |
|---|---|
| Synthetic (`tools/gen-instance`) | every edge case on purpose; runs everywhere, in `tools/test.sh` and CI |
| A small real instance (`STRATA_REAL_INSTANCES`) | real-world files read only; the launch corpus for check 7 on a copied prefix |
| A large real list (1,000+ mods), as a reflink copy | the scale corpus: round trip, index, mount, merged-tree parity, perf. Never launched |

## Checks (each one a test command that exits non-zero on mismatch)
1. **Format round trip**: parse + write every `ModOrganizer.ini`, `modlist.txt`, `meta.ini`, `plugins.txt`,
   `loadorder.txt`, `*.meta` of the corpora: byte-identical.
2. **Merged tree parity**: Strata's merged tree (path → winning layer) vs
   a. a ~150-line Python reference (`tools/oracle/merge.py`) written from MO2's `DirectoryEntry` rules, and
   b. Fluorine's real view: mount Fluorine's VFS by launching a harmless executable through it (e.g. a script
      that sleeps; Fluorine logs the mount point), walk the mount with `find`, unmount. Store as
      `_stage/oracle/<name>/fluorine-tree.tsv`. Differences must be explained in `DECISIONS.md` (e.g. Root
      deploy vs Root overlay) or fixed.
3. **Conflicts parity**: winners/losers per file and the per-mod flags (overwrites, overwritten, redundant)
   from the reference vs Strata.
4. **Plugin list parity**: active plugins, order, masters, missing masters, light flags vs MO2's rules; LOOT
   sort result equals lootcli/libloot's on the same input.
5. **Install golden tests**: a fixture set of archives (simple, wrapped, Data+Root (SKSE, ENB), BAIN, FOMOD with
   conditions, bundle) with expected trees. Leet's `rootsorter.cpp` cases from its notes are the first
   fixtures. Build archives in the test, don't commit big binaries.
6. **Mount behaviour**: a FUSE test suite run against a mounted synthetic instance: case-insensitive open/stat,
   readdir merge, copy-on-write to overwrite, create/rename/delete, whiteouts, unicode, the casefold ioctl,
   concurrent readers, stale-mount recovery after `kill -9` of vfsd.
7. **End to end in the game** (needs Steam running and the GPU free):
   `strata run SKSE` on an oracle copy, then wait for the probe. The **probe** is a tiny SKSE plugin
   (`tools/probe/`, built with clang-cl + xwin) that on
   `kDataLoaded` writes `strata-probe.json` (loaded plugin count, SKSE plugin list, game version) into the
   overlay and quits the game. Pass = the file lands in the oracle copy's `overwrite/` and matches the expected
   plugin list. This proves overlay + Root + Proton + SKSE without a human.
8. **Performance**: index time cold/warm, mount time, `find` over the mount, a game load to `kDataLoaded`
   (from the probe's timestamp) vs Fluorine on the same instance. 

## What the machine can't judge
The look next to MO2, the feel of the lists (drag-and-drop, filters), a real play session.
