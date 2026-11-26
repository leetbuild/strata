#!/usr/bin/env bash
# Offscreen pictures of the conflicts area's dialogs on a generated instance (with a few real
# textures from the first instance of STRATA_REAL_INSTANCES when set): every mod info tab, and the preview of a DDS
# and a text file. Pictures in _stage/conflicts-shell/, a contact sheet in sheet.png.
# Exits non-zero when a run fails or leaves no picture. The pictures are for a person to judge.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .strata-test.env ] && { set -a; . ./.strata-test.env; set +a; }
[ -x build/strata ] || { echo "build/strata not built (tools/build.sh)"; exit 1; }
out=_stage/conflicts-shell
rm -rf "$out"; mkdir -p "$out/config"
inst="$out/instance"
tools/gen-instance "$inst" > /dev/null
mod="$inst/mods/Case B"
printf 'Hello\nreadme text\n' > "$mod/readme.txt"
printf '[General]\nfoo=1\n' > "$mod/settings.ini"
mkdir -p "$mod/optional"; touch "$mod/optional/extra.esp" "$mod/main.esp"
printf '1|Armour|0\n2|Heavy|1\n3|Weapons|0\n' > "$inst/categories.dat"
printf '[General]\nmodid=1234\nversion=2.0\ncategory="2,1"\ncomments=a comment\nnotes=<b>some notes</b>\nnexusDescription=<p>The <b>description</b> from Nexus.</p>\n' > "$mod/meta.ini"
real="$(echo "${STRATA_REAL_INSTANCES:-}" | cut -d: -f1)"; realmods="${real:+$real/mods}"
if [ -n "$realmods" ] && [ -d "$realmods" ]; then
  find "$realmods" -iname '*.dds' -size +20k -size -300k ! -iname '*_n.dds' ! -iname '*_g.dds' -print0 2>/dev/null \
    | head -z -n 4 | while IFS= read -r -d '' f; do cp "$f" "$mod/TEXTURES/"; done || true
fi
export XDG_CONFIG_HOME="$PWD/$out/config" QT_QPA_PLATFORM=offscreen
shot() {
  local name=$1; shift
  env STRATA_SHELL_SCREENSHOT="$out/$name.png" STRATA_SHELL_SCREENSHOT_DELAY=2500 "$@" \
    timeout 60 build/strata -i "$inst" > "$out/$name.log" 2>&1 || { echo "FAIL: $name (see $out/$name.log)"; exit 1; }
  [ -s "$out/$name.png" ] || { echo "FAIL: $name: no picture"; exit 1; }
}
for t in 0 1 2 3 4 5 6 7 8; do shot "tab$t" STRATA_SHELL_MODINFO="Case B" STRATA_SHELL_MODINFO_TAB=$t; done
dds=$(cd "$mod/TEXTURES" && ls *.dds 2>/dev/null | head -1 || true)
[ -z "$dds" ] || shot preview-dds STRATA_SHELL_PREVIEW="TEXTURES/$dds"
shot preview-text STRATA_SHELL_PREVIEW=SKSE/Plugins/p.ini
magick montage "$out"/*.png -tile 4x -geometry 600x450+4+4 "$out/sheet.png" 2> /dev/null || true
echo "ok: pictures in $out (sheet.png)"
