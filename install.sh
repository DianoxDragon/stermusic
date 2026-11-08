#!/usr/bin/env bash
# Compile SterMusic en mode release et l'installe pour l'utilisateur courant
# (~/.local/bin, icône et entrée dans le menu des applications). Aucun sudo nécessaire.
set -euo pipefail
cd "$(dirname "$0")"

export PATH="$HOME/.cargo/bin:$PATH"

if ! pkg-config --exists alsa; then
    echo "Il manque la bibliothèque ALSA de développement. Lance d'abord :"
    echo "    sudo apt install libasound2-dev"
    exit 1
fi

cargo build --release

TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"

install -Dm755 "$TARGET_DIR/release/stermusic" "$HOME/.local/bin/stermusic"
install -Dm644 assets/icon.png "$HOME/.local/share/icons/hicolor/256x256/apps/stermusic.png"
install -Dm644 stermusic.desktop "$HOME/.local/share/applications/stermusic.desktop"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "SterMusic est installé : lance « stermusic » ou cherche-le dans le menu des applications."
