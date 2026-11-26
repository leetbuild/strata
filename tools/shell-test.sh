#!/usr/bin/env bash
# Smoke tests of the window (build/strata, offscreen), on a generated instance:
#   1. no instance: the window opens with a bundled sheet;
#   2. -i <instance>: the mod list shows, the instance's own stylesheet (named in
#      ModOrganizer.ini) is used;
#   3. hot reload: the sheet is replaced by rename while the window runs, through a symlink,
#      and the mod list takes the new colours; then the same for an edit in place;
#   4. the layout is saved under MO2's keys when it changes (a tab picked);
#   5. the last instance is opened when none is given;
#   6. Run goes through `strata run` in a child process (--dry-run), output in the log;
#   7-8. the live mod list and a profile switch while deployed;
#   9. a separator keeps its colour under a sheet whose `::item` rule sets a background.
# Every picture is left in _stage/shell-test/ to look at. Exits non-zero on the first failure.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -x build/strata ] || { echo "build/strata not built (tools/build.sh)"; exit 1; }
command -v magick > /dev/null || { echo "needs ImageMagick (magick) to read the pictures"; exit 1; }

out=_stage/shell-test
rm -rf "$out"
mkdir -p "$out/config" "$out/elsewhere" "$out/rendered"
inst="$out/instance"
tools/gen-instance "$inst" > /dev/null
export XDG_CONFIG_HOME="$PWD/$out/config"   # never the user's last instance
export QT_QPA_PLATFORM=offscreen

fail() { echo "FAIL: $*"; exit 1; }

# shot <name> <delay ms> [strata args...]: picture in $out/<name>.png, log in $out/<name>.log
shot() {
  local name=$1 delay=$2; shift 2
  STRATA_SHELL_SCREENSHOT="$out/$name.png" STRATA_SHELL_SCREENSHOT_DELAY=$delay \
    timeout 60 build/strata "$@" > "$out/$name.log" 2>&1 || fail "$name: exit $? (see $out/$name.log)"
  [ -s "$out/$name.png" ] || fail "$name: no picture"
  if grep -q -i -E 'QCss|Could not parse' "$out/$name.log"; then fail "$name: stylesheet error (see $out/$name.log)"; fi
}

# pixels <png> <hex colour>: how many pixels have exactly that colour
pixels() {
  magick "$1" -alpha off -fill black +opaque "#$2" -fill white -opaque "#$2" -format '%[fx:int(mean*w*h+0.5)]' info:
}

echo "== 1. no instance"
(cd "$out/elsewhere" && STRATA_SHELL_SCREENSHOT="../none.png" timeout 30 ../../../build/strata > ../none.log 2>&1) \
  || fail "no instance: exit $?"
grep -q "no instance to open" "$out/none.log" || fail "no instance: no message (see $out/none.log)"
grep -q -E 'stylesheet .* not found' "$out/none.log" && fail "no instance: the bundled sheet was not found"

echo "== 2. the instance and its own stylesheet"
# The rendered sheet lives elsewhere and is linked into the instance, as desktop themes do.
sheet() { printf 'QTreeView#modList { background-color: #%s; alternate-background-color: #%s; }\n' "$1" "$1"; }
sheet ff0000 > "$out/rendered/test.qss"
mkdir -p "$inst/stylesheets"
ln -s "$PWD/$out/rendered/test.qss" "$inst/stylesheets/strata-test.qss"
sed -i "s/^style=.*/style=strata-test.qss\r/" "$inst/ModOrganizer.ini"
shot instance 1500 -i "$inst"
grep -q "opened .*instance, profile \"Default\"" "$out/instance.log" || fail "instance not opened (see $out/instance.log)"
red=$(pixels "$out/instance.png" ff0000)
[ "$red" -gt 2000 ] || fail "the instance's sheet is not applied to the mod list ($red red pixels)"
echo "ok ($red pixels of the instance's colour)"

echo "== 3. hot reload: replace by rename through the symlink"
(sleep 2; sheet 00ff00 > "$out/rendered/test.qss.new"; mv "$out/rendered/test.qss.new" "$out/rendered/test.qss") &
shot reload 4000 -i "$inst"
wait
grep -q "stylesheet reloaded" "$out/reload.log" || fail "no reload (see $out/reload.log)"
green=$(pixels "$out/reload.png" 00ff00)
red=$(pixels "$out/reload.png" ff0000)
[ "$green" -gt 2000 ] && [ "$red" -eq 0 ] || fail "the mod list kept the old sheet ($green new, $red old pixels)"
echo "ok ($green pixels in the new colour, none in the old)"

echo "== 3b. hot reload: an edit in place"
(sleep 2; sheet 0000ff > "$out/rendered/test.qss") &
shot reload-in-place 4000 -i "$inst"
wait
blue=$(pixels "$out/reload-in-place.png" 0000ff)
green=$(pixels "$out/reload-in-place.png" 00ff00)
[ "$blue" -gt 2000 ] && [ "$green" -eq 0 ] || fail "an edit in place was not picked up ($blue new, $green old pixels)"
echo "ok"

