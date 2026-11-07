//! Power: whether the Mac runs on battery or in Low Power Mode, and what that means
//! for motion. On battery the wallpaper holds still unless the motion setting is
//! Showcase; in Low Power Mode it always holds still.

use std::ffi::c_void;

use vw_core::config::MotionMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PowerState {
    pub low_power: bool,
    pub on_battery: bool,
}

impl PowerState {
    pub fn read() -> Self {
        PowerState { low_power: crate::desktop::low_power_mode(), on_battery: on_battery() }
    }

    /// Whether anything may move (glow breathing, edit bursts) under `mode`.
    pub fn motion_allowed(self, mode: MotionMode) -> bool {
        !self.low_power && (!self.on_battery || mode == MotionMode::Showcase)
    }
}

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFRunLoopRef = *const c_void;
type CFRunLoopSourceRef = *const c_void;
type Callback = extern "C" fn(*mut c_void);

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSGetProvidingPowerSourceType(snapshot: CFTypeRef) -> CFStringRef;
    fn IOPSNotificationCreateRunLoopSource(callback: Callback, context: *mut c_void) -> CFRunLoopSourceRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: CFStringRef;
    fn CFRunLoopGetMain() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopSourceInvalidate(source: CFRunLoopSourceRef);
    fn CFRelease(cf: CFTypeRef);
}

/// Whether the Mac is drawing from its battery right now (`false` on a desktop Mac,
/// or when the power sources can't be read).
pub fn on_battery() -> bool {
    // SAFETY: plain IOKit calls; the snapshot is released, and the type string it
    // returns is borrowed from the snapshot and only read before the release.
    unsafe {
        let blob = IOPSCopyPowerSourcesInfo();
        if blob.is_null() {
            return false;
        }
        let kind = IOPSGetProvidingPowerSourceType(blob);
        // CFString and NSString are toll-free bridged.
        let battery = !kind.is_null() && (*(kind as *const objc2_foundation::NSString)).to_string() == "Battery Power";
        CFRelease(blob);
        battery
    }
}

/// Calls `f` on the main thread whenever a power source changes (plugged in or out,
/// battery level). Registered on the main run loop; removed on drop.
pub struct SourceWatch {
    source: CFRunLoopSourceRef,
    context: *mut Box<dyn Fn()>,
}

extern "C" fn power_changed(context: *mut c_void) {
    // SAFETY: the context is the boxed closure SourceWatch owns, alive until the
    // source is invalidated in drop.
    let f = unsafe { &*(context as *const Box<dyn Fn()>) };
    f();
}

impl SourceWatch {
    pub fn new(f: impl Fn() + 'static) -> Option<Self> {
        let context: *mut Box<dyn Fn()> = Box::into_raw(Box::new(Box::new(f)));
        // SAFETY: the callback and context live as long as the source (see drop).
        unsafe {
            let source = IOPSNotificationCreateRunLoopSource(power_changed, context.cast());
            if source.is_null() {
                drop(Box::from_raw(context));
                return None;
            }
            CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopDefaultMode);
            Some(SourceWatch { source, context })
        }
    }
}

impl Drop for SourceWatch {
    fn drop(&mut self) {
        // SAFETY: invalidating removes the source from every run loop, so the
        // callback can't run after the context is freed.
        unsafe {
            CFRunLoopSourceInvalidate(self.source);
            CFRelease(self.source);
            drop(Box::from_raw(self.context));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_stops_motion_unless_showcase() {
        let battery = PowerState { low_power: false, on_battery: true };
        assert!(!battery.motion_allowed(MotionMode::Light));
        assert!(!battery.motion_allowed(MotionMode::Balanced));
        assert!(!battery.motion_allowed(MotionMode::Still));
        assert!(battery.motion_allowed(MotionMode::Showcase));
    }

    #[test]
    fn low_power_always_stops_motion() {
        for on_battery in [false, true] {
            let s = PowerState { low_power: true, on_battery };
            assert!(!s.motion_allowed(MotionMode::Showcase));
            assert!(!s.motion_allowed(MotionMode::Balanced));
        }
    }

    #[test]
    fn mains_power_allows_motion() {
        assert!(PowerState::default().motion_allowed(MotionMode::Balanced));
    }

    #[test]
    fn reads_power_source() {
        // Only checks the call is sound; the answer depends on the machine.
        let _ = on_battery();
    }
}
