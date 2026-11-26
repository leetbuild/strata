#!/usr/bin/env bash
# The mod list property (spec/07-oracle.md, check 1's companion): after random enable,
# disable and move operations through Strata, MO2's reading rules (tools/oracle/modlist.py,
# written from Profile::refreshModStatus) give the order and states Strata shows, and MO2's
# own operations (setModPriority, changeModPriority, shiftModsPriority) give the same.
#   tools/oracle/check-modlist.sh <instance> [steps] [seed]
# Never writes to <instance>: it works on a skeleton copy in _stage/oracle/modlist/ (the
# ini, the profiles, and every mod folder with only its meta.ini). The game folder is only
# read (foreign plugins, Skyrim.ccc). Exits non-zero on a mismatch.
set -euo pipefail
here="$(cd "$(dirname "$0")/../.." && pwd)"
strata="${STRATA:-$here/target/release/strata-headless}"
[ -x "$strata" ] || { echo "build first: cargo build --release -p strata-cli"; exit 2; }
src="$(cd "$1" && pwd)"
steps="${2:-200}"
seed="${3:-1}"
name="$(basename "$src")"
copy="$here/_stage/oracle/modlist/$name"
rm -rf "$copy"
mkdir -p "$copy/mods" "$copy/overwrite"
cp --reflink=auto -a "$src/ModOrganizer.ini" "$copy/"
cp --reflink=auto -a "$src/profiles" "$copy/"
# Strata's mod_directory etc. would point elsewhere if the ini moved them: refuse that.
if grep -qE '^(base|mod|profiles|overwrite)_directory=.' "$copy/ModOrganizer.ini"; then
  echo "the ini moves its folders; this check only handles the default layout"; exit 2
fi
( cd "$src/mods" && find . -mindepth 1 -maxdepth 1 -type d -print0 ) | while IFS= read -r -d '' d; do
  mkdir -p "$copy/mods/$d"
  [ -f "$src/mods/$d/meta.ini" ] && cp --reflink=auto "$src/mods/$d/meta.ini" "$copy/mods/$d/"
done
echo "check modlist: $src ($steps steps, seed $seed)"
python3 "$here/tools/oracle/modlist.py" check "$copy" --steps "$steps" --seed "$seed" --strata "$strata"
