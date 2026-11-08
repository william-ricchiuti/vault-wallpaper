#!/usr/bin/env bash
# Build the Mac App Store flavour: a universal, sandboxed, App-Store-signed
# "dist/appstore/Vault Wallpaper.app" and the installer package App Store Connect takes.
#
# Usage:
#   packaging/appstore/build.sh --dry-run                       # local test: ad-hoc signature, unsigned .pkg
#                                                               # (same as VW_APPSTORE_ADHOC=1)
#   VW_TEAM_ID=ABCDE12345 \
#   VW_APP_IDENTITY="Apple Distribution: Name (ABCDE12345)" \
#   VW_INSTALLER_IDENTITY="3rd Party Mac Developer Installer: Name (ABCDE12345)" \
#   VW_PROVISIONING_PROFILE=~/Downloads/Vault_Wallpaper_Mac_App_Store.provisionprofile \
#   packaging/appstore/build.sh                                 # real upload build
#
# Needs: rustup, Xcode command line tools (lipo, iconutil, sips, plutil, codesign, productbuild).
# The app is built with --no-default-features: no self-updater (the `updater` feature,
# Sparkle) in an App Store build; the store updates it.
# Uploading is a separate, manual step (Transporter or `xcrun altool`); see packaging/appstore/README.md.
# The last line on stdout is the .pkg path (for packaging/release.sh); everything else is progress.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
cd "$ROOT"

APP_NAME="Vault Wallpaper"
BUNDLE_ID="com.williamricchiuti.vaultwallpaper"
BIN_NAME="vault-wallpaper"
MIN_MACOS="13.0"
CATEGORY="public.app-category.productivity"
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)

OUT="$ROOT/dist/appstore"
APP="$OUT/$APP_NAME.app"
WORK="$OUT/.work"
ENTITLEMENTS="$HERE/entitlements.plist"

ADHOC="${VW_APPSTORE_ADHOC:-}"
for arg in "$@"; do
  case "$arg" in
    --dry-run) ADHOC=1 ;;
    *) echo "unknown argument: $arg (only --dry-run)" >&2; exit 64 ;;
  esac
