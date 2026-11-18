#!/usr/bin/env bash
# Build Laralah.app and a .dmg for one macOS architecture.
#
#   scripts/package-macos.sh                          # host architecture
#   scripts/package-macos.sh aarch64-apple-darwin     # Apple Silicon
#   scripts/package-macos.sh x86_64-apple-darwin      # Intel
#
# Signing: set APPLE_SIGNING_IDENTITY to a "Developer ID Application: ..." identity
# already in the keychain. Notarization: also set APPLE_ID, APPLE_PASSWORD
# (app-specific password) and APPLE_TEAM_ID. With REQUIRE_SIGNING=1 (release builds)
# a missing identity or notarization credential is an error; otherwise the bundle
# falls back to an ad-hoc signature for local testing.
#
# Output: dist/Laralah_<version>_<arch>.dmg
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

TARGET="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
case "$TARGET" in
  aarch64-apple-darwin) ARCH="aarch64" ;;
  x86_64-apple-darwin) ARCH="x64" ;;
  *) echo "unsupported macOS target: $TARGET" >&2; exit 1 ;;
esac

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

cargo build --release --locked --target "$TARGET"

BUILD_DIR="target/$TARGET/release"
APP_BUNDLE="$BUILD_DIR/Laralah.app"
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"

cp "$BUILD_DIR/Laralah" "$APP_BUNDLE/Contents/MacOS/Laralah"
cp "assets/icons/laralah.icns" "$APP_BUNDLE/Contents/Resources/laralah.icns"
cp "assets/macos/Info.plist" "$APP_BUNDLE/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$APP_BUNDLE/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $VERSION" "$APP_BUNDLE/Contents/Info.plist"

IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
NOTARIZE=""
[[ -n "${APPLE_ID:-}" && -n "${APPLE_PASSWORD:-}" && -n "${APPLE_TEAM_ID:-}" ]] && NOTARIZE=1
if [[ -n "${REQUIRE_SIGNING:-}" && ( -z "$IDENTITY" || -z "$NOTARIZE" ) ]]; then
  echo "REQUIRE_SIGNING is set but APPLE_SIGNING_IDENTITY / APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID are incomplete" >&2
  exit 1
fi
if [[ -n "$IDENTITY" ]]; then
  codesign --force --options runtime --timestamp --sign "$IDENTITY" "$APP_BUNDLE"
else
  codesign --force --sign - "$APP_BUNDLE"
fi
codesign --verify --strict "$APP_BUNDLE"

mkdir -p dist
DMG="dist/Laralah_${VERSION}_${ARCH}.dmg"
STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING"' EXIT
cp -R "$APP_BUNDLE" "$STAGING/"
ln -s /Applications "$STAGING/Applications"
rm -f "$DMG"
hdiutil create -volname "Laralah" -srcfolder "$STAGING" -ov -format UDZO "$DMG"

if [[ -n "$IDENTITY" ]]; then
  codesign --force --timestamp --sign "$IDENTITY" "$DMG"
  if [[ -n "$NOTARIZE" ]]; then
    xcrun notarytool submit "$DMG" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
      --team-id "$APPLE_TEAM_ID" --wait
    xcrun stapler staple "$DMG"
    xcrun stapler validate "$DMG"
    spctl --assess --type open --context context:primary-signature --verbose "$DMG"
  fi
fi

echo "Packaged: $DMG"
