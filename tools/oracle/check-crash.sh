#!/usr/bin/env bash
# Oracle check 6, the crash part: after `kill -9` of the overlay daemon, the next deploy
# cleans the dead mount up and works, and the real game folder is as it was.
#   tools/oracle/check-crash.sh
# Works on a generated instance in _stage/. Exits non-zero on a failure.
set -euo pipefail
here="$(cd "$(dirname "$0")/../.." && pwd)"
strata="${STRATA:-$here/target/release/strata-headless}"
[ -x "$strata" ] || { echo "build first: cargo build --release -p strata-cli"; exit 2; }
[ -e /dev/fuse ] || { echo "SKIPPED: no /dev/fuse"; exit 0; }

inst="$here/_stage/oracle/crash"
game="$inst/Game Root"
mountpoint -q "$game" 2>/dev/null && fusermount3 -u -z "$game"
rm -rf "$inst"
"$here/tools/gen-instance" "$inst" > /dev/null
hash_game() { (cd "$game" && find . -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1); }
before="$(hash_game)"

fail() { echo "check 6 (crash) FAILED: $*"; "$strata" undeploy --force -i "$inst" > /dev/null 2>&1 || true; exit 1; }

"$strata" deploy -i "$inst" > /dev/null
pid="$("$strata" status -i "$inst" --json | sed -n 's/.*"pid": *\([0-9]*\).*/\1/p')"
[ -n "$pid" ] || fail "no daemon pid"
echo crash > "$game/Data/written-before-crash.txt"
kill -9 "$pid"
sleep 0.3
ls "$game" > /dev/null 2>&1 && fail "the mount still answers after kill -9"
grep -q "fuse.strata" /proc/self/mountinfo || fail "expected a dead mount to be left"

"$strata" deploy -i "$inst" > /dev/null || fail "deploy after the crash"
[ "$(cat "$game/Data/written-before-crash.txt")" = crash ] || fail "the file written before the crash is gone"
[ -f "$game/skse64_loader.exe" ] || fail "Root files missing after redeploy"
"$strata" undeploy -i "$inst" > /dev/null || fail "undeploy"
grep -q "fuse.strata.*crash" /proc/self/mountinfo && fail "a mount is left"
[ "$(hash_game)" = "$before" ] || fail "the real game folder changed"
[ -f "$inst/overwrite/written-before-crash.txt" ] || fail "the write did not land in overwrite"
echo "check 6 (crash) ok: dead mount cleaned, redeploy works, game folder unchanged"
