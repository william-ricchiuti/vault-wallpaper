# Packaging

Scripts that turn the `vault-wallpaper` crate into a signed, notarized DMG. Run them from
anywhere; they find the repository root themselves. Output goes to `dist/` (git-ignored).

| Step | Script | Needs | Output |
|---|---|---|---|
| 1 | `build-app.sh` | rustup, Xcode command line tools | `dist/Vault Wallpaper.app` (universal, unsigned) |
| 2 | `sign.sh` | `VW_SIGN_IDENTITY`, a Developer ID Application certificate | the app, signed with the hardened runtime |
| 3 | `notarize.sh <app>` | `VW_NOTARY_PROFILE` (one-time setup below), network | the app, notarized and stapled |
| 4 | `make-dmg.sh` | nothing extra (signs the DMG too if `VW_SIGN_IDENTITY` is set) | `dist/Vault-Wallpaper-<version>.dmg` |
| 5 | `notarize.sh <dmg>` | as step 3 | the DMG, notarized and stapled |
| 6 | `make-appcast.sh` | the Sparkle private key (Keychain, or `VW_SPARKLE_KEY_FILE`) | `dist/appcast/appcast.xml` |

```bash
export VW_SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)"
export VW_NOTARY_PROFILE=vault-wallpaper-notary
packaging/build-app.sh
packaging/sign.sh
packaging/notarize.sh "dist/Vault Wallpaper.app"
packaging/make-dmg.sh
packaging/notarize.sh
```

The app is notarized and stapled before it goes into the DMG, so the copy users end up with
(dragged out of the DMG, installed by Homebrew, or swapped in by Sparkle) carries its own
ticket and opens offline without a Gatekeeper network check.

For a local, unsigned test build, steps 1 and 4 are enough. Gatekeeper will block an
unsigned DMG downloaded from the web, so publish only after steps 2, 3 and 5 (signing and
notarizing). `release.sh` runs the same steps in this order: its steps 2 (build), 3 (sign),
3b (notarize the app), 4 (DMG), 5 (notarize the DMG) and 6 (appcast).

## 1. `build-app.sh`

Builds the app for `aarch64-apple-darwin` and `x86_64-apple-darwin` (it runs
`rustup target add` for a missing target and says so), joins them with `lipo`, and writes the
bundle with its `Info.plist`:

- `CFBundleIdentifier` `com.williamricchiuti.vaultwallpaper`
- `CFBundleName` / `CFBundleDisplayName` "Vault Wallpaper"
- `CFBundleShortVersionString`: the numeric part of the version in `crates/app/Cargo.toml`
  (`1.0.0-beta.1` -> `1.0.0`); `CFBundleVersion`: a build number that only goes up
  (`VW_BUILD_NUMBER`, else the commit count); `VWVersion`: the full version, used for the DMG name
- `LSUIElement` true (no Dock icon), `LSMinimumSystemVersion` 13.0, `NSHighResolutionCapable` true,
  `LSApplicationCategoryType` `public.app-category.productivity` (same as the App Store build)

**Build number.** Sparkle and App Store Connect both refuse a `CFBundleVersion` that isn't
higher than the last release's. The commit count only rises along `main`, so
[`build-number.sh`](build-number.sh) (used by both build scripts and `release.sh`) stops the
build on a shallow clone (`git rev-parse --is-shallow-repository`; it would count 1) and when
`HEAD` isn't an ancestor of `origin/main`. Overrides:

