#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

cargo build --release

APP_BUNDLE="target/release/Laralah.app"
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"

cp "target/release/Laralah" "$APP_BUNDLE/Contents/MacOS/Laralah"
cp "assets/icons/laralah.icns" "$APP_BUNDLE/Contents/Resources/laralah.icns"
cp "assets/macos/Info.plist" "$APP_BUNDLE/Contents/Info.plist"

echo "Packaged macOS app bundle at: $APP_BUNDLE"
