#!/usr/bin/env bash
# Oracle check 7 (docs/DESIGN.md, "Oracle checks"): the game really starts through Strata.
#
#   tools/e2e.sh [instance folder]        default: the first instance of STRATA_REAL_INSTANCES
#
# `strata run SKSE` on the instance, in a COPY of its prefix. The probe plugin
# (tools/probe) writes strata-probe.json at kDataLoaded and ends the game. Pass means:
#   - the file landed in the instance's overwrite/ (overlay + Root + Proton + SKSE work),
#   - the game is 1.5.97 (Skyrim SE) or 1.9.32 (Skyrim LE, a 32-bit probe) and loaded exactly
#     the plugins the profile's plugins.txt activates (for LE also in Strata's load order)
#     (so the profile mount in the prefix works too),
#   - every DLL in the merged Data/SKSE/Plugins was loaded,
#   - the real Game Root is byte for byte what it was.
#
# Needs: Steam running, the GPU free, a display. It opens the game's window for a minute.
# Writes: the probe mod and one line of modlist.txt in the instance (first run only),
# overwrite/, logs/, and the prefix copy in _stage/oracle/prefix.
# E2E_SKIP_HASH=1 skips hashing the Game Root (16 GB for Skyrim SE). E2E_PREFIX: see step 1.
set -euo pipefail
repo_env="$(dirname "$0")/../.strata-test.env"
[ -f "$repo_env" ] && { set -a; . "$repo_env"; set +a; }
here="$(cd "$(dirname "$0")/.." && pwd)"
strata="${STRATA:-$here/target/release/strata-headless}"
instance="${1:-$(echo "${STRATA_REAL_INSTANCES:-}" | cut -d: -f1)}"
[ -n "$instance" ] || { echo "give an instance folder (or set STRATA_REAL_INSTANCES)"; exit 2; }
out="$here/_stage/oracle/e2e"
mkdir -p "$out"

fail() { echo "check 7 FAILED: $*"; exit 1; }
[ -x "$strata" ] || fail "build first: cargo build --release -p strata-cli"
[ -f "$instance/ModOrganizer.ini" ] || fail "$instance is not an instance"
pgrep -x steam > /dev/null || fail "Steam is not running (the game needs it). Start Steam."
info() { "$strata" info -i "$instance" --json | python3 -c "import json,sys; print(json.load(sys.stdin)['$1'] or '')"; }
profile="$(info profile)"
profile_dir="$(info profile_dir)"
real_prefix="$(info prefix)"
game="$(info game_path)"
[ -d "$game" ] || fail "game folder not found: $game"
# The game's facts the judge needs (strata-game: SkyrimSE / SkyrimLE).
case "$(info game)" in
  "Skyrim Special Edition") arch=x64; probe_dll="$here/_stage/probe/StrataProbe.dll"; want_version=1.5.97
    documents="Skyrim Special Edition"; skse_log_name=skse64.log; le=0 ;;
  "Skyrim") arch=x86; probe_dll="$here/_stage/probe/x86/StrataProbe.dll"; want_version=1.9.32
    documents="Skyrim"; skse_log_name=skse.log; le=1 ;;
  *) fail "check 7 does not know the game \"$(info game)\"" ;;
esac
[ -f "$probe_dll" ] || "$here/tools/probe/build.sh" "$arch" > "$out/probe-build.log" 2>&1 \
  || fail "the probe did not build, see $out/probe-build.log"
[ -n "${E2E_PREFIX:-}" ] || [ -d "$real_prefix/pfx" ] || fail "prefix not found: $real_prefix"

# 1. The prefix: never the real one (it may be shared by other instances).
# E2E_PREFIX=<folder> uses that prefix as it is (one made by `strata prefix setup` on a copy).
prefix="${E2E_PREFIX:-$here/_stage/oracle/prefix}"
if [ -n "${E2E_PREFIX:-}" ]; then
  [ -d "$prefix/pfx" ] || fail "E2E_PREFIX has no pfx/: $prefix"
