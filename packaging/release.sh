#!/usr/bin/env bash
# One-command release of Vault Wallpaper: the direct build (DMG + Sparkle appcast + Homebrew
# cask values) and, if its script exists, the Mac App Store package.
#
# Usage: packaging/release.sh <version> [--dry-run] [--no-appstore]
#
#   <version>      semver, e.g. 1.0.0 or 1.0.0-beta.2 (no leading "v")
#   --dry-run      build everything, but don't sign, notarize or upload. The version bump in
#                  crates/app/Cargo.toml and Cargo.lock is undone when the script exits, and
#                  the appcast is an unsigned test appcast.
#   --no-appstore  skip the App Store package (CI uses this; the App Store build needs
#                  different certificates).
#   --bump-only    only set the version in crates/app/Cargo.toml and Cargo.lock, then stop
#                  (for releasing by tag: bump, commit, tag, push).
#
# Real run needs (see docs/release-checklist.md, "One-time setup"):
#   VW_SIGN_IDENTITY                          "Developer ID Application: Name (TEAMID)"
#   VW_NOTARY_PROFILE, or VW_NOTARY_KEY_PATH + VW_NOTARY_KEY_ID + VW_NOTARY_ISSUER
#   a Sparkle key for make-appcast.sh: VW_SPARKLE_PRIVATE_KEY, VW_SPARKLE_KEY_FILE, or the
#   login Keychain (where sparkle-keys.sh puts it)
#
# It uploads nothing. At the end it prints the git and `gh release create` commands to run.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PKG="$ROOT/packaging"
DIST="$ROOT/dist"
REPO="william-ricchiuti/vault-wallpaper"
APP_TOML="crates/app/Cargo.toml"

usage() { sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//'; }

VERSION=""
DRY_RUN=0
APPSTORE=1
BUMP_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --no-appstore) APPSTORE=0 ;;
    --bump-only) BUMP_ONLY=1 ;;
    -h | --help) usage; exit 0 ;;
    -*) echo "error: unknown option $arg" >&2; usage >&2; exit 64 ;;
    *)
      if [[ -n "$VERSION" ]]; then echo "error: more than one version given" >&2; exit 64; fi
      VERSION="$arg"
      ;;
  esac
done
if [[ -z "$VERSION" ]]; then usage >&2; exit 64; fi
VERSION="${VERSION#v}"
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "error: '$VERSION' is not a version like 1.0.0 or 1.0.0-beta.2" >&2
  exit 64
fi
TAG="v$VERSION"
DMG="$DIST/Vault-Wallpaper-$VERSION.dmg"
APPCAST="$DIST/appcast.xml"
PRERELEASE=""
[[ "$VERSION" == *-* ]] && PRERELEASE="--prerelease"

step() { printf '\n==> [release] %s\n' "$*"; }
skip() { printf '    [dry-run] skipped: %s\n' "$*"; }
die() { echo "error: $*" >&2; exit 1; }

if ((BUMP_ONLY && DRY_RUN)); then
  die "--bump-only and --dry-run don't go together"
fi
if ((BUMP_ONLY)); then
  echo "Vault Wallpaper $VERSION: version bump only"
elif ((DRY_RUN)); then
  echo "Vault Wallpaper $VERSION: DRY RUN (no signing, notarizing or uploading)"
else
  echo "Vault Wallpaper $VERSION: release build"
fi

# ---------------------------------------------------------------------------------------
# 0. Preflight
# ---------------------------------------------------------------------------------------
step "preflight"
for tool in cargo rustup lipo hdiutil shasum git; do
  command -v "$tool" >/dev/null || die "$tool not found"
done
DIRTY="$(git status --porcelain --untracked-files=no)"
if [[ -n "$DIRTY" ]]; then
  if ((DRY_RUN)); then
    echo "    warning: the working tree has uncommitted changes (fine for a dry run)"
  else
    die "the working tree has uncommitted changes; commit or stash them first"
  fi
fi
if ((!DRY_RUN && !BUMP_ONLY)); then
  [[ -n "${VW_SIGN_IDENTITY:-}" ]] || die "VW_SIGN_IDENTITY is not set (run packaging/sign.sh for help)"
  [[ -n "${VW_NOTARY_PROFILE:-}${VW_NOTARY_KEY_PATH:-}" ]] ||
    die "set VW_NOTARY_PROFILE (or VW_NOTARY_KEY_PATH/_ID/ISSUER); run packaging/notarize.sh for help"
  [[ -x "$PKG/make-appcast.sh" ]] || die "packaging/make-appcast.sh is missing; the Sparkle appcast can't be made"
  if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
    echo "    note: tag $TAG already exists locally"
  fi
fi

# ---------------------------------------------------------------------------------------
# 1. Version bump
# ---------------------------------------------------------------------------------------
step "version $VERSION in $APP_TOML"
OLD_VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$APP_TOML" | head -n 1)"
[[ -n "$OLD_VERSION" ]] || die "could not read the version from $APP_TOML"
if [[ "$OLD_VERSION" == "$VERSION" ]]; then
  echo "    already $VERSION"
