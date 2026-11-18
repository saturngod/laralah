#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"

cd "$ROOT_DIR"
cargo build --release

install -Dm755 "target/release/Laralah" "$PREFIX/bin/Laralah"
ln -sf "$PREFIX/bin/Laralah" "$PREFIX/bin/laralah"
install -Dm644 "assets/linux/Laralah.desktop" "$PREFIX/share/applications/Laralah.desktop"

for size in 16 32 48 64 128 256 512; do
  install -Dm644 \
    "assets/linux/icons/hicolor/${size}x${size}/apps/laralah.png" \
    "$PREFIX/share/icons/hicolor/${size}x${size}/apps/laralah.png"
done

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" || true
fi

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$PREFIX/share/applications" || true
fi

echo "Installed Laralah binary, desktop entry, and hicolor icons to: $PREFIX"
