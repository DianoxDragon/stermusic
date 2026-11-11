#!/usr/bin/env bash
# Installe SterMusic pour l'utilisateur courant, sans sudo : ~/.local/bin, icône, entrée dans le
# menu des applications et, sous KDE, « Modifier les tags avec SterMusic » dans Dolphin.
set -euo pipefail
cd "$(dirname "$0")"

BIN="$HOME/.local/bin/stermusic"
install -Dm755 stermusic "$BIN"
install -Dm644 stermusic.png "$HOME/.local/share/icons/hicolor/256x256/apps/stermusic.png"
# Chemin complet dans le lanceur : le menu marche même si ~/.local/bin n'est pas dans le PATH.
mkdir -p "$HOME/.local/share/applications"
sed "s|^Exec=stermusic|Exec=$BIN|" stermusic.desktop > "$HOME/.local/share/applications/stermusic.desktop"
SERVICE_MENU="$HOME/.local/share/kio/servicemenus/stermusic-tags.desktop"
mkdir -p "$(dirname "$SERVICE_MENU")"
sed "s|^Exec=stermusic|Exec=$BIN|" stermusic-tags.desktop > "$SERVICE_MENU"
chmod +x "$SERVICE_MENU"

update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
for tool in kbuildsycoca6 kbuildsycoca5; do
    if command -v "$tool" >/dev/null; then
        "$tool" >/dev/null 2>&1 || true
        break
    fi
done

echo "SterMusic est installé : cherche-le dans le menu des applications, ou lance :"
echo "    $BIN"
if ! command -v ffmpeg >/dev/null; then
    echo
    echo "Conseil : installe ffmpeg pour lire l'Opus, le WMA, l'APE, le DSD… et pour convertir."
fi
