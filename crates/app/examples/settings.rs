//! Opens just the settings window against a throwaway config, for testing it alone.
//!
//!   cargo run -p vault-wallpaper --example settings [-- <config.json>]
//!
//! Without an argument it uses a fresh `config.json` in a temp folder (printed at
//! start). It also runs the config watcher, the way the app does, and prints each change
//! it sees, so you can watch a click in the page reach the file.
//!
//! `VW_SETTINGS_SCRIPT=<file.js>` runs that script in the page three seconds after it
//! opens (for scripted checks: click a tile, apply a preset, read the status line).

#[allow(dead_code)]
#[path = "../src/settings/mod.rs"]
mod settings;

use std::path::PathBuf;

use settings::SettingsWindow;
use vw_core::config::{self, ConfigWatcher};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::WindowId;

enum Event {
    ConfigChanged,
    RunScript(String),
}

struct App {
    config_path: PathBuf,
    proxy: EventLoopProxy<Event>,
    settings: Option<SettingsWindow>,
    _watcher: Option<ConfigWatcher>,
}

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.settings.is_some() {
            return;
        }
        let current = config::load(&self.config_path)
            .map(|s| s.config)
            .unwrap_or_default();
        let proxy = self.proxy.clone();
        self._watcher = config::watch(&self.config_path, current, move |c| {
            println!(
                "config changed on disk: theme={} accent={} layout={:?}",
                c.theme, c.accent, c.layout
            );
            let _ = proxy.send_event(Event::ConfigChanged);
        })
        .map_err(|e| eprintln!("config watch failed: {e}"))
        .ok();

        let s = SettingsWindow::open(event_loop, self.config_path.clone())
            .expect("open settings window");
        // Stand-in tags until the app supplies the live graph's.
        s.set_tags(vec![
            ("project".into(), 42),
            ("journal".into(), 17),
            ("idea".into(), 5),
        ]);
        println!("settings window id: {:?}", s.window_id());
        self.settings = Some(s);

        if let Some(path) = std::env::var_os("VW_SETTINGS_SCRIPT") {
            let js = std::fs::read_to_string(path).expect("read VW_SETTINGS_SCRIPT");
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(3));
                let _ = proxy.send_event(Event::RunScript(js));
            });
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: Event) {
        if let Some(s) = &self.settings {
            match event {
                Event::ConfigChanged => s.config_changed(),
                Event::RunScript(js) => s.evaluate_script(&js),
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(s) = &mut self.settings
            && s.window_id() == id
            && !s.handle_event(&event)
        {
            self.settings = None;
            event_loop.exit();
        }
    }
}

fn main() {
    let config_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let dir =
                std::env::temp_dir().join(format!("vw-settings-example-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            dir.join("config.json")
        });
    println!("config file: {}", config_path.display());
    let event_loop = EventLoop::<Event>::with_user_event()
        .build()
        .expect("event loop");
    let proxy = event_loop.create_proxy();
    let mut app = App {
        config_path,
        proxy,
        settings: None,
        _watcher: None,
    };
    event_loop.run_app(&mut app).expect("run");
}
