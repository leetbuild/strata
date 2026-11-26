# strata-archive

Archives: Bethesda's BSA / BA2 (read and write) and install archives (7z, zip, rar, through the `7z` program).

| Module | What it holds |
|---|---|
| `bsa` | `GameArchive`: BSA v103 / v104 / v105 and BA2 (GNRL, DX10) reading and extraction |
| `pack` | writing BSAs; MO2's `bsapacker` (a mod's loose files into archives) and `bsa_extractor` |
| `sevenzip` | `Archive`: listing and extracting 7z / zip / rar through the installed `7z`, with progress; `create` packs a folder (`7z a`) |

Tests: `cargo test -p strata-archive` (archives built in the test; some read a real game's BSAs with `STRATA_REAL_GAME`; the 7z
tests need the `7z` program).
