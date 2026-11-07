//! Edit bursts: short, event-driven animations over the still scene when a note is
//! saved (Ink & Paper's ink bleed, Pulse's travelling light).
//!
//! A burst is baked, not played: the theme describes it as a few *actors* (a sprite
//! image, or a short strip of frames, plus where it sits and how opaque it is over
//! time), [`BurstRenderer::bake`] draws every image once into one small atlas, and
//! the app hands the result to Core Animation, which moves and fades the pieces in
//! the window server. The app presents no frames for a burst. Measured 2026-11-06:
//! any GPU work leaves ~210 MB of Metal driver memory dirty until ~1 s after the GPU
//! goes idle, so presenting frames for 2 s (at any size or rate) held it for ~3 s;
//! a bake is one ~1 ms render inside the edit redraw's own window, and the burst then
//! costs only its atlas (~1-2 MB).
//!
//! The last key of every actor is fully transparent, so the still scene underneath
//! is the settled state.

use bytemuck::{Pod, Zeroable};

/// What a sprite looks like; each kind is one fragment shader in `burst.wgsl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BurstKind {
    /// Fresh ink on the dot and a wet front spreading into the paper (Ink & Paper),
    /// drawn as frames by `progress`.
    InkBleed,
    /// A ring of light opening out from a note (Pulse), drawn as frames by `progress`.
    PulseRing,
    /// A travelling pulse: a bright head at the sprite centre with a tail trailing
    /// towards -x (Pulse). One image, moved and turned along a link.
    PulseComet,
    /// A soft glint on a note a pulse arrives at (Pulse). One image, faded in and out.
    PulseGlint,
}

/// How a theme reacts to a saved note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditBurst {
    /// Seconds from start to fully settled.
    pub duration: f32,
    /// Frame-rate cap for the window server while the burst runs.
    pub fps: f32,
    /// Atlas resolution as a fraction of the screen's (1 = full Retina, 0.5 = one
    /// pixel per point). Soft effects bake at less.
    pub resolution: f32,
}

/// One burst sprite, in points. Only the shape fields matter for baking: an actor's
/// image is drawn centred in its own cell.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct BurstInstance {
    pub center: [f32; 2],
    /// The note's solid dot radius (comets: head radius).
    pub core: f32,
    /// How far the effect reaches (comets: tail length).
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

/// Where an actor is at time `t` (0..1 of the burst): centre in screen points
/// (top-left origin) and opacity. Between keys both change linearly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BurstKey {
    pub t: f32,
    pub at: [f32; 2],
    pub opacity: f32,
}

/// One moving or changing piece of a burst.
#[derive(Debug, Clone, PartialEq)]
pub struct BurstActor {
    pub kind: BurstKind,
    pub sprite: BurstInstance,
    /// Images to bake: 1 = a still image; more = `progress` 0..1 in equal steps,
    /// shown one after another during `play`.
    pub frames: u32,
    /// The part of the burst (0..1) the frames play over.
    pub play: [f32; 2],
    /// Rotation of the image, radians, clockwise on screen (y down).
    pub angle: f32,
    /// At least one key; times ascending from 0 to 1.
    pub keys: Vec<BurstKey>,
}

impl BurstActor {
    /// A still actor at `at`, fully opaque for the whole burst (its frames do the work).
    pub fn fixed(kind: BurstKind, sprite: BurstInstance, frames: u32, play: [f32; 2], at: [f32; 2]) -> Self {
        BurstActor {
            kind,
            sprite,
            frames,
            play,
            angle: 0.0,
            keys: vec![BurstKey { t: 0.0, at, opacity: 1.0 }, BurstKey { t: 1.0, at, opacity: 1.0 }],
        }
    }

    /// Side of one frame's square cell, in points.
    pub fn side(&self) -> f32 {
        self.sprite.extent() * 2.0
    }

