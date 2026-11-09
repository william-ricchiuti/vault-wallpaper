#!/usr/bin/env bash
# Regenerate THIRD-PARTY-NOTICES.txt: the licence of every Rust crate compiled into the app
# (cargo-about, macOS targets, with the updater feature, which is a superset of the App Store
# build's crates), then Sparkle's LICENSE with its external licences (bsdiff, sais, ...).
# build-app.sh and appstore/build.sh copy it into Contents/Resources, make-dmg.sh into the
# DMG. Run it after any Cargo.lock or Sparkle version change and commit the result;
# release.sh runs --check.
#
# Usage: packaging/notices.sh [--check]
#   --check  regenerate into a temporary file and fail (exit 1) if it differs from the
#            committed one
# Needs: cargo-about (cargo install --locked cargo-about --version 0.9.3 --features cli, the
# version CI pins), and the Sparkle download
# (packaging/fetch-sparkle.sh, called here).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="$ROOT/THIRD-PARTY-NOTICES.txt"
CHECK=0
case "${1:-}" in
  --check) CHECK=1 ;;
  "") ;;
  *) echo "usage: packaging/notices.sh [--check]" >&2; exit 64 ;;
esac

if ! command -v cargo-about >/dev/null; then
  echo "error: cargo-about isn't installed: cargo install --locked cargo-about --version 0.9.3 --features cli" >&2
  exit 2
fi

# shellcheck source=sparkle.env
source "$ROOT/packaging/sparkle.env"
SPARKLE_DIR="$("$ROOT/packaging/fetch-sparkle.sh" | tail -n 1)"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/vw-notices.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

# --offline: licence texts come only from the crates in the local registry, so the output
# depends on Cargo.lock alone (no clearlydefined.io lookups).
cargo about generate --offline --locked \
  -c "$ROOT/packaging/notices/about.toml" \
  -m "$ROOT/crates/app/Cargo.toml" --features updater \
  "$ROOT/packaging/notices/notices.hbs" -o "$TMP/crates.txt" 2> >(grep -v "doesn't have a license field\|unable to synthesize license expression\|failed to parse license 'GPL-2.0'\|deprecated license identifier" >&2 || true)

{
  cat "$TMP/crates.txt"
  cat <<EOF

==============================================================================
SPARKLE $SPARKLE_VERSION (direct-download build only: Contents/Frameworks/Sparkle.framework)
https://sparkle-project.org
==============================================================================

EOF
  cat "$SPARKLE_DIR/LICENSE"
} >"$TMP/notices.txt"

if ((CHECK)); then
  if ! cmp -s "$TMP/notices.txt" "$OUT"; then
    echo "THIRD-PARTY-NOTICES.txt is out of date; run packaging/notices.sh and commit it" >&2
    diff -u "$OUT" "$TMP/notices.txt" | head -n 40 >&2 || true
    exit 1
  fi
  exit 0
fi
mv "$TMP/notices.txt" "$OUT"
echo "==> wrote $OUT ($(wc -l <"$OUT" | tr -d ' ') lines)"
