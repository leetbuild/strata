#!/usr/bin/env bash
# The drive tests under load: two busy loops per CPU while tools/shell-drive.sh runs N times
# (default 30). Prints the counts; each failing run's log is kept as _stage/shell-drive-stress/fail-<n>.log.
#   tools/shell-drive-stress.sh [runs] [test function...]
set -uo pipefail
cd "$(dirname "$0")/.."
n=${1:-30}
shift || true
out=_stage/shell-drive-stress
rm -rf "$out"
mkdir -p "$out"
pids=()
for _ in $(seq $((2 * $(nproc)))); do (while :; do :; done) & pids+=($!); done
trap 'kill "${pids[@]}" 2>/dev/null' EXIT
pass=0
fail=0
for i in $(seq "$n"); do
  if tools/shell-drive.sh "$@" > /dev/null 2>&1; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    cp _stage/shell-drive/drive.log "$out/fail-$i.log"
  fi
done
echo "runs $n pass $pass fail $fail (load: $((2 * $(nproc))) busy loops)"
[ "$fail" -eq 0 ]
