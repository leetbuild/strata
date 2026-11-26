#!/usr/bin/env bash
# The installer dialogs, offscreen: each one is shown by "$b/strata-dialog-shot" with a
# request like the ones strata_ui::install sends, photographed and closed (closing = Cancel,
# which must come back as the answer). One contact sheet: _stage/shell-test-install/sheet.png.
#   tools/shell-test-install.sh [stylesheet name, default "vs15 Dark"]
set -euo pipefail
cd "$(dirname "$0")/.."
b="${STRATA_BUILD:-build}"  # after tools/build.sh Debug: STRATA_BUILD=build-debug
[ -x "$b/strata-dialog-shot" ] || { echo ""$b/strata-dialog-shot" not built (tools/build.sh)"; exit 1; }
out=_stage/shell-test-install
rm -rf "$out"; mkdir -p "$out"
export QT_QPA_PLATFORM=offscreen
export STRATA_STYLE_FILE="$b/stylesheets/${1:-vs15 Dark}.qss"
[ -f "$STRATA_STYLE_FILE" ] || { echo "no stylesheet $STRATA_STYLE_FILE"; exit 1; }

# a picture for the FOMOD dialog
img="$out/images"; mkdir -p "$img/fomod"
magick -size 320x180 gradient:steelblue-navy -gravity center -pointsize 28 -fill white -annotate 0 "2K textures" "$img/fomod/2k.png"

cat > "$out/quick.json" <<'EOF'
{"name":"Cool Mod","variants":["Cool Mod","Cool Mod-1234-1-0"]}
EOF
cat > "$out/bain.json" <<'EOF'
{"name":"Bain Mod","variants":["Bain Mod"],"packages":[{"name":"00 Core","checked":true},{"name":"10 High res","checked":false},{"name":"20 Low res","checked":false}]}
EOF
cat > "$out/bundle.json" <<'EOF'
{"archives":["pack/Main File.7z","pack/Optional Patch.7z"]}
EOF
cat > "$out/clash.json" <<'EOF'
{"name":"SkyUI"}
EOF
cat > "$out/root_sort.json" <<'EOF'
{"mod_name":"SKSE64","to_root":["skse64_1_5_97.dll","skse64_loader.exe","skse64_steam_loader.dll"],"dropped":["skse64_2_00_20/skse64_readme.txt","skse64_2_00_20/src"],"unsure":[],"data_split":true,"base":"skse64_2_00_20"}
EOF
cat > "$out/question.json" <<'EOF'
{"id":"unpackBSA","title":"Extract BSA","text":"This mod contains at least one BSA. Do you want to unpack it?\n(If you don't know about BSAs, just select no)","default":false}
EOF
cat > "$out/manual.json" <<'EOF'
{"name":"Odd Layout","variants":["Odd Layout"],"paths":["one/","one/meshes/","one/meshes/a.nif","two/","two/notes.txt","readme.txt"],"valid":false,"data_dir":"Data"}
EOF
cat > "$out/fomod.json" <<EOF
{"name":"Fancy Textures","variants":["Fancy","Fancy Textures"],
 "info":{"name":"Fancy Textures","author":"Someone","version":"2.1","website":"https://example.org"},
 "images":"$PWD/$img","can_back":false,"error":"",
 "step":{"index":0,"name":"Size","is_last":false,"blocking":[],
  "groups":[
   {"name":"Resolution","kind":"SelectExactlyOne","plugins":[
     {"name":"2K","description":"Big textures for big cards.","image":"fomod\\\\2k.png","kind":"Recommended","checked":true,"enabled":true},
     {"name":"1K","description":"Small textures.","image":null,"kind":"Optional","checked":false,"enabled":true}]},
   {"name":"Patches","kind":"SelectAny","plugins":[
     {"name":"SkyUI patch","description":"","image":null,"kind":"Recommended","checked":true,"enabled":true},
     {"name":"Old patch","description":"","image":null,"kind":"NotUsable","checked":false,"enabled":false}]},
   {"name":"Extras","kind":"SelectAtMostOne","plugins":[
     {"name":"Snow","description":"","image":null,"kind":"Optional","checked":false,"enabled":true}]}]}}
EOF

cat > "$out/password.json" <<'EOF'
{"archive":"Locked Mod-12-1-0.7z","wrong":true}
EOF
cat > "$out/ini_tweaks.json" <<'EOF'
{"name":"Tweaked Mod"}
EOF