done
if [[ -z "$ADHOC" ]]; then
  missing=()
  for v in VW_TEAM_ID VW_APP_IDENTITY VW_INSTALLER_IDENTITY VW_PROVISIONING_PROFILE; do
    [[ -n "${!v:-}" ]] || missing+=("$v")
  done
  if ((${#missing[@]})); then
    cat >&2 <<MSG
Not set: ${missing[*]}. Nothing was built.

An App Store build needs (see packaging/appstore/README.md):
  VW_TEAM_ID               your 10-character team ID
  VW_APP_IDENTITY          "Apple Distribution: Name (TEAMID)"
                           (or the older "3rd Party Mac Developer Application: Name (TEAMID)")
  VW_INSTALLER_IDENTITY    "3rd Party Mac Developer Installer: Name (TEAMID)"
  VW_PROVISIONING_PROFILE  path to the Mac App Store provisioning profile for $BUNDLE_ID

Find the identities with:  security find-identity -v
For a local sandbox test instead:  VW_APPSTORE_ADHOC=1 packaging/appstore/build.sh
MSG
    exit 2
  fi
  if [[ ! -f "$VW_PROVISIONING_PROFILE" ]]; then
    echo "error: provisioning profile not found: $VW_PROVISIONING_PROFILE" >&2
    exit 1
  fi
fi

VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' crates/app/Cargo.toml | head -n 1)"
[[ -n "$VERSION" ]] || { echo "error: could not read version from crates/app/Cargo.toml" >&2; exit 1; }
SHORT_VERSION="${VERSION%%[-+]*}"
# App Store Connect rejects an upload whose CFBundleVersion isn't higher than the last one
# for the same CFBundleShortVersionString. The commit count only goes up.
BUILD_NUMBER="${VW_BUILD_NUMBER:-$(git -C "$ROOT" rev-list --count HEAD 2>/dev/null || echo 1)}"
echo "==> $APP_NAME $VERSION for the App Store (short $SHORT_VERSION, build $BUILD_NUMBER)"

INSTALLED="$(rustup target list --installed)"
for t in "${TARGETS[@]}"; do
  if ! grep -qx "$t" <<<"$INSTALLED"; then
    echo "==> rustup target $t is missing; installing it (rustup target add $t)"
    rustup target add "$t"
  fi
done

# See packaging/build-app.sh: MACOSX_DEPLOYMENT_TARGET + --target breaks proc macros here.
unset MACOSX_DEPLOYMENT_TARGET
SLICES=()
for t in "${TARGETS[@]}"; do
  echo "==> cargo build --release --no-default-features -p $BIN_NAME --target $t"
  cargo build --release --locked --no-default-features -p "$BIN_NAME" --target "$t"
  SLICES+=("${CARGO_TARGET_DIR:-$ROOT/target}/$t/release/$BIN_NAME")
done

rm -rf "$APP" "$WORK"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$WORK"

lipo -create "${SLICES[@]}" -output "$APP/Contents/MacOS/$BIN_NAME"
lipo -info "$APP/Contents/MacOS/$BIN_NAME"

echo "==> icon"
ICON_PNG="$ROOT/packaging/icon/icon-1024.png"
SET="$WORK/AppIcon.iconset"
mkdir -p "$SET"
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" "$ICON_PNG" --out "$SET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z "$d" "$d" "$ICON_PNG" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$SET" -o "$APP/Contents/Resources/AppIcon.icns"

cp "$HERE/PrivacyInfo.xcprivacy" "$APP/Contents/Resources/PrivacyInfo.xcprivacy"

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
	<key>CFBundleIconFile</key>
	<string>AppIcon</string>
	<key>CFBundleSupportedPlatforms</key>
	<array>
		<string>MacOSX</string>
	</array>
	<key>NSHumanReadableCopyright</key>
	<string>© William Ricchiuti</string>
	<key>LSMinimumSystemVersion</key>
	<string>$MIN_MACOS</string>
	<key>LSUIElement</key>
	<true/>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>LSApplicationCategoryType</key>
	<string>$CATEGORY</string>
	<!-- No encryption beyond what macOS itself does: no network, no crypto code. -->
	<key>ITSAppUsesNonExemptEncryption</key>
	<false/>
</dict>
</plist>
PLIST
printf 'APPL????' >"$APP/Contents/PkgInfo"
plutil -lint "$APP/Contents/Info.plist"

# Extended attributes in the bundle become "._" AppleDouble files in the package payload,
# which App Store validation can reject. Clear what can be cleared before signing.
export COPYFILE_DISABLE=1
xattr -cr "$APP" 2>/dev/null || true

PKG="$OUT/Vault-Wallpaper-$SHORT_VERSION-$BUILD_NUMBER.pkg"
rm -f "$PKG"
if [[ -n "$ADHOC" ]]; then
  echo "==> ad-hoc signing with the sandbox entitlements (local testing only; not uploadable)"
  codesign --force --sign - --entitlements "$ENTITLEMENTS" "$APP"
  codesign --verify --strict --verbose=2 "$APP"
  echo "==> unsigned installer package (structure check only)"
  productbuild --component "$APP" /Applications "$PKG"
else
  echo "==> embedding the provisioning profile"
  cp "$VW_PROVISIONING_PROFILE" "$APP/Contents/embedded.provisionprofile"
  # The signed entitlements must match the profile: add the application and team ids.
  SIGNED_ENT="$WORK/entitlements.plist"
  cp "$ENTITLEMENTS" "$SIGNED_ENT"
  # PlistBuddy, not plutil: plutil reads the dots in these keys as a key path.
  /usr/libexec/PlistBuddy -c "Add :com.apple.application-identifier string $VW_TEAM_ID.$BUNDLE_ID" "$SIGNED_ENT"
  /usr/libexec/PlistBuddy -c "Add :com.apple.developer.team-identifier string $VW_TEAM_ID" "$SIGNED_ENT"
  echo "==> signing the app: $VW_APP_IDENTITY"
  codesign --force --timestamp --options runtime --entitlements "$SIGNED_ENT" --sign "$VW_APP_IDENTITY" "$APP"
  codesign --verify --strict --verbose=2 "$APP"
  echo "==> installer package: $VW_INSTALLER_IDENTITY"
  productbuild --component "$APP" /Applications --sign "$VW_INSTALLER_IDENTITY" "$PKG"
  pkgutil --check-signature "$PKG"
fi

DOTS="$(pkgutil --payload-files "$PKG" | grep -c '/\._' || true)"
if [[ "$DOTS" != 0 ]]; then
  echo "warning: the package has $DOTS '._' AppleDouble entries (extended attributes such as" >&2
  echo "         com.apple.provenance that xattr -c can't remove). Files written by a process macOS" >&2
  echo "         tracks for provenance (some app-launched shells) carry it. Build from Terminal and" >&2
  echo "         check: pkgutil --payload-files \"$PKG\" | grep '/\._'" >&2
  [[ -n "$ADHOC" ]] || exit 1
fi

echo "==> entitlements in the signature"
codesign -d --entitlements :- "$APP" 2>/dev/null
rm -rf "$WORK"
echo "==> built: $APP"
echo "$PKG"
