# Local changes to wry 0.57.1

Source: crates.io `wry` 0.57.1 (Apache-2.0 OR MIT), upstream commit
`2446feadae623d6f2ac8d2136e3ccef3d7197e0e`. Copied from the published crate: `src/`,
`build.rs`, `Cargo.toml`, `LICENSE-APACHE`, `LICENSE-MIT`, `LICENSE.spdx`, `README.md`.
Left out: `examples/`, `CHANGELOG.md`, `MOBILE.md`, `SECURITY.md`, `Cargo.lock`,
`Cargo.toml.orig` and the repo dotfiles. In `Cargo.toml` the `[[example]]` and
`dev-dependencies` tables are removed, `"tao/x11"` is dropped from the `x11` feature
(`tao` was only a dev-dependency; the feature is Linux-only and this app doesn't use it), and
`cfg(macos_12_unavailable)` (set by `build.rs`) is added to the `unexpected_cfgs` check-cfg
list. As a path dependency wry's warnings are no longer capped, so `platform_webview_version`
in `src/wkwebview/mod.rs` also gets `#[allow(dead_code)]` (it is only used with the
`os-webview` feature, which the app turns off).
Hooked up by `[patch.crates-io] wry = { path = "vendor/wry" }` in the workspace
`Cargo.toml`.

## The patch

All in `src/wkwebview/mod.rs`; each spot has a `vault-wallpaper patch` comment.

1. `InnerWebView::new_ns_view`: no longer sets the private `allowsPictureInPictureMediaPlayback`
   KVC key on `WKPreferences`. Effect: no picture-in-picture button on `<video>`.
2. Same function: no longer sets the private `drawsBackground` KVC key on the
   `WKWebViewConfiguration` when `transparent` or `background_color` is set. Effect: those
   options no longer clear WebKit's white default background.
3. Same function: the `fullScreenEnabled` private-KVC fallback for macOS < 12.3 / iOS < 15.4
   is gone. The public `setElementFullscreenEnabled` is still used on newer systems; the app's
   minimum macOS is above 12.3, so this changes nothing.
4. `InnerWebView::set_background_color`: no longer sets `drawsBackground` on the `WKWebView`
   instance. The public `underPageBackgroundColor` and the NSView colour are still set.

The app uses none of these: `crates/app/src/settings/` builds the web view without
`with_transparent` or `with_background_color` (it avoids the white first frame by keeping the
web view hidden until the page loads) and plays no video.

## Why

They are private WebKit API. The KVC key names are plain strings in the binary, and the Mac
App Store upload scanner (ITMS-90338) can flag private API names.

## Left as published

- `developerExtrasEnabled` and the `_inspector` selector: only in the devtools code, which is
  compiled under `#[cfg(any(debug_assertions, feature = "devtools"))]`. Release builds have
  neither (the app doesn't enable the `devtools` feature).
- `proxyConfigurations` (only with `mac-proxy`, not enabled), `tabFocusesLinks` and
  `inactiveSchedulingPolicy`: public properties set through KVC.
- `allowsInlineMediaPlayback`: iOS only.

## Re-applying on a wry upgrade

1. `grep -n 'setValue_forKey\|msg_send!\[.*, _' <wry>/src/wkwebview/mod.rs` in the new
   version. If none of the four keys above is set any more, delete `vendor/wry` and the `wry`
   line in `[patch.crates-io]`.
2. Otherwise copy the new crate from `~/.cargo/registry/src/*/wry-<version>/` (the files
   listed above), strip the same `Cargo.toml` tables and the `tao/x11` feature entry, and
   delete the same `setValue_forKey` calls.
3. Build the App Store app (`packaging/appstore/build.sh --dry-run`) and check
   `strings -a "dist/appstore/Vault Wallpaper.app/Contents/MacOS/vault-wallpaper" | grep -E 'drawsBackground|allowsPictureInPicture|fullScreenEnabled'`
   finds nothing. Update the version and commit hash at the top of this file.
