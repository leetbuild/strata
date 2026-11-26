#!/usr/bin/env bash
# Builds everything (Rust + C++) into build/strata.
#   tools/build.sh            release build
#   tools/build.sh Debug      debug build (in build-debug/)
# STRATA_JOBS caps the parallel jobs (default 8, so the desktop stays usable).
set -euo pipefail
cd "$(dirname "$0")/.."

type="${1:-Release}"
dir=build
[ "$type" = Release ] || dir="build-${type,,}"
jobs="${STRATA_JOBS:-8}"

export CARGO_BUILD_JOBS="$jobs"
cmake -S . -B "$dir" -G Ninja -DCMAKE_BUILD_TYPE="$type"
cmake --build "$dir" -j "$jobs"
echo "built $dir/strata"
