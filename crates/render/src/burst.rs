//! Edit bursts: short, event-driven animations (Ink & Paper's ink bleed; later
//! Pulse) drawn as a handful of sprites into a small overlay region above the cached
//! scene. The scene itself is never redrawn for them, and when a burst ends its last
//! frame is fully transparent, so the still scene underneath is the settled state.
//!
//! Lightness: a burst frame is one small render pass of a few instanced quads into a
//! drawable the size of the bursts' bounding box. The CPU work per frame is updating
//! one `progress` number per sprite. Between bursts nothing runs.

use std::time::Duration;

use bytemuck::{Pod, Zeroable};

/// What a burst looks like; each kind is one fragment shader in `burst.wgsl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BurstKind {
    /// Fresh ink on the dot and a wet front spreading into the paper (Ink & Paper).
    InkBleed,
}

/// How a theme reacts to a saved note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditBurst {
    pub kind: BurstKind,
    /// Seconds from start to fully settled.
    pub duration: f32,
    /// Frame-rate cap while the burst runs.
    pub fps: f32,
}

/// One burst sprite, in screen points.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct BurstInstance {
    pub center: [f32; 2],
    /// The note's solid dot radius.
    pub core: f32,
    /// How far the effect reaches when it settles.
    pub reach: f32,
    pub color: [f32; 4],
    /// 0 at the start, 1 when settled.
    pub progress: f32,
}

/// Must match `FRONT_SLACK` in burst.wgsl.
const FRONT_SLACK: f32 = 1.2;

impl BurstInstance {
    /// Half the side of the square this sprite can touch, in points.
    pub fn extent(&self) -> f32 {
        self.reach * FRONT_SLACK + 2.0
    }
}

/// Linear progress of a burst `elapsed` into `duration`: 0..1, clamped.
pub fn progress(elapsed: Duration, duration: f32) -> f32 {
    if duration <= 0.0 {
        return 1.0;
    }
    (elapsed.as_secs_f32() / duration).clamp(0.0, 1.0)
}

/// The region (x, y, width, height in points, top-left origin) that covers every
/// sprite, clipped to a `screen` of that many points. `None` when nothing is on screen.
pub fn region(sprites: &[BurstInstance], screen: [f32; 2]) -> Option<[f32; 4]> {
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    for s in sprites {
        let e = s.extent();
        for k in 0..2 {
            lo[k] = lo[k].min(s.center[k] - e);
            hi[k] = hi[k].max(s.center[k] + e);
        }
    }
    // Whole points, so the drawable maps onto pixels cleanly.
    let x0 = lo[0].max(0.0).floor();
    let y0 = lo[1].max(0.0).floor();
    let x1 = hi[0].min(screen[0]).ceil();
    let y1 = hi[1].min(screen[1]).ceil();
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BurstUniforms {
    origin: [f32; 2],
    size: [f32; 2],
    scale: f32,
    _pad: [f32; 3],
}

/// Draws burst sprites into an overlay target. Create it the first time a burst
/// runs: until then a wallpaper pays nothing for it.
pub struct BurstRenderer {
    ink_bleed: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
    buf: Option<wgpu::Buffer>,
}

impl BurstRenderer {
    pub fn new(gpu: &crate::Gpu, out_format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("burst"),
            source: wgpu::ShaderSource::Wgsl(include_str!("burst.wgsl").into()),
        });
        let layout = crate::uniform_layout(device, &[crate::buffer_entry(0)]);
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let instance = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<BurstInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32, 3 => Float32x4, 4 => Float32],
        };
        let pipeline = |fs: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("burst_vs"),
                    buffers: &[Some(instance.clone())],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: out_format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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
        let uniforms = crate::uniform_buffer::<BurstUniforms>(device, "burst uniforms");
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        BurstRenderer { ink_bleed: pipeline("ink_bleed_fs"), uniforms, bind, buf: None }
    }

    /// Clears `view` (a target covering `region` = x, y, w, h in screen points at
    /// `scale` pixels per point) to transparent and draws `sprites` into it.
    pub fn draw(&mut self, gpu: &crate::Gpu, view: &wgpu::TextureView, region: [f32; 4], scale: f32, kind: BurstKind, sprites: &[BurstInstance]) {
        let u = BurstUniforms { origin: [region[0], region[1]], size: [region[2], region[3]], scale, _pad: [0.0; 3] };
        gpu.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
        let count = crate::instance_buffer(gpu, &mut self.buf, sprites, "bursts");
        let pipeline = match kind {
            BurstKind::InkBleed => &self.ink_bleed,
        };
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = crate::begin(&mut enc, view, "bursts");
            pass.set_bind_group(0, &self.bind, &[]);
            crate::draw_instances(&mut pass, pipeline, &self.buf, count);
        }
        gpu.queue.submit([enc.finish()]);
    }
}

