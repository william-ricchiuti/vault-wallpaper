#!/usr/bin/env bash
# Make a compressed DMG holding the app and an /Applications shortcut.
#
# Usage: packaging/make-dmg.sh [app]
# Output: dist/Vault-Wallpaper-<version>.dmg
# Needs: hdiutil (built in). If VW_SIGN_IDENTITY is set, the DMG is signed too.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="${1:-$ROOT/dist/Vault Wallpaper.app}"
VOLNAME="Vault Wallpaper"

if [[ ! -d "$APP" ]]; then
  echo "error: $APP not found; run packaging/build-app.sh first" >&2
  exit 1
fi

VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist")"
DMG="$ROOT/dist/Vault-Wallpaper-$VERSION.dmg"
STAGE="$(mktemp -d "${TMPDIR:-/tmp}/vw-dmg.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT

echo "==> staging"
# ditto keeps the code signature, extended attributes and symlinks intact.
ditto "$APP" "$STAGE/$(basename "$APP")"
ln -s /Applications "$STAGE/Applications"

echo "==> hdiutil create $DMG"
rm -f "$DMG"
hdiutil create -volname "$VOLNAME" -srcfolder "$STAGE" -fs HFS+ \
  -format UDZO -imagekey zlib-level=9 -ov "$DMG" >/dev/null

if [[ -n "${VW_SIGN_IDENTITY:-}" ]]; then
  echo "==> signing the DMG"
  codesign --force --timestamp --sign "$VW_SIGN_IDENTITY" "$DMG"
fi

hdiutil verify "$DMG" >/dev/null
echo "==> built: $DMG ($(du -h "$DMG" | cut -f1))"
