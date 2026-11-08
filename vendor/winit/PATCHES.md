# Local changes to winit 0.30.13

Source: crates.io `winit` 0.30.13 (Apache-2.0), upstream commit
`e9809ef54b18499bb4f2cac945719ecc2a61061b`. Copied from the published crate: `src/`,
`build.rs`, `Cargo.toml`, `LICENSE`, `README.md`, `FEATURES.md`. Left out: `examples/`,
`tests/`, `docs/` (only images that docs link to by URL), `Cargo.lock`, `Cargo.toml.orig`.
In `Cargo.toml` the `[[example]]`, `[[test]]` and `dev-dependencies` tables are removed.
Hooked up by `[patch.crates-io] winit = { path = "vendor/winit" }` in the workspace
`Cargo.toml`.

## The patch

1. `src/platform_impl/macos/ffi.rs`: the `extern "C"` declarations of
   `CGSMainConnectionID` and `CGSSetWindowBackgroundBlurRadius` are deleted (a comment
   marks the spot), plus the `NSInteger` and `AnyObject` imports only they used.
2. `src/platform_impl/macos/window_delegate.rs`, `WindowDelegate::set_blur`: the body is
   gone; the function is a no-op that ignores its argument.

Effect: `Window::set_blur` and `WindowAttributes::with_blur` do nothing on macOS. This app
never calls them.

## Why

Both functions are private SkyLight (CoreGraphics Services) API. The Mac App Store upload
scanner rejects binaries that import private symbols, and these were imported even though
nothing calls `set_blur`: Rust keeps the reference because `set_blur` is a public method,
and LTO did not drop it. Checked with
`nm -u "dist/appstore/Vault Wallpaper.app/Contents/MacOS/vault-wallpaper" | grep -E "CGS|_SL"`:
before the patch it listed `_CGSMainConnectionID`, `_CGSSetWindowBackgroundBlurRadius` and
`_CGShieldingWindowLevel`; after, only `_CGShieldingWindowLevel`, which is public
CoreGraphics API (2026-11-08).

## Left as published

winit also uses private Objective-C *selectors*: the undocumented `NSCursor` class methods
in `cursor.rs` (`_helpCursor`, `_zoomInCursor`, `_windowResize…Cursor`, used only when a
window sets those cursor icons, which this app never does) and the `_wantsKeyDownForEvent:`
override in `view.rs`. They aren't patched: objc2 registers selectors at runtime, so the
binary has no `__objc_selrefs`/`__objc_methname` sections and these names are plain strings,
not selector references. If App Review ever names them, empty those cursor arms (fall back
to the arrow cursor) and drop the override here.

## Re-applying on a winit upgrade

1. Check whether the new winit still has the calls:
   `grep -rn "CGS\|SLS" <winit>/src/platform_impl/macos/`. If none are left except
   `CGShieldingWindowLevel`, delete `vendor/winit` and the `winit` line in
   `[patch.crates-io]`, and you are done.
2. Otherwise copy the new crate from `~/.cargo/registry/src/*/winit-<version>/` (the files
   listed above), strip the same `Cargo.toml` tables, and redo the two edits: delete the
   private `extern` declarations in `ffi.rs` and empty `set_blur` in `window_delegate.rs`.
   Remove any import the compiler then reports unused.
3. Build the App Store app (`packaging/appstore/build.sh --dry-run`) and run the `nm` check
   above. Update the version and commit hash at the top of this file.
