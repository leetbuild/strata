#!/usr/bin/env bash
# Numbers: the overlay on a big instance (a reflink copy, never the real one).
#   tools/bench-overlay.sh <instance copy> <scratch prefix> [threads]
# Deploys (the game mount plus the profile mounts in <scratch prefix>/pfx), then times: find,
# a second find, stat of every file, reading every winning .bsa once through the mount and
# from the real files, 16 parallel readers, and the daemon's memory. Undeploys at the end.
# The page cache cannot be dropped without root: every pass runs warm or half warm, and the
# order of the passes is printed with them.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
strata="${STRATA:-$here/target/release/strata-headless}"
inst="$1"
pfx="$2"
[ -x "$strata" ] || { echo "build first: cargo build --release -p strata-cli"; exit 2; }
case "$inst" in "$HOME/Games/"*) echo "refusing to mount a real instance: use a copy"; exit 2;; esac
case "$pfx" in "$HOME/Games/"*) echo "refusing to mount over a real prefix: use a scratch folder"; exit 2;; esac

json() { python3 -c "import json,sys; print(json.load(sys.stdin)$1)"; }
now() { date +%s.%N; }
secs() { python3 -c "print(f'{$2 - $1:.2f}')"; }
game="$("$strata" info -i "$inst" --json | json "['game_path']")"
out="$here/_stage/bench"
mkdir -p "$out"

t0=$(now); "$strata" deploy -i "$inst" --prefix "$pfx" > /dev/null; t1=$(now)
echo "deploy: $(secs "$t0" "$t1") s"
pid="$("$strata" status -i "$inst" --json | json "['pid']")"
rss() { awk '/^VmRSS|^VmHWM/ {printf "%s %d MiB  ", $1, $2/1024}' "/proc/$pid/status"; echo; }
echo "vfsd memory after deploy: $(rss)"
trap '"$strata" undeploy -i "$inst" > /dev/null 2>&1 || true' EXIT
trap 'exit 1' INT TERM PIPE

mods="$("$strata" info -i "$inst" --json | json "['mods']")"
t0=$(now); n=$(find "$mods" | wc -l); t1=$(now)
echo "for comparison, find over the real mod folders: $n entries, $(secs "$t0" "$t1") s"
t0=$(now); find "$mods" -type f -printf '%s\n' > /dev/null; t1=$(now)
echo "for comparison, find with stat over the real mod folders: $(secs "$t0" "$t1") s"
t0=$(now); n=$(find "$game" | wc -l); t1=$(now)
echo "find (first): $n entries, $(secs "$t0" "$t1") s"
t0=$(now); find "$game" > /dev/null; t1=$(now)
echo "find (second): $(secs "$t0" "$t1") s"
t0=$(now); n=$(find "$game" -type f -printf '%s\n' | wc -l); t1=$(now)
echo "find with stat of every file: $n files, $(secs "$t0" "$t1") s"
t0=$(now); find "$game" -type f -printf '%s\n' > /dev/null; t1=$(now)
echo "find with stat of every file (second): $(secs "$t0" "$t1") s"

find "$game/Data" -maxdepth 1 -iname '*.bsa' | sort > "$out/mounted.txt"
: > "$out/real.txt"
while IFS= read -r file; do
  # the game folder's own files are reached through the daemon's handle on it
  "$strata" resolve -i "$inst" --json "Data/$(basename "$file")" | json "['real']" \
    | sed "s|^/proc/self/|/proc/$pid/|" >> "$out/real.txt"
done < "$out/mounted.txt"
bytes=$(xargs -d '\n' stat -c %s < "$out/real.txt" | awk '{s+=$1} END {print s}')
gb=$(python3 -c "print(f'{$bytes/1e9:.1f}')")
echo "winning .bsa files: $(wc -l < "$out/mounted.txt"), $gb GB"
pass() { # label list parallel
  local t0 t1
  # dd: read() into a buffer, as a game does (cat may splice to /dev/null without copying)
  t0=$(now)
  xargs -d '\n' -P "$3" -n 8 sh -c 'for f; do dd if="$f" of=/dev/null bs=1M status=none; done' sh < "$2"
  t1=$(now)
  python3 -c "d=$t1-$t0; print(f'$1: {d:.1f} s, {$bytes/1e9/d:.2f} GB/s')"
}
[ "${BENCH_COLD:-1}" = 1 ] || { echo "(BENCH_COLD=0: the passes over every .bsa skipped)"; skip=1; }
[ -n "${skip:-}" ] || {
pass "read every .bsa, real files (1)" "$out/real.txt" 1
pass "read every .bsa, through the mount (1)" "$out/mounted.txt" 1
pass "read every .bsa, real files (2)" "$out/real.txt" 1
pass "read every .bsa, through the mount (2)" "$out/mounted.txt" 1
pass "16 parallel readers, real files" "$out/real.txt" 16
pass "16 parallel readers, through the mount" "$out/mounted.txt" 16
}
# A set that fits in memory: the cost of the overlay itself, once the disk is out of the way.
paste -d '\t' "$out/mounted.txt" "$out/real.txt" | while IFS=$'\t' read -r m r; do
  printf '%s\t%s\t%s\n' "$(stat -c %s "$r")" "$m" "$r"
done | awk -F'\t' '{ if (t + $1 <= 6e9) { t += $1; print $2 > "'"$out"'/warm-mounted.txt"; print $3 > "'"$out"'/warm-real.txt" } }'
bytes=$(xargs -d '\n' stat -c %s < "$out/warm-real.txt" | awk '{s+=$1} END {print s}')
echo "warm set: $(wc -l < "$out/warm-real.txt") .bsa files, $(python3 -c "print(f'{$bytes/1e9:.1f}')") GB"
pass "warm set, real files (1)" "$out/warm-real.txt" 1
pass "warm set, real files (2)" "$out/warm-real.txt" 1
pass "warm set, through the mount (1)" "$out/warm-mounted.txt" 1
pass "warm set, through the mount (2)" "$out/warm-mounted.txt" 1
pass "warm set, 16 readers, real files" "$out/warm-real.txt" 16
pass "warm set, 16 readers, through the mount" "$out/warm-mounted.txt" 16
echo "vfsd memory at the end: $(rss)"
# A new mount starts with no cached pages of its own: 16 readers through the daemon's threads.
"$strata" undeploy -i "$inst" > /dev/null
"$strata" deploy -i "$inst" --prefix "$pfx" > /dev/null
pass "warm set, 16 readers, through a new mount" "$out/warm-mounted.txt" 16