    /// Position and opacity at `t`, interpolated between keys.
    pub fn at(&self, t: f32) -> ([f32; 2], f32) {
        let Some(first) = self.keys.first() else { return ([0.0; 2], 0.0) };
        let mut prev = *first;
        for k in &self.keys {
            if t <= k.t {
                let span = k.t - prev.t;
                let f = if span > 0.0 { ((t - prev.t) / span).clamp(0.0, 1.0) } else { 1.0 };
                let lerp = |a: f32, b: f32| a + (b - a) * f;
                return ([lerp(prev.at[0], k.at[0]), lerp(prev.at[1], k.at[1])], lerp(prev.opacity, k.opacity));
            }
            prev = *k;
        }
        (prev.at, prev.opacity)
    }

    /// The frame shown at `t` (0-based).
    pub fn frame_at(&self, t: f32) -> u32 {
        if self.frames <= 1 {
            return 0;
        }
        let [t0, t1] = self.play;
        let f = if t1 > t0 { ((t - t0) / (t1 - t0)).clamp(0.0, 1.0) } else { 1.0 };
        ((f * (self.frames - 1) as f32).floor() as u32).min(self.frames - 1)
    }
}

/// A baked actor: its strip of frames in the atlas.
#[derive(Debug, Clone, PartialEq)]
pub struct BakedActor {
    pub actor: BurstActor,
    /// Top-left pixel of frame 0 in the atlas; frames follow to the right.
    pub origin: [u32; 2],
    /// One frame's square side in pixels.
    pub cell: u32,
}

/// A burst ready for the window server: one RGBA8 premultiplied atlas and the
/// actors that show parts of it.
#[derive(Debug, Clone, PartialEq)]
pub struct BakedBurst {
    pub rgba: Vec<u8>,
    pub size: [u32; 2],
    /// Atlas pixels per point.
    pub scale: f32,
    pub actors: Vec<BakedActor>,
}

impl BakedBurst {
    /// Bytes of image data the burst holds.
    pub fn bytes(&self) -> usize {
        self.rgba.len()
    }
}

/// Atlas rows are packed up to this many pixels wide.
const ATLAS_WIDTH: u32 = 2048;

