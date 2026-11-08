# Mac App Store build

The App Store flavour of Vault Wallpaper: the same app, sandboxed, without the self-updater
(the store updates it), signed for the store and wrapped in an installer package. Free.
The direct build (DMG, Homebrew, Sparkle) is unchanged; see [`../README.md`](../README.md).

| File | What it is |
|---|---|
| `build.sh` | builds the universal app with `--no-default-features` (no `updater`), writes the App Store `Info.plist`, embeds the privacy manifest and provisioning profile, signs, runs `productbuild` |
| `entitlements.plist` | App Sandbox and the four entitlements the features need (below) |
| `PrivacyInfo.xcprivacy` | privacy manifest: no tracking, no data collected, two required-reason APIs |
| `sandbox-check.sh` | guided check of every feature inside the sandbox, on a local ad-hoc build |
| `listing.md` | App Store Connect text: name, subtitle, description, keywords, privacy answers, review notes |
| `screenshots/` | 2880×1800 screenshots rendered from a real vault |

## How the app differs in the sandbox

The code is the same in both builds. It checks at run time whether it is sandboxed
(`APP_SANDBOX_CONTAINER_ID`, which macOS sets) and only then does the extra work:

- **Vault access.** The sandbox lets the app read only what the user picked in an open panel,
  until it quits. Choose Vault… (and the first-run panel) saves a read-only, security-scoped
  bookmark as `vault.bookmark` next to the config; at launch it is resolved and
  `startAccessingSecurityScopedResource` reopens the vault for the scan, the FSEvents watcher
  and the Obsidian plugin's `state.json`. If the bookmark is missing or points elsewhere (a
  hand-edited or imported `vaultPath`, a moved vault), the panel opens on that folder and asks
  once. Code: `crates/app/src/bookmark.rs`, `open_vault_or_ask` and `choose_vault` in `main.rs`.
- **Paths.** Everything is under `$HOME`, which the sandbox points at the container, so config,
  layout cache, custom presets and the bookmark live in
  `~/Library/Containers/com.williamricchiuti.vaultwallpaper/Data/Library/Application Support/Vault Wallpaper/`.
  Export PNG writes `~/Pictures/Vault Wallpaper/` through the container's `Pictures` link
  (pictures entitlement) and resolves it so Finder shows the real folder.
- **Import Web Version Settings…** reads the file picked in the open panel. The sandbox lets the
  app see, but not read, the `custom-presets.json` next to it, so it asks for that file with a
  second panel. An imported vault path gets the "allow access" panel above.
- **Finder and files** open through `NSWorkspace` instead of spawning `open`.
- **Start at Login** (`SMAppService.mainApp`) works sandboxed (tested: register → enabled,
  unregister → not registered).
- The desktop-level, click-through, all-Spaces window needs no entitlement.

Moving from the direct build: the App Store build can't read the direct build's settings.
Import Web Version Settings… accepts the native `config.json` too (same format): press
Cmd+Shift+G in the panel and go to `~/Library/Application Support/Vault Wallpaper/`.

## Entitlements

| Key | Why |
|---|---|
| `com.apple.security.app-sandbox` | required for the App Store |
| `com.apple.security.files.user-selected.read-only` | read the picked vault and the picked web config; never writes into the vault |
| `com.apple.security.files.bookmarks.app-scope` | keep reading the vault after a relaunch |
| `com.apple.security.assets.pictures.read-write` | Export PNG to `~/Pictures/Vault Wallpaper`, as in the direct build, with no save panel |
| `com.apple.security.network.client` | the settings window is a WKWebView, and in the sandbox WebKit loads nothing without it, not even the page served in-process through `vw://` (tested 2026-11-08: blank without it, works with it). The app makes no connections |

## Encryption: `ITSAppUsesNonExemptEncryption` = NO

The app has no networking of its own and no cryptography: it reads Markdown files, draws them
and writes PNGs. The only encryption anywhere is the operating system's own (e.g. WebKit's TLS
stack, which the settings page never uses because it loads nothing remote). That is exempt, so
the key is `false` in `Info.plist` and App Store Connect won't ask the export-compliance
questions for each build. No export documentation is needed. If the app ever adds its own
networking with encryption (HTTPS through the OS still counts as exempt), revisit this.

## Local test (anyone)

```bash
VW_APPSTORE_ADHOC=1 packaging/appstore/build.sh
codesign -d --entitlements :- "dist/appstore/Vault Wallpaper.app"
packaging/appstore/sandbox-check.sh
```

The ad-hoc build runs on this Mac only and can't be uploaded. `sandbox-check.sh` asks you to
click through the vault picker, relaunch, an edit, Settings…, Export PNG, Import, Start at Login
and Open Config File, checks each, and lists any sandbox violations from the app. The open
panel and the menu-bar menu need real clicks, so this can't run unattended.

## What William does

Nothing below can be done by an agent: it uses your Apple Developer account, certificates and
App Store Connect, and the last steps publish.

