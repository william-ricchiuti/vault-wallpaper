#!/usr/bin/env bash
# Build "dist/Vault Wallpaper.app": a universal (Apple Silicon + Intel) app bundle.
#
# Usage: packaging/build-app.sh
# Needs: rustup, Xcode command line tools (lipo, iconutil, sips, plutil).
# Output is unsigned; run packaging/sign.sh next.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

APP_NAME="Vault Wallpaper"
BUNDLE_ID="com.williamricchiuti.vaultwallpaper"
BIN_NAME="vault-wallpaper"          # cargo package and binary name (crates/app)
MIN_MACOS="13.0"
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

DIST="$ROOT/dist"
APP="$DIST/$APP_NAME.app"
WORK="$DIST/.work"

# Version from crates/app/Cargo.toml (first `version = "..."` line, which is [package]).
VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' crates/app/Cargo.toml | head -n 1)"
if [[ -z "$VERSION" ]]; then
  echo "error: could not read version from crates/app/Cargo.toml" >&2
  exit 1
fi
echo "==> $APP_NAME $VERSION ($BUNDLE_ID)"

# Install missing rustup targets, and only those.
INSTALLED="$(rustup target list --installed)"
for t in "${TARGETS[@]}"; do
  if ! grep -qx "$t" <<<"$INSTALLED"; then
    echo "==> rustup target $t is missing; installing it (rustup target add $t)"
    rustup target add "$t"
  fi
done

# Build each architecture. The deployment target sets the binary's minimum macOS.
export MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS"
SLICES=()
for t in "${TARGETS[@]}"; do
  echo "==> cargo build --release -p $BIN_NAME --target $t"
  cargo build --release --locked -p "$BIN_NAME" --target "$t"
  SLICES+=("${CARGO_TARGET_DIR:-$ROOT/target}/$t/release/$BIN_NAME")
done

rm -rf "$APP" "$WORK"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$WORK"

echo "==> lipo -> universal binary"
lipo -create "${SLICES[@]}" -output "$APP/Contents/MacOS/$BIN_NAME"
lipo -info "$APP/Contents/MacOS/$BIN_NAME"

# Icon: placeholder PNG -> .iconset -> .icns. TODO: replace the placeholder with a designed icon.
ICON_PNG="$ROOT/packaging/icon/icon-1024.png"
ICON_KEY=""
if [[ -f "$ICON_PNG" ]] && command -v iconutil >/dev/null && command -v sips >/dev/null; then
  echo "==> icon"
  SET="$WORK/AppIcon.iconset"
  mkdir -p "$SET"
  for s in 16 32 128 256 512; do
    sips -z "$s" "$s" "$ICON_PNG" --out "$SET/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    sips -z "$d" "$d" "$ICON_PNG" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$SET" -o "$APP/Contents/Resources/AppIcon.icns"
  ICON_KEY="	<key>CFBundleIconFile</key>
	<string>AppIcon</string>"
else
  echo "==> icon: skipped (no $ICON_PNG or no iconutil/sips); the app gets the generic icon"
fi

echo "==> Info.plist"
cat >"$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>$BIN_NAME</string>
	<key>CFBundleIdentifier</key>
	<string>$BUNDLE_ID</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>$APP_NAME</string>
	<key>CFBundleDisplayName</key>
	<string>$APP_NAME</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$VERSION</string>
	<key>CFBundleVersion</key>
	<string>$VERSION</string>
$ICON_KEY
	<key>LSMinimumSystemVersion</key>
	<string>$MIN_MACOS</string>
	<key>LSUIElement</key>
	<true/>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.utilities</string>
</dict>
</plist>
PLIST
printf 'APPL????' >"$APP/Contents/PkgInfo"
plutil -lint "$APP/Contents/Info.plist"

rm -rf "$WORK"
echo "==> built: $APP"