/// Renders bursts off-screen into a target covering `region` and returns RGBA8
/// pixels (premultiplied). For tests and cost measurements.
pub fn render_bursts_rgba(gpu: &crate::Gpu, region: [f32; 4], scale: f32, kind: BurstKind, sprites: &[BurstInstance]) -> (Vec<u8>, [u32; 2]) {
    let size = [((region[2] * scale).round() as u32).max(1), ((region[3] * scale).round() as u32).max(1)];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bursts offscreen"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut r = BurstRenderer::new(gpu, format);
    r.draw(gpu, &texture.create_view(&Default::default()), region, scale, kind, sprites);
    (crate::read_texture(gpu, &texture, size), size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite(progress: f32) -> BurstInstance {
        BurstInstance { center: [50.0, 40.0], core: 4.0, reach: 22.0, color: [0.1, 0.1, 0.1, 0.9], progress }
    }

    #[test]
    fn progress_runs_zero_to_one() {
        assert_eq!(progress(Duration::ZERO, 2.0), 0.0);
        assert!((progress(Duration::from_secs(1), 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(progress(Duration::from_secs(5), 2.0), 1.0);
        assert_eq!(progress(Duration::from_secs(1), 0.0), 1.0);
    }

    #[test]
    fn region_covers_sprites_and_clips_to_screen() {
        let s = sprite(0.0);
        let [x, y, w, h] = region(&[s], [1000.0, 800.0]).unwrap();
        let e = s.extent();
        assert!(x <= 50.0 - e && y <= 40.0 - e && x + w >= 50.0 + e && y + h >= 40.0 + e);
        let edge = BurstInstance { center: [2.0, 2.0], ..s };
        let [x, y, ..] = region(&[edge], [1000.0, 800.0]).unwrap();
        assert_eq!([x, y], [0.0, 0.0]);
        let off = BurstInstance { center: [-500.0, -500.0], ..s };
        assert!(region(&[off], [1000.0, 800.0]).is_none());
        assert!(region(&[], [1000.0, 800.0]).is_none());
    }

    fn alpha_sum(rgba: &[u8]) -> u64 {
        rgba.chunks(4).map(|p| u64::from(p[3])).sum()
    }

    /// Alpha in a ring between radii `r0` and `r1` points around the sprite centre.
    fn ring_alpha(rgba: &[u8], size: [u32; 2], region: [f32; 4], scale: f32, r0: f32, r1: f32) -> u64 {
        let mut sum = 0;
        for y in 0..size[1] {
            for x in 0..size[0] {
                let pt = [region[0] + (x as f32 + 0.5) / scale, region[1] + (y as f32 + 0.5) / scale];
                let d = ((pt[0] - 50.0).powi(2) + (pt[1] - 40.0).powi(2)).sqrt();
                if d >= r0 && d < r1 {
                    sum += u64::from(rgba[((y * size[0] + x) * 4 + 3) as usize]);
                }
            }
        }
        sum
    }

    #[test]
    fn ink_bleed_spreads_out_then_vanishes() {
        let Some(gpu) = crate::Gpu::headless() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        let scale = 2.0;
        let reg = region(&[sprite(0.0)], [200.0, 200.0]).unwrap();
        let frame = |p: f32| render_bursts_rgba(&gpu, reg, scale, BurstKind::InkBleed, &[sprite(p)]);
        let (start, size) = frame(0.0);
        let (mid, _) = frame(0.5);
        let (end, _) = frame(1.0);
        // At the start the ink is on the dot, not yet out in the paper.
        assert!(alpha_sum(&start) > 0);
        assert_eq!(ring_alpha(&start, size, reg, scale, 14.0, 30.0), 0);
        // Halfway, the wet front has travelled well out from the dot.
        assert!(ring_alpha(&mid, size, reg, scale, 10.0, 26.0) > 0);
        // Settled: fully transparent, so the still scene shows unchanged.
        assert_eq!(alpha_sum(&end), 0);
    }

    /// Cost of one burst frame on this machine: `cargo test -p vw-render --release
    /// burst_frame_cost -- --ignored --nocapture`. Wall time per frame including the
    /// wait for the GPU, so an upper bound on GPU time.
    #[test]
    #[ignore]
    fn burst_frame_cost() {
        let Some(gpu) = crate::Gpu::headless() else { return };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        // Four bleeds on a hub-sized dot, spread out: a worst-ish case region.
        let sprites: Vec<_> = (0..4).map(|i| BurstInstance { center: [100.0 + i as f32 * 120.0, 100.0], ..sprite(0.5) }).collect();
        let reg = region(&sprites, [1512.0, 982.0]).unwrap();
        let size = [(reg[2] * 2.0) as u32, (reg[3] * 2.0) as u32];
        let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let mut r = BurstRenderer::new(&gpu, format);
        let n = 200;
        let t = std::time::Instant::now();
        for i in 0..n {
            let frame: Vec<_> = sprites.iter().map(|s| BurstInstance { progress: i as f32 / n as f32, ..*s }).collect();
            r.draw(&gpu, &view, reg, 2.0, BurstKind::InkBleed, &frame);
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
        eprintln!("burst frame: {ms:.3} ms for {}x{} px ({} sprites)", size[0], size[1], sprites.len());
    }
}
