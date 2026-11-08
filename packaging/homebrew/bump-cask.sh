#!/usr/bin/env bash
# Set the cask's version and sha256 for a release, in a clone of the tap repo.
#
# Usage: packaging/homebrew/bump-cask.sh <version> <dmg> [tap-dir]
#   version  e.g. 1.0.0-beta.2 (a leading "v" is dropped)
#   dmg      the notarized, stapled DMG, whose sha256 goes in the cask
#   tap-dir  a clone of william-ricchiuti/homebrew-tap (default: $VW_TAP_DIR, else
#            ~/dev/homebrew-tap). Writes <tap-dir>/Casks/vault-wallpaper.rb, starting from
#            this repo's template if the tap has no cask yet.
#
# It edits one file and doesn't commit or push. Then, in the tap:
#   brew style Casks/vault-wallpaper.rb
#   git commit -am "vault-wallpaper <version>" && git push
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
TEMPLATE="$HERE/vault-wallpaper.rb"

if [[ $# -lt 2 || $# -gt 3 ]]; then
  sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 64
fi
VERSION="${1#v}"
DMG="$2"
TAP="${3:-${VW_TAP_DIR:-$HOME/dev/homebrew-tap}}"

if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "error: '$VERSION' is not a version like 1.0.0 or 1.0.0-beta.2" >&2
  exit 64
fi
[[ -f "$DMG" ]] || { echo "error: $DMG not found" >&2; exit 1; }
[[ -d "$TAP" ]] || { echo "error: tap directory $TAP not found (clone william-ricchiuti/homebrew-tap)" >&2; exit 1; }

EXPECTED="Vault-Wallpaper-$VERSION.dmg"
if [[ "$(basename "$DMG")" != "$EXPECTED" ]]; then
  echo "warning: $(basename "$DMG") isn't named $EXPECTED, which is what the cask downloads" >&2
fi

CASK="$TAP/Casks/vault-wallpaper.rb"
mkdir -p "$TAP/Casks"
if [[ ! -f "$CASK" ]]; then
  echo "==> $CASK doesn't exist; starting from $TEMPLATE"
  cp "$TEMPLATE" "$CASK"
fi

SHA256="$(shasum -a 256 "$DMG" | cut -d' ' -f1)"
# Only the cask's own two-space-indented `version` and `sha256` lines.
sed -i '' -E \
  -e "s/^  version \"[^\"]*\"/  version \"$VERSION\"/" \
  -e "s/^  sha256 \"[^\"]*\"/  sha256 \"$SHA256\"/" \
  "$CASK"

grep -q "^  version \"$VERSION\"" "$CASK" || { echo "error: version line not updated in $CASK" >&2; exit 1; }
grep -q "^  sha256 \"$SHA256\"" "$CASK" || { echo "error: sha256 line not updated in $CASK" >&2; exit 1; }
echo "==> $CASK: version $VERSION, sha256 $SHA256"
