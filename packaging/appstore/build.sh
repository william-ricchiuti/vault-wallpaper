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
# for the same CFBundleShortVersionString. The commit count only goes up along main;
# build-number.sh refuses a shallow clone or a commit that isn't on origin/main.
# shellcheck source=../build-number.sh
source "$ROOT/packaging/build-number.sh"
vw_build_number "$ROOT" || exit 1
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
# The bundle is put together in a staging folder and copied to $APP with ditto at the end,
# without resource forks, extended attributes or quarantine (see below).
STAGE_APP="$WORK/stage/$APP_NAME.app"
mkdir -p "$STAGE_APP/Contents/MacOS" "$STAGE_APP/Contents/Resources" "$WORK"

lipo -create "${SLICES[@]}" -output "$STAGE_APP/Contents/MacOS/$BIN_NAME"
lipo -info "$STAGE_APP/Contents/MacOS/$BIN_NAME"

echo "==> icon"
ICON_PNG="$ROOT/packaging/icon/icon-1024.png"
SET="$WORK/AppIcon.iconset"
mkdir -p "$SET"
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" "$ICON_PNG" --out "$SET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z "$d" "$d" "$ICON_PNG" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$SET" -o "$STAGE_APP/Contents/Resources/AppIcon.icns"

cp "$ROOT/THIRD-PARTY-NOTICES.txt" "$STAGE_APP/Contents/Resources/THIRD-PARTY-NOTICES.txt"
cp "$HERE/PrivacyInfo.xcprivacy" "$STAGE_APP/Contents/Resources/PrivacyInfo.xcprivacy"

echo "==> Info.plist"
cat >"$STAGE_APP/Contents/Info.plist" <<PLIST
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
printf 'APPL????' >"$STAGE_APP/Contents/PkgInfo"
plutil -lint "$STAGE_APP/Contents/Info.plist"

# Extended attributes in the bundle become "._" AppleDouble entries in the package payload
# (pkgbuild, which productbuild runs, writes one per attributed file). The sources (icon,
# plist, binary, a downloaded provisioning profile) may carry some, such as
# com.apple.quarantine, so the bundle is copied out of the staging folder without them.
# --noclone matters: on APFS ditto otherwise clones, and a clone keeps every attribute
# whatever --noextattr says. That copy isn't enough on its own: macOS 26+ tags every file
# a build writes with com.apple.provenance, which xattr can't remove (seen from Terminal.app
# too), so after productbuild the component's payload is repacked without the AppleDouble
# entries (repack_without_appledouble below).
export COPYFILE_DISABLE=1
echo "==> staging copy without extended attributes (ditto --noclone --norsrc --noextattr --noqtn)"
ditto --noclone --norsrc --noextattr --noqtn "$STAGE_APP" "$APP"
xattr -cr "$APP" 2>/dev/null || true

# Rewrites the component package inside the product archive $1 without "._" entries: the
# payload is a root-owned odc cpio archive of the app made by tar with no extended
# attributes (what pkgbuild writes, minus the AppleDouble entries), and the bill of
# materials is pkgbuild's own with those lines dropped. Distribution and PackageInfo stay
# as productbuild wrote them, apart from the file count. Leaves an unsigned product.
repack_without_appledouble() {
  local pkg="$1" exp="$WORK/pkg-expanded" root="$WORK/payload-root" comp n
  rm -rf "$exp" "$root"
  pkgutil --expand "$pkg" "$exp"
  comp="$(find "$exp" -mindepth 1 -maxdepth 1 -type d -name '*.pkg' | head -n 1)"
  [[ -n "$comp" && -f "$comp/Bom" && -f "$comp/Payload" ]] || { echo "error: no component package in $pkg" >&2; exit 1; }
  mkdir -p "$root"
  ditto --noclone --norsrc --noextattr --noqtn "$APP" "$root/$APP_NAME.app"
  # productbuild lists the install folder itself as ".  0  0/0", which mkbom -i silently
  # turns into an empty bill of materials; 40755 root:wheel is what pkgbuild --root writes.
  lsbom "$comp/Bom" | grep -v '/\._' | sed $'1s/^\\.\t0\t/.\t40755\t/' >"$WORK/bom.txt"
  rm -f "$comp/Bom" "$comp/Payload"
  mkbom -i "$WORK/bom.txt" "$comp/Bom"
  tar -c --format odc --uid 0 --gid 0 --uname root --gname wheel \
    --no-mac-metadata --no-xattrs --no-acls --no-fflags -C "$root" . | gzip -9 >"$comp/Payload"
  # The payload and the bill of materials must list the same files.
  if ! diff <(lsbom -s "$comp/Bom" | sort) <(gzip -dc "$comp/Payload" | cpio -it 2>/dev/null | sort) >&2; then
    echo "error: repacked payload and bill of materials disagree" >&2
    exit 1
  fi
  n="$(lsbom -s "$comp/Bom" | wc -l | tr -d ' ')"
  sed -i '' -E "s/numberOfFiles=\"[0-9]+\"/numberOfFiles=\"$n\"/" "$comp/PackageInfo"
  rm -f "$pkg"
  pkgutil --flatten "$exp" "$pkg"
  rm -rf "$exp" "$root"
}

PKG="$OUT/Vault-Wallpaper-$SHORT_VERSION-$BUILD_NUMBER.pkg"
rm -f "$PKG"
if [[ -n "$ADHOC" ]]; then
  echo "==> ad-hoc signing with the sandbox entitlements (local testing only; not uploadable)"
  codesign --force --sign - --entitlements "$ENTITLEMENTS" "$APP"
  codesign --verify --strict --verbose=2 "$APP"
  echo "==> unsigned installer package (structure check only)"
  productbuild --component "$APP" /Applications "$PKG"
  repack_without_appledouble "$PKG"
else
  echo "==> embedding the provisioning profile"
  ditto --noclone --norsrc --noextattr --noqtn "$VW_PROVISIONING_PROFILE" "$APP/Contents/embedded.provisionprofile"
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
  productbuild --component "$APP" /Applications "$WORK/unsigned.pkg"
  repack_without_appledouble "$WORK/unsigned.pkg"
  productsign --sign "$VW_INSTALLER_IDENTITY" "$WORK/unsigned.pkg" "$PKG"
  pkgutil --check-signature "$PKG"
fi

DOTS="$(pkgutil --payload-files "$PKG" | grep -c '/\._' || true)"
if [[ "$DOTS" != 0 ]]; then
  echo "error: the package still has $DOTS '._' AppleDouble entries after repacking:" >&2
  echo "       pkgutil --payload-files \"$PKG\" | grep '/\._'" >&2
  exit 1
fi

echo "==> entitlements in the signature"
codesign -d --entitlements :- "$APP" 2>/dev/null
rm -rf "$WORK"
echo "==> built: $APP"
echo "$PKG"
