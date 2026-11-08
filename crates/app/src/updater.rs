//! Automatic updates for the direct-download build (DMG and Homebrew), through Sparkle 2.
//!
//! Built only with the `updater` Cargo feature. The Mac App Store build leaves it off, so
//! it contains no Sparkle code at all (Apple forbids self-updaters there); without the
//! feature [`Updater::start`] always returns `None`.
//!
//! Sparkle is not linked. `packaging/build-app.sh` embeds `Sparkle.framework` in
//! `Contents/Frameworks`, and this module loads it from there at run time. When it is
//! missing (`cargo run`, a bare binary) there is no updater and no menu item, and
//! nothing else changes. Sparkle reads `SUFeedURL`, `SUPublicEDKey` and
//! `SUEnableAutomaticChecks` from `Info.plist` and checks once a day on its own schedule.

/// Owns Sparkle's `SPUStandardUpdaterController` for the life of the app.
pub struct Updater {
    #[cfg(feature = "updater")]
    controller: objc2::rc::Retained<objc2::runtime::AnyObject>,
}

impl Updater {
    /// Loads Sparkle from the app bundle and starts its updater (which schedules the
    /// daily check). `None` when the build has no updater or the framework is missing.
    /// Call on the main thread.
    pub fn start() -> Option<Self> {
        #[cfg(feature = "updater")]
        {
            sparkle::start().map(|controller| Updater { controller })
        }
        #[cfg(not(feature = "updater"))]
        {
            None
        }
    }

    /// "Check for Updates…": Sparkle shows its own progress and update windows.
    pub fn check_now(&self) {
        #[cfg(feature = "updater")]
        sparkle::check_now(&self.controller);
    }
}

#[cfg(feature = "updater")]
mod sparkle {
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::msg_send;
    use objc2_foundation::{NSBundle, NSString};

    /// The bundle's `Contents/Frameworks/Sparkle.framework`, loaded; `None` if absent.
    fn load_framework() -> Option<()> {
        let main = NSBundle::mainBundle();
        let frameworks = main.privateFrameworksPath()?;
        let path = format!("{frameworks}/Sparkle.framework");
        if !std::path::Path::new(&path).exists() {
            return None;
        }
        let bundle = NSBundle::bundleWithPath(&NSString::from_str(&path))?;
        // SAFETY: loading runs the framework's initializers; Sparkle.framework is the
        // checksum-pinned copy build-app.sh embedded and signed with the app.
        if let Err(e) = unsafe { bundle.loadAndReturnError() } {
            // NSDebugDescription carries dyld's reason (code signing, architecture, ...).
            let why = e.userInfo().objectForKey(&NSString::from_str("NSDebugDescription"));
            let why = why.and_then(|w| w.downcast::<NSString>().ok());
            let why = why.map_or_else(|| e.localizedDescription().to_string(), |w| w.to_string());
            eprintln!("updates: could not load {path}: {why}");
            return None;
        }
        Some(())
    }

    /// The value build-app.sh writes while no real key is set (see packaging/sparkle.env).
    const PLACEHOLDER_KEY: &str = "REPLACE_WITH_SPARKLE_PUBLIC_ED_KEY";

    /// Sparkle refuses to update (and says so in an alert) without a real EdDSA key, so a
    /// build that still has the placeholder doesn't start it at all.
    fn has_public_key() -> bool {
        let key = NSBundle::mainBundle().objectForInfoDictionaryKey(&NSString::from_str("SUPublicEDKey"));
        let key = key.and_then(|k| k.downcast::<NSString>().ok()).map(|k| k.to_string());
        matches!(key, Some(k) if !k.is_empty() && k != PLACEHOLDER_KEY)
    }

    pub fn start() -> Option<Retained<AnyObject>> {
        if !has_public_key() {
            // Also the `cargo run` case: a bare binary has no Info.plist.
            return None;
        }
        load_framework()?;
        let Some(class) = AnyClass::get(c"SPUStandardUpdaterController") else {
            eprintln!("updates: Sparkle.framework has no SPUStandardUpdaterController");
            return None;
        };
        let nil: *const AnyObject = std::ptr::null();
        // SAFETY: SPUStandardUpdaterController's designated initializer
        // -initWithStartingUpdater:(BOOL)updaterDelegate:(id)userDriverDelegate:(id);
        // nil delegates are allowed. Starting the updater checks the Info.plist keys and
        // schedules the automatic checks.
        let controller: Option<Retained<AnyObject>> = unsafe {
            let alloc: Allocated<AnyObject> = msg_send![class, alloc];
            msg_send![alloc, initWithStartingUpdater: true, updaterDelegate: nil, userDriverDelegate: nil]
        };
        if controller.is_none() {
            eprintln!("updates: Sparkle's updater did not start");
        }
        controller
    }

    pub fn check_now(controller: &AnyObject) {
        let nil: *const AnyObject = std::ptr::null();
        // SAFETY: -[SPUStandardUpdaterController checkForUpdates:(id)sender] is an IBAction;
        // the sender is unused.
        unsafe {
            let _: () = msg_send![controller, checkForUpdates: nil];
        }
    }
}
