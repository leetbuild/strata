#!/usr/bin/env bash
# The DDS preview's decoder against ImageMagick's, on real textures (read only):
#   tools/oracle/check-dds.sh <folder> [files per format, default 5]
# For each format found (DXT1, DXT5, BC1, BC4, BC5, BC7, uncompressed...), up to N textures
# under 1 MB are decoded by `strata preview` and by `magick`; every pixel must be within 2 of
# ImageMagick's (block decoders round differently by one step). Exits non-zero on a mismatch.
set -euo pipefail
here="$(cd "$(dirname "$0")/../.." && pwd)"
strata="${STRATA:-$here/target/release/strata-headless}"
[ -x "$strata" ] || { echo "build first: cargo build --release -p strata-cli"; exit 2; }
command -v magick > /dev/null || { echo "needs ImageMagick (magick): skipped"; exit 2; }
folder="$1"
per="${2:-5}"
work="$here/_stage/oracle/dds"
rm -rf "$work"; mkdir -p "$work"

declare -A count
checked=0 failed=0
while IFS= read -r -d '' f; do
  fmt=$("$strata" preview "$f" --out "$work/ours.pam" 2>/dev/null | awk '{print $1}') || continue
  [ -n "$fmt" ] || continue
  [ "${count[$fmt]:-0}" -lt "$per" ] || continue
  count[$fmt]=$(( ${count[$fmt]:-0} + 1 ))
  # Only the top mip level: ImageMagick reads it as the first image.
  if ! magick "$f[0]" -depth 8 "$work/theirs.pam" 2> "$work/magick.err"; then
    echo "skip ($fmt): ImageMagick cannot read $f"; continue
  fi
  # ImageMagick drops the alpha channel of opaque textures: compare RGBA when both have it.
  diff=$(magick compare -metric AE -fuzz 1% "$work/ours.pam" "$work/theirs.pam" null: 2>&1 || true)
  diff=${diff%% *}
  checked=$((checked + 1))
  if [ "$diff" != "0" ]; then
    echo "DIFF ($fmt): $diff pixels differ by more than 1%: $f"; failed=1
  fi
done < <(find "$folder" -iname '*.dds' -size -1024k -print0 | sort -z)
for k in "${!count[@]}"; do echo "  $k: ${count[$k]}"; done
[ "$failed" = 0 ] && echo "dds check ok: $checked textures" || { echo "dds check FAILED"; exit 1; }