elif [ ! -d "$prefix/pfx" ]; then
  echo "copying the prefix (reflink) to $prefix"
  mkdir -p "$(dirname "$prefix")"
  cp --reflink=auto -a "$real_prefix" "$prefix"
fi

# 2. The probe, as its own mod at the top of the list.
mkdir -p "$instance/mods/Strata Probe/SKSE/Plugins"
cp "$probe_dll" "$instance/mods/Strata Probe/SKSE/Plugins/"
[ -f "$instance/mods/Strata Probe/meta.ini" ] || printf '[General]\r\nmodid=0\r\nversion=1\r\ncomments=Strata oracle check 7. Only acts when STRATA_PROBE is set.\r\n' > "$instance/mods/Strata Probe/meta.ini"
modlist="$profile_dir/modlist.txt"
if ! grep -q '^[+-]Strata Probe' "$modlist"; then
  cp -n "$modlist" "$modlist.before-strata-probe"
  sed -i '1a +Strata Probe\r' "$modlist"
  echo "added \"Strata Probe\" to $modlist (the old file: modlist.txt.before-strata-probe)"
fi
sed -i 's/^-Strata Probe/+Strata Probe/' "$modlist"

# 3. What the game should load, from Strata's own view of the instance.
"$strata" tree -i "$instance" > "$out/tree.txt"
"$strata" info -i "$instance" --json > "$out/info.json"
"$strata" list plugins --json -i "$instance" > "$out/plugins.json"
python3 - "$profile_dir/plugins.txt" "$out/tree.txt" "$out/info.json" "$le" "$out/plugins.json" > "$out/expected.json" <<'PY'
import json, re, sys
le = sys.argv[4] == "1"
tree = [l.rstrip("\n") for l in open(sys.argv[2], encoding="utf-8")]
data = {l[5:].lower() for l in tree if l.lower().startswith("data/") and "/" not in l[5:]}
# The game's own plugins and its Creation Club list load without being in plugins.txt.
primary = [p.lower() for p in json.load(open(sys.argv[3]))["primary_plugins"]]
active = [p for p in primary if p in data]
for line in open(sys.argv[1], "rb").read().decode("utf-8", "replace").splitlines():
    line = line.strip()
    # SE: `*name` is active. LE (GamebryoGamePlugins): every listed name is active.
    name = line if le else (line[1:] if line.startswith("*") else "")
    if name and not name.startswith("#") and name.lower() in data and name.lower() not in active:
        active.append(name.lower())
dlls = sorted(l.rsplit("/", 1)[1].lower() for l in tree if re.fullmatch(r"data/skse/plugins/[^/]+\.dll", l.lower()))
order = [p["name"].lower() for p in json.load(open(sys.argv[5]))["plugins"] if p["enabled"]]
json.dump({"plugins": sorted(active), "order": order, "skse_plugins": dlls}, sys.stdout, indent=1)
PY

hash_game() {
  (cd "$game" && find . -type f -print0 | sort -z | xargs -0 -P 8 -n 64 sha256sum | sort -k2 | sha256sum | cut -d' ' -f1)
}
if [ -z "${E2E_SKIP_HASH:-}" ]; then
  echo "hashing the Game Root (before)"
  before="$(hash_game)"
fi

# 4. Run.
report="$instance/overwrite/SKSE/Plugins/strata-probe.json"
rm -f "$report"
echo "starting the game: a window opens, the probe closes it at the main menu"
STRATA_PROBE=1 "$strata" run SKSE -i "$instance" --prefix "$prefix" --timeout "${E2E_TIMEOUT:-600}" > "$out/run.log" 2>&1 \
  || { tail -5 "$out/run.log"; fail "strata run failed, see $out/run.log and $instance/logs/"; }
# Only this instance's mounts count (other tests may mount at the same time).
mounted_game="$(printf '%s' "$game" | sed 's/ /\\040/g')"
grep "fuse.strata" /proc/self/mountinfo | grep -qF "$mounted_game " && fail "a mount is left after the run"

