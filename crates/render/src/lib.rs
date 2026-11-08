//! The GPU renderer: turns a laid-out graph into pixels, on a window surface or an
//! off-screen texture (PNG export and image tests).
//!
//! Lightness: rendering is split in two.
//! 1. [`WallpaperRenderer::set_scene`] draws everything that only changes with the
//!    graph, layout, config or screen size: background, halos, links, notes and text
//!    into a full-resolution scene texture, and node glow into a quarter-resolution
//!    texture that is then blurred. This runs rarely.
//! 2. [`WallpaperRenderer::composite`] combines those cached textures with glow
//!    strength, vignette and grain onto the screen. It is the only work an animated
//!    frame (glow breathing) does: a few texture reads per pixel.
//!
//! The app composites right after each scene redraw and then frees the textures
//! ([`WallpaperRenderer::release_targets`]); glow breathing runs in Core Animation.
//!
//! Memory: every upload goes into a new buffer created mapped ([`Gpu::buffer_with`]),
//! never through `queue.write_buffer`, so a redraw encodes render passes only and the
//! Metal driver's separate copy/compute pool (~72 MB) stays asleep.
//!
//! Nothing here runs on its own; the app asks for frames.

pub mod burst;
mod framing;
mod look;
mod scene;
mod text;
pub mod theme;

use std::path::Path;

use bytemuck::{Pod, Zeroable};

pub use burst::{BakedActor, BakedBurst, BurstActor, BurstInstance, BurstKey, BurstKind, BurstRenderer, EditBurst};
pub use framing::{Framing, FramingMode, FramingOptions, OuterRing, SafeArea};
pub use look::{LabelPick, Look, LookContext, derive_look};
pub use scene::{BgMode, EdgeStyle, SceneData, SceneStyle, build_scene, parse_hex, style_from_config, tag_color};
pub use theme::{Ambient, Theme};
pub use text::{Anchor, FontFamily, TextItem, TextRenderer, TextStyle};

/// Format of the cached scene texture.
const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Format of the glow layer: float, so overlapping glows add up without clipping.
const GLOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Glow is drawn at 1/GLOW_DOWNSCALE of the screen resolution: it is blurred anyway.
const GLOW_DOWNSCALE: u32 = 4;

/// One link, in screen points.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct EdgeInstance {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub color: [f32; 4],
    pub width: f32,
}

/// One disc (note, halo or glow), in screen points.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct NodeInstance {
    pub center: [f32; 2],
    pub radius: f32,
    pub color: [f32; 4],
}

/// One aurora curtain (Aurora Lite), drawn into the low-resolution glow layer.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct CurtainInstance {
    /// Centre of the curtain's base line, in screen points.
    pub base: [f32; 2],
    /// Half width and height, in points.
    pub size: [f32; 2],
    /// Colour at the base and overall strength.
    pub color: [f32; 4],
    /// Colour at the top; alpha carries the shape seed.
    pub top: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    size: [f32; 2],
    scale: f32,
    bg_mode: f32,
    background: [f32; 4],
    background_glow: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BlurUniforms {
    dir: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CompositeUniforms {
    glow: f32,
    chroma: f32,
    vignette: f32,
    grain: f32,
    size: [f32; 2],
    glow_texel: [f32; 2],
}

/// The GPU device, shared by every window and off-screen target.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// Vertex and uniform buffers can be mapped (unified memory): see [`Gpu::buffer_with`].
    pub mappable: bool,
}

