//! Vault Wallpaper: your Obsidian vault's graph, drawn behind the desktop icons.
//!
//!   vault-wallpaper [--vault <path>] [--config <file>] [--front] [--cache <file>]
//!
//! Settings come from `~/Library/Application Support/Vault Wallpaper/config.json`
//! (the web version's format) and are reloaded when the file changes; `--vault`
//! overrides the vault path.
//!
//! Threads: the vault scan, the vault watcher, the config watcher and the layout engine run in the
//! background and post results to the main thread. The main thread redraws the scene
//! only when something changed, into a new IOSurface shown as a plain layer's contents
//! (`overlay::SceneLayer`), then frees the full-resolution intermediate textures; glow
//! breathing is a Core Animation opacity animation on its own layer. Saving a note can start a short edit burst (Ink & Paper's ink bleed,
//! Pulse's travelling light): baked once into a small atlas and played by Core
//! Animation over the still scene, with no app frames; when it ends its layers are
//! removed. A settled, still wallpaper draws nothing at all.

mod bookmark;
mod desktop;
mod displays;
mod import;
mod menu;
mod overlay;
mod power;
mod presence;
mod settings;
mod system;
mod updater;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use vw_core::config::{self, Config, ConfigWatcher};
use vw_core::layout::{LayoutEngine, LayoutFrame, LayoutGraph};
use vw_core::vault::{Graph, Vault, VaultWatcher, watch};
use displays::Geom;
use menu::{Action, AppMenu};
use power::PowerState;
use system::SysEvent;
use overlay::{BurstOverlay, GlowOverlay, SCENE_LAYER_FORMAT, SceneLayer, SceneTarget};
use presence::Presence;
use settings::SettingsWindow;
use tray_icon::menu::MenuEvent;
use vw_render::{
    Ambient, EditBurst, FramingOptions, Gpu, Look, LookContext, SceneStyle, Theme, WallpaperRenderer, derive_look, preferred_format,
    render_rgba, write_png,
};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalSize};
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

/// While the user is away, how often visibility is re-checked in case the
/// notification that they're back was missed.
const AWAY_CHECK: Duration = Duration::from_secs(60);

/// How often a missing vault (an unmounted drive, a folder not synced yet) is looked
/// for again, besides each time a volume mounts.
const VAULT_RETRY: Duration = Duration::from_secs(60);

enum AppEvent {
    /// A vault edit, from the watcher of vault open number `.0`.
    Graph(u64, Box<Graph>),
    /// Vault open number `generation` (scan and watch) finished in the background.
    Opened { generation: u64, path: PathBuf, result: Result<Box<OpenedVault>, VaultProblem> },
    Layout(Box<LayoutFrame>),
    Config(Box<Config>),
    Menu(MenuEvent),
    System(SysEvent),
    /// The Obsidian plugin's state file changed (`None`: gone or unreadable).
    Presence(Option<Box<vw_core::presence::PresenceState>>),
}

/// A vault scanned and watched off the main thread.
struct OpenedVault {
    graph: Graph,
    notes: usize,
    scan_ms: f64,
    watcher: Option<VaultWatcher>,
}

/// Why the vault couldn't be shown.
#[derive(Debug, Clone, PartialEq)]
enum VaultProblem {
    /// The folder isn't there (moved, renamed, an unmounted drive).
    NotFound,
    /// It is there but can't be listed.
    Unreadable(String),
    /// Sandboxed build: no working permission for it (a hand-edited `vaultPath`).
    NoAccess,
}

impl VaultProblem {
    /// Whether the vault folder can be listed (a cheap check before a full scan).
    fn check(path: &std::path::Path) -> Result<(), VaultProblem> {
        match std::fs::read_dir(path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(VaultProblem::NotFound),
            Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => Err(VaultProblem::Unreadable("not a folder".into())),
            Err(e) => Err(VaultProblem::Unreadable(e.to_string())),
        }
    }

    /// The menu's status line for this problem.
    fn status(&self, path: &std::path::Path) -> String {
        match self {
            VaultProblem::NotFound => format!("Vault not found: {}", short_path(path)),
            VaultProblem::Unreadable(_) => format!("Vault can't be read: {}", short_path(path)),
            VaultProblem::NoAccess => format!("Choose Vault… to allow access to {}", short_path(path)),
        }
    }
}

/// A path short enough for a menu line: `~` for the home folder, and only the last
/// two folders of a long path.
fn short_path(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let (prefix, rest) = match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => ("~/", rest),
        None => ("/", path.strip_prefix("/").unwrap_or(path)),
    };
    let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    if parts.len() > 2 {
        format!("…/{}", parts[parts.len() - 2..].join("/"))
    } else {
        format!("{prefix}{}", parts.join("/"))
    }
}

/// Why a display didn't get its wallpaper window.
#[derive(Debug)]
enum ScreenError {
    Window(String),
    Surface(String),
    /// No Metal device (or none that can draw to the window).
    Gpu,
}

impl std::fmt::Display for ScreenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScreenError::Window(e) => write!(f, "could not create the window: {e}"),
            ScreenError::Surface(e) => write!(f, "could not draw into the window: {e}"),
            ScreenError::Gpu => f.write_str("no usable GPU"),
        }
    }
}

