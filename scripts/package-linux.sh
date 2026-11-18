#!/usr/bin/env bash
# Build Laralah for Linux and package it as a .tar.gz and a .deb.
# Build natively on the target architecture (x86_64 or aarch64).
#
#   scripts/package-linux.sh
#
# Output: dist/Laralah_<version>_linux_<arch>.tar.gz and dist/laralah_<version>_<debarch>.deb
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
case "$(uname -m)" in
  x86_64 | amd64) ARCH="x86_64"; DEB_ARCH="amd64" ;;
  aarch64 | arm64) ARCH="aarch64"; DEB_ARCH="arm64" ;;
  *) echo "unsupported Linux architecture: $(uname -m)" >&2; exit 1 ;;
esac

cargo build --release --locked

BIN="target/release/Laralah"
STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING"' EXIT

# Filesystem layout shared by the tarball and the .deb (prefix /usr).
ROOT="$STAGING/root"
install -Dm755 "$BIN" "$ROOT/usr/bin/Laralah"
ln -s Laralah "$ROOT/usr/bin/laralah"
install -Dm644 "assets/linux/Laralah.desktop" "$ROOT/usr/share/applications/Laralah.desktop"
for size in 16 32 48 64 128 256 512; do
  install -Dm644 "assets/linux/icons/hicolor/${size}x${size}/apps/laralah.png" \
    "$ROOT/usr/share/icons/hicolor/${size}x${size}/apps/laralah.png"
done

mkdir -p dist

TAR_NAME="Laralah_${VERSION}_linux_${ARCH}"
mkdir -p "$STAGING/$TAR_NAME"
cp -R "$ROOT/usr/." "$STAGING/$TAR_NAME/"
tar -C "$STAGING" -czf "dist/$TAR_NAME.tar.gz" "$TAR_NAME"

DEB_ROOT="$STAGING/deb"
cp -R "$ROOT" "$DEB_ROOT"
mkdir -p "$DEB_ROOT/DEBIAN"
INSTALLED_KB="$(du -sk "$DEB_ROOT/usr" | cut -f1)"
cat > "$DEB_ROOT/DEBIAN/control" <<EOF
Package: laralah
Version: $VERSION
Section: devel
Priority: optional
Architecture: $DEB_ARCH
Installed-Size: $INSTALLED_KB
Depends: libc6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libvulkan1, libx11-6, libx11-xcb1, libxcb1, libfontconfig1, libfreetype6, libasound2 | libasound2t64
Recommends: php-cli, openssh-client, libsecret-tools
Maintainer: Laralah maintainers
Description: Laravel Desktop REPL & Inspector
EOF
dpkg-deb --build --root-owner-group "$DEB_ROOT" "dist/laralah_${VERSION}_${DEB_ARCH}.deb"

echo "Packaged: dist/$TAR_NAME.tar.gz dist/laralah_${VERSION}_${DEB_ARCH}.deb"
