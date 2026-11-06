#!/usr/bin/env bash
# Code-sign "dist/Vault Wallpaper.app" with a Developer ID and the hardened runtime.
#
# Usage: VW_SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" packaging/sign.sh [app]
# Needs: a "Developer ID Application" certificate in your login keychain.
# Refuses to run without VW_SIGN_IDENTITY, so it never signs with the wrong identity by accident.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="${1:-$ROOT/dist/Vault Wallpaper.app}"
ENTITLEMENTS="$ROOT/packaging/entitlements.plist"

if [[ -z "${VW_SIGN_IDENTITY:-}" ]]; then
  cat >&2 <<'MSG'
VW_SIGN_IDENTITY is not set, so nothing was signed.

Find your identity with:

    security find-identity -v -p codesigning

and copy the quoted name of the "Developer ID Application" line (not "Apple Development"),
for example:

    export VW_SIGN_IDENTITY="Developer ID Application: Your Name (ABCDE12345)"

Then run packaging/sign.sh again. If no Developer ID Application line appears, create the
certificate in Xcode (Settings > Accounts > Manage Certificates > +) or at
developer.apple.com > Certificates.
MSG
  exit 2
fi

if [[ ! -d "$APP" ]]; then
  echo "error: $APP not found; run packaging/build-app.sh first" >&2
  exit 1
fi

echo "==> signing $APP"
echo "    identity: $VW_SIGN_IDENTITY"
codesign --force --timestamp --options runtime \
  --entitlements "$ENTITLEMENTS" \
  --sign "$VW_SIGN_IDENTITY" \
  "$APP"

echo "==> verifying"
codesign --verify --strict --deep --verbose=2 "$APP"
codesign --display --verbose=2 "$APP" 2>&1 | grep -E '^(Authority|TeamIdentifier|Runtime|Identifier)' || true
echo "==> signed. Next: packaging/make-dmg.sh, then packaging/notarize.sh"