impl ScreenError {
    /// What the alert says when this leaves the app with nothing to draw on.
    fn message(&self) -> &'static str {
        match self {
            ScreenError::Gpu | ScreenError::Surface(_) => "Vault Wallpaper needs a Mac with Metal graphics.",
            ScreenError::Window(_) => "Vault Wallpaper couldn't open its wallpaper window.",
        }
    }

    fn exit(&self) -> ! {
        desktop::fatal(self.message(), &self.to_string())
    }
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

const USAGE: &str = "usage: vault-wallpaper [--vault <path>] [--config <file>] [--front] [--cache <file>]";

fn parse_args() -> Args {
    let defaults = Args {
        vault: std::env::var_os("VW_VAULT").map(PathBuf::from),
        config: config::default_path().unwrap_or_else(|| support_dir().join("config.json")),
        front: false,
        cache: support_dir().join("layout.json"),
    };
    match parse_args_from(defaults, std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            std::process::exit(2);
        }
    }
}

/// Applies the command line to `args`. A flag missing its value is a usage error.
fn parse_args_from(mut args: Args, argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut it = argv.into_iter();
    let value = |flag: &str, it: &mut dyn Iterator<Item = String>| match it.next() {
        Some(v) if !v.is_empty() && !v.starts_with("--") => Ok(PathBuf::from(v)),
        _ => Err(format!("{flag} needs a path")),
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--vault" => args.vault = Some(value("--vault", &mut it)?),
            "--config" => args.config = value("--config", &mut it)?,
            "--front" => args.front = true,
            "--cache" => args.cache = value("--cache", &mut it)?,
            other => eprintln!("unknown argument: {other}"),
        }
    }
    Ok(args)
}

/// Checks, scans and watches the vault at `path` (on a background thread). The
/// watcher's edits are tagged with `generation`.
fn open_in_background(
    path: &std::path::Path,
    opts: vw_core::vault::VaultOptions,
    generation: u64,
    proxy: &EventLoopProxy<AppEvent>,
) -> Result<Box<OpenedVault>, VaultProblem> {
    VaultProblem::check(path)?;
    let t = Instant::now();
    let vault = Vault::scan(path, opts);
    let scan_ms = t.elapsed().as_secs_f64() * 1000.0;
    if vault.is_empty() {
        // Gone between the check and the scan.
        VaultProblem::check(path)?;
    }
    let notes = vault.len();
    let graph = vault.graph();
    let proxy = proxy.clone();
    let watcher = match watch(vault, move |g| {
        let _ = proxy.send_event(AppEvent::Graph(generation, Box::new(g)));
    }) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("not watching the vault: {e}");
            None
        }
    };
    Ok(Box::new(OpenedVault { graph, notes, scan_ms, watcher }))
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

/// Where a screen's composite goes.
enum Output {
    Layer(SceneTarget),
    Frame(wgpu::SurfaceTexture),
}

/// One wallpaper window (one per display).
struct Screen {
    /// The display this window covers.
    geom: Geom,
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
    /// Edit bursts' layers, made on the first burst this screen shows.
    burst_overlay: Option<BurstOverlay>,
    /// The composited scene as a plain layer's contents; `None` falls back to
    /// presenting on `surface`.
    scene_layer: Option<SceneLayer>,
    /// The running bursts must be (re)baked and started on this screen (bursts
    /// started, or the scene moved under them).
    bursts_dirty: bool,
}

impl Screen {
    fn check_visible(&mut self) -> bool {
        let visible = self.ns_window.as_deref().is_none_or(desktop::is_visible);
        if visible != self.visible {
            eprintln!("display {} visible={visible}", self.geom.id);
        }
        self.visible = visible;
        visible
    }

    /// Matches the surface and overlays to a new window size (pixels).
    fn resize(&mut self, gpu: &Gpu, size: PhysicalSize<u32>) {
        let (w, h) = (size.width.max(1), size.height.max(1));
        if (w, h) != (self.config.width, self.config.height) {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&gpu.device, &self.config);
        }
        if let (Some(l), Some(win)) = (&self.scene_layer, &self.ns_window) {
            l.fit(win);
        }
        if let (Some(o), Some(win)) = (&self.overlay, &self.ns_window) {
            o.fit(win);
        }
        if let (Some(o), Some(win)) = (&self.burst_overlay, &self.ns_window) {
            o.fit(win);
        }
        // Running bursts are baked again for the new size with the next scene.
        self.bursts_dirty |= self.burst_overlay.is_some();
        self.scene_dirty = true;
        self.frame_dirty = true;
    }

    /// The screen's rectangle in points, for framing: its place on the desktop and
    /// its drawable size.
    fn rect(&self) -> [f32; 4] {
        let scale = self.window.scale_factor() as f32;
        [self.geom.x as f32, self.geom.y as f32, self.config.width as f32 / scale, self.config.height as f32 / scale]
    }
}

