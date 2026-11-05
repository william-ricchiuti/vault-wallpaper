//! Phase 0 spike: can a Rust + wgpu app behave like a real macOS wallpaper?
//!
//! One borderless window per display, placed at the desktop window level (behind the
//! icons), on every Space, click-through. Draws a background gradient plus N glowing
//! instanced dots orbiting, at a capped frame rate, and stops drawing entirely while
//! the window is occluded.
//!
//!   cargo run --release -- [--fps 30] [--nodes 2500] [--still] [--front] [--seconds N]
//!
//! --front puts the window at the normal level instead (for measuring render cost while
//! guaranteed visible). --still draws one frame and then only redraws on resize.

use std::sync::Arc;
use std::time::{Duration, Instant};

use objc2_app_kit::{NSView, NSWindowCollectionBehavior};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS, WindowAttributesExtMacOS};
use winit::window::{Window, WindowId};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowLevelForKey(key: i32) -> i32;
}
const KCG_DESKTOP_WINDOW_LEVEL_KEY: i32 = 2;
const KCG_DESKTOP_ICON_WINDOW_LEVEL_KEY: i32 = 18;

struct Opts {
    fps: f64,
    nodes: u32,
    still: bool,
    front: bool,
    seconds: Option<f64>,
}

fn parse_opts() -> Opts {
    let mut o = Opts { fps: 30.0, nodes: 2500, still: false, front: false, seconds: None };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--fps" => o.fps = it.next().and_then(|v| v.parse().ok()).unwrap_or(30.0),
            "--nodes" => o.nodes = it.next().and_then(|v| v.parse().ok()).unwrap_or(2500),
            "--still" => o.still = true,
            "--front" => o.front = true,
            "--seconds" => o.seconds = it.next().and_then(|v| v.parse().ok()),
            other => eprintln!("unknown arg {other}"),
        }
    }
    o
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    resolution: [f32; 2],
    time: f32,
    nodes: f32,
}

const SHADER: &str = r#"
struct U { resolution: vec2<f32>, time: f32, nodes: f32 };
@group(0) @binding(0) var<uniform> u: U;

struct BgOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn bg_vs(@builtin(vertex_index) i: u32) -> BgOut {
  let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
  var o: BgOut; o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0); o.uv = p; return o;
}
@fragment fn bg_fs(in: BgOut) -> @location(0) vec4<f32> {
  let c = mix(vec3<f32>(0.02, 0.02, 0.05), vec3<f32>(0.07, 0.04, 0.12), in.uv.y);
  let d = distance(in.uv, vec2<f32>(0.5, 0.5));
  return vec4<f32>(c * (1.1 - d), 1.0);
}

fn hash(n: f32) -> f32 { return fract(sin(n) * 43758.5453); }

struct DotOut { @builtin(position) pos: vec4<f32>, @location(0) q: vec2<f32>, @location(1) col: vec3<f32> };
@vertex fn dot_vs(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> DotOut {
  let fi = f32(i);
  let r = sqrt(hash(fi * 1.7)) * 0.9;
  let a = hash(fi * 3.1) * 6.2831 + u.time * (0.02 + 0.05 * hash(fi * 5.3));
  let center = vec2<f32>(cos(a), sin(a)) * r;
  let aspect = u.resolution.x / u.resolution.y;
  let size = (2.0 + 6.0 * hash(fi * 7.9)) / u.resolution.y * 2.0 * 3.0;
  let corners = array<vec2<f32>, 6>(vec2(-1.,-1.), vec2(1.,-1.), vec2(-1.,1.), vec2(-1.,1.), vec2(1.,-1.), vec2(1.,1.));
  let q = corners[v];
  var o: DotOut;
  o.pos = vec4<f32>(center.x / aspect + q.x * size / aspect, center.y + q.y * size, 0.0, 1.0);
  o.q = q;
  o.col = mix(vec3<f32>(0.49, 0.36, 1.0), vec3<f32>(0.2, 0.9, 0.85), hash(fi * 11.3));
  return o;
}
@fragment fn dot_fs(in: DotOut) -> @location(0) vec4<f32> {
  let d = length(in.q);
  let core = smoothstep(0.35, 0.25, d);
  let glow = exp(-d * d * 6.0) * 0.5;
  let a = clamp(core + glow, 0.0, 1.0);
  return vec4<f32>(in.col * a, a);
}
"#;

struct Surface {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    occluded: bool,
    frames: u64,
}

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    bg_pipeline: Option<wgpu::RenderPipeline>,
    dot_pipeline: Option<wgpu::RenderPipeline>,
    layout: Option<wgpu::BindGroupLayout>,
}

