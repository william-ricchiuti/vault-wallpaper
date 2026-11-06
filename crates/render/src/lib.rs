//! The GPU renderer: turns a laid-out graph into pixels, on a window surface or an
//! off-screen texture (PNG export and image tests).
//!
//! Lightness: nothing here runs on its own. The app asks for a frame only when
//! something changed (new positions, a resize, a config change), so a still
//! wallpaper costs nothing between frames.

mod framing;
mod look;
mod scene;
mod text;

use std::path::Path;

use bytemuck::{Pod, Zeroable};

pub use framing::{Framing, FramingMode, FramingOptions, SafeArea};
pub use look::{LabelPick, Look, LookContext, derive_look};
pub use scene::{SceneStyle, build_instances, parse_hex, tag_color};
pub use text::{Anchor, FontFamily, TextItem, TextRenderer, TextStyle};

/// One link, in screen points.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct EdgeInstance {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub color: [f32; 4],
    pub width: f32,
}

/// One note, in screen points.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct NodeInstance {
    pub center: [f32; 2],
    pub radius: f32,
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    size: [f32; 2],
    scale: f32,
    _pad: f32,
    background: [f32; 4],
    background_glow: [f32; 4],
}

/// The GPU device, shared by every window and off-screen target.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
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
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("vw"),
            // Keep the driver's memory pools small; a wallpaper renders rarely.
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .ok()?;
        Some(Gpu { instance, adapter, device, queue })
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

/// Pipelines and instance buffers for one target format.
pub struct GraphRenderer {
    bg: wgpu::RenderPipeline,
    edges: wgpu::RenderPipeline,
    nodes: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    edge_buf: Option<wgpu::Buffer>,
    node_buf: Option<wgpu::Buffer>,
    edge_count: u32,
    node_count: u32,
}

impl GraphRenderer {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("graph"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let pipeline = |vs: &str, fs: &str, buffers: &[Option<wgpu::VertexBufferLayout>], blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(vs),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &module, entry_point: Some(vs), buffers, compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
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
        GraphRenderer {
            bg: pipeline("bg_vs", "bg_fs", &[], None),
            edges: pipeline("edge_vs", "edge_fs", &[Some(edge_layout)], Some(wgpu::BlendState::ALPHA_BLENDING)),
            nodes: pipeline("node_vs", "node_fs", &[Some(node_layout)], Some(wgpu::BlendState::ALPHA_BLENDING)),
            uniforms,
            bind_group,
            edge_buf: None,
            node_buf: None,
            edge_count: 0,
            node_count: 0,
        }
    }

    /// Uploads instances, growing the GPU buffers only when needed.
    pub fn upload(&mut self, gpu: &Gpu, nodes: &[NodeInstance], edges: &[EdgeInstance]) {
        fn put<T: Pod>(gpu: &Gpu, slot: &mut Option<wgpu::Buffer>, data: &[T], label: &str) {
            let bytes: &[u8] = bytemuck::cast_slice(data);
            if bytes.is_empty() {
                return;
            }
            if slot.as_ref().is_none_or(|b| b.size() < bytes.len() as u64) {
                *slot = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: (bytes.len() as u64).next_power_of_two(),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            gpu.queue.write_buffer(slot.as_ref().unwrap(), 0, bytes);
        }
        put(gpu, &mut self.edge_buf, edges, "edges");
        put(gpu, &mut self.node_buf, nodes, "nodes");
        self.edge_count = edges.len() as u32;
        self.node_count = nodes.len() as u32;
    }

    /// Draws the uploaded scene into `view`. `size` is in pixels, `scale` is pixels
    /// per point.
    pub fn render(&self, gpu: &Gpu, view: &wgpu::TextureView, size: [u32; 2], scale: f32, style: &SceneStyle) {
        let u = Uniforms {
            size: [size[0] as f32 / scale, size[1] as f32 / scale],
            scale,
            _pad: 0.0,
            background: [style.background[0], style.background[1], style.background[2], 1.0],
            background_glow: style.background_glow,
        };
        gpu.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("graph"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_pipeline(&self.bg);
            pass.draw(0..3, 0..1);
            if let (Some(buf), true) = (&self.edge_buf, self.edge_count > 0) {
                pass.set_pipeline(&self.edges);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..6, 0..self.edge_count);
            }
            if let (Some(buf), true) = (&self.node_buf, self.node_count > 0) {
                pass.set_pipeline(&self.nodes);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..6, 0..self.node_count);
            }
        }
        gpu.queue.submit([enc.finish()]);
    }
}

/// Renders a scene off-screen and returns tightly packed RGBA8 pixels.
pub fn render_rgba(gpu: &Gpu, size: [u32; 2], scale: f32, style: &SceneStyle, nodes: &[NodeInstance], edges: &[EdgeInstance]) -> Vec<u8> {
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
    let mut r = GraphRenderer::new(gpu, format);
    r.upload(gpu, nodes, edges);
    r.render(gpu, &texture.create_view(&Default::default()), size, scale, style);

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
