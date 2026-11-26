#!/usr/bin/env bash
# Pictures of the modlist area's dialogs (profiles, executables, delete confirm, every tab
# of the settings dialog, the ini editor), offscreen, on a generated instance with a second profile.
# Contact sheets in _stage/shell-dialogs/sheet*.png to look at next to MO2's dialogs.
# Exits non-zero when a dialog does not open or leaves no picture.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -x build/strata ] || { echo "build/strata not built (tools/build.sh)"; exit 1; }
out=_stage/shell-dialogs
rm -rf "$out"
mkdir -p "$out/config"
tools/gen-instance "$out/instance" > /dev/null
build/strata profile copy Default Second -i "$out/instance" > /dev/null
# something for the Problems dialog to show
printf '[Display]\r\nfGamma=1.0\r\n' > "$out/instance/profiles/Default/profile_tweaks.ini"
export XDG_CONFIG_HOME="$PWD/$out/config" QT_QPA_PLATFORM=offscreen
shots=()
dialogs="profiles executables delete settings:0 settings:1 settings:2 settings:3 settings:4 settings:6 settings:7 settings:8 inieditor problems"
for d in $dialogs; do
  f="$out/${d/:/-}.png"
  STRATA_SHELL_DIALOG=$d STRATA_SHELL_DIALOG_SCREENSHOT="$f" timeout 30 build/strata -i "$out/instance" \
    > "$out/${d/:/-}.log" 2>&1 || { echo "FAIL: $d (see $out/${d/:/-}.log)"; exit 1; }
  [ -s "$f" ] || { echo "FAIL: $d left no picture"; exit 1; }
  shots+=("$f")
done
if command -v magick > /dev/null; then
  magick montage "${shots[@]:0:4}" -tile 2x2 -geometry +4+4 -background '#444' "$out/sheet1.png"
  magick montage "${shots[@]:4:4}" -tile 2x2 -geometry +4+4 -background '#444' "$out/sheet2.png"
  magick montage "${shots[@]:8:3}" -tile 2x2 -geometry +4+4 -background '#444' "$out/sheet3.png"
  magick montage "${shots[@]:11}" -tile 2x2 -geometry +4+4 -background '#444' "$out/sheet4.png"
fi
echo "ok: ${#shots[@]} dialog pictures in $out"
