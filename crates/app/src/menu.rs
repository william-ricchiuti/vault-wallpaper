//! The menu-bar icon: the app's only visible control (it has no Dock icon).

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// What a menu click asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    TogglePause,
    OpenSettings,
    ChooseVault,
    ImportWebConfig,
    OpenConfigFile,
    ExportPng,
    ToggleLogin,
    CheckForUpdates,
    Quit,
}

pub struct AppMenu {
    _tray: TrayIcon,
    pause: CheckMenuItem,
    login: CheckMenuItem,
    status: MenuItem,
    ids: Vec<(MenuId, Action)>,
}

impl AppMenu {
    /// Builds the menu and icon. Call on the main thread once the event loop runs.
    /// Clicks are delivered to `on_event` (from AppKit's thread: forward them to the
    /// event loop). `updates` adds "Check for Updates…" (only when Sparkle is loaded).
    pub fn new(login_enabled: bool, updates: bool, on_event: impl Fn(MenuEvent) + Send + Sync + 'static) -> Option<Self> {
        let menu = Menu::new();
        let status = MenuItem::new("Vault Wallpaper", false, None);
        let pause = CheckMenuItem::new("Pause Wallpaper", true, false, None);
        let settings = MenuItem::new("Settings…", true, None);
        let vault = MenuItem::new("Choose Vault…", true, None);
        let import = MenuItem::new("Import Web Version Settings…", true, None);
        let config = MenuItem::new("Open Config File", true, None);
        let export = MenuItem::new("Export PNG", true, None);
        let login = CheckMenuItem::new("Start at Login", true, login_enabled, None);
        let check = MenuItem::new("Check for Updates…", true, None);
        let quit = MenuItem::new("Quit Vault Wallpaper", true, None);
        let sep = PredefinedMenuItem::separator;
        menu.append_items(&[&status, &sep(), &pause, &settings, &vault, &import, &sep(), &export, &config, &login, &sep()]).ok()?;
        if updates {
            menu.append(&check).ok()?;
        }
        menu.append(&quit).ok()?;
        let ids = vec![
            (check.id().clone(), Action::CheckForUpdates),
            (pause.id().clone(), Action::TogglePause),
            (settings.id().clone(), Action::OpenSettings),
            (vault.id().clone(), Action::ChooseVault),
            (import.id().clone(), Action::ImportWebConfig),
            (config.id().clone(), Action::OpenConfigFile),
            (export.id().clone(), Action::ExportPng),
            (login.id().clone(), Action::ToggleLogin),
            (quit.id().clone(), Action::Quit),
        ];
        MenuEvent::set_event_handler(Some(on_event));
        let (rgba, size) = icon_rgba();
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Vault Wallpaper")
            .with_icon_templated(Icon::from_rgba(rgba, size, size).ok()?)
            .build()
            .ok()?;
        Some(AppMenu { _tray: tray, pause, login, status, ids })
    }

    pub fn action(&self, event: &MenuEvent) -> Option<Action> {
        self.ids.iter().find(|(id, _)| *id == event.id).map(|(_, a)| *a)
    }

    pub fn set_paused(&self, paused: bool) {
        self.pause.set_checked(paused);
    }

    pub fn set_login(&self, on: bool) {
        self.login.set_checked(on);
    }

    /// The greyed-out first line: what the wallpaper is showing.
    pub fn set_status(&self, text: &str) {
        self.status.set_text(text);
    }
}

/// A small graph glyph (three linked dots) as a template image: black on transparent,
/// tinted by macOS to match the menu bar.
fn icon_rgba() -> (Vec<u8>, u32) {
    const S: u32 = 36;
    let mut img = vec![0u8; (S * S * 4) as usize];
    let dots = [(9.0f32, 25.0f32, 5.0f32), (27.0, 25.0, 5.0), (18.0, 9.0, 6.0)];
    let mut cover = |x: u32, y: u32, a: f32| {
        let i = ((y * S + x) * 4) as usize;
        let v = (a.clamp(0.0, 1.0) * 255.0) as u8;
        img[i + 3] = img[i + 3].max(v);
    };
    for y in 0..S {
        for x in 0..S {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut a = 0.0f32;
            for &(cx, cy, r) in &dots {
                let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                a = a.max(r + 0.5 - d);
            }
            for (i, j) in [(0, 1), (0, 2), (1, 2)] {
                let ((ax, ay, _), (bx, by, _)) = (dots[i], dots[j]);
                let (dx, dy) = (bx - ax, by - ay);
                let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                let d = ((px - ax - dx * t).powi(2) + (py - ay - dy * t).powi(2)).sqrt();
                a = a.max(1.75 - d);
            }
            cover(x, y, a);
        }
    }
    (img, S)
}
