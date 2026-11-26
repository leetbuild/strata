#!/usr/bin/env bash
# Builds StrataProbe.dll (a Windows x64 DLL) with clang-cl and the xwin sysroot (the MSVC
# CRT and Windows SDK for clang). No vcpkg, no CommonLibSSE: one source file.
#   tools/probe/build.sh            -> _stage/probe/StrataProbe.dll      (Skyrim SE, x64)
#   tools/probe/build.sh x86        -> _stage/probe/x86/StrataProbe.dll  (Skyrim LE, 32-bit)
# XWIN_SYSROOT overrides ~/.local/share/xwin/sysroot (x64) or ~/.local/share/xwin/sysroot-x86
# (made by `xwin --accept-license --arch x86 splat --use-winsysroot-style --output <it>`).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
arch="${1:-x64}"
case "$arch" in
  x64) target=x86_64-pc-windows-msvc; default_sysroot="$HOME/.local/share/xwin/sysroot"; out_sub="" ;;
  x86) target=i686-pc-windows-msvc; default_sysroot="$HOME/.local/share/xwin/sysroot-x86"; out_sub="/x86" ;;
  *) echo "usage: build.sh [x64|x86]"; exit 2 ;;
esac
sysroot="${XWIN_SYSROOT:-$default_sysroot}"
[ -d "$sysroot/VC/Tools/MSVC" ] || { echo "xwin sysroot not found at $sysroot (set XWIN_SYSROOT)"; exit 2; }
out="$root/_stage/probe$out_sub"
mkdir -p "$out"
cd "$out"
clang-cl --target="$target" /winsysroot "$sysroot" \
  -fuse-ld=lld-link /nologo /std:c++20 /O2 /MT /EHsc /W4 /LD "$here/probe.cpp" \
  /Fe:StrataProbe.dll /Fo:probe.obj \
  /link /winsysroot:"$sysroot" /MACHINE:"$arch" /DLL /NOLOGO /NOIMPLIB
echo "built $out/StrataProbe.dll"
