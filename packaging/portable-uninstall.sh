#!/usr/bin/env bash
# Retire SterMusic installé par install.sh. Tes réglages, playlists et statistiques restent dans
# ~/.local/share/stermusic (supprime ce dossier toi-même pour tout effacer).
set -euo pipefail
rm -f "$HOME/.local/bin/stermusic" \
      "$HOME/.local/share/icons/hicolor/256x256/apps/stermusic.png" \
      "$HOME/.local/share/applications/stermusic.desktop" \
      "$HOME/.local/share/kio/servicemenus/stermusic-tags.desktop"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
for tool in kbuildsycoca6 kbuildsycoca5; do
    if command -v "$tool" >/dev/null; then
        "$tool" >/dev/null 2>&1 || true
        break
    fi
done
echo "SterMusic est désinstallé. Tes réglages sont gardés dans ~/.local/share/stermusic."
