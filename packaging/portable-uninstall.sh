#!/usr/bin/env bash
# Removes SterMusic installed by install.sh. Settings, playlists and statistics stay in
# ~/.local/share/stermusic (delete that folder yourself to erase everything).
set -euo pipefail
case "${LC_ALL:-${LC_MESSAGES:-${LANG:-}}}" in
    fr*) fr=1 ;;
    *) fr=0 ;;
esac
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
if [ "$fr" = 1 ]; then
    echo "SterMusic est désinstallé. Tes réglages sont gardés dans ~/.local/share/stermusic."
else
    echo "SterMusic is uninstalled. Your settings are kept in ~/.local/share/stermusic."
fi
