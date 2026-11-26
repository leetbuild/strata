#!/usr/bin/env bash
# Installs Strata's desktop entry and icon for the current user, so launchers and the
# compositor show the Strata badge (the window's Wayland app id is "strata"):
#   ~/.local/share/applications/strata.desktop
#   ~/.local/share/icons/hicolor/{512x512,256x256,128x128,64x64,48x48,32x32}/apps/strata.png
#   ~/.local/share/strata/bin/strata -> build/strata (a path without spaces: xdg tools split Exec on them)
# Run again after moving the repository. Needs ImageMagick (magick) for the smaller sizes.
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
[ -x build/strata ] || { echo "build/strata not built (tools/build.sh)"; exit 1; }

mkdir -p "$data/strata/bin"
ln -sfn "$repo/build/strata" "$data/strata/bin/strata"

for size in 512 256 128 64 48 32; do
  dir="$data/icons/hicolor/${size}x${size}/apps"
  mkdir -p "$dir"
  magick data/icons/strata-512.png -resize "${size}x${size}" "$dir/strata.png"
done

mkdir -p "$data/applications"
cat > "$data/applications/strata.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Strata
GenericName=Mod Manager
Comment=Linux-native mod manager (MO2 2.5.2 + Leet Edition)
Exec=$data/strata/bin/strata
Icon=strata
Terminal=false
Categories=Game;
StartupWMClass=strata
EOF
gtk-update-icon-cache -q -t "$data/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$data/applications" 2>/dev/null || true
echo "installed: $data/applications/strata.desktop, icon 'strata' (hicolor), $data/strata/bin/strata"
