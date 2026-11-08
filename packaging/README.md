# Packaging

Scripts that turn the `vault-wallpaper` crate into a signed, notarized DMG. Run them from
anywhere; they find the repository root themselves. Output goes to `dist/` (git-ignored).

| Step | Script | Needs | Output |
|---|---|---|---|
| 1 | `build-app.sh` | rustup, Xcode command line tools | `dist/Vault Wallpaper.app` (universal, unsigned) |
| 2 | `sign.sh` | `VW_SIGN_IDENTITY`, a Developer ID Application certificate | the app, signed with the hardened runtime |
| 3 | `make-dmg.sh` | nothing extra (signs the DMG too if `VW_SIGN_IDENTITY` is set) | `dist/Vault-Wallpaper-<version>.dmg` |
| 4 | `notarize.sh` | `VW_NOTARY_PROFILE` (one-time setup below), network | the DMG, notarized and stapled |
| 5 | `make-appcast.sh` | the Sparkle private key (Keychain, or `VW_SPARKLE_KEY_FILE`) | `dist/appcast/appcast.xml` |

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
into the binary, so the bundle has no other resources, except the updater below.

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

- `SUFeedURL`: `https://github.com/william-ricchiuti/vault-wallpaper/releases/latest/download/appcast.xml`.
  GitHub redirects `releases/latest/download/<file>` to that file in the newest release that is
  not a draft or a **pre-release**, so each release uploads its own `appcast.xml`, and the
  repo (or whatever repo holds the releases) must be public. While releases are betas marked
  pre-release, `latest` doesn't see them: publish the beta that should reach users as a
  normal release, or point `SPARKLE_FEED_URL` at a fixed place (a `gh-pages` file).
- `SUPublicEDKey`: from `sparkle.env`. While it is `REPLACE_WITH_SPARKLE_PUBLIC_ED_KEY`, the
  script warns and the app keeps its updater off.
- `SUEnableAutomaticChecks` true (no "check automatically?" prompt on second launch) and
  `SUScheduledCheckInterval` 86400, Sparkle's default of once a day.

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
the new item goes on top, so download the previous release's appcast.xml there first to keep
history. The path is the last line on stdout. The private key is taken from
`VW_SPARKLE_PRIVATE_KEY` (the base64 key, for CI), else `VW_SPARKLE_KEY_FILE`, else the
login Keychain. `--unsigned` writes the item without a signature, for dry runs; Sparkle
rejects such a feed. Upload `appcast.xml` next to the DMG on the GitHub Release.

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

## Release

The full release sequence (sign, notarize, staple, DMG, GitHub Release, Homebrew) is in
[`docs/release-checklist.md`](../docs/release-checklist.md). The cask draft is
[`homebrew/vault-wallpaper.rb`](homebrew/vault-wallpaper.rb).

## Not done yet

- Sparkle's `XPCServices` (only needed by sandboxed apps) are kept in the bundle; removing them
  would save about 0.4 MB of the framework's 2.9 MB.
- A release script that runs all four steps in one go.
- `hdiutil create` prints a deprecation warning (it suggests `diskutil image create`); it still
  works on this macOS.
