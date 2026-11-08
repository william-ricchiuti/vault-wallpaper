#!/usr/bin/env bash
# Build "dist/Vault Wallpaper.app": a universal (Apple Silicon + Intel) app bundle.
#
# Usage: packaging/build-app.sh
# Needs: rustup, Xcode command line tools (lipo, iconutil, sips, plutil).
# Output is unsigned; run packaging/sign.sh next.
#
# VW_FEATURES: Cargo features of the app. Unset means `updater`, the direct-download build
# (DMG, Homebrew): the Sparkle updater plus Sparkle.framework in Contents/Frameworks.
# Set it empty (VW_FEATURES=) for the Mac App Store build: no updater, no framework.
# Sparkle is embedded whenever the list contains `updater`.
# Env: VW_SPARKLE_FEED_URL / VW_SPARKLE_PUBLIC_KEY override SUFeedURL / SUPublicEDKey
#      (packaging/sparkle.env has the release values); VW_VERSION overrides the version;
#      VW_BUILD_NUMBER / VW_BUILD_FROM_ANY_COMMIT: see packaging/build-number.sh.
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
VERSION="${VW_VERSION:-}"
[[ -n "$VERSION" ]] || VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' crates/app/Cargo.toml | head -n 1)"
if [[ -z "$VERSION" ]]; then
  echo "error: could not read version from crates/app/Cargo.toml" >&2
  exit 1
fi
# Info.plist wants plain numbers: CFBundleShortVersionString is the semver core
# ("1.0.0-beta.1" -> "1.0.0") and CFBundleVersion a build number that only goes up
# (VW_BUILD_NUMBER, else the commit count, checked by build-number.sh). The full version goes
# in VWVersion, which make-dmg.sh uses for the file name.
SHORT_VERSION="${VERSION%%[-+]*}"
# shellcheck source=build-number.sh
source "$ROOT/packaging/build-number.sh"
vw_build_number "$ROOT" || exit 1
FEATURE_LIST="${VW_FEATURES-updater}"
FEATURES=()
[[ -n "$FEATURE_LIST" ]] && FEATURES=(--features "$FEATURE_LIST")
UPDATER=0
[[ ",${FEATURE_LIST// /,}," == *,updater,* ]] && UPDATER=1
if [[ "$UPDATER" == 1 ]]; then
  # shellcheck source=sparkle.env
  source "$ROOT/packaging/sparkle.env"
  FEED_URL="${VW_SPARKLE_FEED_URL:-$SPARKLE_FEED_URL}"
  PUBLIC_KEY="${VW_SPARKLE_PUBLIC_KEY:-$SPARKLE_PUBLIC_ED_KEY}"
  SPARKLE_DIR="$("$ROOT/packaging/fetch-sparkle.sh" | tail -n 1)"
fi
echo "==> $APP_NAME $VERSION (short $SHORT_VERSION, build $BUILD_NUMBER, $BUNDLE_ID)"

# Install missing rustup targets, and only those.
INSTALLED="$(rustup target list --installed)"
for t in "${TARGETS[@]}"; do
  if ! grep -qx "$t" <<<"$INSTALLED"; then
    echo "==> rustup target $t is missing; installing it (rustup target add $t)"
    rustup target add "$t"
  fi
done

# Build each architecture.
# The 13.0 minimum is enforced by LSMinimumSystemVersion in Info.plist. Don't export
# MACOSX_DEPLOYMENT_TARGET here: with rustc 1.93 + the Xcode 27 linker, setting it together with
# `--target` produces proc-macro dylibs rustc can't load ("E0463 can't find crate for
# `thiserror_impl`"). The binaries keep rustc's default minimums (11.0 arm64, 10.12 x86_64).
unset MACOSX_DEPLOYMENT_TARGET
SLICES=()
for t in "${TARGETS[@]}"; do
  echo "==> cargo build --release -p $BIN_NAME --target $t ${FEATURES[*]+${FEATURES[*]}}"
  cargo build --release --locked -p "$BIN_NAME" --target "$t" ${FEATURES[@]+"${FEATURES[@]}"}
  SLICES+=("${CARGO_TARGET_DIR:-$ROOT/target}/$t/release/$BIN_NAME")
done

rm -rf "$APP" "$WORK"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$WORK"

echo "==> lipo -> universal binary"
lipo -create "${SLICES[@]}" -output "$APP/Contents/MacOS/$BIN_NAME"
lipo -info "$APP/Contents/MacOS/$BIN_NAME"

# Sparkle: the app loads Contents/Frameworks/Sparkle.framework at run time by path
# (crates/app/src/updater.rs), so the binary needs no rpath or link-time dependency, and the
# same framework (itself universal) serves both slices. ditto keeps its symlinks.
SPARKLE_KEYS=""
if [[ "$UPDATER" == 1 ]]; then
  echo "==> Sparkle.framework $SPARKLE_VERSION -> Contents/Frameworks"
  mkdir -p "$APP/Contents/Frameworks"
  ditto "$SPARKLE_DIR/Sparkle.framework" "$APP/Contents/Frameworks/Sparkle.framework"
  if [[ "$PUBLIC_KEY" == REPLACE_WITH_* ]]; then
    echo "warning: SUPublicEDKey is still the placeholder; the updater stays off in this build." >&2
    echo "         Run packaging/sparkle-keys.sh and put the key in packaging/sparkle.env." >&2
  fi
  # SUScheduledCheckInterval is Sparkle's default (one day), written out so it's visible.
  SPARKLE_KEYS="	<key>SUFeedURL</key>
	<string>$FEED_URL</string>
	<key>SUPublicEDKey</key>
	<string>$PUBLIC_KEY</string>
	<key>SUEnableAutomaticChecks</key>
	<true/>
	<key>SUScheduledCheckInterval</key>
	<integer>86400</integer>"
fi

# Licences of everything compiled into the binary, plus Sparkle's (packaging/notices.sh).
cp "$ROOT/THIRD-PARTY-NOTICES.txt" "$APP/Contents/Resources/THIRD-PARTY-NOTICES.txt"

# Icon: PNG -> .iconset -> .icns. The PNG is drawn by
# `cargo run --release -p vw-render --example app_icon -- packaging/icon/icon-1024.png`.
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
	<string>$SHORT_VERSION</string>
	<key>CFBundleVersion</key>
	<string>$BUILD_NUMBER</string>
	<key>VWVersion</key>
	<string>$VERSION</string>
	<key>NSHumanReadableCopyright</key>
	<string>© William Ricchiuti</string>
$ICON_KEY
$SPARKLE_KEYS
	<key>LSMinimumSystemVersion</key>
	<string>$MIN_MACOS</string>
	<key>LSUIElement</key>
	<true/>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.productivity</string>
</dict>
</plist>
PLIST
printf 'APPL????' >"$APP/Contents/PkgInfo"
plutil -lint "$APP/Contents/Info.plist"

rm -rf "$WORK"
echo "==> built: $APP"
