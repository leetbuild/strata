#!/usr/bin/env bash
# Oracle check 4: Strata's plugin list equals the Python reference's (tools/oracle/plugins.py):
# order, active state, load order, index, master and light flags, masters, missing masters.
#   tools/oracle/check-plugins.sh <instance folder> [profile] [--file-order] [--probe strata-probe.json]
# With --file-order (for instances MO2 or Fluorine last wrote), also checks that Strata's order
# is the one in the profile's loadorder.txt for every plugin still present.
# With --probe, also checks the enabled plugins against what the game really loaded (the
# report of tools/e2e.sh's probe): the regular plugins and the light plugins, each in order.
# Reads only. Exits non-zero on any difference; the diff is in _stage/oracle/.
set -euo pipefail
here="$(cd "$(dirname "$0")/../.." && pwd)"
instance=""
profile=""
probe=""
file_order=""
while [ $# -gt 0 ]; do
  case "$1" in
    --probe) probe="$2"; shift 2 ;;
    --file-order) file_order=1; shift ;;
    *) if [ -z "$instance" ]; then instance="$1"; else profile="$1"; fi; shift ;;
  esac
done
[ -n "$instance" ] || { echo "usage: check-plugins.sh <instance> [profile] [--probe json]"; exit 2; }
strata="${STRATA:-$here/target/release/strata-headless}"
[ -x "$strata" ] || { echo "build first: cargo build --release -p strata-cli"; exit 2; }

name="$(basename "$instance" | tr ' /' '__')"
out="$here/_stage/oracle/$name"
mkdir -p "$out"

args=(list plugins --json -i "$instance")
[ -z "$profile" ] || args+=(--profile "$profile")
"$strata" "${args[@]}" > "$out/strata-plugins.json"
python3 - "$out/strata-plugins.json" > "$out/strata-plugins.tsv" <<'EOF'
import json, sys
for p in json.load(open(sys.argv[1]))["plugins"]:
    print("\t".join([str(p["priority"]), p["name"], "+" if p["enabled"] else "-",
                     str(-1 if p["load_order"] is None else p["load_order"]), p["index"],
                     "M" if p["master"] else "-", "L" if p["light"] else "-",
                     ",".join(p["masters"]), ",".join(p["missing_masters"])]))
EOF
python3 "$here/tools/oracle/plugins.py" "$instance" ${profile:+"$profile"} > "$out/reference-plugins.tsv"

if ! diff "$out/reference-plugins.tsv" "$out/strata-plugins.tsv" > "$out/plugins.diff"; then
  echo "check 4 FAILED: $(grep -c '^[<>]' "$out/plugins.diff") differing lines, see $out/plugins.diff"
  head -10 "$out/plugins.diff"
  exit 1
fi
total=$(wc -l < "$out/strata-plugins.tsv")
active=$(awk -F'\t' '$3 == "+"' "$out/strata-plugins.tsv" | wc -l)
missing=$(awk -F'\t' '$9 != ""' "$out/strata-plugins.tsv" | wc -l)
echo "check 4 ok: $total plugins, $active active, $missing with missing masters, $instance"

# The profile's loadorder.txt was written by MO2 (or Fluorine) from its own list: for the plugins
# still present, Strata's order must be the file's.
profile_dir="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["profile"])' "$out/strata-plugins.json")"
lo="$instance/profiles/$profile_dir/loadorder.txt"
if [ -n "$file_order" ] && [ -f "$lo" ]; then
  python3 - "$out/strata-plugins.tsv" "$lo" <<'EOF'
import sys
mine = [l.split("\t")[1] for l in open(sys.argv[1])]
present = {n.lower() for n in mine}
lines = open(sys.argv[2], "rb").read().decode("utf-8", "replace").splitlines()
file = [l.strip() for l in lines if l.strip() and not l.startswith("#") and l.strip().lower() in present]
if [n.lower() for n in file] != [n.lower() for n in mine if n.lower() in {f.lower() for f in file}]:
    print("check 4 FAILED: the order differs from the profile's loadorder.txt")
    sys.exit(1)
gone = len([l for l in lines if l.strip() and not l.startswith("#")]) - len(file)
print(f"check 4 loadorder.txt ok: same order as the file ({len(file)} plugins; {gone} listed but not present)")
EOF
  # Skyrim LE (GamebryoGamePlugins): plugins.txt is exactly the enabled plugins in load order.
  game_name="$("$strata" info -i "$instance" --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["game"])')"
  if [ "$game_name" = "Skyrim" ]; then
    python3 - "$out/strata-plugins.tsv" "$instance/profiles/$profile_dir/plugins.txt" <<'EOF'
import sys
mine = [l.split("\t")[1] for l in open(sys.argv[1]) if l.split("\t")[2] == "+"]
lines = open(sys.argv[2], "rb").read().decode("utf-8", "replace").splitlines()
file = [l.strip() for l in lines if l.strip() and not l.startswith("#")]
if file != mine:
    print("check 4 FAILED: plugins.txt is not the enabled plugins in Strata's order")
    sys.exit(1)
print(f"check 4 plugins.txt ok: the {len(mine)} enabled plugins, in order (Skyrim LE format)")
EOF
  fi
fi

if [ -n "$probe" ]; then
  python3 - "$out/strata-plugins.tsv" "$probe" <<'EOF'
import json, sys
rows = [l.rstrip("\n").split("\t") for l in open(sys.argv[1])]
on = [r for r in rows if r[2] == "+"]
mine = {"plugins": [r[1].lower() for r in on if r[6] != "L"], "light_plugins": [r[1].lower() for r in on if r[6] == "L"]}
game = json.load(open(sys.argv[2]))
bad = False
for key in ("plugins", "light_plugins"):
    theirs = [n.lower() for n in game[key]]
    if theirs != mine[key]:
        bad = True
        print(f"check 4 probe FAILED: {key} differ from what the game loaded")
        print("  game:  ", theirs)
        print("  strata:", mine[key])
if bad:
    sys.exit(1)
print(f"check 4 probe ok: {len(mine['plugins'])} regular and {len(mine['light_plugins'])} light plugins in the game's order")
EOF
fi