fail=0
for kind in quick bain bundle clash root_sort question manual fomod password ini_tweaks; do
  if ! timeout 30 "$b/strata-dialog-shot" "$kind" "$out/$kind.json" "$out/$kind.png" "$out/$kind.answer" > "$out/$kind.log" 2>&1; then
    echo "FAIL: $kind (see $out/$kind.log)"; fail=1; continue
  fi
  answer="$(cat "$out/$kind.answer")"
  case "$kind" in
    root_sort) want='"move":false' ;;
    question|ini_tweaks) want='"yes":false' ;;
    *) want='"action":"cancel"' ;;
  esac
  if [[ "$answer" != *"$want"* ]]; then echo "FAIL: $kind answered $answer"; fail=1; else echo "ok: $kind -> $answer"; fi
done
# a whole install through the window's code path: worker thread, dialogs, bridge, deploy
inst="$out/instance"
tools/gen-instance "$inst" > /dev/null
src="$out/src"; mkdir -p "$src/Wrap/fomod" "$src/Wrap/core" "$src/Wrap/hi" "$src/Wrap/lo"
printf '<config><moduleName>Shot</moduleName><requiredInstallFiles><folder source="core" destination=""/></requiredInstallFiles><installSteps order="Explicit"><installStep name="Size"><optionalFileGroups><group name="Res" type="SelectExactlyOne"><plugins order="Explicit"><plugin name="Hi"><description>high</description><files><folder source="hi" destination="textures"/></files><typeDescriptor><type name="Recommended"/></typeDescriptor></plugin><plugin name="Lo"><description>low</description><files><folder source="lo" destination="textures"/></files><typeDescriptor><type name="Optional"/></typeDescriptor></plugin></plugins></group></optionalFileGroups></installStep></installSteps></config>' > "$src/Wrap/fomod/ModuleConfig.xml"
echo esp > "$src/Wrap/core/Shot.esp"; echo hi > "$src/Wrap/hi/a.dds"; echo lo > "$src/Wrap/lo/a.dds"
(cd "$src" && 7z a -bso0 -bsp0 ../Shot.7z .)
if report="$(timeout 120 "$b/strata-dialog-shot" install "$PWD/$inst" Default "$PWD/$out/Shot.7z" "$out/run")" \
   && [[ "$report" == *'"installer":"Fomod"'* ]] && [ "$(cat "$inst/mods/Shot/textures/a.dds")" = hi ] \
   && grep -q '^+Shot' "$inst/profiles/Default/modlist.txt"; then
  echo "ok: install through the dialogs -> mods/Shot, enabled on top"
else
  echo "FAIL: install through the dialogs: $report"; fail=1
fi

# the Downloads tab's queue: an nxm:// link from a fake Nexus, paused, resumed
dl="$out/dl"; mkdir -p "$dl/src"
head -c 3000000 /dev/urandom > "$dl/src/big.bin"; echo esp > "$dl/src/Fake.esp"
(cd "$dl/src" && 7z a -bso0 -bsp0 -mx0 "../Fake Mod-5-1-0.7z" .)
python3 tools/fake-nexus.py "$dl/Fake Mod-5-1-0.7z" "$dl/port" & fake=$!
for _ in $(seq 50); do [ -s "$dl/port" ] && break; sleep 0.1; done
python3 - "$dl" <<'PY'
import os, sys
d = sys.argv[1]
open(os.path.join(d, "secret"), "w").write("KEY")
open(os.path.join(d, "secret-tool"), "w").write("#!/bin/sh\n[ \"$1\" = lookup ] && cat '%s/secret'\n" % d)
os.chmod(os.path.join(d, "secret-tool"), 0o755)
PY
dinst="$out/dl-instance"; tools/gen-instance "$dinst" > /dev/null
if dlog="$(STRATA_SECRET_TOOL="$dl/secret-tool" STRATA_NEXUS_API="http://127.0.0.1:$(cat "$dl/port")/v1" \
     timeout 90 "$b/strata-dialog-shot" download "$PWD/$dinst" \
     "nxm://skyrimspecialedition/mods/5/files/77?key=k&expires=1900000000" "$out/queue")" \
   && [[ "$dlog" == *"paused Fake Mod-5-1-0.7z"* ]] && [[ "$dlog" == *"downloaded Fake Mod-5-1-0.7z"* ]] \
   && cmp -s "$dl/Fake Mod-5-1-0.7z" "$dinst/downloads/Fake Mod-5-1-0.7z" \
   && grep -q '^modID=5' "$dinst/downloads/Fake Mod-5-1-0.7z.meta"; then
  echo "ok: download queue -> paused, resumed, the archive byte-identical with its .meta"
else
  echo "FAIL: download queue: $dlog"; fail=1
fi
kill $fake 2>/dev/null

magick montage "$out"/{quick,bain,bundle,clash,root_sort,question,manual,fomod,password,ini_tweaks}.png "$out"/run-*.png "$out"/queue-*.png -tile 4x -geometry '+8+8' -background '#202020' "$out/sheet.png"
echo "contact sheet: $out/sheet.png"
exit $fail
