#!/usr/bin/env bash
# Notarize a signed DMG (or zip) with Apple and staple the ticket to it.
#
# Usage: VW_NOTARY_PROFILE=vault-wallpaper-notary packaging/notarize.sh [file]
#   file defaults to the newest dist/*.dmg.
#
# Or, with an App Store Connect API key instead of a keychain profile (what CI uses):
#   VW_NOTARY_KEY_PATH=AuthKey_XXXX.p8 VW_NOTARY_KEY_ID=XXXX VW_NOTARY_ISSUER=<uuid> \
#     packaging/notarize.sh [file]
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

AUTH=()
if [[ -n "${VW_NOTARY_KEY_PATH:-}" ]]; then
  if [[ -z "${VW_NOTARY_KEY_ID:-}" || -z "${VW_NOTARY_ISSUER:-}" ]]; then
    echo "error: VW_NOTARY_KEY_PATH needs VW_NOTARY_KEY_ID and VW_NOTARY_ISSUER too" >&2
    exit 2
  fi
  AUTH=(--key "$VW_NOTARY_KEY_PATH" --key-id "$VW_NOTARY_KEY_ID" --issuer "$VW_NOTARY_ISSUER")
  AUTH_DESC="API key $VW_NOTARY_KEY_ID"
elif [[ -n "${VW_NOTARY_PROFILE:-}" ]]; then
  AUTH=(--keychain-profile "$VW_NOTARY_PROFILE")
  AUTH_DESC="profile $VW_NOTARY_PROFILE"
else
  cat >&2 <<'MSG'
VW_NOTARY_PROFILE is not set, so nothing was submitted.

Set it to the keychain profile you created once with:

    xcrun notarytool store-credentials vault-wallpaper-notary \
      --apple-id "you@example.com" --team-id ABCDE12345

    export VW_NOTARY_PROFILE=vault-wallpaper-notary

(or set VW_NOTARY_KEY_PATH, VW_NOTARY_KEY_ID and VW_NOTARY_ISSUER for an API key)
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

echo "==> submitting $FILE ($AUTH_DESC); this usually takes a few minutes"
xcrun notarytool submit "$FILE" "${AUTH[@]}" --wait
# If it reports "Invalid", fetch the reason with:
#   xcrun notarytool log <submission-id> --keychain-profile "$VW_NOTARY_PROFILE"

echo "==> stapling"
xcrun stapler staple "$FILE"
xcrun stapler validate "$FILE"

echo "==> Gatekeeper check"
spctl --assess --type open --context context:primary-signature --verbose=2 "$FILE" || true
echo "==> notarized and stapled: $FILE"