/// Packs each actor's strip of frames into rows. Returns the strips' origins, each
/// actor's cell side and the atlas size.
fn pack(actors: &[BurstActor], scale: f32) -> (Vec<([u32; 2], u32)>, [u32; 2]) {
    // One pixel of clear border around every frame, so linear filtering at a frame's
    // edge never reads its neighbour.
    let cells: Vec<u32> = actors.iter().map(|a| (a.side() * scale).ceil() as u32 + 2).collect();
    let widest = actors.iter().zip(&cells).map(|(a, c)| c * a.frames.max(1)).max().unwrap_or(1);
    let width = ATLAS_WIDTH.max(widest);
    let (mut x, mut y, mut row_h, mut w_used) = (0u32, 0u32, 0u32, 1u32);
    let mut out = Vec::with_capacity(actors.len());
    for (a, &c) in actors.iter().zip(&cells) {
        let strip = c * a.frames.max(1);
        if x + strip > width {
            x = 0;
            y += row_h;
            row_h = 0;
        }
        out.push(([x, y], c));
        x += strip;
        w_used = w_used.max(x);
        row_h = row_h.max(c);
    }
    (out, [w_used, (y + row_h).max(1)])
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BurstUniforms {
    origin: [f32; 2],
    size: [f32; 2],
    scale: f32,
    _pad: [f32; 3],
}

const KINDS: [BurstKind; 4] = [BurstKind::InkBleed, BurstKind::PulseRing, BurstKind::PulseComet, BurstKind::PulseGlint];

/// Bakes bursts. Create it the first time a burst runs: until then a wallpaper
/// pays nothing for it.
pub struct BurstRenderer {
    pipelines: Vec<(BurstKind, wgpu::RenderPipeline)>,
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
    buf: Option<wgpu::Buffer>,
}

/// Bakes into RGBA8 (premultiplied), the layout Core Graphics images use.
const BAKE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl BurstRenderer {
    pub fn new(gpu: &crate::Gpu) -> Self {
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
                        format: BAKE_FORMAT,
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
        let pipelines = KINDS
            .iter()
            .map(|&k| {
                let fs = match k {
                    BurstKind::InkBleed => "ink_bleed_fs",
                    BurstKind::PulseRing => "pulse_ring_fs",
                    BurstKind::PulseComet => "pulse_comet_fs",
                    BurstKind::PulseGlint => "pulse_glint_fs",
                };
                (k, pipeline(fs))
            })
            .collect();
        let uniforms = crate::uniform_buffer::<BurstUniforms>(device, "burst uniforms");
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        BurstRenderer { pipelines, uniforms, bind, buf: None }
    }

    /// Draws every frame of every actor into one atlas at `scale` pixels per point,
    /// in one render pass, and reads it back. `None` when there is nothing to bake.
    pub fn bake(&mut self, gpu: &crate::Gpu, actors: &[BurstActor], scale: f32) -> Option<BakedBurst> {
        if actors.is_empty() {
            return None;
        }
        let scale = scale.max(0.25);
        let (placed, size) = pack(actors, scale);
        // Every frame as one instance at its cell centre, in atlas points; grouped
        // by kind so each kind is one instanced draw.
        let mut instances: Vec<BurstInstance> = Vec::new();
        let mut ranges = Vec::new();
        for &kind in &KINDS {
            let start = instances.len() as u32;
            for (a, &(origin, cell)) in actors.iter().zip(&placed).filter(|(a, _)| a.kind == kind) {
                let n = a.frames.max(1);
                for k in 0..n {
                    let cx = origin[0] as f32 + (k * cell) as f32 + cell as f32 * 0.5;
                    let cy = origin[1] as f32 + cell as f32 * 0.5;
                    let progress = if n > 1 { k as f32 / (n - 1) as f32 } else { a.sprite.progress };
                    instances.push(BurstInstance { center: [cx / scale, cy / scale], progress, ..a.sprite });
                }
            }
            ranges.push((kind, start..instances.len() as u32));
        }
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("burst atlas"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: BAKE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let points = [size[0] as f32 / scale, size[1] as f32 / scale];
        let u = BurstUniforms { origin: [0.0; 2], size: points, scale, _pad: [0.0; 3] };
        gpu.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
        crate::instance_buffer(gpu, &mut self.buf, &instances, "burst sprites");
        let view = texture.create_view(&Default::default());
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = crate::begin(&mut enc, &view, "burst bake");
            pass.set_bind_group(0, &self.bind, &[]);
            if let Some(buf) = &self.buf {
                pass.set_vertex_buffer(0, buf.slice(..));
                for (kind, range) in ranges.into_iter().filter(|(_, r)| !r.is_empty()) {
                    let pipeline = &self.pipelines.iter().find(|(k, _)| *k == kind).expect("a pipeline per kind").1;
                    pass.set_pipeline(pipeline);
                    pass.draw(0..6, range);
                }
            }
        }
        gpu.queue.submit([enc.finish()]);
        let rgba = crate::read_texture(gpu, &texture, size);
        let actors = actors
            .iter()
            .zip(placed)
            .map(|(a, (origin, cell))| BakedActor { actor: a.clone(), origin, cell })
            .collect();
        Some(BakedBurst { rgba, size, scale, actors })
    }
}

/// Plays a baked burst back on the CPU at time `t` (0..1) into a `screen`-sized
/// RGBA8 premultiplied image at `scale` pixels per point, as the window server
/// would show it (nearest-neighbour, no rotation for still images). For tests and
/// preview images.
pub fn composite_at(baked: &BakedBurst, t: f32, screen: [u32; 2], scale: f32) -> Vec<u8> {
    let mut out = vec![0f32; (screen[0] * screen[1] * 4) as usize];
    for b in &baked.actors {
        let a = &b.actor;
        let (at, opacity) = a.at(t);
        if opacity <= 0.0 {
            continue;
        }
        let frame = a.frame_at(t);
        let fx = b.origin[0] + frame * b.cell;
        let half = b.cell as f32 * 0.5;
        let (s, c) = a.angle.sin_cos();
        // Every screen pixel the rotated cell can touch.
        let r = half * std::f32::consts::SQRT_2 / baked.scale;
        let x0 = ((at[0] - r) * scale).floor().max(0.0) as u32;
        let y0 = ((at[1] - r) * scale).floor().max(0.0) as u32;
        let x1 = (((at[0] + r) * scale).ceil() as u32).min(screen[0]);
        let y1 = (((at[1] + r) * scale).ceil() as u32).min(screen[1]);
        for y in y0..y1 {
            for x in x0..x1 {
                // Screen point to the actor's unrotated local frame, then to atlas pixels.
                let dx = (x as f32 + 0.5) / scale - at[0];
                let dy = (y as f32 + 0.5) / scale - at[1];
                let lx = dx * c + dy * s;
                let ly = -dx * s + dy * c;
                let px = lx * baked.scale + half;
                let py = ly * baked.scale + half;
                if px < 0.0 || py < 0.0 || px >= b.cell as f32 || py >= b.cell as f32 {
                    continue;
                }
                let src = (((b.origin[1] + py as u32) * baked.size[0] + fx + px as u32) * 4) as usize;
                let dst = ((y * screen[0] + x) * 4) as usize;
                let sa = f32::from(baked.rgba[src + 3]) / 255.0 * opacity;
                for k in 0..3 {
                    out[dst + k] = f32::from(baked.rgba[src + k]) / 255.0 * opacity + out[dst + k] * (1.0 - sa);
                }
                out[dst + 3] = sa + out[dst + 3] * (1.0 - sa);
            }
        }
    }
    out.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite(kind_reach: f32) -> BurstInstance {
        BurstInstance { center: [0.0; 2], core: 4.0, reach: kind_reach, color: [0.1, 0.1, 0.1, 0.9], progress: 0.0 }
    }

    #[test]
    fn keys_interpolate_and_frames_step() {
        let mut a = BurstActor::fixed(BurstKind::InkBleed, sprite(22.0), 5, [0.0, 0.8], [10.0, 20.0]);
        a.keys = vec![
            BurstKey { t: 0.0, at: [0.0, 0.0], opacity: 0.0 },
            BurstKey { t: 0.5, at: [10.0, 0.0], opacity: 1.0 },
            BurstKey { t: 1.0, at: [10.0, 0.0], opacity: 0.0 },
        ];
        assert_eq!(a.at(0.25), ([5.0, 0.0], 0.5));
        assert_eq!(a.at(1.0).1, 0.0);
        assert_eq!(a.frame_at(0.0), 0);
        assert_eq!(a.frame_at(0.4), 2);
        assert_eq!(a.frame_at(0.9), 4);
    }

    #[test]
    fn packing_keeps_strips_apart() {
        let actors: Vec<_> = (0..40).map(|i| BurstActor::fixed(BurstKind::PulseGlint, sprite(10.0 + i as f32), 1 + i % 7, [0.0, 1.0], [0.0; 2])).collect();
        let (placed, size) = pack(&actors, 2.0);
        let rects: Vec<[u32; 4]> = actors.iter().zip(&placed).map(|(a, (o, c))| [o[0], o[1], c * a.frames, *c]).collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(a[0] + a[2] <= size[0] && a[1] + a[3] <= size[1]);
            for b in &rects[i + 1..] {
                let apart = a[0] + a[2] <= b[0] || b[0] + b[2] <= a[0] || a[1] + a[3] <= b[1] || b[1] + b[3] <= a[1];
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    fn alpha_sum(rgba: &[u8]) -> u64 {
        rgba.chunks(4).map(|p| u64::from(p[3])).sum()
    }

    /// Alpha of one baked frame of `actor` in a ring between radii `r0` and `r1`
    /// points around its centre.
    fn ring_alpha(b: &BakedBurst, actor: usize, frame: u32, r0: f32, r1: f32) -> u64 {
        let a = &b.actors[actor];
        let mut sum = 0;
        for y in 0..a.cell {
            for x in 0..a.cell {
                let d = ((x as f32 + 0.5 - a.cell as f32 * 0.5).powi(2) + (y as f32 + 0.5 - a.cell as f32 * 0.5).powi(2)).sqrt() / b.scale;
                if d >= r0 && d < r1 {
                    let i = (((a.origin[1] + y) * b.size[0] + a.origin[0] + frame * a.cell + x) * 4 + 3) as usize;
                    sum += u64::from(b.rgba[i]);
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
        let actor = BurstActor::fixed(BurstKind::InkBleed, sprite(22.0), 5, [0.0, 1.0], [50.0, 40.0]);
        let baked = BurstRenderer::new(&gpu).bake(&gpu, &[actor], 2.0).unwrap();
        // At the start the ink is on the dot, not yet out in the paper.
        assert!(ring_alpha(&baked, 0, 0, 0.0, 4.0) > 0);
        assert_eq!(ring_alpha(&baked, 0, 0, 14.0, 30.0), 0);
        // Halfway, the wet front has travelled well out from the dot.
        assert!(ring_alpha(&baked, 0, 2, 10.0, 26.0) > 0);
        // Settled: fully transparent, so the still scene shows unchanged.
        assert_eq!(ring_alpha(&baked, 0, 4, 0.0, 100.0), 0);
    }

    #[test]
    fn pulse_sprites_bake() {
        let Some(gpu) = crate::Gpu::headless() else { return };
        let glow = BurstInstance { color: [0.9, 0.85, 0.7, 1.0], ..sprite(16.0) };
        let actors = [
            BurstActor::fixed(BurstKind::PulseRing, glow, 6, [0.0, 0.5], [0.0; 2]),
            BurstActor::fixed(BurstKind::PulseComet, glow, 1, [0.0, 1.0], [0.0; 2]),
            BurstActor::fixed(BurstKind::PulseGlint, glow, 1, [0.0, 1.0], [0.0; 2]),
        ];
        let baked = BurstRenderer::new(&gpu).bake(&gpu, &actors, 1.0).unwrap();
        // The ring moves outwards and is gone at its last frame.
        assert!(ring_alpha(&baked, 0, 1, 0.0, 8.0) > ring_alpha(&baked, 0, 1, 14.0, 20.0));
        assert!(ring_alpha(&baked, 0, 4, 10.0, 20.0) > 0);
        assert_eq!(ring_alpha(&baked, 0, 5, 0.0, 40.0), 0);
        // The comet's head is at the centre, its tail behind (towards -x), nothing ahead.
        let a = &baked.actors[1];
        let px = |x: f32| {
            let c = a.cell as f32 * 0.5;
            let i = (((a.origin[1] + c as u32) * baked.size[0] + a.origin[0] + (c + x * baked.scale) as u32) * 4 + 3) as usize;
            baked.rgba[i]
        };
        assert!(px(0.0) > 150);
        assert!(px(-8.0) > 0 && px(-8.0) < px(0.0));
        assert!(px(8.0) < px(-8.0));
        // The glint is a soft dot.
        assert!(ring_alpha(&baked, 2, 0, 0.0, 3.0) > ring_alpha(&baked, 2, 0, 10.0, 13.0));
        assert!(alpha_sum(&baked.rgba) > 0);
    }

    /// Cost of baking a burst on this machine: `cargo test -p vw-render --release
    /// burst_bake_cost -- --ignored --nocapture`. Wall time including the readback.
    #[test]
    #[ignore]
    fn burst_bake_cost() {
        let Some(gpu) = crate::Gpu::headless() else { return };
        let mut r = BurstRenderer::new(&gpu);
        let glow = BurstInstance { color: [0.9, 0.85, 0.7, 1.0], ..sprite(16.0) };
        let mut actors = vec![BurstActor::fixed(BurstKind::PulseRing, BurstInstance { reach: 36.0, ..glow }, 12, [0.0, 0.45], [0.0; 2])];
        actors.extend((0..56).map(|_| BurstActor::fixed(BurstKind::PulseComet, glow, 1, [0.0, 1.0], [0.0; 2])));
        actors.extend((0..56).map(|_| BurstActor::fixed(BurstKind::PulseGlint, glow, 1, [0.0, 1.0], [0.0; 2])));
        let ink = vec![BurstActor::fixed(BurstKind::InkBleed, sprite(30.0), 25, [0.0, 0.97], [0.0; 2])];
        for (name, actors, scale) in [("pulse", actors, 1.0), ("ink bleed", ink, 2.0)] {
            let n = 50;
            let t = std::time::Instant::now();
            let mut bytes = 0;
            for _ in 0..n {
                bytes = r.bake(&gpu, &actors, scale).unwrap().bytes();
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
            eprintln!("{name}: bake {ms:.2} ms, atlas {} KB, {} actors", bytes / 1024, actors.len());
        }
    }
}
