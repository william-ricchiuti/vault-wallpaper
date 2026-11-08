# Sparkle update: local end-to-end test

How the direct-download updater was tested before any real key or release existed, and how
to repeat it. Run on 2026-11-08 (macOS 27, Apple Silicon, Sparkle 2.10.1).

## Setup

Nothing here touches William's Keychain, signing identity or GitHub.

1. **Throwaway EdDSA key, in a file.** `generate_keys` always stores its key in the login
   Keychain (its `-x`/`-f` options only export/import an existing one), so the test key comes
   from openssl. Sparkle's key format is base64 of the 32-byte Ed25519 seed (private) and of
   the 32-byte public key:

   ```bash
   openssl genpkey -algorithm ed25519 -out throwaway.pem
   openssl pkey -in throwaway.pem -outform DER | tail -c 32 | base64 > private-seed.b64
   openssl pkey -in throwaway.pem -pubout -outform DER | tail -c 32 | base64 > public.b64
   ```

2. **Two builds** with the test feed and key (`VW_VERSION` only changes the plist and DMG
   name; Sparkle compares `CFBundleVersion`):

   ```bash
   for n in 1 2; do
     VW_VERSION=1.0.0-test$n VW_BUILD_NUMBER=900$n \
     VW_SPARKLE_FEED_URL=http://127.0.0.1:8765/appcast.xml \
     VW_SPARKLE_PUBLIC_KEY="$(cat public.b64)" packaging/build-app.sh
     VW_SIGN_IDENTITY=- packaging/sign.sh
     packaging/make-dmg.sh
     ditto "dist/Vault Wallpaper.app" "test$n.app"
   done
   ```

3. **Feed for test2**, signed with the throwaway key, served on localhost:

   ```bash
   mkdir serve && cp dist/Vault-Wallpaper-1.0.0-test2.dmg serve/
   VW_SPARKLE_KEY_FILE=private-seed.b64 VW_APPCAST_URL_PREFIX=http://127.0.0.1:8765/ \
     packaging/make-appcast.sh serve/Vault-Wallpaper-1.0.0-test2.dmg --out serve/appcast.xml
   python3 -m http.server 8765 --bind 127.0.0.1 --directory serve
   ```

   Plain `http://` to `127.0.0.1` worked: App Transport Security doesn't block it, and Sparkle
   relies on the EdDSA signature, not the transport. Real feeds are HTTPS.

4. **Run test1** from a writable folder, with a scratch home holding a `config.json` that
   points at a tiny vault, and the check triggered at launch:

   ```bash
   open --env HOME=/tmp/vw-home --env VW_CHECK_FOR_UPDATES=1 "install/Vault Wallpaper.app"
   ```

   `VW_CHECK_FOR_UPDATES` runs the menu's Check for Updates… action once at launch. Scripts
   can't open a tray-icon status-item menu (System Events' AXPress does nothing there, and
   computer use can't grant an app outside /Applications), so the trigger stands in for the
   click; it calls the same `Action::CheckForUpdates` → `-[SPUStandardUpdaterController
   checkForUpdates:]`. Sparkle's own windows are ordinary windows, so their buttons were
   pressed with System Events (`click button "Install Update" of window "Software Update"`).

## What was needed

- **Ad-hoc signing without the hardened runtime.** The first attempt (ad hoc *with*
  `--options runtime`) could not load the framework. dyld said: *"code signature … not valid
  for use in process: mapping process and mapped file (non-platform) have different Team
  IDs"*. That is library validation, and ad hoc signatures have no Team ID. `sign.sh` now
  leaves out `--options runtime` (and `--timestamp`) when `VW_SIGN_IDENTITY=-`. Developer ID
  builds sign the app and the framework with the same Team ID, so the release build keeps
  the hardened runtime.
- The DMG must hold the app as `Vault Wallpaper.app`. `make-dmg.sh` uses the folder name of
  the app it is given, so stage a copy under that name.

## Results

Third run, driven entirely by script (`run3.log`):

```
14:46:19 launch test1 (1.0.0-test1, build 9001)
14:46:20 GET /appcast.xml 200
14:46:21 window "Software Update": "Vault Wallpaper 1.0.0 (9002) is now available—you have 1.0.0 (9001)."
14:46:21 click "Install Update"; GET /Vault-Wallpaper-1.0.0-test2.dmg 200
14:46:22 "Ready to Install" (download verified against SUPublicEDKey, extracted)
14:46:22 click "Install and Relaunch"
14:46:24 relaunched: new pid; bundle is now 1.0.0-test2, build 9002
         codesign --verify --deep --strict: OK
```

**Negative test:** the same DMG with an appcast signature from a *different* key. After
Install Update, Sparkle stopped with "Update Error! The update is improperly signed and could
not be validated", and the app stayed at build 9001.

The first two runs also ended at test2. In both, a step went ahead without the script
pressing it (in run 2 the "Install and Relaunch" click hit a Sparkle helper's pid by mistake).
Sparkle's windows appear on the real screen, so most likely someone at the Mac clicked them.
Run 3 used exact process matching.

**Inert cases:**

- A bare `cargo build --features updater` binary run with `VW_CHECK_FOR_UPDATES=1` started no
  updater and printed nothing about updates (no `Info.plist`, so no key; and no framework).
- A release build without the feature contains no Sparkle symbols
  (`strings … | grep -c 'SPUStandardUpdaterController|Sparkle'` → 0; with the feature: 4).

Screenshots (update found, ready to install, bad signature rejected) are in the session
notes folder of the 2026-11-08 sessions, not in the repo.

## Cleanup after a run

Sparkle keeps its download and installer state in
`~/Library/Caches/com.williamricchiuti.vaultwallpaper/org.sparkle-project.Sparkle/`. The
test also uses the real preferences domain `com.williamricchiuti.vaultwallpaper` (cfprefsd
ignores `HOME`). Remove any `SU*` keys the test leaves there with
`defaults delete com.williamricchiuti.vaultwallpaper <key>`.
