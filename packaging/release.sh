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
#   the real SPARKLE_PUBLIC_ED_KEY in packaging/sparkle.env (checked first)
#   a "## [<version>]" section in CHANGELOG.md, a clean tree, HEAD on origin/main
#   VW_SIGN_IDENTITY                          "Developer ID Application: Name (TEAMID)"
#   VW_NOTARY_PROFILE, or VW_NOTARY_KEY_PATH + VW_NOTARY_KEY_ID + VW_NOTARY_ISSUER
#   a Sparkle key for make-appcast.sh: VW_SPARKLE_PRIVATE_KEY, VW_SPARKLE_KEY_FILE, or the
#   login Keychain (where sparkle-keys.sh puts it)
#   VW_PREVIOUS_APPCAST (optional): the current feed, if it can't be downloaded; or "none"
#   to start a new feed on purpose (first release only). Without it the feed history comes
#   from gh (logged in, able to see the repo) or a 200 from the feed URL; anything else stops.
#
# It uploads nothing. At the end it prints the git and `gh release` commands to run.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PKG="$ROOT/packaging"
DIST="$ROOT/dist"
REPO="william-ricchiuti/vault-wallpaper"
APP_TOML="crates/app/Cargo.toml"

usage() { sed -n '2,27p' "$0" | sed 's/^# \{0,1\}//'; }

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
# shellcheck source=sparkle.env
source "$PKG/sparkle.env"
# The Sparkle feed lives on its own release (tag $SPARKLE_APPCAST_TAG), so its URL always
# exists, whatever the newest version release is (packaging/README.md, "Updates").
FEED_URL="https://github.com/$REPO/releases/download/$SPARKLE_APPCAST_TAG/appcast.xml"

step() { printf '\n==> [release] %s\n' "$*"; }
skip() { printf '    [dry-run] skipped: %s\n' "$*"; }
die() { echo "error: $*" >&2; exit 1; }
# A dry run reports what would stop a real one and carries on.
check_fail() {
  if ((DRY_RUN)); then echo "    warning (a real run stops here): $*"; else die "$*"; fi
}

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
if ((!BUMP_ONLY)); then
  # First, before anything else: a release built with the placeholder key ships an app whose
  # updater is off for good (no later update can reach it).
  if [[ "$SPARKLE_PUBLIC_ED_KEY" == REPLACE_WITH_* ]]; then
    check_fail "SPARKLE_PUBLIC_ED_KEY in packaging/sparkle.env is still the placeholder; run packaging/sparkle-keys.sh and commit the public key (docs/release-checklist.md, A3)"
  else
    echo "    Sparkle public key: ${SPARKLE_PUBLIC_ED_KEY:0:8}..."
  fi
  for v in VW_SPARKLE_PUBLIC_KEY VW_SPARKLE_FEED_URL; do
    [[ -z "${!v:-}" ]] || check_fail "$v is set (a test override); a release must use packaging/sparkle.env"
  done
  if [[ "$SPARKLE_FEED_URL" != "$FEED_URL" ]]; then
    check_fail "SPARKLE_FEED_URL in packaging/sparkle.env is $SPARKLE_FEED_URL, but the appcast is published at $FEED_URL"
  fi
  if grep -qF "## [$VERSION]" CHANGELOG.md; then
    echo "    CHANGELOG.md has a [$VERSION] section"
  else
    check_fail "CHANGELOG.md has no '## [$VERSION]' section; move the Unreleased notes under it"
  fi