impl Gpu {
    /// A device compatible with `compatible_surface` (if given). Asks for the
    /// low-power GPU: a wallpaper never needs the fast one.
    pub fn new(compatible_surface: Option<&wgpu::Surface<'_>>, instance: wgpu::Instance) -> Option<Gpu> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .ok()?;
        let mappable = adapter.features().contains(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("vw"),
            required_features: if mappable { wgpu::Features::MAPPABLE_PRIMARY_BUFFERS } else { wgpu::Features::empty() },
            // Keep the driver's memory pools small; a wallpaper renders rarely.
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .ok()?;
        Some(Gpu { instance, adapter, device, queue, mappable })
    }

    /// A new buffer holding `bytes`, for `usage` (vertex or uniform).
    ///
    /// Memory: on Apple GPUs the driver keeps one ~100 MB working pool for render
    /// passes and presents and a separate ~72 MB one for copy and compute work, each
    /// for about a second after the GPU goes idle (measured 2026-11-07, also in a bare
    /// Metal program). `queue.write_buffer` uploads with a copy, so a redraw that used
    /// it held both pools. A buffer created mapped and written by the CPU (unified
    /// memory) needs no copy, so a redraw touches only the render pool.
    pub fn buffer_with(&self, label: &str, usage: wgpu::BufferUsages, bytes: &[u8]) -> wgpu::Buffer {
        let size = (bytes.len() as u64).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT).max(wgpu::COPY_BUFFER_ALIGNMENT);
        if !self.mappable {
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&buffer, 0, bytes);
            return buffer;
        }
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: usage | wgpu::BufferUsages::MAP_WRITE,
            mapped_at_creation: true,
        });
        buffer.slice(..).get_mapped_range_mut().expect("buffer mapped at creation").slice(..bytes.len()).copy_from_slice(bytes);
        buffer.unmap();
        buffer
    }

    pub fn headless() -> Option<Gpu> {
        Gpu::new(None, wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle()))
    }
}

/// Picks a non-sRGB 8-bit format when offered: colours are specified as sRGB values
/// and blended in that space, like the web version's canvas.
pub fn preferred_format(formats: &[wgpu::TextureFormat]) -> wgpu::TextureFormat {
    formats
        .iter()
        .copied()
        .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
        .unwrap_or(formats[0])
}

fn uniform_layout(device: &wgpu::Device, entries: &[wgpu::BindGroupLayoutEntry]) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries })
}

fn buffer_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn uniform_with<T: Pod>(gpu: &Gpu, label: &str, value: &T) -> wgpu::Buffer {
    gpu.buffer_with(label, wgpu::BufferUsages::UNIFORM, bytemuck::bytes_of(value))
}

struct Pipelines {
    bg: wgpu::RenderPipeline,
    halos: wgpu::RenderPipeline,
    edges: wgpu::RenderPipeline,
    nodes: wgpu::RenderPipeline,
    stars: wgpu::RenderPipeline,
    inks: wgpu::RenderPipeline,
    ink_edges: wgpu::RenderPipeline,
    glow: wgpu::RenderPipeline,
    curtains: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    glow_overlay: wgpu::RenderPipeline,
    scene_layout: wgpu::BindGroupLayout,
    blur_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
}

impl Pipelines {
    fn new(device: &wgpu::Device, out_format: wgpu::TextureFormat) -> Self {
        let scene_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let post_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()),
        });
        let scene_layout = uniform_layout(device, &[buffer_entry(0)]);
        let blur_layout = uniform_layout(device, &[texture_entry(0), sampler_entry(1), buffer_entry(2)]);
        let composite_layout = uniform_layout(device, &[texture_entry(0), texture_entry(1), sampler_entry(2), buffer_entry(3)]);
        let make = |module: &wgpu::ShaderModule,
                    layout: &wgpu::BindGroupLayout,
                    vs: &str,
                    fs: &str,
                    buffers: &[Option<wgpu::VertexBufferLayout>],
                    format: wgpu::TextureFormat,
                    blend: Option<wgpu::BlendState>| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(layout)],
                ..Default::default()
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module, entry_point: Some(vs), buffers, compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let edge_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<EdgeInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32],
        };
        let node_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<NodeInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32x4],
        };
        let curtain_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CurtainInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4],
        };
        let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
        let additive = Some(wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        });
        let node = Some(node_layout);
        let edge = Some(edge_layout);
        Pipelines {
            bg: make(&scene_mod, &scene_layout, "bg_vs", "bg_fs", &[], SCENE_FORMAT, None),
            halos: make(&scene_mod, &scene_layout, "node_vs", "halo_fs", std::slice::from_ref(&node), SCENE_FORMAT, alpha),
            edges: make(&scene_mod, &scene_layout, "edge_vs", "edge_fs", std::slice::from_ref(&edge), SCENE_FORMAT, alpha),
            nodes: make(&scene_mod, &scene_layout, "node_vs", "node_fs", std::slice::from_ref(&node), SCENE_FORMAT, alpha),
            stars: make(&scene_mod, &scene_layout, "node_vs", "star_fs", std::slice::from_ref(&node), SCENE_FORMAT, alpha),
            inks: make(&scene_mod, &scene_layout, "ink_vs", "ink_fs", std::slice::from_ref(&node), SCENE_FORMAT, alpha),
            ink_edges: make(&scene_mod, &scene_layout, "ink_edge_vs", "ink_edge_fs", std::slice::from_ref(&edge), SCENE_FORMAT, alpha),
            glow: make(&scene_mod, &scene_layout, "node_vs", "glow_fs", std::slice::from_ref(&node), GLOW_FORMAT, additive),
            curtains: make(&scene_mod, &scene_layout, "curtain_vs", "curtain_fs", &[Some(curtain_layout)], GLOW_FORMAT, additive),
            blur: make(&post_mod, &blur_layout, "full_vs", "blur_fs", &[], GLOW_FORMAT, None),
            composite: make(&post_mod, &composite_layout, "full_vs", "composite_fs", &[], out_format, None),
            glow_overlay: make(&post_mod, &composite_layout, "full_vs", "glow_overlay_fs", &[], out_format, None),
            scene_layout,
            blur_layout,
            composite_layout,
        }
    }
}

