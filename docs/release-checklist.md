# Release checklist

How to ship Vault Wallpaper: a notarized DMG on GitHub Releases, Sparkle auto-updates fed by
an appcast on the same release, a Homebrew cask in William's tap, and the Mac App Store build.

- **Part A** is one-time setup, done once before the first release.
- **Part B** is what every release takes: either one command on William's Mac, or one tag push.

Steps marked **[William]** use his Apple Developer account, certificates or GitHub account,
or publish something. Everything else anyone can run.

The app is free, and nothing here costs money except the Apple Developer Program membership
William already has. GitHub Actions is free while the repo is public. Costs while it is still
private are under [Cost of CI](#cost-of-ci).

## Part A. One-time setup

### A1. Developer ID certificate **[William]**

1. Xcode > Settings > Accounts > (your team) > Manage Certificates > **+** >
   **Developer ID Application**. Check it's in the login keychain:

   ```bash
   security find-identity -v -p codesigning
   ```

   Use the `Developer ID Application: William Ricchiuti (TEAMID)` line, not
   `Apple Development: …`. `TEAMID` is the 10-character team ID.

2. For CI, export it as a `.p12`. In Keychain Access > login > My Certificates, right-click
   *Developer ID Application: …* > Export, choose Personal Information Exchange (.p12) and
   set a strong password. Keep the file out of the repo, for example `~/Secure/DeveloperID.p12`.

### A2. Notarization credentials **[William]**

Two kinds; you may want both.

- **Local runs: keychain profile.** Make an app-specific password at account.apple.com >
  Sign-In and Security > App-Specific Passwords, then:

  ```bash
  xcrun notarytool store-credentials vault-wallpaper-notary --apple-id "<apple id>" --team-id <TEAMID>
  ```

- **CI: App Store Connect API key.** At appstoreconnect.apple.com > Users and Access >
  Integrations > App Store Connect API > Team Keys > **+**, name it "vault-wallpaper notary",
  give it the **Developer** role and generate it. Download `AuthKey_<KEYID>.p8`; Apple lets you
  download it only once. Note the **Key ID** (next to the key) and the **Issuer ID** (above
  the list). The same key works locally too:

  ```bash
  xcrun notarytool store-credentials vault-wallpaper-notary \
    --key ~/Secure/AuthKey_<KEYID>.p8 --key-id <KEYID> --issuer <ISSUER-ID>
  ```

### A3. Sparkle signing keys **[William]**

Sparkle checks every update against an EdDSA public key built into the app. Make the key pair
once and never lose the private half: without it, installed copies can't be updated.

```bash
packaging/sparkle-keys.sh        # added by the Sparkle work; wraps Sparkle's generate_keys
```

It keeps the private key in the login keychain, which local `release.sh` runs and
`make-appcast.sh` use, and prints the public key. Put that public key in
`packaging/sparkle.env` (`SPARKLE_PUBLIC_ED_KEY=...`) and commit it: `build-app.sh` writes it
into the app's `SUPublicEDKey`. Until then, builds leave the updater switched off and the menu
has no Check for Updates….

For CI and as a backup, export the private key to a file
(`packaging/sparkle-keys.sh -x ~/sparkle-key.txt`, or Sparkle's `generate_keys -x <file>`). Keep
the file in your password manager, not in the repo, then delete it.

### A4. Make the repository public **[William]**

Homebrew and Sparkle download from release asset URLs, which need a GitHub login while the
repo is private. Then `brew install` and auto-updates fail for everyone else. Public repos also
get free macOS CI minutes. Before switching:

```bash
git log --all -p | grep -inE 'BEGIN (EC |RSA |OPENSSH )?PRIVATE KEY|api[_-]?key|password' | head
```

That command should find nothing real. Then go to GitHub > the repo > Settings > General >
Danger Zone > Change visibility > Public.

### A5. The Homebrew tap **[William]**

1. Create the public repo `william-ricchiuti/homebrew-tap`; `brew tap william-ricchiuti/tap`
   finds it by that name. Add a `Casks/` folder and a README.
2. For CI's cask bump PR, make a fine-grained personal access token at GitHub > Settings >
   Developer settings > Fine-grained tokens. Give it access to `william-ricchiuti/homebrew-tap`
   only, with the **Contents: Read and write** and **Pull requests: Read and write**
   permissions, and an expiry of up to a year. Put a renewal reminder in your calendar.

### A6. GitHub Actions secrets **[William]**

Set these once with `gh secret set` (or the repo's Settings > Secrets and variables > Actions).
Each `gh secret set` call prompts for the value or reads it from stdin, so nothing ends up in
your shell history.

| Secret | What it is | How to set it |
|---|---|---|
| `MACOS_CERTIFICATE_P12` | Developer ID Application certificate and key, `.p12`, base64 | `base64 -i ~/Secure/DeveloperID.p12 \| gh secret set MACOS_CERTIFICATE_P12` |
| `MACOS_CERTIFICATE_PASSWORD` | the password you set when exporting the `.p12` | `gh secret set MACOS_CERTIFICATE_PASSWORD` |
| `APPSTORE_CONNECT_API_KEY_P8` | the `.p8` file's text (A2) | `gh secret set APPSTORE_CONNECT_API_KEY_P8 < ~/Secure/AuthKey_<KEYID>.p8` |
| `APPSTORE_CONNECT_API_KEY_ID` | the Key ID (A2) | `gh secret set APPSTORE_CONNECT_API_KEY_ID` |
| `APPSTORE_CONNECT_ISSUER_ID` | the Issuer ID (A2) | `gh secret set APPSTORE_CONNECT_ISSUER_ID` |
| `SPARKLE_PRIVATE_KEY` | the exported Sparkle EdDSA private key (A3) | `gh secret set SPARKLE_PRIVATE_KEY < sparkle-private-key.txt`, then delete the file |
| `TAP_GITHUB_TOKEN` | the fine-grained token for the tap (A5). Optional: without it CI skips the cask PR with a warning | `gh secret set TAP_GITHUB_TOKEN` |

`GITHUB_TOKEN`, which creates the release, is automatic. The workflow asks for
`contents: write`. The signing keychain is temporary, with a random password, and is deleted
at the end of the run.

Check the names with `gh secret list`.

### A7. Mac App Store **[William]**

The App Store build has its own certificates (Apple Distribution, Mac Installer
Distribution), a provisioning profile and an App Store Connect app record. These are covered
in `packaging/appstore/README.md`. Once `VW_TEAM_ID`, `VW_APP_IDENTITY`,
`VW_INSTALLER_IDENTITY` and `VW_PROVISIONING_PROFILE` are set, `release.sh` builds the signed
`.pkg` through `packaging/appstore/build.sh`. Until then it says the App Store step was
skipped. CI doesn't build it.

### A8. Try it without publishing (anyone)

```bash
packaging/release.sh 1.0.0-beta.2 --dry-run
```

This builds the universal app, an unsigned DMG and an unsigned test appcast, and prints the
commands a real run would end with. It leaves `Cargo.toml` as it was.

## Part B. Every release

### B1. Prepare (anyone)

1. In `CHANGELOG.md`, move the `Unreleased` notes under `## [<version>] - <date>`. The
   release notes are taken from that section.
2. `cargo test --workspace` and `cargo clippy --workspace --all-targets`. CI runs both on
   every pull request.
3. Commit the changelog and merge to `main`. A real `release.sh` run refuses a dirty tree.

Then pick **one** of B2 or B3.

### B2. Path 1: one command on the Mac **[William]**

```bash
export VW_SIGN_IDENTITY="Developer ID Application: William Ricchiuti (TEAMID)"
export VW_NOTARY_PROFILE=vault-wallpaper-notary
packaging/release.sh 1.0.0-beta.2
```

Run it from Terminal.app, not from a shell that an app such as an editor or Claude
started. Files those shells write get `com.apple.provenance` attributes, which end up as
`._` files in the App Store package, and `build.sh` refuses such a package.

It bumps the version, builds the universal app with the Sparkle updater, signs, makes and
signs the DMG, notarizes and staples, makes the signed appcast, and builds the App Store
package if that script exists. It uploads nothing: at the end it prints these, filled in:

```bash
git commit -am "chore(release): 1.0.0-beta.2"
git push origin HEAD
gh release create v1.0.0-beta.2 dist/Vault-Wallpaper-1.0.0-beta.2.dmg dist/appcast.xml \
  --repo william-ricchiuti/vault-wallpaper --target <the commit's sha> \
  --title "Vault Wallpaper 1.0.0-beta.2" \
  --notes-file dist/release-notes-1.0.0-beta.2.md --prerelease
git fetch --tags
packaging/homebrew/bump-cask.sh 1.0.0-beta.2 dist/Vault-Wallpaper-1.0.0-beta.2.dmg ~/dev/homebrew-tap
```

`gh release create --target` creates the tag on GitHub once the release and its files exist.
If that tag starts the release workflow, the workflow finds the DMG already there and stops
without building. Don't push the tag yourself before the release exists: CI would start a
second build. Then commit and push the cask in the tap:

```bash
cd ~/dev/homebrew-tap && brew style Casks/vault-wallpaper.rb && git commit -am "vault-wallpaper 1.0.0-beta.2" && git push
```

### B3. Path 2: one tag, and CI does the rest **[William]**

```bash
packaging/release.sh 1.0.0-beta.2 --bump-only
git commit -am "chore(release): 1.0.0-beta.2"
git tag -a v1.0.0-beta.2 -m "Vault Wallpaper 1.0.0-beta.2"
git push origin HEAD v1.0.0-beta.2
```

`.github/workflows/release.yml` checks that the tag matches `crates/app/Cargo.toml`, imports
the certificate, and runs `packaging/release.sh <version> --no-appstore`. That signs,
notarizes with the API key and signs the appcast with `SPARKLE_PRIVATE_KEY`. Then it creates
the GitHub Release (a pre-release if the version has a `-`: Sparkle users aren't offered
pre-releases, because the feed follows `releases/latest`) with the DMG and `appcast.xml`,
and opens a cask bump PR in the tap. Merge that PR. Watch the run with `gh run watch`.

Build the App Store package on the Mac (`packaging/appstore/build.sh`) and upload it
as described there.

### B4. Check the published release (anyone, on a Mac)

```bash
V=1.0.0-beta.2
curl -fLO "https://github.com/william-ricchiuti/vault-wallpaper/releases/download/v$V/Vault-Wallpaper-$V.dmg"
spctl --assess --type open --context context:primary-signature --verbose=2 "Vault-Wallpaper-$V.dmg"
hdiutil attach -nobrowse -readonly "Vault-Wallpaper-$V.dmg"
spctl --assess --type execute --verbose=2 "/Volumes/Vault Wallpaper/Vault Wallpaper.app"
hdiutil detach "/Volumes/Vault Wallpaper"
```

Both `spctl` checks should say `accepted` with `source=Notarized Developer ID`. Then:

- **Smoke test:** install from the DMG to `/Applications`. It appears in the menu bar and not
  the Dock, and with no config it asks for a vault. Settings… opens, and Export PNG writes
  `~/Pictures/Vault Wallpaper/`. Start at Login shows up in System Settings > General > Login
  Items.
- **Homebrew:** `brew update && brew install --cask william-ricchiuti/tap/vault-wallpaper`,
  and `brew audit --cask --online william-ricchiuti/tap/vault-wallpaper`.
- **Sparkle:** with the previous version installed, choose Check for Updates… It should offer
  this version, install it and relaunch.

### B5. After the release

- Update `docs/plan.md` and the vault project note.

## Cost of CI

- **Public repo:** macOS runners are free, with no minute limit.
- **Private repo, GitHub Free:** 2,000 minutes a month, and macOS minutes count 10x, so about
  200 real macOS minutes.
  - A release run is about 15-25 minutes (two release builds plus notarization), which counts
    as 150-250.
  - A PR CI run (`ci.yml`, about 8-15 minutes) counts as 80-150.
  - Past the free minutes, jobs stop unless you set a spending limit; the default is $0.
- **While the repo is private:** release from the Mac (B2), which uses no minutes, and keep
  PR CI for PRs that matter.

## If something fails

- **Notarization "Invalid":** run
  `xcrun notarytool log <submission-id> --keychain-profile vault-wallpaper-notary`, or, in CI,
  read the log printed by the step.
- **`E0463 can't find crate` during the build:** `MACOSX_DEPLOYMENT_TARGET` was set. Run
  `cargo clean`; `packaging/README.md` has the details.
- **CI "tag doesn't match":** the tag was pushed without the version bump commit. Delete the
  tag (`git push origin :refs/tags/v<version>`), bump, then tag again.
- **CI succeeded but there's no cask PR:** `TAP_GITHUB_TOKEN` is missing or expired. Run
  `bump-cask.sh` by hand.
