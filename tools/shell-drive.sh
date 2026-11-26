#!/usr/bin/env bash
# The window driven offscreen by build/strata-shell-test (shell/tests/): clicks, keys,
# drops, context menus and dialogs on generated instances, checking the files they write.
#   tools/shell-drive.sh               every test
#   tools/shell-drive.sh <function>    one test function (Qt Test name), e.g. aCheckboxClickEnablesAndDisablesAMod
# Output in _stage/shell-drive/drive.log; exits non-zero when a test fails.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -x build/strata-shell-test ] || { echo "build/strata-shell-test not built (tools/build.sh)"; exit 1; }
mkdir -p _stage/shell-drive
log=_stage/shell-drive/drive.log
export STRATA_REPO="$PWD" QT_QPA_PLATFORM=offscreen STRATA_TEST_ARGS="${*:-}"
status=0
timeout 600 build/strata-shell-test > "$log" 2>&1 || status=$?
grep -E '^(FAIL!|XPASS|Totals:)' "$log" || true
if [ "$status" -ne 0 ]; then
  echo "FAIL: strata-shell-test exit $status (see $log)"
  exit 1
fi
echo "shell drive tests passed ($log)"