/// Cached textures for one target size.
struct Targets {
    size: [u32; 2],
    scene: wgpu::Texture,
    glow_a: wgpu::Texture,
    glow_b: wgpu::Texture,
    blur_h: wgpu::BindGroup,
    blur_v: wgpu::BindGroup,
}

/// Puts `data` in a new vertex buffer in `slot` (see [`Gpu::buffer_with`]); empties
/// the slot when there is nothing to draw. Returns the instance count.
fn instance_buffer<T: Pod>(gpu: &Gpu, slot: &mut Option<wgpu::Buffer>, data: &[T], label: &str) -> u32 {
    let bytes: &[u8] = bytemuck::cast_slice(data);
    *slot = (!bytes.is_empty()).then(|| gpu.buffer_with(label, wgpu::BufferUsages::VERTEX, bytes));
    data.len() as u32
}

/// Renders one screen: cached scene + glow, and the cheap per-frame composite.
pub struct WallpaperRenderer {
    pipelines: Pipelines,
    text: TextRenderer,
    sampler: wgpu::Sampler,
    targets: Option<Targets>,
    halo_buf: Option<wgpu::Buffer>,
    edge_buf: Option<wgpu::Buffer>,
    node_buf: Option<wgpu::Buffer>,
    star_buf: Option<wgpu::Buffer>,
    ink_buf: Option<wgpu::Buffer>,
    ink_edge_buf: Option<wgpu::Buffer>,
    glow_buf: Option<wgpu::Buffer>,
    curtain_buf: Option<wgpu::Buffer>,
    has_glow: bool,
}

