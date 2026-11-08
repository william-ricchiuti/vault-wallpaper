# Release checklist

From a clean `main` to a signed, notarized DMG on GitHub Releases and a Homebrew cask.
Steps marked **[William]** use his Apple Developer account, his signing certificate or his
GitHub account, or publish something. Everything else anyone can run.

Set the version once per release (the example is the first beta):

```bash
export V=1.0.0-beta.1
```

## 0. One-time setup

1. **[William]** A *Developer ID Application* certificate in the login keychain (Xcode >
   Settings > Accounts > Manage Certificates > + > Developer ID Application). Check:

   ```bash
   security find-identity -v -p codesigning
   ```

   Use the `Developer ID Application: … (TEAMID)` line, not `Apple Development: …`.

2. **[William]** An app-specific password (account.apple.com > Sign-In and Security >
   App-Specific Passwords), stored for `notarytool`:

   ```bash
   xcrun notarytool store-credentials vault-wallpaper-notary --apple-id "<apple id>" --team-id <TEAMID>
   ```

3. **[William]** For the cask: the download URL must work without a GitHub login. Either make
   `vault-wallpaper` public, or publish releases from a separate public repo and change `url`
   in the cask. Homebrew can't read release assets of a private repo.

4. **[William]** Sparkle update key (once; details in `packaging/README.md`, "Updates"):

   ```bash
   packaging/sparkle-keys.sh                       # private key -> login Keychain, prints the public key
   packaging/sparkle-keys.sh -x ~/sparkle-key.txt  # backup: store in a password manager, then delete the file
   ```

   Put the printed public key in `packaging/sparkle.env` (`SPARKLE_PUBLIC_ED_KEY=...`) and
   commit it. Until then builds have the updater switched off.

5. **[William]** Create the tap repo `william-ricchiuti/homebrew-tap` (public, empty, with a
   `Casks/` folder). `brew tap william-ricchiuti/tap` finds it by that name.

## 1. Prepare the release (anyone)

1. Update the version in `crates/app/Cargo.toml` (`version = "$V"`), then `cargo check` so
   `Cargo.lock` follows.
2. Move the `Unreleased` notes in `CHANGELOG.md` under `## [$V] - <date>`.
3. Check:

   ```bash
   cargo test --workspace
   cargo clippy --workspace --all-targets
   ```

4. Commit (`chore(release): $V`) and merge to `main`.

## 2. Build (anyone)

```bash
packaging/build-app.sh
lipo -info "dist/Vault Wallpaper.app/Contents/MacOS/vault-wallpaper"   # x86_64 arm64
plutil -p "dist/Vault Wallpaper.app/Contents/Info.plist" | grep -E 'Version|LSUIElement|Minimum'
```

`CFBundleShortVersionString` is the numeric part (`1.0.0`), `CFBundleVersion` the commit
count (set `VW_BUILD_NUMBER` to override), and `VWVersion` the full version.

## 3. Sign the app **[William]**

```bash
export VW_SIGN_IDENTITY="Developer ID Application: William Ricchiuti (TEAMID)"
packaging/sign.sh
codesign --verify --deep --strict --verbose=2 "dist/Vault Wallpaper.app"
```

## 4. Make and sign the DMG **[William]**

With `VW_SIGN_IDENTITY` still set, `make-dmg.sh` signs the DMG too:

```bash
packaging/make-dmg.sh
# -> dist/Vault-Wallpaper-$V.dmg
```

## 5. Notarize and staple **[William]**

```bash
VW_NOTARY_PROFILE=vault-wallpaper-notary packaging/notarize.sh
```

The script runs `xcrun notarytool submit --wait`, then `xcrun stapler staple` and
`xcrun stapler validate` on the DMG. If Apple says "Invalid", read why:

```bash
xcrun notarytool log <submission-id> --keychain-profile vault-wallpaper-notary
```

Then check Gatekeeper accepts it, as a download would be checked:

