//! Vault Wallpaper: your Obsidian vault's graph, drawn behind the desktop icons.
//!
//!   vault-wallpaper --vault <path> [--front] [--cache <file>]
//!
//! Threads: the vault watcher and the layout engine run in the background and post
//! results to the main thread, which draws only when something changed. A settled,
//! unchanged wallpaper draws nothing at all.

mod desktop;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use vw_core::layout::{LayoutEngine, LayoutFrame, LayoutGraph, LayoutOptions};
use vw_core::vault::{Graph, Vault, VaultOptions, VaultWatcher, watch};
use vw_render::{Framing, FramingOptions, Gpu, GraphRenderer, SceneStyle, build_instances, preferred_format};
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
}

struct Args {
    vault: PathBuf,
    front: bool,
    cache: PathBuf,
}

fn parse_args() -> Args {
    let mut vault = None;
    let mut front = false;
    let mut cache = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--vault" => vault = it.next().map(PathBuf::from),
            "--front" => front = true,
            "--cache" => cache = it.next().map(PathBuf::from),
            other => eprintln!("unknown argument: {other}"),
        }
    }
    let Some(vault) = vault.or_else(|| std::env::var_os("VW_VAULT").map(PathBuf::from)) else {
        eprintln!("usage: vault-wallpaper --vault <path> [--front] [--cache <file>]");
        std::process::exit(2);
    };
    let cache = cache.unwrap_or_else(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join("Library/Application Support/Vault Wallpaper/layout.json")
    });
    Args { vault, front, cache }
}

/// One wallpaper window (one per display).
struct Screen {
    window: Arc<Window>,
    ns_window: Option<Retained<NSWindow>>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: GraphRenderer,
    /// Something changed since this screen last drew.
    dirty: bool,
    visible: bool,
}

impl Screen {
    fn check_visible(&mut self) -> bool {
        self.visible = self.ns_window.as_deref().is_none_or(desktop::is_visible);
        self.visible
    }
}

struct App {
    args: Args,
    proxy: EventLoopProxy<AppEvent>,
    gpu: Option<Gpu>,
    screens: Vec<Screen>,
    style: SceneStyle,
    framing: FramingOptions,
    /// Graphs sent to the layout engine, oldest first. The engine numbers its runs, so
    /// frame generation N belongs to the Nth graph sent.
    sent: Vec<LayoutGraph>,
    sent_base: u64,
    current: Option<(LayoutGraph, Vec<[f32; 2]>)>,
    engine: Option<LayoutEngine>,
    _watcher: Option<VaultWatcher>,
    started: Instant,
    frames: u64,
}

impl App {
    fn start_background(&mut self) {
        let proxy = self.proxy.clone();
        let engine = LayoutEngine::spawn(
            LayoutOptions::default(),
            Some(self.args.cache.clone()),
            Duration::from_secs_f64(1.0 / LAYOUT_FPS),
            move |f| {
                let _ = proxy.send_event(AppEvent::Layout(Box::new(f)));
            },
        );
        let t = Instant::now();
        let vault = Vault::scan(&self.args.vault, VaultOptions::default());
        eprintln!("[{:.0} ms] scanned {} notes in {:.0} ms", self.ms(), vault.len(), t.elapsed().as_secs_f64() * 1000.0);
        let graph = vault.graph();
        let proxy = self.proxy.clone();
        match watch(vault, move |g| {
            let _ = proxy.send_event(AppEvent::Graph(Box::new(g)));
        }) {
            Ok(w) => self._watcher = Some(w),
            Err(e) => eprintln!("not watching the vault: {e}"),
        }
        self.engine = Some(engine);
        self.send_graph(graph);
    }

    fn ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    fn send_graph(&mut self, graph: Graph) {
        self.sent.push(LayoutGraph::from_graph(&graph));
        if let Some(engine) = &self.engine {
            engine.set_graph(graph);
        }
    }

    fn on_layout(&mut self, frame: LayoutFrame) {
        // Drop graphs older than the one this frame was computed for.
        let Some(index) = (frame.generation.checked_sub(1 + self.sent_base)).map(|i| i as usize) else { return };
        if index >= self.sent.len() {
            return;
        }
        self.sent.drain(..index);
        self.sent_base += index as u64;
        let graph = &self.sent[0];
        if graph.len() != frame.positions.len() {
            return;
        }
        if frame.settled {
            eprintln!("[{:.0} ms] layout settled ({} frames drawn so far)", self.ms(), self.frames);
        }
        self.current = Some((graph.clone(), frame.positions));
        self.mark_dirty();
    }

    fn mark_dirty(&mut self) {
        for s in &mut self.screens {
            s.dirty = true;
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
        let renderer = GraphRenderer::new(gpu, format);
        self.screens.push(Screen { window, ns_window, surface, config, renderer, dirty: true, visible: true });
    }

    fn draw(&mut self, id: WindowId) {
        let Some(gpu) = &self.gpu else { return };
        let Some(s) = self.screens.iter_mut().find(|s| s.window.id() == id) else { return };
        if !s.dirty || !s.check_visible() {
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
        let (w, h) = (s.config.width, s.config.height);
        match &self.current {
            Some((graph, positions)) => {
                let framing = Framing::fit(positions, w as f32 / scale, h as f32 / scale, &self.framing);
                let (nodes, edges) = build_instances(graph, positions, &framing, &self.style);
                s.renderer.upload(gpu, &nodes, &edges);
            }
            None => s.renderer.upload(gpu, &[], &[]),
        }
        let view = frame.texture.create_view(&Default::default());
        s.renderer.render(gpu, &view, [w, h], scale, &self.style);
        s.window.pre_present_notify();
        gpu.queue.present(frame);
        s.dirty = false;
        self.frames += 1;
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
        self.start_background();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Graph(g) => self.send_graph(*g),
            AppEvent::Layout(f) => self.on_layout(*f),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => self.draw(id),
            WindowEvent::Occluded(_) => {
                if let Some(s) = self.screens.iter_mut().find(|s| s.window.id() == id)
                    && s.check_visible()
                    && s.dirty
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
                    s.dirty = true;
                    s.window.request_redraw();
                }
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Only a screen that is hidden *and* has something new to show needs watching:
        // check once a second (events alone can be missed). Otherwise sleep until the
        // next event.
        let mut waiting = false;
        for s in &mut self.screens {
            if s.dirty && !s.visible {
                waiting = true;
                if s.check_visible() {
                    s.window.request_redraw();
                }
            }
        }
        event_loop.set_control_flow(if waiting {
            ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(1))
        } else {
            ControlFlow::Wait
        });
    }
}

fn main() {
    let args = parse_args();
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .with_activation_policy(ActivationPolicy::Accessory)
        .build()
        .expect("event loop");
    let mut app = App {
        args,
        proxy: event_loop.create_proxy(),
        gpu: None,
        screens: Vec::new(),
        style: SceneStyle::default(),
        framing: FramingOptions::default(),
        sent: Vec::new(),
        sent_base: 0,
        current: None,
        engine: None,
        _watcher: None,
        started: Instant::now(),
        frames: 0,
    };
    event_loop.run_app(&mut app).expect("run");
}
