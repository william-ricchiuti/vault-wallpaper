#!/usr/bin/env bash
# Run Script build phase of xcode/VaultWallpaper.xcodeproj (the Mac App Store build in Xcode).
# Xcode runs it with its build settings in the environment; don't run it by hand.
#
# It does what packaging/appstore/build.sh does before signing:
#   - cargo build --release --no-default-features -p vault-wallpaper for each arch in $ARCHS
#     (arm64 and x86_64 for Archive), lipo'd into the bundle as the executable;
#   - AppIcon.icns from packaging/icon/icon-1024.png;
#   - the version (crates/app/Cargo.toml) and build number (commit count, packaging/build-number.sh)
#     written into the built Info.plist.
# Xcode copies PrivacyInfo.xcprivacy and THIRD-PARTY-NOTICES.txt, then signs with
# packaging/appstore/entitlements.plist.
set -euo pipefail

: "${SRCROOT:?run from Xcode}" "${TARGET_BUILD_DIR:?}" "${EXECUTABLE_PATH:?}" "${INFOPLIST_PATH:?}"
ROOT="$(cd "$SRCROOT/.." && pwd)"
BIN_NAME="vault-wallpaper"

# Xcode starts scripts with a short PATH and none of the user's shell setup.
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
if ! command -v cargo >/dev/null || ! command -v rustup >/dev/null; then
  echo "error: cargo/rustup not found. Install Rust from https://rustup.rs (it goes in ~/.cargo/bin)." >&2
  exit 1
fi

VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$ROOT/crates/app/Cargo.toml" | head -n 1)"
[[ -n "$VERSION" ]] || { echo "error: could not read version from crates/app/Cargo.toml" >&2; exit 1; }
SHORT_VERSION="${VERSION%%[-+]*}"
# shellcheck source=../build-number.sh
source "$ROOT/packaging/build-number.sh"
vw_build_number "$ROOT" || exit 1
echo "Vault Wallpaper $VERSION (short $SHORT_VERSION, build $BUILD_NUMBER), archs: $ARCHS"

TARGETS=()
for a in $ARCHS; do
  case "$a" in
    arm64) TARGETS+=(aarch64-apple-darwin) ;;
    x86_64) TARGETS+=(x86_64-apple-darwin) ;;
    *) echo "error: unsupported architecture $a" >&2; exit 1 ;;
  esac
done
INSTALLED="$(rustup target list --installed)"
for t in "${TARGETS[@]}"; do
  grep -qx "$t" <<<"$INSTALLED" || rustup target add "$t"
done

# cargo runs in a clean environment, as from Terminal: Xcode exports hundreds of build settings
# (MACOSX_DEPLOYMENT_TARGET breaks proc macros with --target, see packaging/build-app.sh; SDKROOT,
# ARCHS and others change what the cc crate does) and they would also make cargo rebuild
# everything that build.sh already built.
CARGO_ENV=(env -i HOME="$HOME" USER="${USER:-}" PATH="$PATH" TMPDIR="${TMPDIR:-/tmp}" LANG="${LANG:-en_US.UTF-8}")
for v in CARGO_HOME RUSTUP_HOME CARGO_TARGET_DIR RUSTUP_TOOLCHAIN; do
  [[ -n "${!v:-}" ]] && CARGO_ENV+=("$v=${!v}")
done
SLICES=()
for t in "${TARGETS[@]}"; do
  echo "cargo build --release --locked --no-default-features -p $BIN_NAME --target $t"
  (cd "$ROOT" && "${CARGO_ENV[@]}" cargo build --release --locked --no-default-features -p "$BIN_NAME" --target "$t")
  SLICES+=("${CARGO_TARGET_DIR:-$ROOT/target}/$t/release/$BIN_NAME")
done

EXE="$TARGET_BUILD_DIR/$EXECUTABLE_PATH"
mkdir -p "$(dirname "$EXE")"
rm -f "$EXE"
lipo -create "${SLICES[@]}" -output "$EXE"
lipo -info "$EXE"

RES="$TARGET_BUILD_DIR/$UNLOCALIZED_RESOURCES_FOLDER_PATH"
ICON_PNG="$ROOT/packaging/icon/icon-1024.png"
mkdir -p "$RES"
if [[ ! -f "$RES/AppIcon.icns" || "$ICON_PNG" -nt "$RES/AppIcon.icns" ]]; then
  SET="$DERIVED_FILE_DIR/AppIcon.iconset"
  rm -rf "$SET"
  mkdir -p "$SET"
  for s in 16 32 128 256 512; do
    sips -z "$s" "$s" "$ICON_PNG" --out "$SET/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    sips -z "$d" "$d" "$ICON_PNG" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$SET" -o "$RES/AppIcon.icns"
fi

PLIST="$TARGET_BUILD_DIR/$INFOPLIST_PATH"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $SHORT_VERSION" \
  -c "Set :CFBundleVersion $BUILD_NUMBER" -c "Set :VWVersion $VERSION" "$PLIST"
