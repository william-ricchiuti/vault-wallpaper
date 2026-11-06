//! Vault Wallpaper: your Obsidian vault's graph, drawn behind the desktop icons.
//!
//!   vault-wallpaper [--vault <path>] [--config <file>] [--front] [--cache <file>]
//!
//! Settings come from `~/Library/Application Support/Vault Wallpaper/config.json`
//! (the web version's format) and are reloaded when the file changes; `--vault`
//! overrides the vault path.
//!
//! Threads: the vault watcher, the config watcher and the layout engine run in the
//! background and post results to the main thread. The main thread redraws the cached
//! scene only when something changed, and runs cheap composite-only frames while glow
//! breathes. A settled, still wallpaper draws nothing at all.

mod desktop;
mod menu;
mod overlay;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use vw_core::config::{self, Config, ConfigWatcher};
use vw_core::layout::{LayoutEngine, LayoutFrame, LayoutGraph};
use vw_core::vault::{Graph, Vault, VaultWatcher, watch};
use menu::{Action, AppMenu};
use overlay::GlowOverlay;
use tray_icon::menu::MenuEvent;
use vw_render::{
    Framing, FramingOptions, Gpu, Look, LookContext, SceneStyle, WallpaperRenderer, build_scene, derive_look, preferred_format,
    render_rgba, style_from_config, write_png,
};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS, WindowAttributesExtMacOS};
use winit::window::{Window, WindowId};

/// How often moving layout frames are drawn while the graph settles. Settling is
/// short and rare (first run, big edits); a low rate keeps it cheap.
const LAYOUT_FPS: f64 = 12.0;

enum AppEvent {
    Graph(Box<Graph>),
    Layout(Box<LayoutFrame>),
    Config(Box<Config>),
    Menu(MenuEvent),
}

struct Args {
    vault: Option<PathBuf>,
    config: PathBuf,
    front: bool,
    cache: PathBuf,
}

fn support_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/Application Support/Vault Wallpaper")
}

fn parse_args() -> Args {
    let mut args = Args {
        vault: std::env::var_os("VW_VAULT").map(PathBuf::from),
        config: config::default_path().unwrap_or_else(|| support_dir().join("config.json")),
        front: false,
        cache: support_dir().join("layout.json"),
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--vault" => args.vault = it.next().map(PathBuf::from),
            "--config" => args.config = it.next().map(PathBuf::from).expect("--config needs a path"),
            "--front" => args.front = true,
            "--cache" => args.cache = it.next().map(PathBuf::from).expect("--cache needs a path"),
            other => eprintln!("unknown argument: {other}"),
        }
    }
    args
}

fn load_config(path: &std::path::Path) -> Config {
    match config::load(path) {
        Ok(s) => {
            for issue in &s.issues {
                eprintln!("config: {} ({})", issue.key, issue.message);
            }
            s.config
        }
        Err(e) if e.is_not_found() => Config::default(),
        Err(e) => {
            eprintln!("config: {e}; using defaults");
            Config::default()
        }
    }
}

/// One wallpaper window (one per display).
struct Screen {
    window: Arc<Window>,
    ns_window: Option<Retained<NSWindow>>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: WallpaperRenderer,
    /// Glow in its own layer, so Core Animation can breathe it without frames.
    overlay: Option<GlowOverlay>,
    /// The cached scene is out of date (graph, positions, look, config or size).
    scene_dirty: bool,
    /// The screen needs a composite (after a scene change, or an animation step).
    frame_dirty: bool,
    visible: bool,
}

impl Screen {
    fn check_visible(&mut self) -> bool {
        self.visible = self.ns_window.as_deref().is_none_or(desktop::is_visible);
        self.visible
    }
}

/// The graph a layout frame belongs to, with its look.
struct Shown {
    graph: Graph,
    layout: LayoutGraph,
    look: Look,
    positions: Vec<[f32; 2]>,
}

