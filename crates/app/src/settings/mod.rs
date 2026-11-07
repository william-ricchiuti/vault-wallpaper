//! The settings window: the web version's `settings.html` in a `wry` web view.
//!
//! The page and every endpoint it calls are answered in-process through the `vw://`
//! custom scheme ([`api`]); there is no HTTP server and no port. Saves go straight to the
//! config file, so the app's config watcher restyles the live wallpaper, which is the
//! preview.
//!
//! Lightness: the window and its web view exist only while the window is open. Closing
//! it drops both (WebKit's web-content process exits with them), so a closed settings
//! window costs nothing.
//!
//! # What the main loop must do
//!
//! ```ignore
//! // Open (e.g. from a menu item); if one is already open, call `focus()` instead.
//! self.settings = Some(SettingsWindow::open(event_loop, config_path)?);
//! settings.set_tags(graph_tag_counts);           // optional, see `set_tags`
//!
//! // In `window_event`, before anything else for that id:
//! if let Some(s) = &mut self.settings && s.window_id() == id {
//!     if !s.handle_event(&event) { self.settings = None; } // drop = close
//!     return;
//! }
//!
//! // When the config watcher reports a change (any source, including this window):
//! if let Some(s) = &self.settings { s.config_changed(); }
//! ```
//!
//! - Everything here runs on the main thread (WebKit requires it); `open` needs the
//!   `ActiveEventLoop`, so open it from an event handler, not before `run_app`.
//! - The window must not make the app quit when it closes. Don't exit on its
//!   `CloseRequested`; just drop the `SettingsWindow`.
//! - Text fields need an Edit menu for Cmd+C/V/X/A/Z. [`open`](SettingsWindow::open)
//!   adds one to the app's main menu if it has none ([`install_edit_menu`]). If the app
//!   replaces its main menu later, include an Edit menu or call `install_edit_menu` again.
//! - A patch that sets `ignorePaths` or `showUnresolvedLinks` needs a vault rescan. The
//!   window doesn't do that; the app's config-watcher callback should compare the old and
//!   new values, as it does for hand edits.
//! - With an accessory (menu-bar-only) activation policy the window still opens in
//!   front: `open` and `focus` activate the app.

mod api;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSApplication, NSAutoresizingMaskOptions, NSMenu, NSMenuItem};
use objc2_foundation::NSString;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};
use wry::{WebView, WebViewBuilder, WebViewExtMacOS};

use api::Backend;

/// The page's address. The scheme is registered for this web view only.
const START_URL: &str = "vw://app/settings.html";
const SCHEME: &str = "vw";

#[derive(Debug)]
pub enum SettingsError {
    Window(winit::error::OsError),
    WebView(wry::Error),
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::Window(e) => write!(f, "can't create the settings window: {e}"),
            SettingsError::WebView(e) => write!(f, "can't create the settings web view: {e}"),
        }
    }
}

impl std::error::Error for SettingsError {}

/// An open settings window. Drop it to close it.
pub struct SettingsWindow {
    // Field order is drop order: the web view goes before the window that hosts it.
    webview: WebView,
    window: Window,
    backend: Arc<Mutex<Backend>>,
}

