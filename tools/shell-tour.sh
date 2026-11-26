#!/usr/bin/env bash
# A picture tour of the window for a person to flip through: the main window, each tab,
# the context menus and every dialog, on a reflink copy of a real instance (read only:
# the original is never opened), offscreen.
#   tools/shell-tour.sh [instance]     default: the first instance of STRATA_REAL_INSTANCES
# Numbered PNGs and contact sheets in _stage/tour/. The copy is in _stage/tour-instance/
# (mods, profiles, overwrite, downloads, the ini; the Game Root is read in place, never
# written: nothing is mounted).
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .strata-test.env ] && { set -a; . ./.strata-test.env; set +a; }
src="${1:-$(echo "${STRATA_REAL_INSTANCES:-}" | cut -d: -f1)}"
[ -n "$src" ] || { echo "give an instance folder (or set STRATA_REAL_INSTANCES)"; exit 2; }
[ -f "$src/ModOrganizer.ini" ] || { echo "no instance at $src"; exit 1; }
[ -x build/strata-shell-test ] || { echo "build/strata-shell-test not built (tools/build.sh)"; exit 1; }
out=_stage/tour
copy=_stage/tour-instance
rm -rf "$out" "$copy"
mkdir -p "$out" "$copy"
for item in ModOrganizer.ini categories.dat nexuscatmap.dat mods profiles overwrite downloads stylesheets; do
  [ -e "$src/$item" ] && cp --reflink=auto -a "$src/$item" "$copy/"
done
# a 1920x1200 offscreen screen (the default 800x600 cuts menus near the bottom into scrolling ones)
printf '{"screens":[{"name":"tour","x":0,"y":0,"width":1920,"height":1200,"logicalDpi":96,"logicalBaseDpi":96,"dpr":1}]}' \
  > "$out/screen.json"
export STRATA_REPO="$PWD" QT_QPA_PLATFORM="offscreen:configfile=$PWD/$out/screen.json"
STRATA_TOUR_INSTANCE="$PWD/$copy" STRATA_TOUR_DIR="$PWD/$out" \
  timeout 900 build/strata-shell-test > "$out/tour.log" 2>&1 || { echo "FAIL: the tour stopped (see $out/tour.log)"; exit 1; }
shots=("$out"/[0-9]*.png)
[ "${#shots[@]}" -gt 10 ] || { echo "FAIL: only ${#shots[@]} pictures (see $out/tour.log)"; exit 1; }
if command -v magick > /dev/null; then
  # one sheet per 12 pictures, each picture at most 800 px wide, labelled with its file name
  i=0; n=1
  while [ $i -lt ${#shots[@]} ]; do
    magick montage "${shots[@]:$i:12}" -label '%t' -tile 3x -geometry '800x600>+6+6' -background '#333' \
      -fill white "$out/sheet-$n.png"
    i=$((i + 12)); n=$((n + 1))
  done
fi
echo "ok: ${#shots[@]} pictures in $out (sheets: $out/sheet-*.png)"
