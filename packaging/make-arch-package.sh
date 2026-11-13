#!/usr/bin/env bash
# Builds the Arch Linux package of SterMusic (EndeavourOS, Manjaro, CachyOS…) from the code of
# the checkout as it is (uncommitted changes included), with packaging/arch/PKGBUILD.
# The package is copied to dist/: stermusic-<version>-<rel>-x86_64.pkg.tar.zst.
#
# Options are passed on to makepkg, for example:
#   ./packaging/make-arch-package.sh -si    # also installs the missing dependencies and the package
# To install a package already built: sudo pacman -U dist/stermusic-….pkg.tar.zst
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"

if ! command -v makepkg >/dev/null; then
    echo "makepkg not found: this script is for Arch Linux and its derivatives."
    exit 1
fi

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

# Built outside the project (NTFS disk: Unix permissions and build scripts don't hold up there).
BUILD="${XDG_CACHE_HOME:-$HOME/.cache}/stermusic-arch"
rm -rf "$BUILD"
mkdir -p "$BUILD" "$ROOT/dist"
sed "s/^pkgver=.*/pkgver=$VERSION/" packaging/arch/PKGBUILD > "$BUILD/PKGBUILD"

# Archive of the local tree, named like the one the PKGBUILD downloads: makepkg takes it as is
# (--skipchecksums, since it differs from the GitHub one).
git ls-files -z --cached --others --exclude-standard --deduplicate \
    | tar --null --files-from=- --ignore-failed-read \
          --transform "s,^,stermusic-$VERSION/," --owner=0 --group=0 \
          -czf "$BUILD/stermusic-$VERSION.tar.gz"

cd "$BUILD"
makepkg -f --skipchecksums "$@"

for pkg in $(makepkg --packagelist); do
    [ -f "$pkg" ] || continue
    cp -f "$pkg" "$ROOT/dist/"
    echo "Package ready: $ROOT/dist/$(basename "$pkg")"
done
