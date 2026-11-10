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
| `screenshots/` | 2880×1800 JPEG screenshots rendered from a real vault |
| `xcode-build-phase.sh` | the Run Script phase of the Xcode project: cargo build, icon, version |
| [`../../xcode/`](../../xcode) | `VaultWallpaper.xcodeproj`, the Xcode way to build, archive and upload (below) |

There are two ways to make the App Store build. They produce the same app.

- **Xcode (recommended):** open `xcode/VaultWallpaper.xcodeproj`, Archive, Distribute.
  Xcode makes the certificates and profiles itself and uploads. Steps in
  [Uploading with Xcode](#uploading-with-xcode-click-by-click).
- **Command line (fallback):** `build.sh`, then Transporter. You make the certificates and the
  profile on the developer website. Steps in [What William does](#what-william-does).

## The Xcode project

`xcode/VaultWallpaper.xcodeproj` is a macOS App target with no source files. Its one Run
Script phase, `xcode-build-phase.sh`, runs
`cargo build --release --no-default-features -p vault-wallpaper` for each architecture Xcode
builds (Apple silicon and Intel for Archive), joins them with `lipo` and puts the result in
the bundle as `Contents/MacOS/vault-wallpaper`. It also makes `AppIcon.icns` and writes the
version from `crates/app/Cargo.toml` and the build number (the commit count, from
`packaging/build-number.sh`) into the built `Info.plist`. Unlike `build.sh` it builds with
line tables (in its own cargo folder, `target/xcode-appstore`) and makes a dSYM with
`dsymutil`, which Archive keeps and Distribute uploads, so crash reports in App Store Connect
show function names and lines; the shipped binary is then stripped of the debug map. Without
it the upload ends with "Upload Symbols Failed" (harmless, but now gone). Everything else is
ordinary Xcode:

| | |
|---|---|
| Info.plist | `xcode/Info.plist`, the same keys as `build.sh` writes (keep the two in step) |
| Resources | `PrivacyInfo.xcprivacy`, `THIRD-PARTY-NOTICES.txt` |
| Signing | automatic, team `3AP82DL922`, hardened runtime, `entitlements.plist`, App Sandbox on |
| Bundle ID | `com.williamricchiuti.vaultwallpaper`, category Productivity, macOS 13.0+ |

The build-number check applies here too: archive from `main` after `git pull`, or Xcode stops
with "HEAD is not on origin/main". The project is generated from `xcode/project.yml` by
[XcodeGen](https://github.com/yonaskolb/XcodeGen); the `.xcodeproj` is committed, so you only
need XcodeGen to change the project (`cd xcode && xcodegen generate`). Command-line check
(2026-11-08 this archived fine unsigned, and also signed with automatic signing and the Apple
Development certificate; only the export needs the App Store profile Xcode makes in step 5):

```bash
xcodebuild -project xcode/VaultWallpaper.xcodeproj -scheme VaultWallpaper archive -archivePath /tmp/VW.xcarchive CODE_SIGNING_ALLOWED=NO
```

## Uploading with Xcode, click by click

You do this part: it signs in to your Apple account and the last step publishes. Steps 1–3
are once; 4–6 are every release.

### 1. Open the project

1. In Terminal: `cd ~/dev/vault-wallpaper && git checkout main && git pull`.
2. `open xcode/VaultWallpaper.xcodeproj` (or double-click it in Finder).
3. If Xcode asks whether to trust or open the project, say yes: its build script runs
   `cargo` from this repository.

### 2. Signing: let Xcode make the certificate and profile

1. **Xcode > Settings… > Accounts.** Your Apple ID should be listed, with the team
   "William Ricchiuti". If not: **+** > Apple ID > sign in.
2. Close Settings. In the left sidebar click the blue **VaultWallpaper** project icon, then
   under TARGETS click **VaultWallpaper**, then the **Signing & Capabilities** tab.
3. Check: **Automatically manage signing** is ticked, **Team** is "William Ricchiuti",
   **Bundle Identifier** is `com.williamricchiuti.vaultwallpaper`. App Sandbox and Hardened
   Runtime are listed below with the entitlements from `entitlements.plist`.
4. Xcode now registers the bundle ID and makes a development certificate and profile for
   you; wait for the spinner next to "Provisioning Profile" to finish. It should say
   "Xcode Managed Profile" with no red error.

The Apple Distribution and Mac Installer Distribution certificates are made later, by Xcode,
when you first click Distribute (step 5). You don't need to make them.

**If Xcode refuses** (as it did for the Developer ID certificate: "only the account holder
can…", or "No signing certificate found"), make the certificates on the website instead, the
way you did for Developer ID:

1. Keychain request file: open `/System/Library/CoreServices/Certificate Assistant.app` >
   Certificate Assistant > Request a Certificate From a Certificate Authority…, your email,
   **Saved to disk**.
2. developer.apple.com > Account > Certificates, IDs & Profiles > Certificates > **+**:
   make an **Apple Distribution** certificate, then do it again for **Mac Installer
   Distribution**, uploading the request file each time. Download both `.cer` files.
3. Import each one (double-clicking gave error -25294 before):
   `security import ~/Downloads/<file>.cer -k ~/Library/Keychains/login.keychain-db`.
   Check with `security find-identity -v`.
4. Identifiers > **+** > App IDs > App > macOS, explicit `com.williamricchiuti.vaultwallpaper`
   (skip if Xcode already registered it). Profiles > **+** > **Mac App Store Connect** > that
   App ID > your Apple Distribution certificate > name "Vault Wallpaper Mac App Store" >
   Download, then double-click the profile.
5. Back in Xcode, keep automatic signing; in step 5 below choose **Manually manage signing**
   and pick that certificate and profile. (Or skip Xcode: [What William does](#what-william-does)
   uses the same files with `build.sh` and Transporter.)

### 3. Make the app record in App Store Connect

1. appstoreconnect.apple.com > **Apps** > **+** (top left) > **New App**.
2. Platform **macOS**; Name **Vault Wallpaper** (if taken, see `listing.md`); Primary language
   **English (U.S.)**; Bundle ID **com.williamricchiuti.vaultwallpaper** (it appears in the
   list once Xcode or you registered it); SKU **vaultwallpaper-mac**; User Access **Full
   Access**. Click **Create**.

### 4. Archive

1. At the top of the Xcode window, the scheme reads **VaultWallpaper** and the destination
   **My Mac** (or Any Mac).
2. **Product > Archive.** The first archive takes several minutes (cargo builds the app for
   Apple silicon and Intel; watch the progress bar). Later ones are quicker.
3. The **Organizer** window opens with the new archive selected. Its version is the
   `Cargo.toml` version and its build number the commit count.

### 5. Validate, then upload

1. In the Organizer, with the archive selected, click **Validate App** > **App Store
   Connect** > Next. Leave the options as they are, **Automatically manage signing** > Next.
   The first time, Xcode makes your Apple Distribution and Mac Installer Distribution
   certificates here. Click **Validate**. Fix anything it reports before going on.
2. Click **Distribute App** > **App Store Connect** > **Distribute** (on older Xcode:
   App Store Connect > Upload > Next > Automatically manage signing > Upload).
3. Wait for the email "has completed processing" (10–30 minutes). The build then shows under
   the app's **TestFlight** tab.

**About `._` files.** The archive itself has none. But macOS 26+ tags every file a build
writes with `com.apple.provenance` (Xcode's builds too: tested 2026-11-08, also from a build
started outside Claude), and Xcode makes the upload package with `/usr/bin/productbuild`,
which turns each tag into a `._` entry. `productbuild` on the archived app gave 12 of them.
Nothing in Xcode can switch that off; `build.sh` avoids it by repacking its package. It is not
known whether App Store Connect minds: **Validate App** (step 1) checks the real package on
Apple's side, so if it passes, upload. To look yourself: **Distribute App** > **Custom** >
**App Store Connect** > **Export**, save it, then

```bash
pkgutil --payload-files ~/Desktop/VaultWallpaper*/*.pkg | grep '/\._'
```

If Validate, the upload or review complains about `._` files or an invalid bundle or
signature, use the command-line route (`build.sh` + Transporter) instead: same app, package
without them.

### 6. Fill in the listing and submit

1. App Store Connect > Vault Wallpaper > **macOS App** > **1.0 Prepare for Submission**.
2. Paste from `listing.md`: promotional text, description, keywords, support URL, marketing
   URL. Drag the five `screenshots/*.jpg` into the screenshots box, in their numbered order.
3. **Build** > **+** > pick the build you uploaded.
4. Copyright `2026 William Ricchiuti`. **App Review Information**: sign-in not required, your
   contact details, and the review notes from `listing.md`.
5. Left sidebar **App Information**: category Productivity, secondary Utilities; privacy
   policy URL `https://github.com/william-ricchiuti/vault-wallpaper/blob/main/PRIVACY.md`.
6. **App Privacy** > Get Started > "No, we do not collect data from this app" > **Publish**.
7. **Pricing and Availability** > Free. **Age Rating** > None for everything (4+).
8. Back on the 1.0 page: **Version Release** > **Manually release this version**. Then
   **Add for Review** > **Submit**. Review usually takes a day or two; answers go in the
   App Review messages.

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
packaging/appstore/build.sh --dry-run
codesign -d --entitlements :- "dist/appstore/Vault Wallpaper.app"
packaging/appstore/sandbox-check.sh
```

The ad-hoc build runs on this Mac only and can't be uploaded. `sandbox-check.sh` asks you to
click through the vault picker, relaunch, an edit, Settings…, Export PNG, Import, Start at Login
and Open Config File, checks each, and lists any sandbox violations from the app. The open
panel and the menu-bar menu need real clicks, so this can't run unattended.

## What William does

This is the command-line route, the fallback to [Uploading with Xcode](#uploading-with-xcode-click-by-click).
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

   Output: `dist/appstore/Vault-Wallpaper-<version>-<build>.pkg`. Any shell works. macOS tags
   every file a build writes with `com.apple.provenance`, and `productbuild` would turn each
   tag into a `._` file in the package, so the script repacks the payload without them and
   stops if any are left.
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
   the text from `listing.md`, upload `screenshots/*.jpg`, pick the build, set the copyright,
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
- **Private API (ITMS-90338).** Checked 2026-11-08 on the `--dry-run` binary:
  - *Imported symbols* (`nm -u`: 374 arm64, 377 x86_64; the x86_64 slice adds `$INODE64` /
    `$NOCANCEL` variants, `___bzero`, `_fma`/`_fmaf` and `_objc_msgSend_stret`; re-measured
    2026-11-08): every one is in the public macOS SDK headers, apart
    from compiler and runtime ABI symbols every app has (`objc_retain`/`release`/`autorelease*`,
    weak-reference and exception symbols, `dyld_stub_binder`, `realpath$DARWIN_EXTSN`). The
    private SkyLight imports `_CGSMainConnectionID` and `_CGSSetWindowBackgroundBlurRadius`
    came from winit's unused blur support and are patched out (`vendor/winit/PATCHES.md`).
    `_CGShieldingWindowLevel` is public CoreGraphics. To re-check:
    `nm -u "dist/appstore/Vault Wallpaper.app/Contents/MacOS/vault-wallpaper" | grep -E "CGS|_SL"`
    must print only `_CGShieldingWindowLevel`.
  - *Objective-C selectors:* the binary has no `__objc_selrefs`/`__objc_methname` sections
    (objc2 registers selectors at runtime), so there are no selector references for the
    scanner to read. Private selector names used to be in the binary as plain strings; they
    are now patched out of the source too: winit's undocumented `NSCursor` class methods
    (`_helpCursor`, `_zoomInCursor`, `_windowResize…Cursor`, `busyButClickableCursor`; those
    icons show the arrow) and its `_wantsKeyDownForEvent:` override
    (`vendor/winit/PATCHES.md`). To re-check:
    `strings -a <binary> | grep -E '^_[a-z][a-zA-Z]{3,}[A-Z][a-zA-Z:]*$'` should print only
    byte noise (2026-11-08: one hit, `_indirecI`). The looser `'^_[a-zA-Z]+[A-Z:]'` also
    matches random bytes in code; read its output rather than counting it.
  - *WebKit settings in wry 0.57.1* (the settings window): wry is vendored
    (`vendor/wry/PATCHES.md`) without its private key-value-coding keys
    `allowsPictureInPictureMediaPlayback`, `drawsBackground` (config and web view) and the
    pre-12.3 `fullScreenEnabled` fallback. The app never used `with_transparent` or
    `with_background_color`; the window background and a hidden-until-loaded web view
    prevent the white flash. `developerExtrasEnabled` and `_inspector` exist only in debug
    builds. To re-check:
    `strings -a <binary> | grep -E 'drawsBackground|allowsPictureInPicture|fullScreenEnabled'`
    must print nothing. Still set through KVC, all public `WKPreferences` properties:
    `tabFocusesLinks` and `inactiveSchedulingPolicy` (the latter only if the app asks for a
    background-throttling policy, which it doesn't).
- **Name:** "Vault Wallpaper" may be taken on the store; fallbacks are in `listing.md`. Don't
  put "Obsidian" in the name or keywords (someone else's trademark, guideline 5.2.1).
