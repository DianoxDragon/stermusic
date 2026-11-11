#!/usr/bin/env bash
# Fabrique les installateurs de SterMusic à partager, dans dist/ :
#   - stermusic_<version>_amd64.deb : Kubuntu, Ubuntu, Debian, Mint… (double-clic → Discover,
#     ou « sudo apt install ./stermusic_….deb ») ;
#   - stermusic-<version>-x86_64.tar.gz : toute autre distribution, installation sans sudo
#     (./install.sh dans le dossier décompressé).
# Le binaire demande glibc 2.35 ou plus (Ubuntu 22.04, Debian 12, Fedora 36…) : voir src/compat.rs.
#
# Variables facultatives : MAINTAINER="Nom <adresse>" (champ « Maintainer » du paquet .deb),
# HOMEPAGE="https://…" (adresse du dépôt, où trouver le code source).
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
export PATH="$HOME/.cargo/bin:$PATH"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
MAINTAINER="${MAINTAINER:-SterMusic <stermusic@localhost>}"
HOMEPAGE="${HOMEPAGE:-}"

cargo build --release
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
BIN="$TARGET_DIR/release/stermusic"

# Garde-fou : aucune version de glibc au-delà de 2.35 ne doit être exigée (hors références faibles).
TOO_NEW="$(objdump -T "$BIN" | awk '$2 != "w" && /GLIBC_2\.(3[6-9]|[4-9][0-9])/' || true)"
if [ -n "$TOO_NEW" ]; then
    echo "Le binaire exige une glibc trop récente pour être partagé :"
    echo "$TOO_NEW"
    echo "Ajouter les fonctions en cause à src/compat.rs."
    exit 1
fi

# Le paquet est assemblé hors du projet (disque NTFS : les droits Unix n'y tiennent pas).
STAGE="$TARGET_DIR/package"
rm -rf "$STAGE"
mkdir -p "$STAGE" "$ROOT/dist"

# Licences des bibliothèques compilées dans le programme (à joindre au binaire).
python3 packaging/third_party.py "$STAGE/THIRD-PARTY-LICENSES.txt"

# ---------- Paquet .deb ----------
DEB="$STAGE/deb"
install -Dm755 "$BIN" "$DEB/usr/bin/stermusic"
install -Dm644 stermusic.desktop "$DEB/usr/share/applications/stermusic.desktop"
install -Dm644 assets/icon.png "$DEB/usr/share/icons/hicolor/256x256/apps/stermusic.png"
# Clic droit dans Dolphin : « Modifier les tags avec SterMusic ».
install -Dm644 stermusic-tags.desktop "$DEB/usr/share/kio/servicemenus/stermusic-tags.desktop"
install -Dm644 /dev/stdin "$DEB/usr/share/doc/stermusic/copyright" <<COPYRIGHT
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: SterMusic
${HOMEPAGE:+Source: $HOMEPAGE}

Files: *
Copyright: Dianox
License: GPL-3+
 This program is free software: you can redistribute it and/or modify it under the terms of the
 GNU General Public License as published by the Free Software Foundation, either version 3 of the
 License, or (at your option) any later version.
 .
 On Debian systems, the full text of the GNU General Public License version 3 can be found in
 /usr/share/common-licenses/GPL-3.

Files: assets/earth.jpg
Copyright: NASA
License: public-domain

Files: assets/cities.tsv
Copyright: GeoNames
License: CC-BY-4.0

Files: assets/icons/*
Copyright: Cole Bemis (Feather), Lucide Contributors
License: ISC

Comment: The Rust libraries compiled into the program, with their license texts, are listed in
 /usr/share/doc/stermusic/THIRD-PARTY-LICENSES.txt.
COPYRIGHT
install -Dm644 CREDITS.md "$DEB/usr/share/doc/stermusic/CREDITS.md"
install -Dm644 "$STAGE/THIRD-PARTY-LICENSES.txt" "$DEB/usr/share/doc/stermusic/THIRD-PARTY-LICENSES.txt"
SIZE_KB="$(du -sk "$DEB/usr" | cut -f1)"
mkdir -p "$DEB/DEBIAN"
cat > "$DEB/DEBIAN/control" <<CONTROL
Package: stermusic
Version: $VERSION
Architecture: amd64
Maintainer: $MAINTAINER
Installed-Size: $SIZE_KB
Depends: libc6 (>= 2.35), libasound2t64 | libasound2
Recommends: ffmpeg, pulseaudio-utils
Suggests: kio-extras
${HOMEPAGE:+Homepage: $HOMEPAGE
}Section: sound
Priority: optional
Description: music player for Linux (Windows 11 look, KDE integration)
 SterMusic plays your music library (MP3, FLAC, OGG, Opus, M4A…), web radios
 and podcasts, with 50 GPU visualizers, synced lyrics, a tag editor,
 ReplayGain, smart playlists, Subsonic/Jellyfin servers and Chromecast/DLNA
 casting. Interface in French and English.
 .
 ffmpeg adds Opus, WMA, APE, WavPack and DSD playback, conversion and CD
 ripping; pactl (pulseaudio-utils) lists the audio outputs.
CONTROL
DEB_FILE="$ROOT/dist/stermusic_${VERSION}_amd64.deb"
dpkg-deb --root-owner-group -Zxz --build "$DEB" "$DEB_FILE" >/dev/null

# ---------- Archive portable ----------
NAME="stermusic-$VERSION-x86_64"
PORT="$STAGE/$NAME"
install -Dm755 "$BIN" "$PORT/stermusic"
install -Dm644 assets/icon.png "$PORT/stermusic.png"
install -Dm644 stermusic.desktop "$PORT/stermusic.desktop"
install -Dm644 stermusic-tags.desktop "$PORT/stermusic-tags.desktop"
install -Dm755 packaging/portable-install.sh "$PORT/install.sh"
install -Dm755 packaging/portable-uninstall.sh "$PORT/uninstall.sh"
install -Dm644 packaging/README.txt "$PORT/README.txt"
install -Dm644 LICENSE "$PORT/LICENSE"
install -Dm644 CREDITS.md "$PORT/CREDITS.md"
install -Dm644 "$STAGE/THIRD-PARTY-LICENSES.txt" "$PORT/THIRD-PARTY-LICENSES.txt"
TAR_FILE="$ROOT/dist/$NAME.tar.gz"
tar -C "$STAGE" --owner=0 --group=0 -czf "$TAR_FILE" "$NAME"

echo "Installateurs prêts :"
for f in "$DEB_FILE" "$TAR_FILE"; do
    echo "    $(du -h "$f" | cut -f1)  $f"
done
