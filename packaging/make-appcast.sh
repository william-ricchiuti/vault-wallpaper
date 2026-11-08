#!/usr/bin/env bash
# Make appcast.xml for a release DMG with Sparkle's generate_appcast, which reads the app's
# version from inside the DMG, signs the DMG with the EdDSA private key and writes an
# <item> with its URL, length, sparkle:edSignature and minimum macOS version.
#
# Usage: packaging/make-appcast.sh <dmg> [<version>] [--out <file>] [--unsigned]
#   <dmg>       the signed, notarized, stapled DMG (the signature covers its exact bytes)
#   <version>   the full version (release tag v<version>); default: VWVersion of the app inside
#   --out       where to write the appcast (default: dist/appcast/appcast.xml). An appcast
#               already there is kept and the new item added on top, so download the
#               previous release's appcast.xml to that path first to keep the history.
#   --unsigned  no EdDSA signature (for dry runs); never publish such a feed
# The appcast path is the last line on stdout.
#
# Private key, first found wins:
#   VW_SPARKLE_PRIVATE_KEY  the base64 key itself (CI secret SPARKLE_PRIVATE_KEY)
#   VW_SPARKLE_KEY_FILE     a file holding it (tests)
#   the login Keychain      where packaging/sparkle-keys.sh put it (William's Mac)
# VW_APPCAST_URL_PREFIX overrides the download folder
# (default https://github.com/william-ricchiuti/vault-wallpaper/releases/download/v<version>/).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DMG="" VERSION="" OUT_FILE="$ROOT/dist/appcast/appcast.xml" UNSIGNED=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT_FILE="$2"; shift 2 ;;
    --unsigned) UNSIGNED=1; shift ;;
    -h|--help) sed -n '2,21p' "$0"; exit 0 ;;
    -*) echo "error: unknown option $1" >&2; exit 64 ;;
    *) if [[ -z "$DMG" ]]; then DMG="$1"; elif [[ -z "$VERSION" ]]; then VERSION="$1"; else
         echo "error: unexpected argument $1" >&2; exit 64; fi; shift ;;
  esac
done
if [[ -z "$DMG" || ! -f "$DMG" ]]; then
  echo "error: pass the DMG (packaging/make-dmg.sh makes it): make-appcast.sh <dmg> [<version>]" >&2
  exit 64
fi

# What's inside: the app's versions and minimum macOS.
MNT="$(mktemp -d "${TMPDIR:-/tmp}/vw-appcast.XXXXXX")"
hdiutil attach -nobrowse -readonly -mountpoint "$MNT" "$DMG" >/dev/null 2>&1
APP_IN_DMG="$(find "$MNT" -maxdepth 1 -name '*.app' -print -quit)"
PLIST="$APP_IN_DMG/Contents/Info.plist"
read_key() { [[ -f "$PLIST" ]] && /usr/libexec/PlistBuddy -c "Print :$1" "$PLIST" 2>/dev/null || true; }
APP_VERSION="$(read_key VWVersion)"
BUILD="$(read_key CFBundleVersion)"
SHORT="$(read_key CFBundleShortVersionString)"
MIN_OS="$(read_key LSMinimumSystemVersion)"
hdiutil detach "$MNT" >/dev/null 2>&1
rmdir "$MNT"
if [[ -z "$BUILD" ]]; then
  echo "error: $DMG has no app with a CFBundleVersion" >&2
  exit 1
fi
VERSION="${VERSION:-$APP_VERSION}"
PREFIX="${VW_APPCAST_URL_PREFIX:-https://github.com/william-ricchiuti/vault-wallpaper/releases/download/v$VERSION/}"
NAME="$(basename "$DMG")"

OUT_DIR="$(dirname "$OUT_FILE")"
mkdir -p "$OUT_DIR"

if [[ "$UNSIGNED" == 1 ]]; then
  # Dry run: the item generate_appcast would write, minus the signature. Needs no key.
  LENGTH="$(stat -f %z "$DMG")"
  cat >"$OUT_FILE" <<XML
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
    <channel>
        <title>Vault Wallpaper</title>
        <item>
            <title>$SHORT</title>
            <pubDate>$(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S +0000')</pubDate>
            <sparkle:version>$BUILD</sparkle:version>
            <sparkle:shortVersionString>$SHORT</sparkle:shortVersionString>
            <sparkle:minimumSystemVersion>$MIN_OS</sparkle:minimumSystemVersion>
            <enclosure url="$PREFIX$NAME" length="$LENGTH" type="application/octet-stream"/>
        </item>
    </channel>
</rss>
XML
  echo "==> $OUT_FILE (UNSIGNED: for dry runs only)" >&2
  echo "$OUT_FILE"
  exit 0
fi

SPARKLE_DIR="$("$ROOT/packaging/fetch-sparkle.sh" | tail -n 1)"

# generate_appcast works on a folder and signs every archive in it: use a scratch folder
# holding just this DMG and the existing appcast, then copy the result out.
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vw-appcast-work.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
cp "$DMG" "$WORK/"
[[ -f "$OUT_FILE" ]] && cp "$OUT_FILE" "$WORK/appcast.xml"

ARGS=(--download-url-prefix "$PREFIX" --link "https://github.com/william-ricchiuti/vault-wallpaper" --maximum-versions 10)
echo "==> generate_appcast ($VERSION, build $BUILD, downloads from $PREFIX)" >&2
if [[ -n "${VW_SPARKLE_PRIVATE_KEY:-}" ]]; then
  printf '%s' "$VW_SPARKLE_PRIVATE_KEY" | "$SPARKLE_DIR/bin/generate_appcast" --ed-key-file - "${ARGS[@]}" "$WORK" >&2
elif [[ -n "${VW_SPARKLE_KEY_FILE:-}" ]]; then
  "$SPARKLE_DIR/bin/generate_appcast" --ed-key-file "$VW_SPARKLE_KEY_FILE" "${ARGS[@]}" "$WORK" >&2
else
  "$SPARKLE_DIR/bin/generate_appcast" "${ARGS[@]}" "$WORK" >&2
fi

if ! grep -q 'sparkle:edSignature=' "$WORK/appcast.xml"; then
  echo "error: appcast.xml has no EdDSA signature" >&2
  exit 1
fi
cp "$WORK/appcast.xml" "$OUT_FILE"
grep -E 'sparkle:version>|<enclosure' "$OUT_FILE" | head -n 4 >&2
echo "$OUT_FILE"