fi
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
if ((!BUMP_ONLY)); then
  # CFBundleVersion is the commit count, which only rises along main (build-number.sh).
  # shellcheck source=build-number.sh
  source "$PKG/build-number.sh"
  if BUILD_ERR="$(vw_build_number "$ROOT" 2>&1)"; then
    vw_build_number "$ROOT"
    echo "    build number (CFBundleVersion): $BUILD_NUMBER"
  else
    printf '%s\n' "$BUILD_ERR" | sed 's/^/    /'
    check_fail "the build number can't be trusted (above)"
    # Dry run: build anyway, from whatever commit this is.
    export VW_BUILD_FROM_ANY_COMMIT=1
  fi
  # Licence notices shipped in the app and the DMG (packaging/notices.sh). The version CI
  # installs (.github/workflows/*.yml, CARGO_ABOUT_VERSION); other versions may word the
  # output differently and fail the check.
  CARGO_ABOUT_VERSION=0.9.3
  if command -v cargo-about >/dev/null; then
    if "$PKG/notices.sh" --check; then
      echo "    THIRD-PARTY-NOTICES.txt is up to date"
    else
      check_fail "THIRD-PARTY-NOTICES.txt is out of date; run packaging/notices.sh and commit it"
    fi
  else
    # A real release must not ship notices that no longer match Cargo.lock.
    check_fail "cargo-about isn't installed, so THIRD-PARTY-NOTICES.txt can't be checked (cargo install --locked cargo-about --version $CARGO_ABOUT_VERSION --features cli)"
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
BUILD="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$APP/Contents/Info.plist")"

# ---------------------------------------------------------------------------------------
# 3b. Notarize and staple the app itself, so the copy in /Applications (from the DMG,
#     Homebrew or a Sparkle update) carries its own ticket and passes Gatekeeper offline.
# ---------------------------------------------------------------------------------------
step "notarize and staple the app"
if ((DRY_RUN)); then
  skip "packaging/notarize.sh \"$APP\" (zip, notarytool submit --wait, staple the .app)"