### Once

1. **Certificates** (Xcode > Settings > Accounts > your team > Manage Certificates > +):
   - **Apple Distribution** (signs the app),
   - **Mac Installer Distribution** (signs the package; the keychain shows it as
     "3rd Party Mac Developer Installer: …").
   Check: `security find-identity -v` lists both. (The Developer ID certificate for the DMG
   is a different one and can't be used here.)
2. **App ID** (developer.apple.com > Certificates, IDs & Profiles > Identifiers > +): App IDs >
   App, platform macOS, explicit bundle ID `com.williamricchiuti.vaultwallpaper`, description
   "Vault Wallpaper". No capabilities need ticking (App Sandbox isn't one).
3. **Provisioning profile** (Profiles > +): Distribution > **Mac App Store Connect**, pick that
   App ID and your Apple Distribution certificate, name it "Vault Wallpaper Mac App Store",
   download the `.provisionprofile`.
4. **App record** (App Store Connect > Apps > + > New App): platform macOS, name
   "Vault Wallpaper" (if taken, see `listing.md`), primary language English (U.S.), bundle ID
   `com.williamricchiuti.vaultwallpaper`, SKU `vaultwallpaper-mac`, full access.
5. **Agreements**: Business > the Free Apps agreement is active by default; nothing to sign for
   a free app beyond the Apple Developer Program License Agreement.
6. For command-line uploads, an **App Store Connect API key** (Users and Access > Integrations >
   App Store Connect API > +, role Developer): note the Key ID and Issuer ID and put
   `AuthKey_<KEYID>.p8` in `~/.appstoreconnect/private_keys/`. Not needed with Transporter.

### Each release

1. Build and sign (the build number is the commit count, so it goes up by itself; set
   `VW_BUILD_NUMBER` to override):

   ```bash
   export VW_TEAM_ID=ABCDE12345
   export VW_APP_IDENTITY="Apple Distribution: William Ricchiuti (ABCDE12345)"
   export VW_INSTALLER_IDENTITY="3rd Party Mac Developer Installer: William Ricchiuti (ABCDE12345)"
   export VW_PROVISIONING_PROFILE="$HOME/Downloads/Vault_Wallpaper_Mac_App_Store.provisionprofile"
   packaging/appstore/build.sh
   ```

   Output: `dist/appstore/Vault-Wallpaper-<version>-<build>.pkg`. Run it from Terminal: the
   script stops if the package contains `._` files, which come from extended attributes on
   files written by some app-launched shells.
2. Optional check that it installs and runs signed: `sudo installer -store -pkg dist/appstore/Vault-Wallpaper-*.pkg -target /`
   (then delete the installed copy).
3. **Upload** with either
   - **Transporter** (free on the Mac App Store): sign in, drag in the `.pkg`, Deliver; or
   - the command line:

     ```bash
     xcrun altool --validate-app -f dist/appstore/Vault-Wallpaper-*.pkg -t macos --apiKey <KEYID> --apiIssuer <ISSUER>
     xcrun altool --upload-app   -f dist/appstore/Vault-Wallpaper-*.pkg -t macos --apiKey <KEYID> --apiIssuer <ISSUER>
     ```

   App Store builds are **not** notarized: `notarytool` is only for the DMG. Apple processes the
   build for 10–30 minutes and emails if it finds a problem.
4. Optional: TestFlight > macOS > add yourself as an internal tester and install the build from
   the TestFlight app, to try the real store-signed app before review.
5. **App page** (App Store Connect > the app > macOS App > 1.0 Prepare for Submission): paste
   the text from `listing.md`, upload `screenshots/*.png`, pick the build, set the copyright,
   App Review contact and notes (in `listing.md`). Then:
   - App Privacy > Get Started > "No, we do not collect data from this app"; publish it.
     Privacy policy URL is required even so (placeholder in `listing.md`).
   - Pricing and Availability > Free, all territories (or pick).
   - Age Rating > answer None to everything (4+).
   - Category: Productivity, secondary Utilities.
6. **Submit for Review** (Add for Review > Submit). Choose "Manually release this version" so
   you can publish it alongside a DMG release. Review usually takes a day or two; answer any
   questions in Resolution Center.

## Known risks for review

- **Guideline 2.4.5 (Mac apps):** the app must work without asking for more than it needs. Each
  entitlement is justified above and in the review notes.
- **Reviewers need a vault.** Any folder of Markdown files works; the review notes say so and
  point at a sample vault (link to fill in).
- **Minimum OS:** the binary's own minimum is older than the 13.0 in `Info.plist` (see
  `../README.md` on `MACOSX_DEPLOYMENT_TARGET`). That is allowed; if validation ever objects,
  that is the place to look.
- **Name:** "Vault Wallpaper" may be taken on the store; fallbacks are in `listing.md`. Don't
  put "Obsidian" in the name or keywords (someone else's trademark, guideline 5.2.1).
