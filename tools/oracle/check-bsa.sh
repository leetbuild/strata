#!/usr/bin/env bash
# BSA reading checked against Fluorine's two readers (tools/oracle/bsa_reference.py): the `ba2`
# crate's listing must equal Strata's; bsatk (MO2's reader, through Fluorine's Linux libbsarch)
# must count the same and give the same bytes for every file it finds.
#   tools/oracle/check-bsa.sh <archive or folder>...    (a folder: every *.bsa directly in it)
# Reads the archives only; extracts into _stage/oracle/bsa/ (removed after each archive).
# LIBBSARCH=<liblibbsarch.so> overrides where the library is. Exits non-zero on any mismatch.
set -euo pipefail
here="$(cd "$(dirname "$0")/../.." && pwd)"
strata="${STRATA:-$here/target/release/strata-headless}"
[ -x "$strata" ] || { echo "build first: cargo build --release -p strata-cli"; exit 2; }
lib="${LIBBSARCH:-$HOME/src/Projects/Windows Apps/MO2/Fluorine-Manager/build/libs/libbsarch/liblibbsarch.so}"
[ -f "$lib" ] || { echo "no libbsarch at $lib (build Fluorine, or set LIBBSARCH): skipped"; exit 2; }

work="$here/_stage/oracle/bsa"
(cd "$here/tools/oracle/ba2-list" && CARGO_TARGET_DIR="$here/_stage/ba2-list-target" cargo build -q --release)
ba2list="$here/_stage/ba2-list-target/release/ba2-list"
mkdir -p "$work/lib"
cp "$lib" "$work/lib/liblibbsarch.so"
# Fluorine was built against Boost 1.88; the boost::thread symbols it uses are the same in the
# installed Boost, so the installed library stands in under the old name.
for want in $(ldd "$work/lib/liblibbsarch.so" | awk '/not found/ {print $1}'); do
  have=$(ls /usr/lib/"${want%%.so*}".so.* 2>/dev/null | head -1)
  [ -n "$have" ] || { echo "libbsarch needs $want, not installed: skipped"; exit 2; }
  ln -sf "$have" "$work/lib/$want"
done

archives=()
for arg in "$@"; do
  if [ -d "$arg" ]; then
    while IFS= read -r -d '' f; do archives+=("$f"); done < <(find "$arg" -maxdepth 1 -iname '*.bsa' -print0 | sort -z)
  else
    archives+=("$arg")
  fi
done

failed=0
for a in "${archives[@]}"; do
  rm -rf "$work/x"
  "$strata" archive "$a" --json > "$work/list.json"
  "$strata" archive "$a" --extract "$work/x"
  ba2out="$work/ba2.txt"
  "$ba2list" "$a" > "$ba2out" 2> "$work/ba2.err" || ba2out=-
  LD_LIBRARY_PATH="$work/lib" python3 "$here/tools/oracle/bsa_reference.py" "$work/lib/liblibbsarch.so" "$a" "$work/list.json" "$work/x" "$ba2out" || failed=1
  rm -rf "$work/x"
done
[ "$failed" = 0 ] && echo "bsa check ok: ${#archives[@]} archives" || { echo "bsa check FAILED"; exit 1; }
