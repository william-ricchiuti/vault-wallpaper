#!/usr/bin/env bash
# Download the pinned Sparkle release (packaging/sparkle.env), verify its SHA-256 and unpack
# it into a cache folder (not committed). Prints the unpacked folder on the last line:
# it holds Sparkle.framework and bin/ (generate_appcast, sign_update, generate_keys).
#
# Usage: packaging/fetch-sparkle.sh
# Env:   VW_SPARKLE_CACHE  cache folder (default: <repo>/.cache/sparkle)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=sparkle.env
source "$ROOT/packaging/sparkle.env"
CACHE="${VW_SPARKLE_CACHE:-$ROOT/.cache/sparkle}"
DIR="$CACHE/$SPARKLE_VERSION"
ARCHIVE="$CACHE/Sparkle-$SPARKLE_VERSION.tar.xz"

if [[ -f "$DIR/.verified" && -d "$DIR/Sparkle.framework" ]]; then
  echo "$DIR"
  exit 0
fi

mkdir -p "$CACHE"
if [[ ! -f "$ARCHIVE" ]]; then
  echo "==> downloading Sparkle $SPARKLE_VERSION" >&2
  curl -fsSL --retry 3 -o "$ARCHIVE.part" "$SPARKLE_URL"
  mv "$ARCHIVE.part" "$ARCHIVE"
fi

GOT="$(shasum -a 256 "$ARCHIVE" | cut -d' ' -f1)"
if [[ "$GOT" != "$SPARKLE_SHA256" ]]; then
  echo "error: Sparkle archive checksum mismatch" >&2
  echo "  expected $SPARKLE_SHA256" >&2
  echo "  got      $GOT" >&2
  rm -f "$ARCHIVE"
  exit 1
fi

rm -rf "$DIR"
mkdir -p "$DIR"
tar -xJf "$ARCHIVE" -C "$DIR"
if [[ ! -d "$DIR/Sparkle.framework" ]]; then
  echo "error: $ARCHIVE has no Sparkle.framework" >&2
  exit 1
fi
touch "$DIR/.verified"
echo "==> Sparkle $SPARKLE_VERSION verified (sha256 $GOT)" >&2
echo "$DIR"
