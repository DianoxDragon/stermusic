#!/usr/bin/env bash
# Prepares the AUR package of the current version, once its tag (v<version>) is pushed to
# GitHub: downloads the release tarball, writes its checksum into a copy of
# packaging/arch/PKGBUILD and generates .SRCINFO, in dist/<version>/aur/.
#
# Publishing then happens in a clone of the AUR repository (an AUR account with an SSH key is
# needed; the first push creates the package):
#   git clone ssh://aur@aur.archlinux.org/stermusic.git
#   cp dist/<version>/aur/{PKGBUILD,.SRCINFO} stermusic/
#   cd stermusic && git add PKGBUILD .SRCINFO && git commit -m "Update to <version>" && git push
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"

if ! command -v makepkg >/dev/null; then
    echo "makepkg not found: this script is for Arch Linux and its derivatives."
    exit 1
fi

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
URL="$(sed -n 's/^url="\(.*\)"/\1/p' packaging/arch/PKGBUILD)"
TARBALL="$URL/archive/refs/tags/v$VERSION.tar.gz"

OUT="$ROOT/dist/$VERSION/aur"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if ! curl -fsSL -o "$WORK/source.tar.gz" "$TARBALL"; then
    echo "Cannot download $TARBALL: push the v$VERSION tag to GitHub first."
    exit 1
fi
SUM="$(sha256sum "$WORK/source.tar.gz" | cut -d' ' -f1)"

mkdir -p "$OUT"
sed -e "s/^pkgver=.*/pkgver=$VERSION/" \
    -e "s/^sha256sums=.*/sha256sums=('$SUM')/" \
    packaging/arch/PKGBUILD > "$OUT/PKGBUILD"
(cd "$OUT" && makepkg --printsrcinfo > .SRCINFO)

echo "AUR files ready in $OUT (sha256 $SUM)."