else
  "$PKG/notarize.sh" "$APP"
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
step "notarize and staple the DMG"
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
# make-appcast.sh adds the new item on top of whatever is already at --out, so never start
# from a leftover file: a real run starts from the published feed (its signed items are the
# update history) or, for the very first release, from nothing.
rm -f "$APPCAST"
fetch_previous_appcast() {
  local tmp code why
  if [[ "${VW_PREVIOUS_APPCAST:-}" == none ]]; then
    echo "    VW_PREVIOUS_APPCAST=none: starting a new feed (only right for the very first release)"
    return
  fi
  if [[ -n "${VW_PREVIOUS_APPCAST:-}" ]]; then
    [[ -f "$VW_PREVIOUS_APPCAST" ]] || die "VW_PREVIOUS_APPCAST=$VW_PREVIOUS_APPCAST doesn't exist"
    cp "$VW_PREVIOUS_APPCAST" "$APPCAST"
    echo "    starting from VW_PREVIOUS_APPCAST ($VW_PREVIOUS_APPCAST)"
    return
  fi
  # Starting a new feed once the `appcast` release exists would publish a one-item feed and
  # drop every older version's item, so a new feed is only started when gh can see the repo
  # and says the release isn't there. Anything less certain stops the run.
  if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
    # Works while the repo is private, too. gh reports "release not found" for any 404,
    # including a repo the token can't see, so check the repo first.
    why="$(gh repo view "$REPO" --json name 2>&1)" ||
      die "gh can't see $REPO ($why); log in with access to it, or set VW_PREVIOUS_APPCAST to the current feed"
    if why="$(gh release view "$SPARKLE_APPCAST_TAG" --repo "$REPO" --json tagName 2>&1)"; then
      gh release download "$SPARKLE_APPCAST_TAG" --repo "$REPO" --pattern appcast.xml --output "$APPCAST" ||
        die "release $SPARKLE_APPCAST_TAG exists but its appcast.xml couldn't be downloaded; set VW_PREVIOUS_APPCAST to the current feed"
      echo "    starting from the published feed (release $SPARKLE_APPCAST_TAG)"
    elif grep -qi 'release not found' <<<"$why"; then
      echo "    no $SPARKLE_APPCAST_TAG release yet: this is the first release, starting a new feed"
    else
      die "couldn't look up release $SPARKLE_APPCAST_TAG ($why); set VW_PREVIOUS_APPCAST to the current feed"
    fi
    return
  fi
  # No usable gh: only a downloaded feed is good enough. A 404 here can't tell "no feed yet"
  # from "no access" (a private repo answers 404), so it stops the run too.
  tmp="$(mktemp "${TMPDIR:-/tmp}/vw-appcast-prev.XXXXXX")"
  code="$(curl -sSL -o "$tmp" -w '%{http_code}' "$FEED_URL" || true)"
  case "$code" in
    200) mv "$tmp" "$APPCAST"; echo "    starting from the published feed ($FEED_URL)" ;;
    404)
      rm -f "$tmp"
      die "$FEED_URL: 404, and gh isn't installed or logged in, so it can't be told whether the feed is missing or just not visible. Log in with gh, set VW_PREVIOUS_APPCAST to the current feed, or, for the very first release only, VW_PREVIOUS_APPCAST=none"
      ;;
    *) rm -f "$tmp"; die "couldn't download $FEED_URL (HTTP ${code:-none}); set VW_PREVIOUS_APPCAST to the current feed" ;;
  esac
}
if [[ -x "$PKG/make-appcast.sh" ]]; then
  if ((DRY_RUN)); then
    echo "    dry run: a new, unsigned feed holding just this version"
    "$PKG/make-appcast.sh" "$DMG" "$VERSION" --out "$APPCAST" --unsigned
  else
    fetch_previous_appcast
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
# Every <item>, old and new, must carry an EdDSA signature: Sparkle refuses an unsigned one.
read -r ITEMS UNSIGNED_ITEMS < <(awk '
  /<item[ >]/ {n++; inside=1; signed=0}
  inside && /sparkle:edSignature=/ {signed=1}
  /<\/item>/ {if (inside && !signed) bad++; inside=0}
  END {print n+0, bad+0}' "$APPCAST")
if ((ITEMS == 0)); then
  die "$APPCAST has no <item>"
elif ((UNSIGNED_ITEMS == 0)); then
  echo "    appcast: $ITEMS item(s), all signed (sparkle:edSignature)"
elif ((DRY_RUN)); then
  echo "    appcast: $UNSIGNED_ITEMS of $ITEMS item(s) UNSIGNED (expected for a dry run)"
else
  die "$APPCAST: $UNSIGNED_ITEMS of $ITEMS <item>s have no sparkle:edSignature"
fi

# ---------------------------------------------------------------------------------------
# 7. Mac App Store package
# ---------------------------------------------------------------------------------------
# The App Store session owns packaging/appstore/. Interface (packaging/README.md):
#   packaging/appstore/build.sh [--dry-run]
#   -> dist/appstore/Vault-Wallpaper-<short>-<build>.pkg, its path on the last stdout line.
#   Exit 2 means "not set up" (no VW_TEAM_ID / identities / profile): skipped, not a failure.
APPSTORE_SCRIPT="$PKG/appstore/build.sh"
APPSTORE_PKG=""
step "Mac App Store package"
if ((!APPSTORE)); then
  echo "    skipped (--no-appstore)"
elif [[ -x "$APPSTORE_SCRIPT" ]]; then
  APPSTORE_LOG="$DIST/appstore-build.log"
  mkdir -p "$DIST"
  # Progress shows as it runs; stdout is also kept so the last line gives the pkg path.
  set +e
  if ((DRY_RUN)); then
    "$APPSTORE_SCRIPT" --dry-run | tee "$APPSTORE_LOG"
  else
    "$APPSTORE_SCRIPT" | tee "$APPSTORE_LOG"
  fi
  APPSTORE_RC=${PIPESTATUS[0]}
  set -e
  case "$APPSTORE_RC" in
    0)
      APPSTORE_PKG="$(tail -n 1 "$APPSTORE_LOG")"
      [[ -f "$APPSTORE_PKG" ]] || die "App Store build said it wrote '$APPSTORE_PKG', which doesn't exist"
      ;;
    2) echo "    App Store step skipped: not set up (see packaging/appstore/README.md)" ;;
    *) die "packaging/appstore/build.sh failed (exit $APPSTORE_RC)" ;;
  esac
