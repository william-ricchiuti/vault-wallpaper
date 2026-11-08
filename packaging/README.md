# Packaging

Scripts that turn the `vault-wallpaper` crate into a signed, notarized DMG. Run them from
anywhere; they find the repository root themselves. Output goes to `dist/` (git-ignored).

| Step | Script | Needs | Output |
|---|---|---|---|
| 1 | `build-app.sh` | rustup, Xcode command line tools | `dist/Vault Wallpaper.app` (universal, unsigned) |
| 2 | `sign.sh` | `VW_SIGN_IDENTITY`, a Developer ID Application certificate | the app, signed with the hardened runtime |
| 3 | `make-dmg.sh` | nothing extra (signs the DMG too if `VW_SIGN_IDENTITY` is set) | `dist/Vault-Wallpaper-<version>.dmg` |
| 4 | `notarize.sh` | `VW_NOTARY_PROFILE` (one-time setup below), network | the DMG, notarized and stapled |

```bash
packaging/build-app.sh
VW_SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" packaging/sign.sh
VW_SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" packaging/make-dmg.sh
VW_NOTARY_PROFILE=vault-wallpaper-notary packaging/notarize.sh
```

For a local, unsigned test build, steps 1 and 3 are enough. Gatekeeper will block an
unsigned DMG downloaded from the web, so publish only after steps 2 and 4.

## 1. `build-app.sh`

Builds the app for `aarch64-apple-darwin` and `x86_64-apple-darwin` (it runs
`rustup target add` for a missing target and says so), joins them with `lipo`, and writes the
bundle with its `Info.plist`:

- `CFBundleIdentifier` `com.williamricchiuti.vaultwallpaper`
- `CFBundleName` / `CFBundleDisplayName` "Vault Wallpaper"
- `CFBundleShortVersionString`: the numeric part of the version in `crates/app/Cargo.toml`
  (`1.0.0-beta.1` -> `1.0.0`); `CFBundleVersion`: a build number that only goes up
  (`VW_BUILD_NUMBER`, else the commit count); `VWVersion`: the full version, used for the DMG name
- `LSUIElement` true (no Dock icon), `LSMinimumSystemVersion` 13.0, `NSHighResolutionCapable` true

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
into the binary, so the bundle has no other resources.

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

## 3. `make-dmg.sh`

Copies the app with `ditto` (keeps the signature) next to an `Applications` symlink and makes
a compressed (UDZO) DMG with `hdiutil`. Sign the app before this step: the DMG holds a copy.

## 4. `notarize.sh`

Submits the DMG with `xcrun notarytool submit --wait`, then `xcrun stapler staple` and
`stapler validate`. Without `VW_NOTARY_PROFILE` it explains the setup and exits with code 2.

One-time setup: create an app-specific password at account.apple.com (Sign-In and Security >
App-Specific Passwords), then store it in the keychain under a profile name:

```bash
xcrun notarytool store-credentials vault-wallpaper-notary --apple-id "you@example.com" --team-id ABCDE12345
```

The team ID is the 10-character code at the end of your Developer ID identity. If a
submission comes back "Invalid", read Apple's reason with
`xcrun notarytool log <submission-id> --keychain-profile vault-wallpaper-notary`.

## One command: `release.sh`

```bash
packaging/release.sh 1.0.0-beta.2 --dry-run   # anyone: builds everything, signs nothing
packaging/release.sh 1.0.0-beta.2             # William: the real build
```

In order, it:

1. checks the version and the tools; for a real run, also a clean tree, `VW_SIGN_IDENTITY`,
   notary credentials and `make-appcast.sh`
2. sets `version` in `crates/app/Cargo.toml` and updates `Cargo.lock`
3. runs `build-app.sh` with `VW_FEATURES=updater` (the direct build has Sparkle; the App Store
   build doesn't)
4. runs `sign.sh`, then `make-dmg.sh`, then `notarize.sh`
5. runs `make-appcast.sh` and checks the result has an `sparkle:edSignature`
6. runs `appstore/build-appstore.sh` if it exists
7. writes the release notes from `CHANGELOG.md` and prints the `git`, `gh release create`
   and `bump-cask.sh` commands to run

It uploads nothing.

`--dry-run` skips signing, notarizing and the App Store upload. It builds an unsigned DMG and
an unsigned appcast. If `make-appcast.sh` isn't there, it writes its own test appcast, marked
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
| `notarize.sh` | packaging | `VW_NOTARY_PROFILE=<profile>`, or `VW_NOTARY_KEY_PATH` + `VW_NOTARY_KEY_ID` + `VW_NOTARY_ISSUER` (API key, used by CI) | the DMG, stapled |
| `make-appcast.sh` | Sparkle session | `make-appcast.sh <dmg> [<version>] [--out <file>] [--unsigned]`; version defaults to the DMG app's `VWVersion`. Key source, in order: `VW_SPARKLE_PRIVATE_KEY` (base64 seed; CI's `SPARKLE_PRIVATE_KEY` secret), `VW_SPARKLE_KEY_FILE` (path), then the login Keychain. Prints the appcast path on its last stdout line | default `dist/appcast/appcast.xml`; `release.sh` passes `--out dist/appcast.xml` |
| `appstore/build-appstore.sh` | App Store session | `build-appstore.sh [--dry-run]`; the version is already bumped | `dist/appstore/Vault-Wallpaper-<version>.pkg` |
| `homebrew/bump-cask.sh` | release automation | `bump-cask.sh <version> <dmg> [tap-dir]` | edits `<tap-dir>/Casks/vault-wallpaper.rb` |

The enclosure URL is
`https://github.com/william-ricchiuti/vault-wallpaper/releases/download/v<version>/Vault-Wallpaper-<version>.dmg`.
The feed (`SUFeedURL`) is
`https://github.com/william-ricchiuti/vault-wallpaper/releases/latest/download/appcast.xml`,
so every release uploads its own `appcast.xml`.

GitHub's `releases/latest` skips pre-releases, so a release published with `--prerelease`
never reaches Sparkle users: their feed keeps pointing at the last stable release's
appcast. `release.sh` warns about this for `-beta`/`-rc` versions. To ship a beta as an
update, drop `--prerelease` from the printed `gh release create` command, which makes it
"latest". CI marks every version with a `-` as a pre-release.

## Homebrew

`homebrew/vault-wallpaper.rb` is the cask template, with `auto_updates true` because Sparkle
updates the app. `homebrew/bump-cask.sh` copies it into a clone of
`william-ricchiuti/homebrew-tap` and sets `version` and `sha256`. To check it locally without
publishing anything:

```bash
brew tap-new --no-git vwtest/local
cp packaging/homebrew/vault-wallpaper.rb "$(brew --repository vwtest/local)/Casks/"
brew style vwtest/local/vault-wallpaper
brew audit --cask --strict vwtest/local/vault-wallpaper
brew untap vwtest/local
```

## Release

The full first-release walkthrough (one-time setup, then each release) is in
[`docs/release-checklist.md`](../docs/release-checklist.md).

## Not done yet

- `hdiutil create` prints a deprecation warning (it suggests `diskutil image create`); it still
  works on this macOS.
