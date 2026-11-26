#!/usr/bin/env bash
# Oracle check 1: every MO2 file of an instance survives reading and writing byte for byte.
#   tools/oracle/check-roundtrip.sh <instance folder>
# Reads only. Exits non-zero on a failure.
set -euo pipefail
here="$(cd "$(dirname "$0")/../.." && pwd)"
strata="${STRATA:-$here/target/release/strata-headless}"
[ -x "$strata" ] || { echo "build first: cargo build --release -p strata-cli"; exit 2; }
echo "check 1: $1"
"$strata" oracle roundtrip -i "$1"