else
  if ((DRY_RUN)); then
    BACKUP="$(mktemp -d "${TMPDIR:-/tmp}/vw-release.XXXXXX")"
    cp "$APP_TOML" "$BACKUP/Cargo.toml.app"
    cp Cargo.lock "$BACKUP/Cargo.lock"
    restore() {
      cp "$BACKUP/Cargo.toml.app" "$ROOT/$APP_TOML"
      cp "$BACKUP/Cargo.lock" "$ROOT/Cargo.lock"
      rm -rf "$BACKUP"
      echo "==> [release] dry run: restored $APP_TOML and Cargo.lock to $OLD_VERSION"
    }
    trap restore EXIT
  fi
  # Only the first `version =` line, which is [package].
  sed -i '' "1,/^version[[:space:]]*=/s/^version[[:space:]]*=.*/version = \"$VERSION\"/" "$APP_TOML"
  # Rewrite the workspace member's entry in Cargo.lock, nothing else.
  cargo update --offline --workspace >/dev/null 2>&1 || cargo update --workspace >/dev/null
  echo "    $OLD_VERSION -> $VERSION"
fi
if ((BUMP_ONLY)); then
  cat <<EOF

Next:
   git commit -am "chore(release): $VERSION"
   git tag -a $TAG -m "Vault Wallpaper $VERSION"
   git push origin HEAD $TAG      # starts .github/workflows/release.yml
EOF
  exit 0
fi

# ---------------------------------------------------------------------------------------
# 2. Build the direct app (universal, with the Sparkle updater)
# ---------------------------------------------------------------------------------------
FEATURES=""
if awk '/^\[features\]/{f=1;next} /^\[/{f=0} f && /^updater[[:space:]]*=/{found=1} END{exit !found}' "$APP_TOML"; then
  FEATURES="updater"
else
  if ((DRY_RUN)); then
    echo "    warning: $APP_TOML has no 'updater' feature yet; building without Sparkle"
  else
    die "$APP_TOML has no 'updater' feature; the direct build must include Sparkle"
  fi
fi
step "build the direct app (features: ${FEATURES:-default})"
VW_FEATURES="$FEATURES" "$PKG/build-app.sh"
APP="$DIST/Vault Wallpaper.app"

# ---------------------------------------------------------------------------------------
# 3. Sign
# ---------------------------------------------------------------------------------------
step "sign the app"
if ((DRY_RUN)); then
  skip "packaging/sign.sh (Developer ID, hardened runtime)"
else
  "$PKG/sign.sh" "$APP"
fi

# ---------------------------------------------------------------------------------------
# 4. DMG (make-dmg.sh signs it when VW_SIGN_IDENTITY is set)
# ---------------------------------------------------------------------------------------
step "make the DMG"
if ((DRY_RUN)); then
  env -u VW_SIGN_IDENTITY "$PKG/make-dmg.sh" "$APP"
else
  "$PKG/make-dmg.sh" "$APP"
fi
[[ -f "$DMG" ]] || die "expected $DMG"

# ---------------------------------------------------------------------------------------
# 5. Notarize and staple
# ---------------------------------------------------------------------------------------
step "notarize and staple"
if ((DRY_RUN)); then
  skip "packaging/notarize.sh $DMG"
else
  "$PKG/notarize.sh" "$DMG"
fi

# The checksum is taken after stapling: stapling changes the file.
SHA256="$(shasum -a 256 "$DMG" | cut -d' ' -f1)"
DMG_BYTES="$(stat -f %z "$DMG")"

# ---------------------------------------------------------------------------------------
# 6. Sparkle appcast
# ---------------------------------------------------------------------------------------
# Interface (agreed in packaging/README.md):
#   packaging/make-appcast.sh <dmg> <version> [--out <file>] [--unsigned]
step "Sparkle appcast"
write_test_appcast() {
  # Stand-in for a dry run before make-appcast.sh exists. NOT for publishing: no EdDSA
  # signature, so Sparkle would refuse the update.
  local short build url
  short="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist")"
  build="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$APP/Contents/Info.plist")"
  url="https://github.com/$REPO/releases/download/$TAG/Vault-Wallpaper-$VERSION.dmg"
  cat >"$APPCAST" <<XML
<?xml version="1.0" encoding="utf-8"?>
<!-- TEST APPCAST from a release.sh dry run: unsigned, do not publish. -->
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Vault Wallpaper</title>
    <item>
      <title>Version $VERSION</title>
      <pubDate>$(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S +0000')</pubDate>
      <sparkle:version>$build</sparkle:version>
      <sparkle:shortVersionString>$short</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>13.0</sparkle:minimumSystemVersion>
      <enclosure url="$url" length="$DMG_BYTES" type="application/octet-stream"/>
    </item>
  </channel>
</rss>
XML
}
if [[ -x "$PKG/make-appcast.sh" ]]; then
  if ((DRY_RUN)); then
    "$PKG/make-appcast.sh" "$DMG" "$VERSION" --out "$APPCAST" --unsigned
  else
    "$PKG/make-appcast.sh" "$DMG" "$VERSION" --out "$APPCAST"
  fi
