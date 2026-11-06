#!/usr/bin/env bash
# Notarize a signed DMG (or zip) with Apple and staple the ticket to it.
#
# Usage: VW_NOTARY_PROFILE=vault-wallpaper-notary packaging/notarize.sh [file]
#   file defaults to the newest dist/*.dmg.
#
# One-time setup (stores an app-specific password in your keychain under a profile name):
#
#   xcrun notarytool store-credentials vault-wallpaper-notary \
#     --apple-id "you@example.com" --team-id ABCDE12345
#
# It prompts for an app-specific password: make one at account.apple.com > Sign-In and
# Security > App-Specific Passwords. The team ID is the 10-character code in your
# "Developer ID Application: Name (TEAMID)" identity.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [[ -z "${VW_NOTARY_PROFILE:-}" ]]; then
  cat >&2 <<'MSG'
VW_NOTARY_PROFILE is not set, so nothing was submitted.

Set it to the keychain profile you created once with:

    xcrun notarytool store-credentials vault-wallpaper-notary \
      --apple-id "you@example.com" --team-id ABCDE12345

    export VW_NOTARY_PROFILE=vault-wallpaper-notary
MSG
  exit 2
fi

FILE="${1:-}"
if [[ -z "$FILE" ]]; then
  FILE="$(ls -t "$ROOT"/dist/*.dmg 2>/dev/null | head -n 1 || true)"
fi
if [[ -z "$FILE" || ! -f "$FILE" ]]; then
  echo "error: nothing to notarize; run build-app.sh, sign.sh and make-dmg.sh first" >&2
  exit 1
fi

echo "==> submitting $FILE (profile $VW_NOTARY_PROFILE); this usually takes a few minutes"
xcrun notarytool submit "$FILE" --keychain-profile "$VW_NOTARY_PROFILE" --wait
# If it reports "Invalid", fetch the reason with:
#   xcrun notarytool log <submission-id> --keychain-profile "$VW_NOTARY_PROFILE"

echo "==> stapling"
xcrun stapler staple "$FILE"
xcrun stapler validate "$FILE"

echo "==> Gatekeeper check"
spctl --assess --type open --context context:primary-signature --verbose=2 "$FILE" || true
echo "==> notarized and stapled: $FILE"
