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

BIN="$HOME/.local/bin/stermusic"
install -Dm755 "$TARGET_DIR/release/stermusic" "$BIN"
install -Dm644 assets/icon.png "$HOME/.local/share/icons/hicolor/256x256/apps/stermusic.png"
# Chemin complet dans le lanceur : le menu marche même si ~/.local/bin n'est pas (encore) dans le PATH.
mkdir -p "$HOME/.local/share/applications"
sed "s|^Exec=stermusic|Exec=$BIN|" stermusic.desktop > "$HOME/.local/share/applications/stermusic.desktop"
# Clic droit dans Dolphin sur des morceaux ou un dossier : « Modifier les tags avec SterMusic ».
SERVICE_MENU="$HOME/.local/share/kio/servicemenus/stermusic-tags.desktop"
mkdir -p "$(dirname "$SERVICE_MENU")"
sed "s|^Exec=stermusic|Exec=$BIN|" stermusic-tags.desktop > "$SERVICE_MENU"
chmod +x "$SERVICE_MENU"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
# Rafraîchit le menu de KDE Plasma tout de suite.
for tool in kbuildsycoca6 kbuildsycoca5; do
    if command -v "$tool" >/dev/null; then
        "$tool" >/dev/null 2>&1 || true
        break
    fi
done

echo "SterMusic est installé : cherche-le dans le menu des applications, ou lance :"
echo "    $BIN"
case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *)
        echo
        echo "Note : ~/.local/bin n'est pas encore dans ton PATH (il le sera à ta prochaine session)."
        echo "Pour la commande « stermusic » dès maintenant dans ce terminal :"
        echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
esac
