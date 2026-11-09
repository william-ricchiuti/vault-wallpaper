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

## Private selectors (second patch)

objc2 registers selectors at runtime, so the binary has no `__objc_selrefs` section and these
were never selector references, but their names were plain strings in the binary
(`strings -a`), which the upload scanner can flag.

3. `src/platform_impl/macos/cursor.rs`: the undocumented `NSCursor` class methods
   `_helpCursor`, `_zoomInCursor`, `_zoomOutCursor`, `_windowResizeNorthEastCursor`,
   `_windowResizeNorthWestCursor`, `_windowResizeSouthEastCursor`,
   `_windowResizeSouthWestCursor`, `_windowResizeNorthEastSouthWestCursor`,
   `_windowResizeNorthWestSouthEastCursor` and `busyButClickableCursor` are deleted, with the
   `def_undocumented_cursor!` macro and `try_cursor_from_selector` that called them. Their
   arms in `cursor_from_icon` are gone, so `Help`, `ZoomIn`, `ZoomOut`, `NeResize`,
   `NwResize`, `SeResize`, `SwResize`, `NeswResize`, `NwseResize`, `Wait` and `Progress` show
   the arrow cursor. This app never sets a cursor icon.
4. `src/platform_impl/macos/view.rs`: the `_wantsKeyDownForEvent:` override on `WinitView`
   is deleted. Effect: winit windows no longer get Ctrl-Tab / Ctrl-Esc key-down events. The
   wallpaper window takes no keyboard input, and the settings window's keys go to wry's
   web view.

Still in the source and harmless: the WebKit cursors (`webkit_move`, `webkit_cell`) are loaded
from image files in HIServices by path, not through any API.

## Re-applying on a winit upgrade

1. Check whether the new winit still has the calls:
   `grep -rn "CGS\|SLS" <winit>/src/platform_impl/macos/`. If none are left except
   `CGShieldingWindowLevel`, delete `vendor/winit` and the `winit` line in
   `[patch.crates-io]`, and you are done.
2. Otherwise copy the new crate from `~/.cargo/registry/src/*/winit-<version>/` (the files
   listed above), strip the same `Cargo.toml` tables, and redo the two edits: delete the
   private `extern` declarations in `ffi.rs`, empty `set_blur` in `window_delegate.rs`,
   delete the undocumented cursors in `cursor.rs` and the `_wantsKeyDownForEvent:` override in
   `view.rs`. Remove any import the compiler then reports unused.
3. Build the App Store app (`packaging/appstore/build.sh --dry-run`) and run the `nm` check
   above, plus `strings -a <binary> | grep -E '^_[a-zA-Z]+[A-Z:]'` for leading-underscore
   selectors. Update the version and commit hash at the top of this file.
