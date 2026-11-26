#!/usr/bin/env bash
# Every test that needs no game: cargo tests (the FUSE mount tests among them), the oracle
# checks that run on a generated instance, then smoke tests of build/strata when it exists.
# Check 7 (the game itself) is tools/e2e.sh.
# Exits non-zero on the first failure.
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_BUILD_JOBS="${STRATA_JOBS:-8}"
# Real data for the read-only tests that use it (skipped otherwise), per machine and never
# committed: `.strata-test.env` may set STRATA_REAL_INSTANCES (instance folders, `:`-separated)
# and STRATA_REAL_GAME (a Skyrim SE 1.5.97 game folder).
[ -f .strata-test.env ] && { set -a; . ./.strata-test.env; set +a; }

echo "== cargo test (log: _stage/cargo-test.log)"
mkdir -p _stage
# --nocapture so that the tests' "SKIPPED: <why>" lines reach the log; they are counted at the end
if ! cargo test --workspace -- --nocapture > _stage/cargo-test.log 2>&1; then
  grep -E '^test .* FAILED|panicked|^error' _stage/cargo-test.log | head -40
  echo "cargo test failed; see _stage/cargo-test.log"; exit 1
fi
grep '^test result' _stage/cargo-test.log | awk '{p+=$4; i+=$8} END {print p " passed, " i " ignored"}'

# Tests that could not run here (missing real data, programs or opt-in variables) print
# "SKIPPED: <test>: <why>"; none passes silently.
report_skips() {
  local skips
  skips=$(grep -o 'SKIPPED: .*' _stage/cargo-test.log | sort -u || true)
  if [ -n "$skips" ]; then
    echo "== $(printf '%s\n' "$skips" | wc -l) tests skipped for missing data:"
    printf '%s\n' "$skips" | sed 's/^/  /'
  else
    echo "== no tests skipped"
  fi
}

echo "== oracle checks on a generated instance"
cargo build --release --quiet -p strata-cli
rm -rf _stage/test-instance
tools/gen-instance _stage/test-instance --files 3000 > /dev/null
tools/oracle/check-roundtrip.sh _stage/test-instance | tail -1
tools/oracle/check-tree.sh _stage/test-instance
tools/oracle/check-plugins.sh _stage/test-instance
tools/oracle/check-conflicts.sh _stage/test-instance
tools/oracle/check-crash.sh
tools/oracle/check-install.sh

echo "== the mod list property on a generated instance (MO2's reading rules after random operations)"
rm -rf _stage/test-modlist
tools/gen-instance _stage/test-modlist > /dev/null
touch "_stage/test-modlist/Game Root/Data/Unmanaged Extra.esp"  # a foreign mod
tools/oracle/check-modlist.sh _stage/test-modlist 300 1 | tail -1

if [ ! -x build/strata ]; then
  echo "== build/strata not built: skipping the shell smoke tests (run tools/build.sh)"
  report_skips
  exit 0
fi

echo "== build/strata --version"
build/strata --version | grep -q '^strata '

echo "== the window opens (offscreen) with a stylesheet"
mkdir -p _stage
shot=_stage/window.png
rm -f "$shot"
# an empty config: the window must not open the user's last instance
mkdir -p _stage/window-config
XDG_CONFIG_HOME="$PWD/_stage/window-config" QT_QPA_PLATFORM=offscreen STRATA_SHELL_SCREENSHOT="$shot" \
  timeout 30 build/strata 2> _stage/window.log
[ -s "$shot" ] || { echo "no screenshot written; see _stage/window.log"; exit 1; }
if grep -q -i -E 'stylesheet .* not found|QCss|Could not parse' _stage/window.log; then
  echo "the stylesheet did not load; see _stage/window.log"; exit 1
fi
echo "ok ($shot)"

echo "== the window on a generated instance (tools/shell-test.sh)"
tools/shell-test.sh | tail -1

echo "== the window driven by clicks, keys, drops and dialogs (tools/shell-drive.sh)"
tools/shell-drive.sh | tail -1

report_skips
