#!/usr/bin/env bash
# M5's idempotence check: `strata prefix setup` on a prefix it already set up changes nothing.
#   tools/oracle/check-prefix.sh <instance folder>
# Runs the setup once (it may do work: a fresh prefix is made here), hashes the prefix's
# registry files and DLL folders (and the .NET folder), runs the setup again, and fails when
# the second run did any step or anything hashed changed.
# Starts Proton when the first run has work: only on a copy, never on a prefix in use.
set -euo pipefail
here="$(cd "$(dirname "$0")/../.." && pwd)"
strata="${STRATA:-$here/target/release/strata-headless}"
instance="$1"
prefix="$("$strata" info -i "$instance" --json | python3 -c "import json,sys; print(json.load(sys.stdin)['prefix'])")"
pfx="$prefix/pfx"

hash_prefix() {
  ( cd "$pfx" && {
      sha256sum system.reg user.reg userdef.reg
      find drive_c/windows/system32 drive_c/windows/syswow64 "drive_c/Program Files/dotnet" -type f -print0 \
        | sort -z | xargs -0 sha256sum
    } | sha256sum | cut -d' ' -f1 )
}

echo "first run"
"$strata" prefix setup -i "$instance" --json > "$here/_stage/prefix-run1.json"
before="$(hash_prefix)"
echo "second run"
"$strata" prefix setup -i "$instance" --json > "$here/_stage/prefix-run2.json"
after="$(hash_prefix)"
python3 - "$here/_stage/prefix-run2.json" <<'EOF'
import json, sys
steps = json.load(open(sys.argv[1]))["steps"]
ran = [s["step"] for s in steps if s["state"] != "already done"]
if ran:
    print("prefix check FAILED: the second run did steps:", ran)
    sys.exit(1)
print(f"  ok   the second run found all {len(steps)} steps done")
EOF
[ "$before" = "$after" ] || { echo "prefix check FAILED: the second run changed the prefix"; exit 1; }
echo "  ok   registry files, system32, syswow64 and dotnet unchanged ($after)"
echo "prefix check ok: $prefix"