/// The graph a layout frame belongs to, with its look.
struct Shown {
    /// The layout generation of `graph` (frames of one generation share the graph).
    generation: u64,
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
    /// Counts vault opens; results and edits from an older open are dropped.
    vault_gen: u64,
    /// The vault couldn't be shown: when to look for it again.
    vault_retry: Option<Instant>,
    /// The menu's status line says something about the vault (cleared once it loads).
    vault_status: bool,
    _config_watcher: Option<ConfigWatcher>,
    started: Instant,
    /// Low Power Mode and the power source, kept current by notifications.
    power: PowerState,
    /// The user can't see the screens (asleep, locked, switched out): draw nothing.
    away: bool,
    /// Span mode: one picture across every display (`VW_SPAN=1`).
    span: bool,
    /// The union of every display, in points, while spanning.
    span_rect: Option<[f32; 4]>,
    _system: Option<system::Observers>,
    /// While away, when to look at visibility again in case a wake was missed.
    away_check: Instant,
    menu: Option<AppMenu>,
    /// Sparkle, in the direct-download build only (`updater` feature, framework embedded).
    updater: Option<updater::Updater>,
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
    /// Atlas bytes baked since the bursts last settled (for the log).
    burst_bytes: usize,
    /// What the Obsidian plugin says is open; inert without the plugin.
    presence: Presence,
    /// Sandboxed builds only: the vault's security-scoped access, held while it is
    /// scanned and watched. Last, so it is given back after the watchers stop.
    vault_access: Option<bookmark::Access>,
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
        self.open_vault_or_ask(true);
    }

    /// [`open_vault`](Self::open_vault). In the sandbox a vault without a working
    /// bookmark (a hand-edited or imported `vaultPath`, or a moved folder) can't be read;
    /// with `ask`, the folder picker opens on it so one click grants access again.
    fn open_vault_or_ask(&mut self, ask: bool) {
        self._vault_watcher = None;
        self.presence = Presence::default();
        self.vault_retry = None;
        self.vault_gen += 1;
        let Some(path) = self.vault_path() else {
            eprintln!("no vault set: pass --vault or set vaultPath in {}", self.args.config.display());
            self.vault_access = None;
            return;
        };
        if bookmark::sandboxed() {
            match bookmark::open(&bookmark::file_for(&self.args.config), &path) {
                Ok(access) => self.vault_access = Some(access),
                Err(e) => {
                    eprintln!("vault access: {e}");
                    self.vault_access = None;
                    if ask {
                        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                        self.choose_vault(Some(&path), &format!("Allow Vault Wallpaper to read “{name}”: select it and click Open"));
                    } else {
                        // Not from a background reload: the user picks it from the menu.
                        self.vault_problem(&path, VaultProblem::NoAccess);
                    }
                    return;
                }
            }
        }
        // Scanning a big vault and arming its watcher take a while: off the main
        // thread, so the menu stays responsive. The graph comes back as an event.
        let generation = self.vault_gen;
        let opts = self.cfg.vault_options();
        let proxy = self.proxy.clone();
        let spawned = std::thread::Builder::new().name("vault-open".into()).spawn(move || {
            let result = open_in_background(&path, opts, generation, &proxy);
            let _ = proxy.send_event(AppEvent::Opened { generation, path, result });
        });
        if let Err(e) = spawned {
            eprintln!("could not start the vault scan: {e}");
        }
    }

    /// The background open's result.
    fn on_opened(&mut self, generation: u64, path: PathBuf, result: Result<Box<OpenedVault>, VaultProblem>) {
        if generation != self.vault_gen {
            return;
        }
        match result {
            Ok(opened) => {
                let OpenedVault { graph, notes, scan_ms, watcher } = *opened;
                eprintln!("[{:.0} ms] scanned {notes} notes in {scan_ms:.0} ms", self.ms());
                self._vault_watcher = watcher;
                self.vault_retry = None;
                if self.vault_status {
                    self.vault_status = false;
                    if let Some(m) = &self.menu {
                        m.set_status("Vault Wallpaper");
                    }
                }
                self.send_graph(graph);
                let proxy = self.proxy.clone();
                self.presence = Presence::start(&path, move |s| {
                    let _ = proxy.send_event(AppEvent::Presence(s.map(Box::new)));
                });
            }
            Err(problem) => self.vault_problem(&path, problem),
        }
    }

    /// Says on the menu why the vault isn't shown, and looks for it again later (and
    /// whenever a volume mounts). The last picture stays up meanwhile.
    fn vault_problem(&mut self, path: &std::path::Path, problem: VaultProblem) {
        let status = problem.status(path);
        eprintln!("[{:.0} ms] {status} ({problem:?}); retrying every {} s and on mount", self.ms(), VAULT_RETRY.as_secs());
        self._vault_watcher = None;
        self.presence = Presence::default();
        if let Some(m) = &self.menu {
            m.set_status(&status);
        }
        self.vault_status = true;
        self.vault_retry = Some(Instant::now() + VAULT_RETRY);
    }

    /// The vault is missing and a volume mounted, or the retry timer is due.
    fn retry_vault(&mut self) {
        if self.vault_retry.is_some() {
            eprintln!("[{:.0} ms] looking for the vault again", self.ms());
            self.open_vault_or_ask(false);
        }
    }

    /// A graph from the vault watcher: an edit. Notes it saved are remembered, and
    /// their bursts start once the layout frame that shows this graph arrives.
    fn on_graph(&mut self, generation: u64, graph: Graph) {
        if generation != self.vault_gen {
            return;
        }
        if graph.nodes.is_empty()
            && let Some(path) = self.vault_path()
            && let Err(problem) = VaultProblem::check(&path)
        {
            // Unmounted or moved away while watched: keep the last picture and say so.
            self.vault_problem(&path, problem);
            return;
        }
        eprintln!("[{:.0} ms] vault changed: {} notes", self.ms(), graph.nodes.len());
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
        if self.sent[0].1.len() != frame.positions.len() {
            return;
        }
        if frame.settled {
            eprintln!("[{:.0} ms] layout settled", self.ms());
        }
        self.shown = Some(match self.shown.take() {
            // Another frame of the graph on screen: only the positions move.
            Some(s) if s.generation == frame.generation => Shown { positions: frame.positions, ..s },
            old => {
                let (graph, layout) = (self.sent[0].0.clone(), self.sent[0].1.clone());
                // The look only depends on the graph and config: reuse it while the
                // notes and links are the same.
                let look = match old {
                    Some(s) if s.layout.id == layout.id && s.layout.links == layout.links => s.look,
                    _ => self.look_for(&graph, &layout),
                };
                Shown { generation: frame.generation, graph, layout, look, positions: frame.positions }
            }
        });
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
        if ids.is_empty() || self.edit_burst.is_none() || !self.motion_allowed() {
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
        for s in &mut self.screens {
            s.bursts_dirty = true;
            if s.check_visible() {
                s.window.request_redraw();
            }
        }
    }

    /// Ends every burst at once (pause, theme or motion change).
    fn stop_bursts(&mut self) {
        self.bursts.clear();
        self.pending_edits.clear();
        for s in &mut self.screens {
            s.bursts_dirty = false;
            if let Some(o) = &mut s.burst_overlay {
                o.clear();
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
            // A background reload never pops the folder picker; the menu says what to do.
            self.open_vault_or_ask(false);
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

    /// Whether anything may move now: not paused, nobody away, and power allows it
    /// (Low Power Mode never; battery only in Showcase).
    fn motion_allowed(&self) -> bool {
        !self.paused && !self.away && self.power.motion_allowed(self.cfg.motion_mode)
    }

    /// Breathing as Core Animation should run it now: off when motion isn't allowed,
    /// or when the style doesn't breathe.
    fn breathing(&self) -> Option<(f32, f32, f32)> {
        if !self.motion_allowed() {
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

    /// Opens the wallpaper window for one display. The first window also picks the GPU.
    fn add_screen(&mut self, event_loop: &ActiveEventLoop, geom: Geom) -> Result<(), ScreenError> {
        let attrs = Window::default_attributes()
            .with_title("Vault Wallpaper")
            .with_decorations(false)
            .with_resizable(false)
            .with_position(LogicalPosition::new(geom.x, geom.y))
            .with_inner_size(LogicalSize::new(geom.w, geom.h))
            .with_has_shadow(false)
            .with_active(false);
        let window = Arc::new(event_loop.create_window(attrs).map_err(|e| ScreenError::Window(e.to_string()))?);
        let size = window.inner_size();
        let ns_window = desktop::ns_window(&window);
        if let (Some(w), false) = (&ns_window, self.args.front) {
            desktop::place_on_desktop(w);
        }
        let instance = match &self.gpu {
            Some(g) => g.instance.clone(),
            None => wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle()),
        };
        let surface = instance.create_surface(window.clone()).map_err(|e| ScreenError::Surface(e.to_string()))?;
        if self.gpu.is_none() {
            // Test aid: VW_FAIL_GPU=1 takes the no-GPU path on any Mac.
            let gpu = if std::env::var_os("VW_FAIL_GPU").is_some() { None } else { Gpu::new(Some(&surface), instance) };
            self.gpu = Some(gpu.ok_or(ScreenError::Gpu)?);
        }
        let Some(gpu) = self.gpu.as_ref() else { return Err(ScreenError::Gpu) };
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
        // The scene layer goes in first, under the glow and burst layers. The wgpu
        // surface stays configured (it keeps the view layer-backed, and is the
        // fallback) but presents nothing, so it never allocates a drawable.
        let scene_layer = (format == SCENE_LAYER_FORMAT)
            .then(|| ns_window.as_deref().and_then(SceneLayer::new))
            .flatten();
        let renderer = WallpaperRenderer::new(gpu, format);
        let overlay = ns_window.as_deref().and_then(|w| GlowOverlay::new(gpu, w, format));
        if overlay.is_none() {
            eprintln!("glow overlay unavailable; glow will not breathe");
        }
        self.screens.push(Screen {
            geom,
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
            scene_layer,
            bursts_dirty: false,
        });
        Ok(())
    }

    /// [`add_screen`](Self::add_screen), where a failure is logged and the app carries
    /// on with the screens it has. Without a GPU there is nothing to carry on with.
    fn try_add_screen(&mut self, event_loop: &ActiveEventLoop, geom: Geom) {
        if let Err(e) = self.add_screen(event_loop, geom) {
            eprintln!("[{:.0} ms] display {}: {e}", self.ms(), geom.id);
            if self.gpu.is_none() {
                e.exit();
            }
        }
    }

    fn draw(&mut self, id: WindowId) {
        if self.away {
            // Nobody can see it; screens stay dirty and draw when the user is back.
            return;
        }
        let breathing = self.breathing();
        let span = self.span_rect;
        let Some(gpu) = &self.gpu else { return };
        let Some(s) = self.screens.iter_mut().find(|s| s.window.id() == id) else { return };
        if !(s.frame_dirty || s.bursts_dirty) || !s.check_visible() {
            // Nothing new, or hidden: a hidden screen stays dirty and draws when seen.
            return;
        }
        if s.frame_dirty {
            Self::draw_scene(gpu, s, self.shown.as_ref(), self.theme, &self.style, displays::Fit { opts: &self.framing, span }, breathing);
        }
        if s.bursts_dirty {
            s.bursts_dirty = false;
            if let (Some(burst), Some(shown)) = (self.edit_burst, &self.shown) {
                self.burst_bytes += Self::play_bursts(gpu, s, shown, self.theme, &self.bursts, displays::Fit { opts: &self.framing, span }, burst);
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
        fit: displays::Fit,
        breathing: Option<(f32, f32, f32)>,
    ) {
        let t0 = Instant::now();
        let size = [s.config.width, s.config.height];
        // Where the composite goes: a fresh surface for the scene layer, or else the
        // window's wgpu surface.
        let target = match &s.scene_layer {
            Some(layer) => match layer.target(gpu, size) {
                Some(t) => Output::Layer(t),
                None => {
                    eprintln!("scene layer: no surface");
                    return;
                }
            },
            None => match s.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => Output::Frame(f),
                wgpu::CurrentSurfaceTexture::Occluded => {
                    s.visible = false;
                    return;
                }
                other => {
                    eprintln!("surface: {other:?}");
                    s.surface.configure(&gpu.device, &s.config);
                    return;
                }
            },
        };
        let scale = s.window.scale_factor() as f32;
        if s.scene_dirty || !s.renderer.has_targets() {
            let points = [size[0] as f32 / scale, size[1] as f32 / scale];
            let data = match shown {
                Some(sh) => {
                    let framing = fit.framing(&sh.positions, &sh.layout.degree, s.rect());
                    theme.build(&sh.layout, &sh.look, &sh.positions, &framing, style, points)
                }
                None => Default::default(),
            };
            s.renderer.set_scene(gpu, size, scale, &data, style);
            s.scene_dirty = false;
            // Notes may have moved under running bursts.
            s.bursts_dirty |= s.burst_overlay.is_some();
        }
        let view = match &target {
            Output::Layer(t) => t.texture.create_view(&Default::default()),
            Output::Frame(f) => f.texture.create_view(&Default::default()),
        };
        match &mut s.overlay {
            Some(o) => {
                s.renderer.composite_scene_only(gpu, &view, style);
                o.draw(gpu, &s.renderer, style);
                o.set_breathing(breathing);
            }
            None => s.renderer.composite(gpu, &view, style, 1.0),
        }
        drop(view);
        match target {
            Output::Layer(t) => {
                if let Some(layer) = &mut s.scene_layer {
                    layer.show(gpu, t);
                }
                // The composite only ever follows a scene redraw, so the cached
                // full-resolution textures (~30 MB on Retina) are not kept between
                // redraws; the driver gives their memory back when it goes idle.
                s.renderer.release_targets();
            }
            Output::Frame(f) => {
                s.window.pre_present_notify();
                gpu.queue.present(f);
            }
        }
        eprintln!("scene drawn in {:.1} ms", t0.elapsed().as_secs_f64() * 1000.0);
        s.frame_dirty = false;
    }

    /// Bakes the running bursts for this screen and hands them to Core Animation,
    /// each `elapsed` into its run. The scene is not touched, and no frame is
    /// presented. Returns the atlas bytes baked.
    fn play_bursts(
        gpu: &Gpu,
        s: &mut Screen,
        shown: &Shown,
        theme: Theme,
        bursts: &[(String, Instant)],
        fit: displays::Fit,
        burst: EditBurst,
    ) -> usize {
        let Some(ns_window) = &s.ns_window else { return 0 };
        if bursts.is_empty() {
            if let Some(o) = &mut s.burst_overlay {
                o.clear();
            }
            return 0;
        }
        if s.burst_overlay.is_none() {
            s.burst_overlay = BurstOverlay::new(gpu, ns_window);
        }
        let scale = s.window.scale_factor() as f32;
        // Framed per screen exactly as its scene is (span mode included).
        let framing = fit.framing(&shown.positions, &shown.layout.degree, s.rect());
        let Some(o) = &mut s.burst_overlay else { return 0 };
        o.retain(|id| bursts.iter().any(|(b, _)| b == id));
        let now = Instant::now();
        let mut bytes = 0;
        for (id, start) in bursts {
            let Some(i) = shown.layout.id.iter().position(|x| x == id) else {
                o.remove(id);
                continue;
            };
            let actors = theme.burst_actors(&shown.layout, &shown.look, &shown.positions, &framing, i, &burst);
            bytes += o.play(gpu, id, &actors, &burst, now.duration_since(*start).as_secs_f32(), scale);
        }
        bytes
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
            Action::ChooseVault => self.choose_vault(None, "Choose your Obsidian vault"),
            Action::ImportWebConfig => {
                // A menu-bar app isn't frontmost: bring the dialogs forward.
                desktop::activate();
                // On success the config watcher picks the new file up, restyles and rescans.
                if import::run(&self.args.config, &self.cfg) {
                    self.args.vault = None;
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
            Action::CheckForUpdates => {
                if let Some(u) = &self.updater {
                    // A menu-bar app isn't frontmost: bring Sparkle's window forward.
                    desktop::activate();
                    u.check_now();
                }
            }
            Action::Quit => event_loop.exit(),
        }
    }

    /// The folder picker. A picked folder becomes the vault: saved in the config (the
    /// config watcher then rescans) and, in the sandbox, bookmarked so it can be read
    /// again after a relaunch.
    fn choose_vault(&mut self, start: Option<&std::path::Path>, title: &str) {
        desktop::activate();
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        let Some(dir) = dialog.pick_folder() else { return };
        if bookmark::sandboxed()
            && let Err(e) = bookmark::save(&bookmark::file_for(&self.args.config), &dir)
        {
            eprintln!("vault bookmark: {e}");
        }
        let same = self.vault_path().as_deref() == Some(dir.as_path());
        let mut cfg = self.cfg.clone();
        if !self.args.config.exists() {
            // A new install starts still (lightness first); motion is opt-in.
            cfg.motion_mode = config::MotionMode::Still;
        }
        cfg.vault_path = Some(dir.to_string_lossy().into_owned());
        self.args.vault = None;
        if let Err(e) = config::save(&self.args.config, &cfg) {
            eprintln!("could not save the config: {e}");
        }
        if same {
            // The config doesn't change, so the watcher won't rescan: the folder was
            // picked again to grant access.
            self.open_vault_or_ask(false);
        }
    }

    /// Removes finished bursts' layers. Core Animation plays the bursts; the app
    /// only wakes when one ends. Returns that time; `None` once all have settled.
    fn step_bursts(&mut self, now: Instant) -> Option<Instant> {
        let burst = self.edit_burst?;
        if self.bursts.is_empty() {
            return None;
        }
        let duration = Duration::from_secs_f32(burst.duration);
        let before = self.bursts.len();
        self.bursts.retain(|(_, start)| now.duration_since(*start) < duration);
        if self.bursts.is_empty() {
            // Settled: the still scene underneath is the final state.
            eprintln!(
                "[{:.0} ms] edit bursts settled ({} KB baked, no frames presented)",
                self.ms(),
                self.burst_bytes.div_ceil(1024)
            );
            self.burst_bytes = 0;
            self.stop_bursts();
            return None;
        }
        if self.bursts.len() != before {
            let running = &self.bursts;
            for s in &mut self.screens {
                if let Some(o) = &mut s.burst_overlay {
                    o.retain(|id| running.iter().any(|(b, _)| b == id));
                }
            }
        }
        self.bursts.iter().map(|(_, start)| *start + duration).min()
    }

    /// Span mode's rectangle: the union of every display, or `None` when not spanning.
    fn update_span(&mut self) {
        self.span_rect = if self.span { displays::union(self.screens.iter().map(|s| s.geom.rect())) } else { None };
    }

    /// Makes the windows match the attached displays: new ones get a window, gone
    /// ones lose theirs, moved or resized ones follow.
    fn sync_displays(&mut self, event_loop: &ActiveEventLoop) {
        let now = displays::monitors(event_loop);
        let old: Vec<Geom> = self.screens.iter().map(|s| s.geom).collect();
        let changes = displays::diff(&old, &now);
        if changes.is_empty() {
            return;
        }
        for change in changes {
            eprintln!("[{:.0} ms] display change: {change:?}", self.ms());
            match change {
                displays::Change::Remove(id) => self.screens.retain(|s| s.geom.id != id),
                displays::Change::Add(geom) => self.try_add_screen(event_loop, geom),
                displays::Change::Update(geom) => {
                    let Some(gpu) = &self.gpu else { continue };
                    let Some(s) = self.screens.iter_mut().find(|s| s.geom.id == geom.id) else { continue };
                    s.geom = geom;
                    s.window.set_outer_position(LogicalPosition::new(geom.x, geom.y));
                    let _ = s.window.request_inner_size(LogicalSize::new(geom.w, geom.h));
                    s.resize(gpu, s.window.inner_size());
                }
            }
        }
        self.update_span();
        self.update_breathing();
        // Span framing depends on every display; redraw them all.
        for s in &mut self.screens {
            s.scene_dirty = true;
            s.frame_dirty = true;
            s.bursts_dirty |= s.burst_overlay.is_some();
        }
        self.invalidate_scene();
    }

    fn on_system(&mut self, event: SysEvent, event_loop: &ActiveEventLoop) {
        match event {
            SysEvent::Displays => self.sync_displays(event_loop),
            SysEvent::Away => {
                if !self.away {
                    eprintln!("[{:.0} ms] away: drawing nothing", self.ms());
                    self.away = true;
                    self.away_check = Instant::now() + AWAY_CHECK;
                    self.stop_bursts();
                    self.update_breathing();
                }
            }
            SysEvent::Back => {
                if self.away {
                    eprintln!("[{:.0} ms] back", self.ms());
                }
                self.away = false;
                self.power = PowerState::read();
                // Displays may have changed while asleep, and recency colours age.
                self.sync_displays(event_loop);
                if let Some(s) = self.shown.take() {
                    let look = self.look_for(&s.graph, &s.layout);
                    let changed = look != s.look;
                    self.shown = Some(Shown { look, ..s });
                    if changed {
                        eprintln!("[{:.0} ms] look aged while away; redrawing", self.ms());
                        self.invalidate_scene();
                    }
                }
                self.update_breathing();
                self.recheck_visibility();
            }
            SysEvent::Space => {
                eprintln!("[{:.0} ms] space changed", self.ms());
                self.recheck_visibility();
            }
            SysEvent::Mounts => {
                if self.vault_retry.is_some() {
                    self.retry_vault();
                } else if let Some(path) = self.vault_path()
                    && self._vault_watcher.is_some()
                    && let Err(problem) = VaultProblem::check(&path)
                {
                    self.vault_problem(&path, problem);
                }
            }
            SysEvent::Power => {
                let power = PowerState::read();
                if power != self.power {
                    eprintln!("[{:.0} ms] power: low power mode {}, on battery {}", self.ms(), power.low_power, power.on_battery);
                    self.power = power;
                    if !self.motion_allowed() {
                        self.stop_bursts();
                    }
                    self.update_breathing();
                }
            }
        }
    }

    /// Looks at every screen's visibility and draws the ones that are now visible
    /// and out of date.
    fn recheck_visibility(&mut self) {
        for s in &mut self.screens {
            if s.check_visible() && s.frame_dirty {
                s.window.request_redraw();
            }
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
        let framing = displays::screen_framing(&sh.positions, &sh.layout.degree, &self.framing, screen.rect(), self.span_rect);
        let data = self.theme.build(&sh.layout, &sh.look, &sh.positions, &framing, &self.style, points);
        let rgba = render_rgba(gpu, size, scale, &self.style, &data, 1.0);
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        // In the sandbox HOME is the app's container, whose Pictures is a link to the
        // real ~/Pictures (allowed by the pictures entitlement); resolve it so Finder
        // shows the real folder.
        let dir = home.join("Pictures/Vault Wallpaper");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let dir = dir.canonicalize().unwrap_or(dir);
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
        let mut failed = None;
        for geom in displays::monitors(event_loop) {
            if let Err(e) = self.add_screen(event_loop, geom) {
                eprintln!("[{:.0} ms] display {}: {e}", self.ms(), geom.id);
                if self.gpu.is_none() {
                    e.exit();
                }
                failed = Some(e);
            }
        }
        if let (Some(e), true) = (failed, self.screens.is_empty()) {
            // Displays, but not one window on them.
            e.exit();
        }
        self.update_span();
        eprintln!(
            "[{:.0} ms] {} screen(s), low power mode {}, on battery {}, span {}",
            self.ms(),
            self.screens.len(),
            self.power.low_power,
            self.power.on_battery,
            self.span
        );
        let proxy = self.proxy.clone();
        self._system = Some(system::observe(move |e| {
            let _ = proxy.send_event(AppEvent::System(e));
        }));
        if std::env::var_os("VW_WINDOW_IDS").is_some() {
            for s in &self.screens {
                if let Some(w) = &s.ns_window {
                    eprintln!("window id {}", w.windowNumber());
                }
            }
        }
        let proxy = self.proxy.clone();
        self.updater = updater::Updater::start();
        self.menu = AppMenu::new(desktop::login::enabled(), self.updater.is_some(), move |e| {
            let _ = proxy.send_event(AppEvent::Menu(e));
        });
        if self.menu.is_none() {
            eprintln!("menu bar icon could not be created");
        }
        // Test aid: run the menu's Check for Updates… at launch (scripts can't click a
        // status-item menu). Inert without the updater.
        if std::env::var_os("VW_CHECK_FOR_UPDATES").is_some() {
            self.on_action(Action::CheckForUpdates, event_loop);
        }
        self.start_background();
        if self.vault_path().is_none() {
            // First run: nothing to show until a vault is picked.
            self.on_action(Action::ChooseVault, event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Graph(generation, g) => self.on_graph(generation, *g),
            AppEvent::Opened { generation, path, result } => self.on_opened(generation, path, result),
            AppEvent::Layout(f) => self.on_layout(*f),
            AppEvent::Config(c) => self.on_config(*c),
            AppEvent::Presence(s) => self.on_presence(s.map(|s| *s)),
            AppEvent::Menu(e) => {
                if let Some(action) = self.menu.as_ref().and_then(|m| m.action(&e)) {
                    self.on_action(action, event_loop);
                }
            }
            AppEvent::System(e) => self.on_system(e, event_loop),
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
                    s.resize(gpu, size);
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
        // While away (asleep, locked) the notification that ends it does the work;
        // a slow check covers a missed one. Power changes arrive as notifications, so
        // nothing else wakes a still wallpaper.
        let mut wake: Option<Instant> = None;
        if self.away {
            if now >= self.away_check {
                self.away_check = now + AWAY_CHECK;
                if self.screens.iter_mut().any(|s| s.check_visible()) {
                    eprintln!("[{:.0} ms] a screen is visible while away; resuming", self.ms());
                    self.on_system(SysEvent::Back, event_loop);
                }
            }
            wake = Some(self.away_check);
        } else {
            for s in &mut self.screens {
                if s.frame_dirty && !s.visible {
                    wake = Some(now + Duration::from_secs(1));
                    if s.check_visible() {
                        s.window.request_redraw();
                    }
                }
            }
        }
        if let Some(at) = self.vault_retry {
            if now >= at {
                self.retry_vault();
            }
            if let Some(at) = self.vault_retry {
                wake = Some(wake.map_or(at, |w| w.min(at)));
            }
        }
        if let Some(next) = self.step_presence() {
            wake = Some(wake.map_or(next, |w| w.min(next)));
        }
        if !self.away && let Some(next) = self.step_bursts(now) {
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
        .unwrap_or_else(|e| desktop::fatal("Vault Wallpaper couldn't start.", &e.to_string()));
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
        vault_gen: 0,
        vault_retry: None,
        vault_status: false,
        _config_watcher: None,
        started: Instant::now(),
        power: PowerState::read(),
        away: false,
        away_check: Instant::now(),
        span: displays::span_from_env(),
        span_rect: None,
        _system: None,
        menu: None,
        updater: None,
        settings: None,
        paused: false,
        edit_burst: None,
        pending_edits: Vec::new(),
        bursts: Vec::new(),
        burst_bytes: 0,
        presence: Presence::default(),
        vault_access: None,
    };
    app.edit_burst = app.theme.edit_burst(&app.cfg);
    if let Err(e) = event_loop.run_app(&mut app) {
        desktop::fatal("Vault Wallpaper stopped unexpectedly.", &e.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Args {
        Args { vault: None, config: "/c.json".into(), front: false, cache: "/l.json".into() }
    }

    fn parse(argv: &[&str]) -> Result<Args, String> {
        parse_args_from(defaults(), argv.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_flags() {
        let a = parse(&["--vault", "/v", "--front", "--config", "/x.json", "--cache", "/y.json"]).unwrap();
        assert_eq!(a.vault.as_deref(), Some(std::path::Path::new("/v")));
        assert!(a.front);
        assert_eq!(a.config, PathBuf::from("/x.json"));
        assert_eq!(a.cache, PathBuf::from("/y.json"));
        let a = parse(&[]).unwrap();
        assert_eq!((a.vault, a.config, a.front), (None, PathBuf::from("/c.json"), false));
    }

    #[test]
    fn a_flag_without_its_value_is_a_usage_error() {
        assert_eq!(parse(&["--config"]).err().as_deref(), Some("--config needs a path"));
        assert_eq!(parse(&["--cache"]).err().as_deref(), Some("--cache needs a path"));
        assert_eq!(parse(&["--vault"]).err().as_deref(), Some("--vault needs a path"));
        assert_eq!(parse(&["--config", "--front"]).err().as_deref(), Some("--config needs a path"));
        assert_eq!(parse(&["--cache", ""]).err().as_deref(), Some("--cache needs a path"));
    }

    #[test]
    fn short_paths_for_the_menu() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(short_path(&PathBuf::from(&home).join("Vault")), "~/Vault");
        assert_eq!(short_path(&PathBuf::from(&home).join("Documents/Vault")), "~/Documents/Vault");
        assert_eq!(short_path(&PathBuf::from(&home).join("Library/Mobile Documents/iCloud~md~obsidian/Documents/Notes")), "…/Documents/Notes");
        assert_eq!(short_path(std::path::Path::new("/Volumes/USB")), "/Volumes/USB");
        assert_eq!(short_path(std::path::Path::new("/Volumes/USB/Notes/Work")), "…/Notes/Work");
    }

    #[test]
    fn vault_problems() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(VaultProblem::check(dir.path()), Ok(()));
        let missing = dir.path().join("gone");
        assert_eq!(VaultProblem::check(&missing), Err(VaultProblem::NotFound));
        let file = dir.path().join("note.md");
        std::fs::write(&file, "").unwrap();
        assert!(matches!(VaultProblem::check(&file), Err(VaultProblem::Unreadable(_))));
        assert_eq!(VaultProblem::NotFound.status(std::path::Path::new("/Volumes/USB")), "Vault not found: /Volumes/USB");
    }

    #[test]
    fn no_gpu_says_metal() {
        assert_eq!(ScreenError::Gpu.message(), "Vault Wallpaper needs a Mac with Metal graphics.");
        assert_eq!(ScreenError::Surface("x".into()).message(), "Vault Wallpaper needs a Mac with Metal graphics.");
        assert!(ScreenError::Window("x".into()).message().contains("window"));
    }
}
