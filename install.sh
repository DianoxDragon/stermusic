#!/usr/bin/env bash
# Builds SterMusic in release mode and installs it for the current user (~/.local/bin, icon
# and application menu entry). No sudo needed.
# On Arch Linux and its derivatives, the pacman package is the better choice:
#     ./packaging/make-arch-package.sh -si
set -euo pipefail
cd "$(dirname "$0")"

export PATH="$HOME/.cargo/bin:$PATH"

# Messages in French on a French desktop, in English otherwise.
case "${LC_ALL:-${LC_MESSAGES:-${LANG:-}}}" in
    fr*) fr=1 ;;
    *) fr=0 ;;
esac
say() { if [ "$fr" = 1 ]; then echo "$1"; else echo "$2"; fi; }

# Command installing the prerequisites, depending on the distribution (Arch, Fedora, openSUSE,
# Debian…).
. /etc/os-release 2>/dev/null || true
case " ${ID:-} ${ID_LIKE:-} " in
    *" arch "*)                PKG_ALSA="sudo pacman -S --needed alsa-lib pkgconf"; PKG_RUST="sudo pacman -S --needed rust" ;;
    *" fedora "*|*" rhel "*)   PKG_ALSA="sudo dnf install alsa-lib-devel";          PKG_RUST="sudo dnf install cargo" ;;
    *" suse "*|*" opensuse"*)  PKG_ALSA="sudo zypper install alsa-devel";           PKG_RUST="sudo zypper install cargo" ;;
    *)                         PKG_ALSA="sudo apt install libasound2-dev";          PKG_RUST="curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" ;;
esac

if ! command -v cargo >/dev/null; then
    say "Il manque Rust (cargo). Lance d'abord :" "Rust (cargo) is missing. Run this first:"
    echo "    $PKG_RUST"
    exit 1
fi
if ! pkg-config --exists alsa; then
    say "Il manque la bibliothèque ALSA de développement. Lance d'abord :" \
        "The ALSA development library is missing. Run this first:"
    echo "    $PKG_ALSA"
    exit 1
fi

cargo build --release

TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"

BIN="$HOME/.local/bin/stermusic"
install -Dm755 "$TARGET_DIR/release/stermusic" "$BIN"
install -Dm644 assets/icon.png "$HOME/.local/share/icons/hicolor/256x256/apps/stermusic.png"
# Full path in the launcher: the menu works even if ~/.local/bin is not (yet) in the PATH.
mkdir -p "$HOME/.local/share/applications"
sed "s|^Exec=stermusic|Exec=$BIN|" stermusic.desktop > "$HOME/.local/share/applications/stermusic.desktop"
# Right click on tracks or a folder in Dolphin: "Edit tags with SterMusic".
SERVICE_MENU="$HOME/.local/share/kio/servicemenus/stermusic-tags.desktop"
mkdir -p "$(dirname "$SERVICE_MENU")"
sed "s|^Exec=stermusic|Exec=$BIN|" stermusic-tags.desktop > "$SERVICE_MENU"
chmod +x "$SERVICE_MENU"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
# Refreshes the KDE Plasma menu right away.
for tool in kbuildsycoca6 kbuildsycoca5; do
    if command -v "$tool" >/dev/null; then
        "$tool" >/dev/null 2>&1 || true
        break
    fi
done

say "SterMusic est installé : cherche-le dans le menu des applications, ou lance :" \
    "SterMusic is installed: find it in the application menu, or run:"
echo "    $BIN"
case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *)
        echo
        say "Note : ~/.local/bin n'est pas encore dans ton PATH (il le sera à ta prochaine session)." \
            "Note: ~/.local/bin is not in your PATH yet (it will be at your next login)."
        say "Pour la commande « stermusic » dès maintenant dans ce terminal :" \
            "To get the \"stermusic\" command in this terminal right now:"
        echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
esac