struct App {
    args: Args,
    proxy: EventLoopProxy<AppEvent>,
    cfg: Config,
    style: SceneStyle,
    framing: FramingOptions,
    gpu: Option<Gpu>,
    screens: Vec<Screen>,
    /// Graphs sent to the layout engine, oldest first. The engine numbers its runs, so
    /// frame generation N belongs to the Nth graph sent.
    sent: Vec<(Graph, LayoutGraph)>,
    sent_base: u64,
    shown: Option<Shown>,
    engine: Option<LayoutEngine>,
    _vault_watcher: Option<VaultWatcher>,
    _config_watcher: Option<ConfigWatcher>,
    started: Instant,
    /// When Low Power Mode was last checked while breathing.
    power_checked: Instant,
    low_power: bool,
    menu: Option<AppMenu>,
    /// Paused from the menu: keep the last frame, draw nothing new.
    paused: bool,
}

impl App {
    fn ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    fn vault_path(&self) -> Option<PathBuf> {
        self.args.vault.clone().or_else(|| self.cfg.vault_path.as_ref().map(PathBuf::from))
    }

    fn start_background(&mut self) {
        let proxy = self.proxy.clone();
        match config::watch(&self.args.config, self.cfg.clone(), move |c| {
            let _ = proxy.send_event(AppEvent::Config(Box::new(c)));
        }) {
            Ok(w) => self._config_watcher = Some(w),
            Err(e) => eprintln!("not watching the config: {e}"),
        }
        let proxy = self.proxy.clone();
        self.engine = Some(LayoutEngine::spawn(
            self.cfg.layout_options(),
            Some(self.args.cache.clone()),
            Duration::from_secs_f64(1.0 / LAYOUT_FPS),
            move |f| {
                let _ = proxy.send_event(AppEvent::Layout(Box::new(f)));
            },
        ));
        self.open_vault();
    }

    /// (Re)scans the vault and starts watching it.
    fn open_vault(&mut self) {
        self._vault_watcher = None;
        let Some(path) = self.vault_path() else {
            eprintln!("no vault set: pass --vault or set vaultPath in {}", self.args.config.display());
            return;
        };
        let t = Instant::now();
        let vault = Vault::scan(&path, self.cfg.vault_options());
        eprintln!("[{:.0} ms] scanned {} notes in {:.0} ms", self.ms(), vault.len(), t.elapsed().as_secs_f64() * 1000.0);
        let graph = vault.graph();
        let proxy = self.proxy.clone();
        match watch(vault, move |g| {
            let _ = proxy.send_event(AppEvent::Graph(Box::new(g)));
        }) {
            Ok(w) => self._vault_watcher = Some(w),
            Err(e) => eprintln!("not watching the vault: {e}"),
        }
        self.send_graph(graph);
    }

    fn send_graph(&mut self, graph: Graph) {
        let layout = LayoutGraph::from_graph(&graph);
        if let Some(engine) = &self.engine {
            engine.set_graph(graph.clone());
        }
        self.sent.push((graph, layout));
    }

    fn look_for(&self, graph: &Graph, layout: &LayoutGraph) -> Look {
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        derive_look(graph, layout, &self.cfg, &LookContext { now_ms, light: false })
    }

    fn on_layout(&mut self, frame: LayoutFrame) {
        // Drop graphs older than the one this frame was computed for.
        let Some(index) = frame.generation.checked_sub(1 + self.sent_base).map(|i| i as usize) else { return };
        if index >= self.sent.len() {
            return;
        }
        self.sent.drain(..index);
        self.sent_base += index as u64;
        let (graph, layout) = &self.sent[0];
        if layout.len() != frame.positions.len() {
            return;
        }
        if frame.settled {
            eprintln!("[{:.0} ms] layout settled", self.ms());
        }
        // The look only depends on the graph and config: reuse it while positions move.
        let same_graph = self.shown.as_ref().is_some_and(|s| s.layout.id == layout.id && s.layout.links == layout.links);
        let (graph, layout) = (graph.clone(), layout.clone());
        let look = match self.shown.take() {
            Some(s) if same_graph => s.look,
            _ => self.look_for(&graph, &layout),
        };
        self.shown = Some(Shown { graph, layout, look, positions: frame.positions });
        self.invalidate_scene();
    }

