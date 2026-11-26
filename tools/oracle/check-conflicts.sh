#!/usr/bin/env bash
# Oracle check 3: Strata's conflicts (every file's providers, every mod's flags) equal the Python
# reference's, written from MO2's rules (tools/oracle/conflicts.py).
#   tools/oracle/check-conflicts.sh <instance folder> [profile]
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

args=(oracle conflicts -i "$instance")
[ -z "$profile" ] || args+=(--profile "$profile")
"$strata" "${args[@]}" | LC_ALL=C sort > "$out/strata-conflicts.tsv"
python3 "$here/tools/oracle/conflicts.py" "$instance" ${profile:+"$profile"} | LC_ALL=C sort > "$out/reference-conflicts.tsv"

if diff "$out/reference-conflicts.tsv" "$out/strata-conflicts.tsv" > "$out/conflicts.diff"; then
  files=$(grep -c '^F' "$out/strata-conflicts.tsv" || true)
  shared=$(grep '^F' "$out/strata-conflicts.tsv" | grep -c '|' || true)
  mods=$(grep -c '^M' "$out/strata-conflicts.tsv" || true)
  flagged=$(awk -F'\t' '$1 == "M" && $3 != "none"' "$out/strata-conflicts.tsv" | wc -l)
  archived=$(grep '^F' "$out/strata-conflicts.tsv" | grep -c '\[' || true)
  aflagged=$(awk -F'\t' '$1 == "M" && ($6 != "none" || $9 != "none")' "$out/strata-conflicts.tsv" | wc -l)
  echo "check 3 ok: $files files ($shared with several providers, $archived in archives), $mods mods ($flagged flagged, $aflagged with archive flags), $instance"
else
  echo "check 3 FAILED: $(grep -c '^[<>]' "$out/conflicts.diff") differing lines, see $out/conflicts.diff"
  head -10 "$out/conflicts.diff"
  exit 1
fi