struct App {
    opts: Opts,
    gpu: Option<Gpu>,
    surfaces: Vec<Surface>,
    start: Instant,
    next_frame: Instant,
    last_report: Instant,
}

fn place_on_desktop(window: &Window, front: bool) {
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    // SAFETY: winit hands us a valid NSView pointer for the lifetime of the window,
    // and we are on the main thread inside the event loop.
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    let Some(ns_window) = view.window() else { return };
    if front {
        return;
    }
    let level = unsafe { CGWindowLevelForKey(KCG_DESKTOP_WINDOW_LEVEL_KEY) } as isize;
    ns_window.setLevel(level);
    ns_window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    ns_window.setIgnoresMouseEvents(true);
    ns_window.setHasShadow(false);
}

impl App {
    fn ensure_pipelines(gpu: &mut Gpu, format: wgpu::TextureFormat) {
        if gpu.bg_pipeline.is_some() {
            return;
        }
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spike"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let make = |vs: &str, fs: &str, blend: wgpu::BlendState| {
            gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vs),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        gpu.bg_pipeline = Some(make("bg_vs", "bg_fs", wgpu::BlendState::REPLACE));
        gpu.dot_pipeline = Some(make("dot_vs", "dot_fs", additive));
        gpu.layout = Some(layout);
    }

    fn add_window(&mut self, event_loop: &ActiveEventLoop, pos: PhysicalPosition<i32>, size: PhysicalSize<u32>) {
        let attrs = Window::default_attributes()
            .with_title("olw-spike")
            .with_decorations(false)
            .with_resizable(false)
            .with_position(pos)
            .with_inner_size(size)
            .with_has_shadow(false)
            .with_active(false);
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        place_on_desktop(&window, self.opts.front);

        if self.gpu.is_none() {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let surface = instance.create_surface(window.clone()).expect("surface");
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            }))
            .expect("adapter");
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");
            eprintln!("adapter: {:?}", adapter.get_info().name);
            self.gpu = Some(Gpu { instance, adapter, device, queue, bg_pipeline: None, dot_pipeline: None, layout: None });
            drop(surface);
        }
        let gpu = self.gpu.as_mut().unwrap();
        let surface = gpu.instance.create_surface(window.clone()).expect("surface");
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(&gpu.device, &config);
        Self::ensure_pipelines(gpu, format);
        let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: gpu.layout.as_ref().unwrap(),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        window.request_redraw();
        self.surfaces.push(Surface { window, surface, config, uniforms, bind_group, occluded: false, frames: 0 });
    }

    fn draw(&mut self, id: WindowId) {
        let t = self.start.elapsed().as_secs_f32();
        let nodes = self.opts.nodes;
        let gpu = self.gpu.as_ref().unwrap();
        let Some(s) = self.surfaces.iter_mut().find(|s| s.window.id() == id) else { return };
        if s.occluded {
            return;
        }
        let frame = match s.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            other => {
                eprintln!("surface: {other:?}");
                s.surface.configure(&gpu.device, &s.config);
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let u = Uniforms { resolution: [s.config.width as f32, s.config.height as f32], time: t, nodes: nodes as f32 };
        gpu.queue.write_buffer(&s.uniforms, 0, bytemuck::bytes_of(&u));
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, &s.bind_group, &[]);
            pass.set_pipeline(gpu.bg_pipeline.as_ref().unwrap());
            pass.draw(0..3, 0..1);
            pass.set_pipeline(gpu.dot_pipeline.as_ref().unwrap());
            pass.draw(0..6, 0..nodes);
        }
        gpu.queue.submit([enc.finish()]);
        s.window.pre_present_notify();
        gpu.queue.present(frame);
        s.frames += 1;
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.surfaces.is_empty() {
            return;
        }
        unsafe {
            eprintln!(
                "levels: desktop={} desktopIcon={}",
                CGWindowLevelForKey(KCG_DESKTOP_WINDOW_LEVEL_KEY),
                CGWindowLevelForKey(KCG_DESKTOP_ICON_WINDOW_LEVEL_KEY)
            );
        }
        let monitors: Vec<_> = event_loop.available_monitors().collect();
        eprintln!("monitors: {}", monitors.len());
        for m in monitors {
            eprintln!("  {:?} {:?} @{:?} scale {}", m.name(), m.size(), m.position(), m.scale_factor());
            self.add_window(event_loop, m.position(), m.size());
        }
        let low_power = objc2_foundation::NSProcessInfo::processInfo().isLowPowerModeEnabled();
        eprintln!("lowPowerMode={low_power} fps={} nodes={} still={} front={}", self.opts.fps, self.opts.nodes, self.opts.still, self.opts.front);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => self.draw(id),
            WindowEvent::Occluded(occ) => {
                if let Some(s) = self.surfaces.iter_mut().find(|s| s.window.id() == id) {
                    s.occluded = occ;
                    eprintln!("[{:.1}s] occluded={occ}", self.start.elapsed().as_secs_f32());
                    if !occ {
                        s.window.request_redraw();
                    }
                }
            }
            WindowEvent::Resized(size) => {
                let gpu = self.gpu.as_ref().unwrap();
                if let Some(s) = self.surfaces.iter_mut().find(|s| s.window.id() == id) {
                    s.config.width = size.width.max(1);
                    s.config.height = size.height.max(1);
                    s.surface.configure(&gpu.device, &s.config);
                    s.window.request_redraw();
                }
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if let Some(limit) = self.opts.seconds {
            if self.start.elapsed().as_secs_f64() >= limit {
                event_loop.exit();
                return;
            }
        }
        if now.duration_since(self.last_report) >= Duration::from_secs(5) {
            let secs = now.duration_since(self.last_report).as_secs_f64();
            for (i, s) in self.surfaces.iter_mut().enumerate() {
                eprintln!("[{:.1}s] win{i} fps={:.1} occluded={}", self.start.elapsed().as_secs_f32(), s.frames as f64 / secs, s.occluded);
                s.frames = 0;
            }
            self.last_report = now;
        }
        let any_visible = self.surfaces.iter().any(|s| !s.occluded);
        if self.opts.still || !any_visible {
            // Nothing to animate: sleep until an event (occlusion change, resize) or the
            // next stats report / time limit.
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.last_report + Duration::from_secs(5)));
            return;
        }
        if now >= self.next_frame {
            for s in &self.surfaces {
                if !s.occluded {
                    s.window.request_redraw();
                }
            }
            let step = Duration::from_secs_f64(1.0 / self.opts.fps);
            self.next_frame += step;
            if self.next_frame < now {
                self.next_frame = now + step;
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

fn main() {
    let opts = parse_opts();
    let event_loop = EventLoop::builder()
        .with_activation_policy(ActivationPolicy::Accessory)
        .build()
        .expect("event loop");
    let now = Instant::now();
    let mut app = App { opts, gpu: None, surfaces: Vec::new(), start: now, next_frame: now, last_report: now };
    event_loop.run_app(&mut app).expect("run");
}