- `VW_BUILD_NUMBER=<n>`: use exactly this number and skip the git checks (local tests).
- `VW_BUILD_FROM_ANY_COMMIT=1`: count commits anyway, for a deliberate hotfix from a branch
  (make sure the count is still above the last release's) or a local test build.
  `release.sh --dry-run` sets it itself when the check fails, with a warning.

The 13.0 floor comes from the plist alone. The script deliberately unsets
`MACOSX_DEPLOYMENT_TARGET`: with rustc 1.93 and the Xcode 27 linker, setting it together with
`--target` breaks proc-macro crates (`E0463 can't find crate for thiserror_impl`). Cargo does not
rebuild when that variable changes, so if you hit E0463 after setting it, run `cargo clean`.

It honours `CARGO_TARGET_DIR`. The icon, `icon/icon-1024.png`, is drawn by code (the menu-bar
glyph, lit, on a dark rounded square) and committed; the script turns it into `AppIcon.icns`
with `sips` and `iconutil`. To redraw it:

```bash
cargo run --release -p vw-render --example app_icon -- packaging/icon/icon-1024.png
```

Everything the app needs at run time (the settings page, presets, thumbnails) is compiled
into the binary, so the bundle has no other resources, except the updater below and
`THIRD-PARTY-NOTICES.txt`.

## Licence notices: `notices.sh`

`THIRD-PARTY-NOTICES.txt` (repo root, committed) lists every crate compiled into the macOS
binary with its licence text, from [cargo-about](https://github.com/EmbarkStudios/cargo-about)
(`notices/about.toml`, `notices/notices.hbs`), followed by Sparkle's `LICENSE` with its
external licences. `build-app.sh` and `appstore/build.sh` copy it to `Contents/Resources/`,
`make-dmg.sh` to the DMG root. Regenerate it after any `Cargo.lock` or Sparkle change:

```bash
cargo install --locked cargo-about --version 0.9.3 --features cli   # once (the version CI pins)
packaging/notices.sh                  # rewrite it; commit the result
packaging/notices.sh --check          # exit 1 if it is stale (release.sh and both workflows run this)
```

A dependency under a licence not listed in `notices/about.toml`'s `accepted` stops the
script until someone reviews it. For a crate offered under a choice ("MIT OR Apache-2.0")
the first accepted licence in that list is the one reproduced.

## Build flavours: the `updater` feature

| Flavour | Cargo | Sparkle | Built by |
|---|---|---|---|
| Direct download (DMG, Homebrew) | `--features updater` | `Contents/Frameworks/Sparkle.framework`, `SU*` keys in `Info.plist` | `build-app.sh` (default: `VW_FEATURES` unset means `updater`) |
| Mac App Store | no `updater` feature | none | `VW_FEATURES= build-app.sh` (set and empty), or the App Store script |
| `cargo run` / `cargo build` | off by default | none | |

`updater` is a feature of the `vault-wallpaper` crate and is **off by default**, so a build
that doesn't ask for it (the App Store build included) contains no Sparkle code: Apple rejects
apps with their own updater. Without the feature, `crates/app/src/updater.rs` compiles to a
stub that returns `None`. With it, the app loads `Contents/Frameworks/Sparkle.framework` at
run time by path (it is not linked, so no rpath is involved and both slices of the universal
binary use the same, itself universal, framework). It stays inert, with no menu item, when
the framework is missing (a bare `cargo run --features updater`) or when `SUPublicEDKey` is
still the placeholder.

## Updates (Sparkle 2)

Pinned in [`sparkle.env`](sparkle.env): version, download URL, SHA-256, the feed URL and the
public key. `fetch-sparkle.sh` downloads the release, refuses it if the checksum differs, and
unpacks it into `.cache/sparkle/<version>/` (git-ignored); the other scripts call it.

`build-app.sh` writes these `Info.plist` keys (override with `VW_SPARKLE_FEED_URL` and
`VW_SPARKLE_PUBLIC_KEY` for tests):

- `SUFeedURL`: `https://github.com/william-ricchiuti/vault-wallpaper/releases/download/appcast/appcast.xml`,
  the `appcast.xml` asset of a release of its own, tag `appcast` (`SPARKLE_APPCAST_TAG`). It
  is created with the first release and its asset replaced (`gh release upload --clobber`)
  with every later one, so the URL exists from the first release on and never moves. It is
  marked pre-release so it never becomes "latest". This replaced
  `releases/latest/download/appcast.xml`, which 404s while every release is a pre-release
  (GitHub's "latest" skips them), so betas would have had no feed at all. The repo (or
  whatever repo holds the releases) must be public.
- `SUPublicEDKey`: from `sparkle.env`. While it is `REPLACE_WITH_SPARKLE_PUBLIC_ED_KEY`, the
  script warns and the app keeps its updater off.
- `SUEnableAutomaticChecks` true (no "check automatically?" prompt on second launch) and
  `SUScheduledCheckInterval` 86400, Sparkle's default of once a day.

The app has no Dock icon, so a window opened by a scheduled check could sit unseen behind
the active app. `crates/app/src/updater.rs` gives Sparkle a user-driver delegate that
answers YES to `supportsGentleScheduledUpdateReminders` and, in
`standardUserDriverWillHandleShowingUpdate:forUpdate:state:`, activates the app so Sparkle's
update window comes to the front. Tested end to end in `docs/sparkle-e2e.md`.

`sign.sh` signs the framework inside out before the app (`Installer.xpc`, `Downloader.xpc`
with its entitlements kept, `Autoupdate`, `Updater.app`, the framework), all with the
hardened runtime. `VW_SIGN_IDENTITY=-` signs ad hoc, for local tests only.

### Keys: `sparkle-keys.sh` (once, William)

Runs Sparkle's `generate_keys`: the private EdDSA key goes into the login Keychain and the
public key is printed. Put it in `sparkle.env` as `SPARKLE_PUBLIC_ED_KEY` and commit that.
Back up the private key with `packaging/sparkle-keys.sh -x <file>` into a password manager;
if it is lost, installed copies can't accept any later update. Never commit it.

### Appcast: `make-appcast.sh`

```bash
packaging/make-appcast.sh <dmg> [<version>] [--out <file>] [--unsigned]
```

Run it after the DMG is signed, notarized and stapled: the signature covers the exact bytes
users download. It runs `generate_appcast` on a scratch folder holding only that DMG. That
reads the build number from the app inside the DMG, signs the DMG and writes the `<item>`,
with the enclosure
`https://github.com/william-ricchiuti/vault-wallpaper/releases/download/v<version>/Vault-Wallpaper-<version>.dmg`
(`VW_APPCAST_URL_PREFIX` overrides the folder). `<version>` defaults to the app's `VWVersion`.
The output goes to `--out`, by default `dist/appcast/appcast.xml`. If a file is already there,
the new item goes on top. `release.sh` never relies on a leftover file: a real run deletes it,
then starts from the published feed (downloaded from the `appcast` release with `gh`, or with
`curl` from the feed URL; `VW_PREVIOUS_APPCAST=<file>` to give it by hand), or from nothing when
that release doesn't exist yet. A new feed would drop every older version's item, so it is
only started when `gh` (logged in, able to see the repo) says there is no `appcast` release,
or with `VW_PREVIOUS_APPCAST=none`. Without `gh`, a 404 from the feed URL stops the run: while
the repo is private it can mean "no access" as well as "no feed yet". It then fails if any `<item>` lacks `sparkle:edSignature`. The path is the last line on stdout. The private key is taken from
`VW_SPARKLE_PRIVATE_KEY` (the base64 key, for CI), else `VW_SPARKLE_KEY_FILE`, else the
login Keychain. `generate_appcast` silently leaves the item unsigned when that key doesn't
match the app's `SUPublicEDKey`, so `make-appcast.sh` checks the pair first (for the env and
file keys) and stops with the reason (`docs/sparkle-e2e.md`, "CI signing path"). `--unsigned` writes the item without a signature, for dry runs; Sparkle
rejects such a feed. Upload `appcast.xml` to the `appcast` release, after the DMG it points to is published.

Sparkle compares `CFBundleVersion` (the build number), so every release needs a higher one.

Check the result:

```bash
lipo -info "dist/Vault Wallpaper.app/Contents/MacOS/vault-wallpaper"
plutil -lint "dist/Vault Wallpaper.app/Contents/Info.plist"
open "dist/Vault Wallpaper.app"
```

To try a first run without touching your real settings, give it a scratch home folder:
`open --env HOME=/tmp/vw-home "dist/Vault Wallpaper.app"`. Quit it from its menu-bar icon.

## 2. `sign.sh`

Signs with `codesign --options runtime --timestamp` and `entitlements.plist` (empty for now:
not sandboxed, no JIT, no unsigned libraries). Without `VW_SIGN_IDENTITY` it prints how to
find the identity and exits with code 2 without signing anything:

```bash
security find-identity -v -p codesigning
```

Use the `Developer ID Application: …` line, not `Apple Development: …`.

## 4. `make-dmg.sh`

Copies the app with `ditto` (keeps the signature and the stapled ticket) next to an
`Applications` symlink and `THIRD-PARTY-NOTICES.txt`, and makes a compressed (UDZO) DMG with
`hdiutil`. Sign and notarize the app before this step: the DMG holds a copy.

## 3 and 5. `notarize.sh`

Given an `.app`, it zips it with `ditto -c -k --keepParent`, submits the zip with
`xcrun notarytool submit --wait` and staples the ticket to the `.app`. Given a DMG (the
default: the newest `dist/*.dmg`), it submits and staples the DMG. Both end with
`stapler validate` and an `spctl` check. Without `VW_NOTARY_PROFILE` (or the API key
variables) it explains the setup and exits with code 2.

One-time setup: create an app-specific password at account.apple.com (Sign-In and Security >
App-Specific Passwords), then store it in the keychain under a profile name:

```bash
xcrun notarytool store-credentials vault-wallpaper-notary --apple-id "you@example.com" --team-id ABCDE12345
```

The team ID is the 10-character code at the end of your Developer ID identity. If a
submission comes back "Invalid", read Apple's reason with
`xcrun notarytool log <submission-id> --keychain-profile vault-wallpaper-notary`.

## Mac App Store

A sandboxed build without the self-updater, signed for the store and packaged with
`productbuild`, is in [`appstore/`](appstore/README.md): `packaging/appstore/build.sh
--dry-run` for a local test, and the steps William takes in App Store
Connect. `release.sh` calls it. The same build can be archived and uploaded from Xcode with
[`../xcode/VaultWallpaper.xcodeproj`](../xcode/VaultWallpaper.xcodeproj).

## One command: `release.sh`

```bash
packaging/release.sh 1.0.0-beta.2 --dry-run   # anyone: builds everything, signs nothing
packaging/release.sh 1.0.0-beta.2             # William: the real build
```

In order, it:

1. checks, first of all, that `SPARKLE_PUBLIC_ED_KEY` in `sparkle.env` isn't the
   `REPLACE_WITH_…` placeholder, that no `VW_SPARKLE_*` test override is set and that
   `CHANGELOG.md` has a `## [<version>]` section; then the tools, a clean tree,
   `VW_SIGN_IDENTITY`, notary credentials, `make-appcast.sh`, the build number
   (`build-number.sh`) and `notices.sh --check` (a real run stops if cargo-about isn't
   installed; a dry run only warns)
2. sets `version` in `crates/app/Cargo.toml` and updates `Cargo.lock`
3. runs `build-app.sh` with `VW_FEATURES=updater` (the direct build has Sparkle; the App Store
   build doesn't)
4. runs `sign.sh`, `notarize.sh <app>` (staples the app), `make-dmg.sh`, `notarize.sh <dmg>`
5. starts `dist/appcast.xml` from the published feed (or empty, for the first release; it
   stops rather than guess when the feed can't be fetched), runs
   `make-appcast.sh` and checks that every `<item>` has an `sparkle:edSignature`
6. runs `appstore/build.sh` if it exists (its exit code 2, "not set up", counts as skipped)
7. writes the release notes from `CHANGELOG.md`, prints the version and `CFBundleVersion`,
   and prints the `git`, `gh release create`, feed upload and `bump-cask.sh` commands to run

It uploads nothing.

`--dry-run` skips signing, notarizing and the App Store upload. It builds an unsigned DMG and
an unsigned appcast holding only this version. Checks that would stop a real run (the
placeholder key, a missing changelog section, a build number from a branch) print
`warning (a real run stops here)` and the dry run carries on. If `make-appcast.sh` isn't there, it writes its own test appcast, marked
"do not publish". It also puts `Cargo.toml` and `Cargo.lock` back when it exits.
`--no-appstore` skips step 6; CI uses it.

The same script runs in CI (`.github/workflows/release.yml`) when a `v*` tag is pushed. If a
release with the DMG already exists, CI does nothing, so publishing from the Mac and pushing
the tag don't collide.

### Interfaces with the other packaging scripts

These are agreed between sessions. If you change one, change `release.sh` too.

| Script | Owner | Call | Output |
|---|---|---|---|
| `build-app.sh` | packaging + Sparkle session | `VW_FEATURES` unset: direct build (`updater`, Sparkle.framework embedded); `VW_FEATURES=` (set, empty): App Store build, no Sparkle; any list containing `updater` embeds Sparkle. `release.sh` sets `VW_FEATURES=updater` explicitly | `dist/Vault Wallpaper.app` |
| `notarize.sh` | packaging | `notarize.sh [<app or dmg>]`; `VW_NOTARY_PROFILE=<profile>`, or `VW_NOTARY_KEY_PATH` + `VW_NOTARY_KEY_ID` + `VW_NOTARY_ISSUER` (API key, used by CI) | the app or DMG, stapled |
| `build-number.sh` | release safety | sourced; `vw_build_number <root>` sets `BUILD_NUMBER` or fails (shallow clone, `HEAD` not on `origin/main`); `VW_BUILD_NUMBER`, `VW_BUILD_FROM_ANY_COMMIT` | |
| `notices.sh` | release safety | `notices.sh [--check]`, needs cargo-about | `THIRD-PARTY-NOTICES.txt` |
| `make-appcast.sh` | Sparkle session | `make-appcast.sh <dmg> [<version>] [--out <file>] [--unsigned]`; version defaults to the DMG app's `VWVersion`. Key source, in order: `VW_SPARKLE_PRIVATE_KEY` (base64 seed; CI's `SPARKLE_PRIVATE_KEY` secret), `VW_SPARKLE_KEY_FILE` (path), then the login Keychain. Prints the appcast path on its last stdout line | default `dist/appcast/appcast.xml`; `release.sh` passes `--out dist/appcast.xml` |
| `appstore/build.sh` | App Store session | `build.sh [--dry-run]` (dry run: ad-hoc signed, unsigned pkg). A real build needs `VW_TEAM_ID`, `VW_APP_IDENTITY`, `VW_INSTALLER_IDENTITY`, `VW_PROVISIONING_PROFILE`, else exit 2. Reads the version from `crates/app/Cargo.toml`, so it runs after the bump | `dist/appstore/Vault-Wallpaper-<short>-<build>.pkg`, path on the last stdout line |
| `homebrew/bump-cask.sh` | release automation | `bump-cask.sh <version> <dmg> [tap-dir]` | edits `<tap-dir>/Casks/vault-wallpaper.rb` |

The enclosure URL is
`https://github.com/william-ricchiuti/vault-wallpaper/releases/download/v<version>/Vault-Wallpaper-<version>.dmg`.
The feed (`SUFeedURL`) is
`https://github.com/william-ricchiuti/vault-wallpaper/releases/download/appcast/appcast.xml`:
one fixed `appcast` release whose `appcast.xml` each release replaces, after its DMG is up.

Every release that updates the feed reaches Sparkle users, pre-release or not: the
`--prerelease` flag (CI sets it for every version with a `-`) only affects GitHub's page
and "latest". To hold a release back from auto-update, don't upload its appcast (skip
step 3 of `release.sh`'s printed commands). CI always publishes the feed; to take a
release back out, download `appcast.xml` from the `appcast` release, delete that version's
`<item>` and upload the file again with `gh release upload appcast appcast.xml --clobber`.

## Homebrew

`homebrew/vault-wallpaper.rb` is the cask template, with `auto_updates true` because Sparkle
updates the app. `homebrew/bump-cask.sh` copies it into a clone of
`william-ricchiuti/homebrew-tap` and sets `version` and `sha256`. To check it locally without
publishing anything:

```bash
brew tap-new --no-git vwtest/local
mkdir -p "$(brew --repository vwtest/local)/Casks"
cp packaging/homebrew/vault-wallpaper.rb "$(brew --repository vwtest/local)/Casks/"
brew style vwtest/local/vault-wallpaper
brew trust vwtest/local     # current Homebrew won't load casks from an untrusted tap
brew audit --cask --strict vwtest/local/vault-wallpaper
brew untrust vwtest/local && brew untap vwtest/local
```

## Release

The full first-release walkthrough (one-time setup, then each release) is in
[`docs/release-checklist.md`](../docs/release-checklist.md).

## Not done yet

- Sparkle's `XPCServices` (only needed by sandboxed apps) are kept in the bundle; removing them
  would save about 0.4 MB of the framework's 2.9 MB.
- `hdiutil create` prints a deprecation warning (it suggests `diskutil image create`); it still
  works on this macOS.
