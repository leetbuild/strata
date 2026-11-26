# strata-nexus

Nexus Mods: link parsing, the API client, single sign-on, the API key store, the download queue.

**Must not:** refresh or store anyone's OAuth token; print a key.

| Module | What it holds |
|---|---|
| `links` | `nxm://` and `modl://` links as pure functions (uibase `NXMUrl`, Leet's ModPub / DwemerMods rules), the `.desktop` entry |
| `api` | the v1 API as MO2 calls it, behind a `Transport` (tests answer from `tests/fixtures/`) |
| `sso` | single sign-on as MO2 2.5.2's `NexusSSOLogin` speaks it |
| `keystore` | the API key in the freedesktop secret service |
| `nxm` | an nxm link → a queued download (file info, mod info, download links) |
| `queue` | downloads one at a time, pause / resume over HTTP ranges, MO2's `.meta` |
| `query` | "query info" by MD5 |
| `fake` | a local stand-in for the API and download servers, for tests (also the CLI's end-to-end test) |

Tests: `cargo test -p strata-nexus` (fixtures and the fake server; the secret service test is `#[ignore]`d:
`cargo test -p strata-nexus -- --ignored` on a desktop with a keyring).