echo "== 4. the layout is saved on change, and only then"
grep -q "^MainWindow_modList_state=" "$inst/ModOrganizer.ini" && fail "the mod list header was saved without a change"
cp "$inst/ModOrganizer.ini" "$out/ini.before"
shot unchanged 2500 -i "$inst"
cmp -s "$inst/ModOrganizer.ini" "$out/ini.before" || fail "opening and closing changed ModOrganizer.ini"
STRATA_SHELL_REFRESH=1 shot refresh 2500 -i "$inst"
grep -q "refreshed" "$out/refresh.log" || fail "no refresh (see $out/refresh.log)"
cmp -s "$inst/ModOrganizer.ini" "$out/ini.before" || fail "a refresh changed ModOrganizer.ini"
STRATA_SHELL_TAB=4 STRATA_SHELL_HIDE_COLUMN=4 shot layout 2500 -i "$inst"
grep -q "^MainWindow_tabWidget_index=4" "$inst/ModOrganizer.ini" || fail "the tab index was not saved"
grep -q "^MainWindow_modList_state=@ByteArray(" "$inst/ModOrganizer.ini" || fail "the mod list header was not saved"
grep -q "^gameName=Skyrim Special Edition" "$inst/ModOrganizer.ini" || fail "other keys were lost"
echo "ok"

echo "== 5. the last instance"
(cd "$out/elsewhere" && STRATA_SHELL_SCREENSHOT="../last.png" timeout 30 ../../../build/strata > ../last.log 2>&1) \
  || fail "last instance: exit $?"
grep -q "opened .*instance, profile" "$out/last.log" || fail "the last instance was not opened (see $out/last.log)"
echo "ok"

echo "== 6. Run through strata run (--dry-run)"
STRATA_SHELL_RUN_DRY=SKSE shot run 4000 -i "$inst"
grep -q "strata run SKSE -i" "$out/run.log" || fail "Run did not start strata run (see $out/run.log)"
grep -q -E "run finished|the run ended|prefix|Proton" "$out/run.log" || fail "no output from strata run (see $out/run.log)"
echo "ok"
echo "== 7. the mod list is live: a checkbox click, a drop onto a separator, a live deploy refreshed"
list="$inst/profiles/Default/modlist.txt"
grep -q "^-Disabled" "$list" || fail "fixture: Disabled is not disabled"
build/strata deploy -i "$inst" > "$out/deploy.log" 2>&1 || fail "deploy (see $out/deploy.log)"
trap 'build/strata undeploy --force -i "$inst" > /dev/null 2>&1 || true' EXIT
[ -e "$inst/Game Root/Data/disabled-only.esp" ] && fail "a disabled mod's file is in the mount"
STRATA_SHELL_TOGGLE=Disabled STRATA_SHELL_DROP="SKSE|Top_separator" shot live 3500 -i "$inst"
grep -q "^+Disabled" "$list" || fail "the checkbox did not enable the mod (see $out/live.log)"
grep -q "refreshed the live deploy" "$out/live.log" || fail "the live deploy was not refreshed (see $out/live.log)"
[ -e "$inst/Game Root/Data/disabled-only.esp" ] || fail "the enabled mod's file is not in the mount"
# Top's own mods end at its first sub-separator, Nest 1: SKSE goes just before it (the file
# lists the highest priority first, so on the line after it), not inside Nest 5 at the end
after=$(grep -A1 '^.Nest 1_separator' "$list" | sed -n 2p | tr -d '\r')
[ "$after" = "+SKSE" ] || fail "the drop onto Top did not put SKSE after Top's own mods (after Nest 1: $after)"
build/strata undeploy -i "$inst" > /dev/null 2>&1 || fail "undeploy"
trap - EXIT
echo "ok"
echo "== 8. switching profile while deployed: undeploy, select, deploy the new one"
build/strata profile copy Default Second -i "$inst" > "$out/profile.log" 2>&1 || fail "profile copy (see $out/profile.log)"
build/strata deploy -i "$inst" > "$out/deploy2.log" 2>&1 || fail "deploy (see $out/deploy2.log)"
trap 'build/strata undeploy --force -i "$inst" > /dev/null 2>&1 || true' EXIT
STRATA_SHELL_PROFILE=Second shot profile 5000 -i "$inst"
grep -q 'profile "Second"' "$out/profile.log" || fail "the window did not switch (see $out/profile.log)"
grep -q "^selected_profile=@ByteArray(Second)" "$inst/ModOrganizer.ini" || fail "selected_profile was not written"
build/strata status -i "$inst" --json > "$out/status.json" 2>&1 || fail "status"
grep -q '"label": *"Second"' "$out/status.json" || fail "the deploy does not show Second (see $out/status.json)"
build/strata undeploy -i "$inst" > /dev/null 2>&1 || fail "undeploy"
trap - EXIT
echo "ok"
echo "== 9. separator colours survive a sheet whose ::item rule has a background (riki's case)"
build/strata mod color Top_separator '#ad373b' -i "$inst" > "$out/color.log" 2>&1 || fail "mod color (see $out/color.log)"
printf 'QAbstractItemView::item { background: #102030; border: none; padding: 2px 0; }\n' > "$inst/stylesheets/itembg.qss"
STRATA_STYLE=itembg.qss shot itembg 2000 -i "$inst"
sep=$(pixels "$out/itembg.png" ad373b)
bg=$(pixels "$out/itembg.png" 102030)
[ "$bg" -gt 2000 ] || fail "the test sheet was not applied ($bg pixels of its item background)"
[ "$sep" -gt 2000 ] || fail "the separator lost its colour under the sheet ($sep pixels of #ad373b)"
echo "ok ($sep pixels of the separator's colour, $bg of the sheet's item background)"
echo "shell tests passed (pictures in $out/)"
