//! What the Mac tells us about the world around the wallpaper: displays coming and
//! going, sleep and wake, screen lock, fast user switching, Space changes, Low Power
//! Mode and the power source. Each arrives as a notification on the main thread and
//! is forwarded to the event loop as a [`SysEvent`]; nothing here polls.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue, NSString};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysEvent {
    /// Displays were added, removed, rearranged, or changed resolution or scale.
    Displays,
    /// The user can't see the screens any more: displays asleep, screen locked, or
    /// another user's session took over.
    Away,
    /// The user is back: displays woke, the screen was unlocked, or this session
    /// became active again.
    Back,
    /// The visible Space changed; occlusion may have changed with it.
    Space,
    /// Low Power Mode or the power source changed.
    Power,
}

type Token = Retained<ProtocolObject<dyn NSObjectProtocol>>;

/// Keeps the observers registered; dropping it unregisters them.
pub struct Observers {
    tokens: Vec<(Retained<NSNotificationCenter>, Token)>,
    _power_source: Option<crate::power::SourceWatch>,
}

impl Drop for Observers {
    fn drop(&mut self) {
        for (center, token) in &self.tokens {
            // SAFETY: the token came from this center's addObserverForName.
            unsafe { center.removeObserver(AsRef::<objc2::runtime::AnyObject>::as_ref(&**token)) };
        }
    }
}

/// Starts forwarding notifications to `send`, which runs on the main thread.
pub fn observe(send: impl Fn(SysEvent) + Clone + 'static) -> Observers {
    use objc2_app_kit::{
        NSApplicationDidChangeScreenParametersNotification, NSWorkspace, NSWorkspaceActiveSpaceDidChangeNotification,
        NSWorkspaceScreensDidSleepNotification, NSWorkspaceScreensDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
        NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification, NSWorkspaceDidWakeNotification,
    };
    use objc2_foundation::NSProcessInfoPowerStateDidChangeNotification;

    let mut tokens = Vec::new();
    let default = NSNotificationCenter::defaultCenter();
    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    let distributed: Retained<NSNotificationCenter> = Retained::into_super(NSDistributedNotificationCenter::defaultCenter());
    // SAFETY: the notification names are AppKit/Foundation constants, valid for the
    // process lifetime.
    unsafe {
        let mut add = |center: &Retained<NSNotificationCenter>, name: &NSString, ev: SysEvent| {
            let send = send.clone();
            let block = RcBlock::new(move |_: NonNull<NSNotification>| send(ev));
            // The main queue: blocks run on the main thread, where the event loop is.
            let token = center.addObserverForName_object_queue_usingBlock(Some(name), None, Some(&NSOperationQueue::mainQueue()), &block);
            tokens.push((center.clone(), token));
        };
        add(&default, NSApplicationDidChangeScreenParametersNotification, SysEvent::Displays);
        add(&default, NSProcessInfoPowerStateDidChangeNotification, SysEvent::Power);
        add(&workspace, NSWorkspaceScreensDidSleepNotification, SysEvent::Away);
        add(&workspace, NSWorkspaceWillSleepNotification, SysEvent::Away);
        add(&workspace, NSWorkspaceSessionDidResignActiveNotification, SysEvent::Away);
        add(&workspace, NSWorkspaceScreensDidWakeNotification, SysEvent::Back);
        add(&workspace, NSWorkspaceDidWakeNotification, SysEvent::Back);
        add(&workspace, NSWorkspaceSessionDidBecomeActiveNotification, SysEvent::Back);
        add(&workspace, NSWorkspaceActiveSpaceDidChangeNotification, SysEvent::Space);
        // Screen lock has no public AppKit notification; loginwindow posts these.
        add(&distributed, &NSString::from_str("com.apple.screenIsLocked"), SysEvent::Away);
        add(&distributed, &NSString::from_str("com.apple.screenIsUnlocked"), SysEvent::Back);
    }
    let power_send = send.clone();
    let power_source = crate::power::SourceWatch::new(move || power_send(SysEvent::Power));
    if power_source.is_none() {
        eprintln!("power source changes are not observed");
    }
    Observers { tokens, _power_source: power_source }
}
