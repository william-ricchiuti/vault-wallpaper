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

## Mac App Store

A sandboxed build without the self-updater, signed for the store and packaged with
`productbuild`, is in [`appstore/`](appstore/README.md): `packaging/appstore/build.sh
--dry-run` for a local test, and the steps William takes in App Store
Connect.

## Release

The full release sequence (sign, notarize, staple, DMG, GitHub Release, Homebrew) is in
[`docs/release-checklist.md`](../docs/release-checklist.md). The cask draft is
[`homebrew/vault-wallpaper.rb`](homebrew/vault-wallpaper.rb).

## Not done yet

- A release script that runs all four steps in one go.
- `hdiutil create` prints a deprecation warning (it suggests `diskutil image create`); it still
  works on this macOS.