else
  # Only reachable in a dry run; the preflight stops a real run.
  echo "    packaging/make-appcast.sh not found; writing an unsigned TEST appcast"
  write_test_appcast
fi
[[ -f "$APPCAST" ]] || die "expected $APPCAST"
if command -v xmllint >/dev/null; then
  xmllint --noout "$APPCAST"
fi
if grep -q 'sparkle:edSignature' "$APPCAST"; then
  echo "    appcast signed (sparkle:edSignature present)"
else
  echo "    appcast UNSIGNED$( ((DRY_RUN)) && echo ' (expected for a dry run)')"
  ((DRY_RUN)) || die "the appcast has no sparkle:edSignature"
fi

# ---------------------------------------------------------------------------------------
# 7. Mac App Store package
# ---------------------------------------------------------------------------------------
# HOOK: the App Store session owns packaging/appstore/. Interface (packaging/README.md):
#   packaging/appstore/build-appstore.sh [--dry-run]  ->  dist/appstore/Vault-Wallpaper-<v>.pkg
APPSTORE_SCRIPT="$PKG/appstore/build-appstore.sh"
APPSTORE_PKG=""
step "Mac App Store package"
if ((!APPSTORE)); then
  echo "    skipped (--no-appstore)"
elif [[ -x "$APPSTORE_SCRIPT" ]]; then
  if ((DRY_RUN)); then
    "$APPSTORE_SCRIPT" --dry-run
  else
    "$APPSTORE_SCRIPT"
  fi
  APPSTORE_PKG="$DIST/appstore/Vault-Wallpaper-$VERSION.pkg"
  [[ -f "$APPSTORE_PKG" ]] || echo "    warning: expected $APPSTORE_PKG"
else
  echo "    HOOK NOT CONNECTED: $APPSTORE_SCRIPT does not exist yet, so no App Store build."
fi

# ---------------------------------------------------------------------------------------
# 8. What to run next
# ---------------------------------------------------------------------------------------
NOTES="$DIST/release-notes-$VERSION.md"
awk -v v="$VERSION" '$0 ~ "^## \\[" v "\\]" {on=1; next} /^## \[/ {on=0} on' CHANGELOG.md >"$NOTES"
[[ -s "$NOTES" ]] || echo "Vault Wallpaper $VERSION" >"$NOTES"

cat <<EOF

==> [release] done$( ((DRY_RUN)) && echo ' (DRY RUN: unsigned, not notarized, nothing uploaded)')
    DMG      $DMG
             sha256 $SHA256  ($DMG_BYTES bytes)
    appcast  $APPCAST
    notes    $NOTES${APPSTORE_PKG:+
    pkg      $APPSTORE_PKG}
EOF

if [[ -n "$PRERELEASE" ]]; then
  cat <<EOF

    WARNING: $VERSION is a pre-release. GitHub's releases/latest (the Sparkle feed URL) skips
    pre-releases, so Sparkle users won't be offered it, and Homebrew users will still get it.
    To ship it as an update anyway, drop --prerelease from the gh command below.
EOF
fi

if ((DRY_RUN)); then
  cat <<EOF

A real run would end by printing these commands (don't run them for a dry-run build):
EOF
fi

cat <<EOF

1. Commit and push the bump (no tag yet):

   git commit -am "chore(release): $VERSION"
   git push origin HEAD

2. Publish the GitHub Release with the DMG and the appcast. --target makes GitHub create the
   tag only after the files are up; if that starts .github/workflows/release.yml, the
   workflow sees the DMG is already there and stops. Don't push the tag before this step.

   gh release create $TAG \\
     "$DMG" \\
     "$APPCAST" \\
     --repo $REPO \\
     --target "\$(git rev-parse HEAD)" \\
     --title "Vault Wallpaper $VERSION" \\
     --notes-file "$NOTES"${PRERELEASE:+ \\
     $PRERELEASE}

   git fetch --tags

3. Bump the Homebrew cask in the tap (a clone of william-ricchiuti/homebrew-tap):

   packaging/homebrew/bump-cask.sh $VERSION "$DMG" ~/dev/homebrew-tap
   (then commit and push in the tap; see docs/release-checklist.md)
EOF
if [[ -n "$APPSTORE_PKG" ]]; then
  cat <<EOF

4. Upload the App Store package (Transporter, or):

   xcrun altool --upload-package "$APPSTORE_PKG" --type macos ...   # see packaging/appstore/
EOF
fi