    fn on_config(&mut self, cfg: Config) {
        let old_vault = self.vault_path();
        let old = std::mem::replace(&mut self.cfg, cfg);
        let vault_changed = self.vault_path() != old_vault
            || old.ignore_paths != self.cfg.ignore_paths
            || old.show_unresolved_links != self.cfg.show_unresolved_links;
        if (old.layout != self.cfg.layout || old.cluster_by_tag != self.cfg.cluster_by_tag)
            && let Some(engine) = &self.engine
        {
            engine.set_options(self.cfg.layout_options());
        }
        self.apply_style();
        if let Some(s) = self.shown.take() {
            let look = self.look_for(&s.graph, &s.layout);
            self.shown = Some(Shown { look, ..s });
        }
        if vault_changed {
            self.open_vault();
        }
        self.invalidate_scene();
        eprintln!("[{:.0} ms] config reloaded", self.ms());
    }

    fn apply_style(&mut self) {
        self.style = style_from_config(&self.cfg, false);
        self.framing = FramingOptions::from_config(&self.cfg);
        self.update_breathing();
    }

    /// Breathing as Core Animation should run it now: off when paused, in Low Power
    /// Mode, or when the style doesn't breathe.
    fn breathing(&self) -> Option<(f32, f32, f32)> {
        if self.paused || self.low_power || !self.style.animates() {
            return None;
        }
        self.style.breathing.map(|(depth, period)| (depth, period, self.style.motion_fps))
    }

    fn update_breathing(&mut self) {
        let b = self.breathing();
        for s in &mut self.screens {
            if let Some(o) = &mut s.overlay {
                o.set_breathing(b);
            }
        }
    }

    fn invalidate_scene(&mut self) {
        if self.paused {
            return;
        }
        for s in &mut self.screens {
            s.scene_dirty = true;
            s.frame_dirty = true;
            if s.check_visible() {
                s.window.request_redraw();
            }
        }
    }