# 5. Judge.
[ -f "$report" ] || { tail -5 "$out/run.log"; fail "no probe report in overwrite/ (did SKSE load? see the prefix's My Games/.../SKSE/$skse_log_name)"; }
cp "$report" "$out/strata-probe.json"
skse_log="$prefix/pfx/drive_c/users/steamuser/Documents/My Games/$documents/SKSE/$skse_log_name"
cp "$skse_log" "$out/$skse_log_name" 2> /dev/null || fail "no $skse_log_name in the prefix"
python3 - "$out/strata-probe.json" "$out/expected.json" "$out/$skse_log_name" "$want_version" "$le" <<'PY' || fail "the game did not load what the profile says"
import json, re, sys
got, want = json.load(open(sys.argv[1])), json.load(open(sys.argv[2]))
log = open(sys.argv[3], encoding="utf-8", errors="replace").read()
ok = True
def check(label, cond, detail=""):
    global ok
    print(("  ok   " if cond else "  FAIL ") + label + (": " + detail if detail and not cond else ""))
    ok = ok and cond
want_version, le = sys.argv[4], sys.argv[5] == "1"
check(f"game version {want_version}", got["game_version"].startswith(want_version), got["game_version"])
check("plugin list read from the game", got["plugins_read"])
loaded = sorted(p.lower() for p in got["plugins"] + got["light_plugins"])
missing, extra = sorted(set(want["plugins"]) - set(loaded)), sorted(set(loaded) - set(want["plugins"]))
check(f"game plugins: {len(loaded)} loaded, {len(want['plugins'])} expected", not missing and not extra,
      f"missing {missing[:8]} extra {extra[:8]}")
if le:
    # LE has one plugin list: the game's load order must be Strata's (plugins.txt order).
    game_order = [p.lower() for p in got["plugins"]]
    check(f"load order as Strata's ({len(game_order)} plugins)", game_order == want["order"],
          next((f"first difference at {i}: game {a}, strata {b}" for i, (a, b) in enumerate(zip(game_order, want["order"])) if a != b), "lengths differ"))

# SKSE's own verdict on every DLL it found in Data/SKSE/Plugins.
verdict = {}
for name, rest in re.findall(r"^plugin .*\\([^\\\r\n]+?\.dll)(.*)$", log, re.M | re.I):
    verdict[name.lower()] = rest.strip()
seen = {n.lower() for n in re.findall(r"^checking plugin .*\\([^\\\r\n]+\.dll)", log, re.M | re.I)}
in_process = {p.lower() for p in got["skse_plugins"]}
check(f"SKSE saw every DLL of the merged Data/SKSE/Plugins ({len(want['skse_plugins'])})",
      set(want["skse_plugins"]) <= seen, f"not seen {sorted(set(want['skse_plugins']) - seen)[:10]}")
good = {n for n, v in verdict.items() if "loaded correctly" in v}
not_plugins = {n for n, v in verdict.items() if "does not appear to be an SKSE plugin" in v}
refused = {n for n, v in verdict.items() if "reported as incompatible during load" in v}
broken = set(want["skse_plugins"]) - good - not_plugins - refused
check(f"SKSE plugins loaded correctly: {len(good)}", not broken and good <= in_process,
      f"failed to load {sorted(broken)[:10]}; loaded but not in the process {sorted(good - in_process)[:10]}")
for name in sorted(not_plugins):
    print(f"  note {name}: not an SKSE plugin (a library other plugins use)")
for name in sorted(refused):
    print(f"  note {name}: refused to load itself (see its own log; not the overlay's doing)")
print(f"  seconds from process start to kDataLoaded: {got['seconds_to_data_loaded']}")
sys.exit(0 if ok else 1)
PY
if [ -z "${E2E_SKIP_HASH:-}" ]; then
  echo "hashing the Game Root (after)"
  [ "$(hash_game)" = "$before" ] || fail "the real Game Root changed"
  echo "  ok   the real Game Root is unchanged"
fi
echo "check 7 ok: $instance"