impl WallpaperRenderer {
    /// `out_format` is the format of the surface (or texture) `composite` draws into.
    pub fn new(gpu: &Gpu, out_format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let pipelines = Pipelines::new(device, out_format);
        WallpaperRenderer {
            text: TextRenderer::new(gpu, SCENE_FORMAT),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                ..Default::default()
            }),
            pipelines,
            targets: None,
            halo_buf: None,
            edge_buf: None,
            node_buf: None,
            star_buf: None,
            ink_buf: None,
            ink_edge_buf: None,
            glow_buf: None,
            curtain_buf: None,
            has_glow: false,
        }
    }

    fn ensure_targets(&mut self, gpu: &Gpu, size: [u32; 2]) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let device = &gpu.device;
        let tex = |label: &str, w: u32, h: u32, format: wgpu::TextureFormat| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let (gw, gh) = (size[0].div_ceil(GLOW_DOWNSCALE), size[1].div_ceil(GLOW_DOWNSCALE));
        let scene = tex("scene", size[0], size[1], SCENE_FORMAT);
        let glow_a = tex("glow a", gw, gh, GLOW_FORMAT);
        let glow_b = tex("glow b", gw, gh, GLOW_FORMAT);
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let texel = [1.0 / gw as f32, 1.0 / gh as f32];
        let blur_bind = |src: &wgpu::Texture, dir: [f32; 2]| {
            let uniforms = uniform_with(gpu, "blur", &BlurUniforms { dir, _pad: [0.0; 2] });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.pipelines.blur_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view(src)) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: uniforms.as_entire_binding() },
                ],
            })
        };
        let blur_h = blur_bind(&glow_a, [texel[0], 0.0]);
        let blur_v = blur_bind(&glow_b, [0.0, texel[1]]);
        self.targets = Some(Targets { size, scene, glow_a, glow_b, blur_h, blur_v });
    }

    /// Redraws the cached layers: call when the graph, positions, look, config or
    /// target size changed. `size` is in pixels, `scale` pixels per point.
    pub fn set_scene(&mut self, gpu: &Gpu, size: [u32; 2], scale: f32, data: &SceneData, style: &SceneStyle) {
        self.ensure_targets(gpu, size);
        let halos = instance_buffer(gpu, &mut self.halo_buf, &data.halos, "halos");
        let edges = instance_buffer(gpu, &mut self.edge_buf, &data.edges, "edges");
        let nodes = instance_buffer(gpu, &mut self.node_buf, &data.nodes, "nodes");
        let stars = instance_buffer(gpu, &mut self.star_buf, &data.stars, "stars");
        let inks = instance_buffer(gpu, &mut self.ink_buf, &data.inks, "inks");
        let ink_edges = instance_buffer(gpu, &mut self.ink_edge_buf, &data.ink_edges, "ink edges");
        let glows = instance_buffer(gpu, &mut self.glow_buf, &data.glow, "glow");
        let curtains = instance_buffer(gpu, &mut self.curtain_buf, &data.curtains, "curtains");
        self.has_glow = glows + curtains > 0 && style.glow > 0.0;
        self.text.prepare(gpu, &data.text, size, scale);

        let points = [size[0] as f32 / scale, size[1] as f32 / scale];
        let mut u = Uniforms {
            size: points,
            scale,
            bg_mode: style.bg_mode as u32 as f32,
            background: [style.background[0], style.background[1], style.background[2], 1.0],
            background_glow: style.background_glow,
        };
        let bind = |u: &Uniforms, label: &str| {
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.pipelines.scene_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_with(gpu, label, u).as_entire_binding() }],
            })
        };
        let scene_bind = bind(&u, "scene uniforms");
        u.scale = scale / GLOW_DOWNSCALE as f32;
        let glow_bind = bind(&u, "glow uniforms");

        let t = self.targets.as_ref().unwrap();
        let p = &self.pipelines;
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let view = t.scene.create_view(&Default::default());
            let mut pass = begin(&mut enc, &view, "scene");
            pass.set_bind_group(0, &scene_bind, &[]);
            pass.set_pipeline(&p.bg);
            pass.draw(0..3, 0..1);
            draw_instances(&mut pass, &p.halos, &self.halo_buf, halos);
            draw_instances(&mut pass, &p.edges, &self.edge_buf, edges);
            draw_instances(&mut pass, &p.ink_edges, &self.ink_edge_buf, ink_edges);
            draw_instances(&mut pass, &p.nodes, &self.node_buf, nodes);
            draw_instances(&mut pass, &p.stars, &self.star_buf, stars);
            draw_instances(&mut pass, &p.inks, &self.ink_buf, inks);
            self.text.render(&mut pass);
        }
        if self.has_glow {
            let glow_a = t.glow_a.create_view(&Default::default());
            let glow_b = t.glow_b.create_view(&Default::default());
            {
                let mut pass = begin(&mut enc, &glow_a, "glow");
                pass.set_bind_group(0, &glow_bind, &[]);
                draw_instances(&mut pass, &p.curtains, &self.curtain_buf, curtains);
                draw_instances(&mut pass, &p.glow, &self.glow_buf, glows);
            }
            for (target, bind) in [(&glow_b, &t.blur_h), (&glow_a, &t.blur_v)] {
                let mut pass = begin(&mut enc, target, "blur");
                pass.set_pipeline(&p.blur);
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        gpu.queue.submit([enc.finish()]);
    }

    /// Draws the cached layers onto `view`. `breath` scales the glow (1 = as
    /// configured).
    pub fn composite(&self, gpu: &Gpu, view: &wgpu::TextureView, style: &SceneStyle, breath: f32) {
        let glow = if self.has_glow { style.glow * breath } else { 0.0 };
        self.post(gpu, view, style, glow, &self.pipelines.composite);
    }

    /// Draws the scene without glow: for screens whose glow lives in a separate
    /// overlay layer (see [`Self::render_glow_overlay`]).
    pub fn composite_scene_only(&self, gpu: &Gpu, view: &wgpu::TextureView, style: &SceneStyle) {
        self.post(gpu, view, style, 0.0, &self.pipelines.composite);
    }

    /// Draws the glow alone, premultiplied, into a target of [`Self::glow_size`]
    /// pixels. Shown in a layer above the scene, it lets Core Animation do the
    /// breathing.
    pub fn render_glow_overlay(&self, gpu: &Gpu, view: &wgpu::TextureView, style: &SceneStyle) {
        let glow = if self.has_glow { style.glow } else { 0.0 };
        self.post(gpu, view, style, glow, &self.pipelines.glow_overlay);
    }

    /// Frees the cached scene and glow textures. The next draw must call
    /// [`Self::set_scene`] first ([`Self::has_targets`] tells).
    pub fn release_targets(&mut self) {
        self.targets = None;
    }

    /// Whether the cached textures exist (see [`Self::release_targets`]).
    pub fn has_targets(&self) -> bool {
        self.targets.is_some()
    }

    /// True when the last scene has a glow layer worth showing.
    pub fn has_glow(&self) -> bool {
        self.has_glow
    }

    /// Size in pixels of the glow layer for the last scene.
    pub fn glow_size(&self) -> Option<[u32; 2]> {
        self.targets.as_ref().map(|t| [t.size[0].div_ceil(GLOW_DOWNSCALE), t.size[1].div_ceil(GLOW_DOWNSCALE)])
    }

    fn post(&self, gpu: &Gpu, view: &wgpu::TextureView, style: &SceneStyle, glow: f32, pipeline: &wgpu::RenderPipeline) {
        let Some(t) = &self.targets else { return };
        let glow_px = [t.size[0].div_ceil(GLOW_DOWNSCALE) as f32, t.size[1].div_ceil(GLOW_DOWNSCALE) as f32];
        let u = CompositeUniforms {
            glow,
            chroma: style.chromatic,
            vignette: style.vignette,
            grain: style.grain,
            size: [t.size[0] as f32, t.size[1] as f32],
            glow_texel: [1.0 / glow_px[0], 1.0 / glow_px[1]],
        };
        let view_of = |t: &wgpu::Texture| t.create_view(&Default::default());
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipelines.composite_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view_of(&t.scene)) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view_of(&t.glow_a)) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: uniform_with(gpu, "composite", &u).as_entire_binding() },
            ],
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = begin(&mut enc, view, "composite");
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([enc.finish()]);
    }
}

