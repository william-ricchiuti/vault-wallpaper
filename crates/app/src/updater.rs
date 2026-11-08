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
//!
//! The app has no Dock icon (`LSUIElement`), so a window that a *scheduled* check opens
//! would sit behind whatever app is in front, unnoticed. The user-driver delegate below
//! tells Sparkle the app handles "gentle reminders" and brings the app (and with it
//! Sparkle's update window) to the front when Sparkle is about to show an update.

/// Owns Sparkle's `SPUStandardUpdaterController` for the life of the app.
pub struct Updater {
    #[cfg(feature = "updater")]
    controller: objc2::rc::Retained<objc2::runtime::AnyObject>,
    /// Sparkle keeps only a weak reference to its user-driver delegate.
    #[cfg(feature = "updater")]
    _user_driver_delegate: objc2::rc::Retained<sparkle::UserDriverDelegate>,
}

impl Updater {
    /// Loads Sparkle from the app bundle and starts its updater (which schedules the
    /// daily check). `None` when the build has no updater or the framework is missing.
    /// Call on the main thread.
    pub fn start() -> Option<Self> {
        #[cfg(feature = "updater")]
        {
            sparkle::start().map(|(controller, delegate)| Updater {
                controller,
                _user_driver_delegate: delegate,
            })
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
    use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol};
    use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
    use objc2_foundation::{NSBundle, NSString};

    define_class!(
        /// Sparkle's `SPUStandardUserDriverDelegate`. Sparkle asks with
        /// `respondsToSelector:` for each (optional) method, so no protocol conformance is
        /// declared.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "VWSparkleUserDriverDelegate"]
        pub struct UserDriverDelegate;

        unsafe impl NSObjectProtocol for UserDriverDelegate {}

        impl UserDriverDelegate {
            /// YES: this background app takes care of making scheduled update alerts
            /// noticeable (below), so Sparkle doesn't warn about it and doesn't steal focus
            /// on its own.
            #[unsafe(method(supportsGentleScheduledUpdateReminders))]
            fn supports_gentle_scheduled_update_reminders(&self) -> bool {
                true
            }

            /// `-standardUserDriverWillHandleShowingUpdate:(BOOL)forUpdate:(SUAppcastItem *)
            /// state:(SPUUserUpdateState *)`: called just before Sparkle shows an update it
            /// found. When Sparkle shows it itself, bring the app forward so its window is
            /// in front instead of behind the active app. (A check from the menu already
            /// activated the app; doing it again is harmless.)
            #[unsafe(method(standardUserDriverWillHandleShowingUpdate:forUpdate:state:))]
            fn will_handle_showing_update(
                &self,
                handle_showing_update: bool,
                _update: &AnyObject,
                state: &AnyObject,
            ) {
                // SAFETY: SPUUserUpdateState has `@property (readonly) BOOL userInitiated`.
                let user_initiated: bool = unsafe { msg_send![state, userInitiated] };
                if handle_showing_update {
                    if !user_initiated {
                        eprintln!("updates: a scheduled check found an update; showing it");
                    }
                    crate::desktop::activate();
                }
            }
        }
    );

    impl UserDriverDelegate {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            // SAFETY: plain NSObject subclass with no ivars; -init is NSObject's.
            unsafe { msg_send![Self::alloc(mtm), init] }
        }
    }

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

    pub fn start() -> Option<(Retained<AnyObject>, Retained<UserDriverDelegate>)> {
        let mtm = MainThreadMarker::new()?;
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
        let delegate = UserDriverDelegate::new(mtm);
        // SAFETY: SPUStandardUpdaterController's designated initializer
        // -initWithStartingUpdater:(BOOL)updaterDelegate:(id)userDriverDelegate:(id);
        // a nil updater delegate is allowed, and the user-driver delegate is weak (kept
        // alive by Updater). Starting the updater checks the Info.plist keys and schedules
        // the automatic checks.
        let controller: Option<Retained<AnyObject>> = unsafe {
            let alloc: Allocated<AnyObject> = msg_send![class, alloc];
            msg_send![alloc, initWithStartingUpdater: true, updaterDelegate: nil, userDriverDelegate: &*delegate]
        };
        match controller {
            Some(c) => Some((c, delegate)),
            None => {
                eprintln!("updates: Sparkle's updater did not start");
                None
            }
        }
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
