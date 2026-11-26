# strata-launch

Running Windows programs under Proton: Steam detection and start, Proton discovery, the Steam Linux Runtime the
Proton asks for, the launch command, waiting for the prefix to go quiet, and the prefix setup.

**Must not:** run anything but through `ProtonCall` / the `Runner` trait.

| Module | What it holds |
|---|---|
| `steam` | finding Steam, its libraries, the Protons (`compatibilitytools.d` too) and their runtime; starting Steam once |
| `plan` | the launch command and its environment (direct SLR, `Runner`) |
| `wait` | waiting until the programs a run started in the prefix are gone |
| `setup` | `strata prefix setup`: steps that check before they act, pinned sha256 downloads |

Tests: `cargo test -p strata-launch` (no Proton is started; plans and steps are checked as data). The real
launch is oracle check 7 (`tools/e2e.sh`, opens a game window: ask first).
