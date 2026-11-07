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
//! breathes. Saving a note can start a short edit burst (Ink & Paper's ink bleed),
//! drawn at a low frame rate into a small overlay layer over the still scene; when it
//! ends the layer hides. A settled, still wallpaper draws nothing at all.

mod desktop;
mod menu;
mod overlay;
mod presence;
mod settings;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use vw_core::config::{self, Config, ConfigWatcher};
use vw_core::layout::{LayoutEngine, LayoutFrame, LayoutGraph};
use vw_core::vault::{Graph, Vault, VaultWatcher, watch};
use menu::{Action, AppMenu};
use overlay::{BurstOverlay, GlowOverlay};
use presence::Presence;
use settings::SettingsWindow;
use tray_icon::menu::MenuEvent;
use vw_render::{
    Ambient, BurstInstance, EditBurst, Framing, FramingOptions, Gpu, Look, LookContext, SceneStyle, Theme, WallpaperRenderer, derive_look, preferred_format,
    render_rgba, write_png,
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

/// A watcher batch that edits more notes than this is a bulk change (a sync, a git
/// checkout), not someone saving a note: it redraws, but doesn't animate.
const MAX_BURSTS: usize = 16;

enum AppEvent {
    Graph(Box<Graph>),
    Layout(Box<LayoutFrame>),
    Config(Box<Config>),
    Menu(MenuEvent),
    /// The Obsidian plugin's state file changed (`None`: gone or unreadable).
    Presence(Option<Box<vw_core::presence::PresenceState>>),
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
    /// Edit bursts' layer, made on the first burst this screen shows.
    burst_overlay: Option<BurstOverlay>,
    /// This screen's sprites for the running bursts, with each burst's start.
    sprites: Vec<(Instant, BurstInstance)>,
    /// `sprites` must be recomputed (bursts started or ended, or the scene moved).
    sprites_dirty: bool,
    /// A burst frame is due.
    burst_due: bool,
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
    theme: Theme,
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
    /// Open only while the user has it open; closing drops the web view.
    settings: Option<SettingsWindow>,
    /// Paused from the menu: keep the last frame, draw nothing new.
    paused: bool,
    /// What saving a note does under the current theme and motion setting.
    edit_burst: Option<EditBurst>,
    /// Notes edited in graphs sent to the layout engine but not shown yet, by the
    /// layout generation that will show them.
    pending_edits: Vec<(u64, Vec<String>)>,
    /// Running bursts: the edited note's id and when its burst started.
    bursts: Vec<(String, Instant)>,
    /// When the next burst frame is due.
    burst_next: Instant,
    /// Burst frames drawn since the bursts last settled (for the log).
    burst_frames: u32,
    /// What the Obsidian plugin says is open; inert without the plugin.
    presence: Presence,
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
        self.presence = Presence::default();
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
        let proxy = self.proxy.clone();
        self.presence = Presence::start(&path, move |s| {
            let _ = proxy.send_event(AppEvent::Presence(s.map(Box::new)));
        });
    }

    /// A graph from the vault watcher: an edit. Notes it saved are remembered, and
    /// their bursts start once the layout frame that shows this graph arrives.
    fn on_graph(&mut self, graph: Graph) {
        // While the Obsidian plugin is live, its save reports start the bursts instead.
        if self.edit_burst.is_some()
            && !self.presence.live(vw_core::presence::now_ms())
            && let Some((old, _)) = self.sent.last()
        {
            let edited = graph.edited_since(old);
            if !edited.is_empty() && edited.len() <= MAX_BURSTS {
                // send_graph makes this the generation after the last one sent.
                let generation = self.sent_base + self.sent.len() as u64 + 1;
                self.pending_edits.push((generation, edited));
            }
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
        let mut look = derive_look(graph, layout, &self.cfg, &LookContext { now_ms, light: false });
        self.presence.apply(&mut look, graph, layout, &self.style, now_ms);
        look
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
        self.send_tags();
        self.invalidate_scene();
        self.start_bursts(frame.generation);
    }

    /// Starts the bursts for edits whose graph is now on screen.
    fn start_bursts(&mut self, generation: u64) {
        let (ready, waiting): (Vec<_>, Vec<_>) = self.pending_edits.drain(..).partition(|(g, _)| *g <= generation);
        self.pending_edits = waiting;
        self.burst_notes(ready.into_iter().flat_map(|(_, ids)| ids).collect());
    }

    /// Starts (or restarts) the edit burst of these notes.
    fn burst_notes(&mut self, ids: Vec<String>) {
        if ids.is_empty() || self.edit_burst.is_none() || self.paused || desktop::low_power_mode() {
            return;
        }
        let now = Instant::now();
        for id in ids {
            // Saving again while it still bleeds starts it over.
            self.bursts.retain(|(b, _)| *b != id);
            self.bursts.push((id, now));
        }
        self.bursts.truncate(MAX_BURSTS);
        eprintln!("[{:.0} ms] edit burst: {} note(s)", self.ms(), self.bursts.len());
        self.burst_next = now;
        for s in &mut self.screens {
            s.sprites_dirty = true;
        }
    }

    /// Ends every burst at once (pause, theme or motion change).
    fn stop_bursts(&mut self) {
        self.bursts.clear();
        self.pending_edits.clear();
        let Some(gpu) = &self.gpu else { return };
        for s in &mut self.screens {
            s.sprites.clear();
            s.burst_due = false;
            if let Some(o) = &mut s.burst_overlay {
                o.hide(gpu);
            }
        }
    }

    /// Gives the settings page the vault's tags (for the tag-colour list).
    fn send_tags(&self) {
        if let (Some(s), Some(sh)) = (&self.settings, &self.shown) {
            let mut tags: Vec<(String, usize)> = sh.graph.discovered_tags.iter().map(|(t, n)| (t.clone(), *n)).collect();
            tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            s.set_tags(tags);
        }
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
        if let Some(s) = &self.settings {
            s.config_changed();
        }
        eprintln!("[{:.0} ms] config reloaded", self.ms());
    }

    fn apply_style(&mut self) {
        self.theme = Theme::from_config(&self.cfg);
        self.style = self.theme.style(&self.cfg, false);
        self.presence.adjust_style(&mut self.style, vw_core::presence::now_ms());
        self.framing = FramingOptions::from_config(&self.cfg);
        self.edit_burst = self.theme.edit_burst(&self.cfg);
        if self.edit_burst.is_none() {
            self.stop_bursts();
        }
        self.update_breathing();
    }

    /// Breathing as Core Animation should run it now: off when paused, in Low Power
    /// Mode, or when the style doesn't breathe.
    fn breathing(&self) -> Option<(f32, f32, f32)> {
        if self.paused || self.low_power {
            return None;
        }
        match self.theme.ambient(&self.style) {
            Ambient::Breathe { depth, period, fps } => Some((depth, period, fps)),
            Ambient::None => None,
        }
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
            burst_overlay: None,
            sprites: Vec::new(),
            sprites_dirty: true,
            burst_due: false,
        });
    }

    fn draw(&mut self, id: WindowId) {
        let breathing = self.breathing();
        let Some(gpu) = &self.gpu else { return };
        let Some(s) = self.screens.iter_mut().find(|s| s.window.id() == id) else { return };
        if !(s.frame_dirty || s.burst_due) || !s.check_visible() {
            // Nothing new, or hidden: a hidden screen stays dirty and draws when seen.
            return;
        }
        if s.frame_dirty {
            Self::draw_scene(gpu, s, self.shown.as_ref(), self.theme, &self.style, &self.framing, breathing);
        }
        if s.burst_due {
            s.burst_due = false;
            if let (Some(burst), Some(shown)) = (self.edit_burst, &self.shown) {
                Self::draw_bursts(gpu, s, shown, self.theme, &self.bursts, &self.framing, burst);
                self.burst_frames += 1;
            }
        }
    }

    /// Redraws the cached scene if it is out of date, and composites it to the screen.
    fn draw_scene(
        gpu: &Gpu,
        s: &mut Screen,
        shown: Option<&Shown>,
        theme: Theme,
        style: &SceneStyle,
        framing: &FramingOptions,
        breathing: Option<(f32, f32, f32)>,
    ) {
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
            let data = match shown {
                Some(sh) => {
                    let framing = Framing::fit_graph(&sh.positions, &sh.layout.degree, points[0], points[1], framing);
                    theme.build(&sh.layout, &sh.look, &sh.positions, &framing, style, points)
                }
                None => Default::default(),
            };
            s.renderer.set_scene(gpu, size, scale, &data, style);
            s.scene_dirty = false;
            // Notes may have moved under running bursts.
            s.sprites_dirty = true;
        }
        let view = frame.texture.create_view(&Default::default());
        match &mut s.overlay {
            Some(o) => {
                s.renderer.composite_scene_only(gpu, &view, style);
                o.draw(gpu, &s.renderer, style);
                o.set_breathing(breathing);
            }
            None => s.renderer.composite(gpu, &view, style, 1.0),
        }
        s.window.pre_present_notify();
        gpu.queue.present(frame);
        s.frame_dirty = false;
    }

    /// One burst frame into the screen's burst layer: a few sprites, each at its own
    /// progress. The scene is not touched.
    fn draw_bursts(
        gpu: &Gpu,
        s: &mut Screen,
        shown: &Shown,
        theme: Theme,
        bursts: &[(String, Instant)],
        framing: &FramingOptions,
        burst: EditBurst,
    ) {
        let Some(ns_window) = &s.ns_window else { return };
        let scale = s.window.scale_factor() as f32;
        let points = [s.config.width as f32 / scale, s.config.height as f32 / scale];
        if s.sprites_dirty {
            s.sprites_dirty = false;
            let framing = Framing::fit_graph(&shown.positions, &shown.layout.degree, points[0], points[1], framing);
            s.sprites = bursts
                .iter()
                .filter_map(|(id, start)| {
                    let i = shown.layout.id.iter().position(|x| x == id)?;
                    let sprite = theme.burst_sprites(&shown.layout, &shown.look, &shown.positions, &framing, &[i]).pop()?;
                    Some((*start, sprite))
                })
                .collect();
            let Some(region) = vw_render::burst::region(&s.sprites.iter().map(|(_, b)| *b).collect::<Vec<_>>(), points) else {
                s.sprites.clear();
                if let Some(o) = &mut s.burst_overlay {
                    o.hide(gpu);
                }
                return;
            };
            if s.burst_overlay.is_none() {
                s.burst_overlay = BurstOverlay::new(gpu, ns_window, s.config.format);
            }
            if let Some(o) = &mut s.burst_overlay {
                o.set_region(gpu, ns_window, region, scale);
            }
        }
        let (Some(o), false) = (&mut s.burst_overlay, s.sprites.is_empty()) else { return };
        let now = Instant::now();
        let frame: Vec<BurstInstance> = s
            .sprites
            .iter()
            .map(|(start, b)| BurstInstance { progress: vw_render::burst::progress(now - *start, burst.duration), ..*b })
            .collect();
        o.draw(gpu, burst.kind, &frame);
    }

    fn on_action(&mut self, action: Action, event_loop: &ActiveEventLoop) {
        match action {
            Action::TogglePause => {
                self.paused = !self.paused;
                if let Some(m) = &self.menu {
                    m.set_paused(self.paused);
                }
                if self.paused {
                    self.stop_bursts();
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
            Action::OpenSettings => {
                if let Some(s) = &self.settings {
                    s.focus();
                } else {
                    if !self.args.config.exists() && config::save(&self.args.config, &self.cfg).is_err() {
                        return;
                    }
                    match SettingsWindow::open(event_loop, self.args.config.clone()) {
                        Ok(w) => {
                            self.settings = Some(w);
                            self.send_tags();
                        }
                        Err(e) => eprintln!("settings window: {e}"),
                    }
                }
                // A menu-bar app isn't frontmost: bring the window forward.
                desktop::activate();
            }
            Action::OpenConfigFile => {
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

    /// Ends finished bursts and asks for the next burst frame when one is due.
    /// Returns when to wake for the next one; `None` once every burst has settled.
    fn step_bursts(&mut self, now: Instant) -> Option<Instant> {
        let burst = self.edit_burst?;
        if self.bursts.is_empty() {
            return None;
        }
        let duration = Duration::from_secs_f32(burst.duration);
        let before = self.bursts.len();
        self.bursts.retain(|(_, start)| now.duration_since(*start) < duration);
        if self.bursts.len() != before {
            for s in &mut self.screens {
                s.sprites_dirty = true;
            }
        }
        if self.bursts.is_empty() {
            // Settled: the still scene underneath is the final state.
            eprintln!("[{:.0} ms] edit bursts settled after {} frame(s)", self.ms(), self.burst_frames);
            self.burst_frames = 0;
            self.stop_bursts();
            return None;
        }
        if now >= self.burst_next {
            for s in &mut self.screens {
                if s.visible {
                    s.burst_due = true;
                    s.window.request_redraw();
                }
            }
            self.burst_next = now + Duration::from_secs_f32(1.0 / burst.fps);
        }
        Some(self.burst_next)
    }

    /// Renders the first screen's current picture to a PNG in ~/Pictures/Vault Wallpaper.
    fn export_png(&self) -> Result<PathBuf, String> {
        let (Some(gpu), Some(screen), Some(sh)) = (&self.gpu, self.screens.first(), &self.shown) else {
            return Err("nothing to export yet".into());
        };
        let scale = screen.window.scale_factor() as f32;
        let size = [screen.config.width, screen.config.height];
        let points = [size[0] as f32 / scale, size[1] as f32 / scale];
        let framing = Framing::fit_graph(&sh.positions, &sh.layout.degree, points[0], points[1], &self.framing);
        let data = self.theme.build(&sh.layout, &sh.look, &sh.positions, &framing, &self.style, points);
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
        if std::env::var_os("VW_WINDOW_IDS").is_some() {
            for s in &self.screens {
                if let Some(w) = &s.ns_window {
                    eprintln!("window id {}", w.windowNumber());
                }
            }
        }
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
            AppEvent::Graph(g) => self.on_graph(*g),
            AppEvent::Layout(f) => self.on_layout(*f),
            AppEvent::Config(c) => self.on_config(*c),
            AppEvent::Presence(s) => self.on_presence(s.map(|s| *s)),
            AppEvent::Menu(e) => {
                if let Some(action) = self.menu.as_ref().and_then(|m| m.action(&e)) {
                    self.on_action(action, event_loop);
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(s) = &mut self.settings
            && s.window_id() == id
        {
            if !s.handle_event(&event) {
                self.settings = None;
            }
            return;
        }
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
        if let Some(next) = self.step_presence() {
            wake = Some(wake.map_or(next, |w| w.min(next)));
        }
        if let Some(next) = self.step_bursts(now) {
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
        theme: Theme::from_config(&cfg),
        style: Theme::from_config(&cfg).style(&cfg, false),
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
        settings: None,
        paused: false,
        edit_burst: None,
        pending_edits: Vec::new(),
        bursts: Vec::new(),
        burst_next: Instant::now(),
        burst_frames: 0,
        presence: Presence::default(),
    };
    app.edit_burst = app.theme.edit_burst(&app.cfg);
    event_loop.run_app(&mut app).expect("run");
}