impl SettingsWindow {
    /// Opens the window on the page, backed by the config at `config_path` (normally
    /// [`vw_core::config::default_path`]). Custom presets live next to it in
    /// `custom-presets.json`.
    pub fn open(
        event_loop: &ActiveEventLoop,
        config_path: PathBuf,
    ) -> Result<SettingsWindow, SettingsError> {
        let attrs = Window::default_attributes()
            .with_title("Vault Wallpaper Settings")
            .with_inner_size(LogicalSize::new(980.0, 820.0))
            .with_min_inner_size(LogicalSize::new(480.0, 400.0));
        let window = event_loop
            .create_window(attrs)
            .map_err(SettingsError::Window)?;

        let backend = Arc::new(Mutex::new(Backend::new(config_path)));
        let handler = Arc::clone(&backend);
        let size = window.inner_size();
        let webview = WebViewBuilder::new()
            .with_bounds(wry::Rect {
                position: PhysicalPosition::new(0, 0).into(),
                size: size.into(),
            })
            .with_custom_protocol(SCHEME.into(), move |_id, req| match handler.lock() {
                Ok(b) => b.handle(&req),
                Err(poisoned) => poisoned.into_inner().handle(&req),
            })
            // Matches the page's dark background, so opening doesn't flash white.
            .with_background_color((14, 14, 20, 255))
            .with_url(START_URL)
            // `window.ipc.postMessage(text)` from the page goes to stderr: page errors
            // and the example's scripted checks report through it.
            .with_ipc_handler(|req| eprintln!("[settings page] {}", req.body()))
            // Stay on our page; there is nothing on the web to navigate to.
            .with_navigation_handler(|url| url.starts_with("vw://"))
            .with_new_window_req_handler(|_url, _features| wry::NewWindowResponse::Deny)
            // As a child of winit's content view, not `build`: `build` swaps the window's
            // content view for wry's own, and winit's window delegate casts the content
            // view back to its view type on every focus change. Clicking another window
            // then aborted the process ("tried to access uninitialized instance variable"
            // in `windowDidResignKey:`).
            .build_as_child(&window)
            .map_err(SettingsError::WebView)?;
        // Fill the window and follow its size without per-event work.
        webview.webview().setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        // Keyboard focus in the page from the start (Tab, typing), not in winit's view.
        let _ = webview.focus();

        if let Some(mtm) = MainThreadMarker::new() {
            install_edit_menu(mtm);
        }
        let this = SettingsWindow {
            webview,
            window,
            backend,
        };
        this.focus();
        Ok(this)
    }

    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    /// Feed it every `WindowEvent` for [`window_id`](Self::window_id). Returns `false`
    /// once the user closed the window; drop the `SettingsWindow` then. Resizing is
    /// handled by the web view itself.
    pub fn handle_event(&mut self, event: &WindowEvent) -> bool {
        !matches!(event, WindowEvent::CloseRequested | WindowEvent::Destroyed)
    }

    /// Brings the window to the front (for a second "Settings…" click).
    pub fn focus(&self) {
        self.window.set_minimized(false);
        self.window.focus_window();
        let _ = self.webview.focus();
    }

    /// Tells the page the config file changed (a hand edit, a theme rotation, or its own
    /// save coming back through the watcher), so it re-reads `/api/config`. The web
    /// version did this with a `config` server-sent event.
    pub fn config_changed(&self) {
        let _ = self
            .webview
            .evaluate_script("window.vwConfigChanged && window.vwConfigChanged()");
    }

    /// Runs `js` in the page. For tests and debugging (the example's `VW_SETTINGS_SCRIPT`).
    #[allow(dead_code)]
    pub fn evaluate_script(&self, js: &str) {
        let _ = self.webview.evaluate_script(js);
    }

    /// Supplies the vault's tags (name, note count) for the tag-colour list, and refreshes
    /// the page if it's showing. Call after opening and whenever the graph changes.
    /// The app sends the live graph's `discoveredTags` after opening and on each new
    /// graph.
    pub fn set_tags(&self, tags: Vec<(String, usize)>) {
        if let Ok(mut b) = self.backend.lock() {
            b.tags = tags;
        }
        let _ = self
            .webview
            .evaluate_script("window.vwTagsChanged && window.vwTagsChanged()");
    }
}

/// Adds a standard Edit menu (Undo, Redo, Cut, Copy, Paste, Select All) to the app's main
/// menu, unless it already has one. macOS routes those shortcuts through the menu bar, so
/// without it the web view's text fields can't copy or paste. Creates a main menu if the
/// app has none.
pub fn install_edit_menu(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let main = app.mainMenu().unwrap_or_else(|| {
        let m = NSMenu::new(mtm);
        app.setMainMenu(Some(&m));
        m
    });
    let title = NSString::from_str("Edit");
    if main.itemWithTitle(&title).is_some() {
        return;
    }
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &title);
    let items = [
        ("Undo", sel!(undo:), "z"),
        ("Redo", sel!(redo:), "Z"),
        ("", sel!(separator), ""),
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ];
    for (name, action, key) in items {
        if name.is_empty() {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        }
        // SAFETY: standard responder-chain actions; nil target means "first responder".
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(name),
                Some(action),
                &NSString::from_str(key),
            )
        };
        menu.addItem(&item);
    }
    let top = NSMenuItem::new(mtm);
    top.setTitle(&title);
    top.setSubmenu(Some(&menu));
    main.addItem(&top);
}