fn begin<'a>(enc: &'a mut wgpu::CommandEncoder, view: &'a wgpu::TextureView, label: &str) -> wgpu::RenderPass<'a> {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
        })],
        ..Default::default()
    })
}

fn draw_instances(pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline, buf: &Option<wgpu::Buffer>, count: u32) {
    if let (Some(buf), true) = (buf, count > 0) {
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, buf.slice(..));
        pass.draw(0..6, 0..count);
    }
}

/// Renders a scene off-screen and returns tightly packed RGBA8 pixels.
pub fn render_rgba(gpu: &Gpu, size: [u32; 2], scale: f32, style: &SceneStyle, data: &SceneData, breath: f32) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut r = WallpaperRenderer::new(gpu, format);
    r.set_scene(gpu, size, scale, data, style);
    r.composite(gpu, &texture.create_view(&Default::default()), style, breath);
    read_texture(gpu, &texture, size)
}

fn read_texture(gpu: &Gpu, texture: &wgpu::Texture, size: [u32; 2]) -> Vec<u8> {
    let row = size[0] * 4;
    let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded * size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(size[1]) },
        },
        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
    );
    gpu.queue.submit([enc.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let data = slice.get_mapped_range().expect("mapped readback buffer");
    let mut out = Vec::with_capacity((row * size[1]) as usize);
    for y in 0..size[1] as usize {
        out.extend_from_slice(&data[y * padded as usize..y * padded as usize + row as usize]);
    }
    out
}

/// Writes RGBA8 pixels to a PNG file.
pub fn write_png(path: &Path, size: [u32; 2], rgba: &[u8]) -> std::io::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, size[0], size[1]);
    enc.set_color(png::ColorType::Rgba);
    let mut w = enc.write_header().map_err(std::io::Error::other)?;
    w.write_image_data(rgba).map_err(std::io::Error::other)
}