```bash
spctl --assess --type open --context context:primary-signature --verbose=2 "dist/Vault-Wallpaper-$V.dmg"
hdiutil attach -nobrowse -readonly "dist/Vault-Wallpaper-$V.dmg"
spctl --assess --type execute --verbose=2 "/Volumes/Vault Wallpaper/Vault Wallpaper.app"
hdiutil detach "/Volumes/Vault Wallpaper"
```

Both should say `accepted` with `source=Notarized Developer ID`.

## 5b. Appcast **[William]** (uses the Sparkle private key)

Only after notarizing and stapling: the appcast signs the DMG's final bytes.

```bash
mkdir -p dist/appcast
# keep the feed's history: start from the current appcast, if there is one
curl -fsSL -o dist/appcast/appcast.xml \
  https://github.com/william-ricchiuti/vault-wallpaper/releases/latest/download/appcast.xml || true
packaging/make-appcast.sh "dist/Vault-Wallpaper-$V.dmg" "$V"
```

Check `dist/appcast/appcast.xml`: the new `<item>` has `sparkle:version` (the build number),
`sparkle:edSignature`, and an enclosure URL ending `/releases/download/v$V/Vault-Wallpaper-$V.dmg`.

## 6. Smoke test the notarized build (anyone, on William's Mac or a test account)

Install from the DMG to `/Applications`, then:

- It appears in the menu bar, not the Dock; with no config, it asks for a vault.
- Start at Login: tick it, then check System Settings > General > Login Items lists
  Vault Wallpaper. Untick it again.
- Settings… opens; Export PNG writes `~/Pictures/Vault Wallpaper/vault-wallpaper-*.png`.
- Check for Updates… is in the menu (it is missing if the public key is still the placeholder).
  After the release is published, it says the app is up to date.

## 7. GitHub Release **[William]** (publishes)

```bash
git tag -a "v$V" -m "Vault Wallpaper $V"
git push origin "v$V"
gh release create "v$V" "dist/Vault-Wallpaper-$V.dmg" dist/appcast/appcast.xml \
  --title "Vault Wallpaper $V" \
  --notes-file <(awk -v v="$V" '$0 ~ "^## \\[" v "\\]" {on=1; next} /^## \[/ {on=0} on' CHANGELOG.md) \
  --prerelease            # drop --prerelease for 1.0.0
gh release view "v$V"
```

Installed copies find the update through
`https://github.com/william-ricchiuti/vault-wallpaper/releases/latest/download/appcast.xml`,
which follows the newest release that is **not** a pre-release. A `--prerelease` release
reaches nobody through Sparkle; publish without `--prerelease` when a beta should update
existing installs. Check:

```bash
curl -fsSIL https://github.com/william-ricchiuti/vault-wallpaper/releases/latest/download/appcast.xml | grep -i '^location'
```

## 8. Homebrew cask **[William]** (publishes)

```bash
shasum -a 256 "dist/Vault-Wallpaper-$V.dmg"
```

Copy `packaging/homebrew/vault-wallpaper.rb` to `Casks/vault-wallpaper.rb` in the tap, set
`version` and `sha256`, then:

```bash
cd ~/dev/homebrew-tap
brew style Casks/vault-wallpaper.rb
git add Casks/vault-wallpaper.rb && git commit -m "vault-wallpaper $V" && git push
brew tap william-ricchiuti/tap
brew audit --cask --online william-ricchiuti/tap/vault-wallpaper
brew install --cask william-ricchiuti/tap/vault-wallpaper
```

Users then install with `brew install --cask william-ricchiuti/tap/vault-wallpaper`.

## Mac App Store

The App Store build is separate: build, sign and upload with
[`packaging/appstore/README.md`](../packaging/appstore/README.md) ("Each release").

## 9. After the release

- Point the web version's README at the release (Phase 8; that repo is separate).
- Update `docs/plan.md` phase status and the vault project note.
