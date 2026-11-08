//! Making a window behave like a wallpaper on macOS, and reading whether it is
//! actually visible.

use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindow, NSWindowCollectionBehavior, NSWindowOcclusionState};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowLevelForKey(key: i32) -> i32;
}
/// `kCGDesktopWindowLevelKey`: one level above the system wallpaper, below the icons
/// (checked in Phase 0 against the window server's list).
const KCG_DESKTOP_WINDOW_LEVEL_KEY: i32 = 2;

/// The AppKit window behind a winit window.
pub fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return None };
    // SAFETY: winit hands out a valid NSView for the window's lifetime, and this runs
    // on the main thread inside the event loop.
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    view.window()
}

/// Behind the desktop icons, on every Space, click-through, out of Mission Control.
pub fn place_on_desktop(window: &NSWindow) {
    let level = unsafe { CGWindowLevelForKey(KCG_DESKTOP_WINDOW_LEVEL_KEY) } as isize;
    window.setLevel(level);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    window.setIgnoresMouseEvents(true);
    window.setHasShadow(false);
}

/// Whether any part of the window can be seen right now. This is the source of truth:
/// winit's occlusion events can disagree with it for many seconds (Phase 0 finding).
pub fn is_visible(window: &NSWindow) -> bool {
    window.occlusionState().contains(NSWindowOcclusionState::Visible)
}

pub fn low_power_mode() -> bool {
    objc2_foundation::NSProcessInfo::processInfo().isLowPowerModeEnabled()
}

/// Start at Login, through `SMAppService` (macOS 13+). Only works from the bundled
/// app; a bare binary gets an error back.
pub mod login {
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    pub fn enabled() -> bool {
        let service = unsafe { SMAppService::mainAppService() };
        let status = unsafe { service.status() };
        status == SMAppServiceStatus::Enabled
    }

    pub fn set(on: bool) -> Result<(), String> {
        let service = unsafe { SMAppService::mainAppService() };
        let result = if on { unsafe { service.registerAndReturnError() } } else { unsafe { service.unregisterAndReturnError() } };
        result.map_err(|e| e.localizedDescription().to_string())
    }
}

/// Shows a file in Finder. Through NSWorkspace rather than `open -R`, which also works
/// in the App Sandbox without spawning a process.
pub fn reveal(path: &std::path::Path) {
    if let Some(url) = objc2_foundation::NSURL::from_file_path(path) {
        let urls = objc2_foundation::NSArray::from_retained_slice(&[url]);
        objc2_app_kit::NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&urls);
    }
}

/// Opens a file with its default app.
pub fn open(path: &std::path::Path) {
    if let Some(url) = objc2_foundation::NSURL::from_file_path(path)
        && !objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(&url)
    {
        eprintln!("could not open {}", path.display());
    }
}

/// Brings this (Dock-less) app to the front, so a window it opens gets focus.
pub fn activate() {
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        #[allow(deprecated)]
        objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}

/// A start-up failure the app can't recover from: logs it, shows it in an alert (so
/// someone who launched the app from Finder sees why nothing appeared) and exits 1.
///
/// `VW_NO_ALERT=1` skips the alert (scripts and tests).
pub fn fatal(message: &str, detail: &str) -> ! {
    eprintln!("fatal: {message} ({detail})");
    if std::env::var_os("VW_NO_ALERT").is_none()
        && let Some(mtm) = objc2::MainThreadMarker::new()
    {
        activate();
        let alert = objc2_app_kit::NSAlert::new(mtm);
        alert.setMessageText(&objc2_foundation::NSString::from_str(message));
        alert.setInformativeText(&objc2_foundation::NSString::from_str(detail));
        alert.addButtonWithTitle(&objc2_foundation::NSString::from_str("Quit"));
        alert.runModal();
    }
    std::process::exit(1)
}
