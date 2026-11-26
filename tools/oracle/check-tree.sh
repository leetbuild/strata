#!/usr/bin/env bash
# Oracle check 2a: Strata's merged tree equals the Python reference's (tools/oracle/merge.py).
#   tools/oracle/check-tree.sh <instance folder> [profile]
# Reads only. Exits non-zero on any difference; the diff is in _stage/oracle/.
set -euo pipefail
here="$(cd "$(dirname "$0")/../.." && pwd)"
instance="$1"
profile="${2:-}"
strata="${STRATA:-$here/target/release/strata-headless}"
[ -x "$strata" ] || { echo "build first: cargo build --release -p strata-cli"; exit 2; }

name="$(basename "$instance" | tr ' /' '__')"
out="$here/_stage/oracle/$name"
mkdir -p "$out"

args=(tree --origin -i "$instance")
[ -z "$profile" ] || args+=(--profile "$profile")
"$strata" "${args[@]}" | LC_ALL=C sort > "$out/strata-tree.tsv"
python3 "$here/tools/oracle/merge.py" "$instance" ${profile:+"$profile"} | LC_ALL=C sort > "$out/reference-tree.tsv"

if diff "$out/reference-tree.tsv" "$out/strata-tree.tsv" > "$out/tree.diff"; then
  echo "check 2a ok: $(wc -l < "$out/strata-tree.tsv") files, $instance"
else
  echo "check 2a FAILED: $(grep -c '^[<>]' "$out/tree.diff") differing lines, see $out/tree.diff"
  head -10 "$out/tree.diff"
  exit 1
fi