    fn add_screen(&mut self, event_loop: &ActiveEventLoop, pos: PhysicalPosition<i32>, size: PhysicalSize<u32>) {
        let attrs = Window::default_attributes()
            .with_title("Vault Wallpaper")
            .with_decorations(false)
            .with_resizable(false)
            .with_position(pos)
            .with_inner_size(size)
            .with_has_shadow(false)
            .with_active(false);
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let ns_window = desktop::ns_window(&window);
        if let (Some(w), false) = (&ns_window, self.args.front) {
            desktop::place_on_desktop(w);
        }
        let instance = match &self.gpu {
            Some(g) => g.instance.clone(),
            None => wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle()),
        };
        let surface = instance.create_surface(window.clone()).expect("surface");
        if self.gpu.is_none() {
            self.gpu = Some(Gpu::new(Some(&surface), instance).expect("no usable GPU"));
        }
        let gpu = self.gpu.as_ref().unwrap();
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = preferred_format(&caps.formats);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            // One frame in flight: we draw rarely, so latency never builds up, and
            // fewer full-screen buffers stay alive.
            desired_maximum_frame_latency: 1,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(&gpu.device, &config);
        let renderer = WallpaperRenderer::new(gpu, format);
        let overlay = ns_window.as_deref().and_then(|w| GlowOverlay::new(gpu, w, format));
        if overlay.is_none() {
            eprintln!("glow overlay unavailable; glow will not breathe");
        }
        self.screens.push(Screen {
            window,
            ns_window,
            surface,
            config,
            renderer,
            overlay,
            scene_dirty: true,
            frame_dirty: true,
            visible: true,
        });
    }

    fn draw(&mut self, id: WindowId) {
        let breathing = self.breathing();
        let Some(gpu) = &self.gpu else { return };
        let Some(s) = self.screens.iter_mut().find(|s| s.window.id() == id) else { return };
        if !s.frame_dirty || !s.check_visible() {
            // Nothing new, or hidden: a hidden screen stays dirty and draws when seen.
            return;
        }
        let frame = match s.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Occluded => {
                s.visible = false;
                return;
            }
            other => {
                eprintln!("surface: {other:?}");
                s.surface.configure(&gpu.device, &s.config);
                return;
            }
        };
        let scale = s.window.scale_factor() as f32;
        let size = [s.config.width, s.config.height];
        if s.scene_dirty {
            let points = [size[0] as f32 / scale, size[1] as f32 / scale];
            let data = match &self.shown {
                Some(sh) => {
                    let framing = Framing::fit(&sh.positions, points[0], points[1], &self.framing);
                    build_scene(&sh.layout, &sh.look, &sh.positions, &framing, &self.style, points)
                }
                None => Default::default(),
            };
            s.renderer.set_scene(gpu, size, scale, &data, &self.style);
            s.scene_dirty = false;
        }
        let view = frame.texture.create_view(&Default::default());
        match &mut s.overlay {
            Some(o) => {
                s.renderer.composite_scene_only(gpu, &view, &self.style);
                o.draw(gpu, &s.renderer, &self.style);
                o.set_breathing(breathing);
            }
            None => s.renderer.composite(gpu, &view, &self.style, 1.0),
        }
        s.window.pre_present_notify();
        gpu.queue.present(frame);
        s.frame_dirty = false;
    }

    fn on_action(&mut self, action: Action, event_loop: &ActiveEventLoop) {
        match action {
            Action::TogglePause => {
                self.paused = !self.paused;
                if let Some(m) = &self.menu {
                    m.set_paused(self.paused);
                }
                self.update_breathing();
                self.invalidate_scene();
            }
            Action::ChooseVault => {
                let picked = rfd::FileDialog::new().set_title("Choose your Obsidian vault").pick_folder();
                if let Some(dir) = picked {
                    let mut cfg = self.cfg.clone();
                    if !self.args.config.exists() {
                        // A new install starts still (lightness first); motion is opt-in.
                        cfg.motion_mode = config::MotionMode::Still;
                    }
                    cfg.vault_path = Some(dir.to_string_lossy().into_owned());
                    self.args.vault = None;
                    // The config watcher picks the change up and rescans.
                    if let Err(e) = config::save(&self.args.config, &cfg) {
                        eprintln!("could not save the config: {e}");
                    }
                }
            }
            // The settings window lands in Phase 5; until then, the file itself.
            Action::OpenSettings | Action::OpenConfigFile => {
                if !self.args.config.exists() && config::save(&self.args.config, &self.cfg).is_err() {
                    return;
                }
                desktop::open(&self.args.config);
            }
            Action::ExportPng => match self.export_png() {
                Ok(path) => desktop::reveal(&path),
                Err(e) => eprintln!("export failed: {e}"),
            },
            Action::ToggleLogin => {
                let want = !desktop::login::enabled();
                if let Err(e) = desktop::login::set(want) {
                    eprintln!("start at login: {e}");
                    if let Some(m) = &self.menu {
                        m.set_status("Start at Login needs the installed app");
                    }
                }
                if let Some(m) = &self.menu {
                    m.set_login(desktop::login::enabled());
                }
            }
            Action::Quit => event_loop.exit(),
        }
    }

    /// Renders the first screen's current picture to a PNG in ~/Pictures/Vault Wallpaper.
    fn export_png(&self) -> Result<PathBuf, String> {
        let (Some(gpu), Some(screen), Some(sh)) = (&self.gpu, self.screens.first(), &self.shown) else {
            return Err("nothing to export yet".into());
        };
        let scale = screen.window.scale_factor() as f32;
        let size = [screen.config.width, screen.config.height];
        let points = [size[0] as f32 / scale, size[1] as f32 / scale];
        let framing = Framing::fit(&sh.positions, points[0], points[1], &self.framing);
        let data = build_scene(&sh.layout, &sh.look, &sh.positions, &framing, &self.style, points);
        let rgba = render_rgba(gpu, size, scale, &self.style, &data, 1.0);
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let dir = home.join("Pictures/Vault Wallpaper");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let path = dir.join(format!("vault-wallpaper-{stamp}.png"));
        write_png(&path, size, &rgba).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.screens.is_empty() {
            return;
        }
        for m in event_loop.available_monitors().collect::<Vec<_>>() {
            self.add_screen(event_loop, m.position(), m.size());
        }
        eprintln!("[{:.0} ms] {} screen(s), low power mode {}", self.ms(), self.screens.len(), desktop::low_power_mode());
        let proxy = self.proxy.clone();
        self.menu = AppMenu::new(desktop::login::enabled(), move |e| {
            let _ = proxy.send_event(AppEvent::Menu(e));
        });
        if self.menu.is_none() {
            eprintln!("menu bar icon could not be created");
        }
        self.start_background();
        if self.vault_path().is_none() {
            // First run: nothing to show until a vault is picked.
            self.on_action(Action::ChooseVault, event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Graph(g) => self.send_graph(*g),
            AppEvent::Layout(f) => self.on_layout(*f),
            AppEvent::Config(c) => self.on_config(*c),
            AppEvent::Menu(e) => {
                if let Some(action) = self.menu.as_ref().and_then(|m| m.action(&e)) {
                    self.on_action(action, event_loop);
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => self.draw(id),
            WindowEvent::Occluded(_) => {
                if let Some(s) = self.screens.iter_mut().find(|s| s.window.id() == id)
                    && s.check_visible()
                    && s.frame_dirty
                {
                    s.window.request_redraw();
                }
            }
            WindowEvent::Resized(size) => {
                let Some(gpu) = &self.gpu else { return };
                if let Some(s) = self.screens.iter_mut().find(|s| s.window.id() == id) {
                    s.config.width = size.width.max(1);
                    s.config.height = size.height.max(1);
                    s.surface.configure(&gpu.device, &s.config);
                    if let (Some(o), Some(w)) = (&s.overlay, &s.ns_window) {
                        o.fit(w);
                    }
                    s.scene_dirty = true;
                    s.frame_dirty = true;
                    s.window.request_redraw();
                }
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        // A screen that is hidden *and* has something new to show is re-checked once a
        // second (occlusion events alone can be missed).
        let mut wake: Option<Instant> = None;
        for s in &mut self.screens {
            if s.frame_dirty && !s.visible {
                wake = Some(now + Duration::from_secs(1));
                if s.check_visible() {
                    s.window.request_redraw();
                }
            }
        }
        // While breathing, look at Low Power Mode once a minute and stop or restart
        // the animation to match. Nothing else wakes a still wallpaper.
        if self.style.animates() && !self.paused {
            if now.duration_since(self.power_checked) >= Duration::from_secs(60) {
                self.power_checked = now;
                let low = desktop::low_power_mode();
                if low != self.low_power {
                    self.low_power = low;
                    self.update_breathing();
                }
            }
            let next = self.power_checked + Duration::from_secs(60);
            wake = Some(wake.map_or(next, |w| w.min(next)));
        }
        event_loop.set_control_flow(match wake {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }
}

fn main() {
    let args = parse_args();
    let cfg = load_config(&args.config);
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .with_activation_policy(ActivationPolicy::Accessory)
        .build()
        .expect("event loop");
    let mut app = App {
        proxy: event_loop.create_proxy(),
        style: style_from_config(&cfg, false),
        framing: FramingOptions::from_config(&cfg),
        cfg,
        args,
        gpu: None,
        screens: Vec::new(),
        sent: Vec::new(),
        sent_base: 0,
        shown: None,
        engine: None,
        _vault_watcher: None,
        _config_watcher: None,
        started: Instant::now(),
        power_checked: Instant::now(),
        low_power: desktop::low_power_mode(),
        menu: None,
        paused: false,
    };
    event_loop.run_app(&mut app).expect("run");
}
