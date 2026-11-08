#!/usr/bin/env bash
# One-time setup for Sparkle updates: create the EdDSA signing key pair with Sparkle's
# generate_keys. The PRIVATE key goes into your login Keychain (item "Private key for
# signing Sparkle updates", account "ed25519") and never leaves it; the script prints the
# PUBLIC key, which goes into packaging/sparkle.env as SPARKLE_PUBLIC_ED_KEY.
#
# Usage: packaging/sparkle-keys.sh            create the key, or print it if it exists
#        packaging/sparkle-keys.sh -p         only print the existing public key
#        packaging/sparkle-keys.sh -x <file>  export the private key (for a backup or CI
#                                             secret; never commit it)
#
# Run it once, on your own Mac. Keep a backup of the private key (the -x export, in a
# password manager): if it is lost, installed copies can't verify any future update.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SPARKLE_DIR="$("$ROOT/packaging/fetch-sparkle.sh" | tail -n 1)"
"$SPARKLE_DIR/bin/generate_keys" "$@"
if [[ $# -eq 0 ]]; then
  cat <<'MSG'

Next: put the public key above into packaging/sparkle.env:

    SPARKLE_PUBLIC_ED_KEY=<the key>

and commit that (it is public). Back up the private key with
`packaging/sparkle-keys.sh -x ~/sparkle-private-key.txt`, store the file somewhere safe
(a password manager), then delete it.
MSG
fi