else
  echo "    HOOK NOT CONNECTED: $APPSTORE_SCRIPT is not on this branch yet, so no App Store build."
fi

# ---------------------------------------------------------------------------------------
# 8. What to run next
# ---------------------------------------------------------------------------------------
NOTES="$DIST/release-notes-$VERSION.md"
# This version's section, up to the next "## [" heading or the trailing link definitions
# ("[1.0.0]: https://..."), which mean nothing in a GitHub Release body.
awk -v v="$VERSION" '$0 ~ "^## \\[" v "\\]" {on=1; next} /^## \[/ || /^\[[^]]+\]: / {on=0} on' CHANGELOG.md >"$NOTES"
[[ -s "$NOTES" ]] || echo "Vault Wallpaper $VERSION" >"$NOTES"

cat <<EOF

==> [release] done$( ((DRY_RUN)) && echo ' (DRY RUN: unsigned, not notarized, nothing uploaded)')
    version  $VERSION, CFBundleVersion (build number) $BUILD
    DMG      $DMG
             sha256 $SHA256  ($DMG_BYTES bytes)
    appcast  $APPCAST
    notes    $NOTES${APPSTORE_PKG:+
    pkg      $APPSTORE_PKG}
EOF

if [[ -n "$PRERELEASE" ]]; then
  cat <<EOF

    Note: $VERSION is marked a GitHub pre-release. The Sparkle feed is the fixed
    $SPARKLE_APPCAST_TAG release, not releases/latest, so step 3 below still offers it to
    Sparkle users. Skip step 3 to hold a release back from auto-update.
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

2. Publish the GitHub Release with the DMG. --target makes GitHub create the tag only after
   the file is up; if that starts .github/workflows/release.yml, the workflow sees the DMG
   is already there and stops. Don't push the tag before this step.

   gh release create $TAG \\
     "$DMG" \\
     --repo $REPO \\
     --target "\$(git rev-parse HEAD)" \\
     --title "Vault Wallpaper $VERSION" \\
     --notes-file "$NOTES"${PRERELEASE:+ \\
     $PRERELEASE}

   git fetch --tags

3. Publish the Sparkle feed, after the DMG it points to is up. The feed is appcast.xml on
   the fixed release $SPARKLE_APPCAST_TAG (created once, marked pre-release so it is never
   "latest"). --clobber replaces the previous feed, whose items this one already holds.

   gh release view $SPARKLE_APPCAST_TAG --repo $REPO >/dev/null 2>&1 ||
     gh release create $SPARKLE_APPCAST_TAG --repo $REPO --prerelease --latest=false \\
       --target "\$(git rev-parse HEAD)" --title "Sparkle update feed" \\
       --notes "appcast.xml for Vault Wallpaper's automatic updates. Not a release; don't delete it."
   gh release upload $SPARKLE_APPCAST_TAG "$APPCAST" --repo $REPO --clobber
   curl -fsSL $FEED_URL | grep -c '<item'   # the feed is live

4. Bump the Homebrew cask in the tap (a clone of william-ricchiuti/homebrew-tap):

   packaging/homebrew/bump-cask.sh $VERSION "$DMG" ~/dev/homebrew-tap
   (then commit and push in the tap; see docs/release-checklist.md)
EOF
if [[ -n "$APPSTORE_PKG" ]]; then
  cat <<EOF

5. Upload the App Store package with Transporter, or with xcrun altool --upload-app
   (steps in packaging/appstore/README.md, "Each release"):

   $APPSTORE_PKG
EOF
fi
