//! Text on the wallpaper: hub labels and overlays (clock, vault stats, note of the
//! hour). Laid out and uploaded only when the text or positions change; drawing it
//! is part of the cached scene, so it costs nothing per animated frame.
//!
//! Glyphs come from `glyphon` (cosmic-text shaping and a growable glyph atlas). Under
//! the glyphs this module draws its own quads: rounded plates for `Badge` and
//! pre-blurred halo sprites for `Glow`, the same idea as the web version's
//! `labelSprite` cache. All CPU work (shaping, rasterising, blurring) happens in
//! `prepare`; `render` only binds buffers and issues two draws.
//!
//! Memory: glyphon's atlases start at 256x256 (one R8 mask texture, one RGBA texture
//! for colour glyphs such as emoji) and double only when a frame's glyphs don't fit.
//! Glow sprites are stored at one texel per point (half resolution on Retina; the
//! halo is soft anyway) in one R8 texture sized to what the current items need.

use bytemuck::{Pod, Zeroable};
use glyphon::cosmic_text::{Align, Attrs, Buffer, Color, Family, FamilyOwned, FontSystem, Metrics, Shaping, SwashCache};
use glyphon::{Cache, ColorMode, Resolution, TextArea, TextAtlas, TextBounds, Viewport};

use crate::Gpu;

/// Font family, matching the config's `labelFont`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontFamily {
    #[default]
    Sans,
    Mono,
    Serif,
}

/// How a label is dressed, matching `labelStyle.mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextStyle {
    /// Text on a rounded translucent plate.
    #[default]
    Badge,
    /// Text with a soft coloured glow behind it.
    Glow,
    /// Plain text.
    Minimal,
}

/// Where `pos` sits relative to the text block.
///
/// The text block is the laid-out lines (width of the widest line, height of all
/// lines). A `Badge` plate extends outside it by its padding, so switching styles
/// never moves the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    /// Centred horizontally, top edge at `pos` (labels under a note).
    #[default]
    TopCenter,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextItem {
    pub text: String,
    /// Position in points.
    pub pos: [f32; 2],
    pub anchor: Anchor,
    /// Font size in points.
    pub size: f32,
    /// Straight sRGB colour and alpha.
    pub color: [f32; 4],
    pub family: FontFamily,
    pub style: TextStyle,
    /// Plate colour for `Badge`, glow colour for `Glow` (alpha = strength).
    pub accent: [f32; 4],
    /// Wrap width in points (`None` = one line).
    pub max_width: Option<f32>,
}

/// Line height as a multiple of the font size.
const LINE_HEIGHT: f32 = 1.25;
/// Badge plate, in points, as in the web version: 8 left and right, 5 above and
/// below a one-line box of the font size, corner radius 10, 1pt outline.
const BADGE_PAD_X: f32 = 8.0;
const BADGE_PAD_Y: f32 = 5.0;
const BADGE_RADIUS: f32 = 10.0;
const BADGE_STROKE: f32 = 1.0;
const BADGE_STROKE_ALPHA: f32 = 0.25;
/// Glow: the web version uses canvas `shadowBlur = 6`, a Gaussian with sigma 3.
const GLOW_SIGMA: f32 = 3.0;
/// Canvas shadows of thin text are faint; this brings the halo to the same weight.
const GLOW_GAIN: f32 = 1.8;

const SANS: &[&str] = &["SF Pro", "SF Pro Text", "Helvetica Neue", "Helvetica"];
const MONO: &[&str] = &["SF Mono", "Menlo", "Monaco"];
const SERIF: &[&str] = &["New York", "Georgia", "Times New Roman"];

/// GPU text renderer for one target format.
pub struct TextRenderer {
    font_system: FontSystem,
    swash: SwashCache,
    families: [FamilyOwned; 3],
    atlas: TextAtlas,
    viewport: Viewport,
    glyphs: glyphon::TextRenderer,
    /// One shaping buffer per item, reused across `prepare` calls.
    buffers: Vec<Buffer>,
    quads: QuadLayer,
    /// The inputs of the last `prepare`, so an identical call does nothing.
    last: Option<(Vec<TextItem>, [u32; 2], f32)>,
}

impl TextRenderer {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let font_system = FontSystem::new();
        let families = [
            pick_family(&font_system, SANS, Family::SansSerif),
            pick_family(&font_system, MONO, Family::Monospace),
            pick_family(&font_system, SERIF, Family::Serif),
        ];
        let cache = Cache::new(device);
        // Colours are sRGB values blended as-is on the usual (non-sRGB) targets,
        // like the web canvas; glyphon calls that "Web" mode.
        let mode = if format.is_srgb() { ColorMode::Accurate } else { ColorMode::Web };
        let mut atlas = TextAtlas::with_color_mode(device, &gpu.queue, &cache, format, mode);
        let glyphs = glyphon::TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        TextRenderer {
            font_system,
            swash: SwashCache::new(),
            families,
            viewport: Viewport::new(device, &cache),
            atlas,
            glyphs,
            buffers: Vec::new(),
            quads: QuadLayer::new(gpu, format),
            last: None,
        }
    }

    /// The font family names chosen for `Sans`, `Mono` and `Serif` (a generic name
    /// such as "SansSerif" when none of the preferred fonts is installed).
    pub fn resolved_families(&self) -> [String; 3] {
        self.families.clone().map(|f| match f {
            FamilyOwned::Name(n) => n.to_string(),
            other => format!("{other:?}"),
        })
    }

    /// Lays out and uploads `items` for a target of `size` pixels at `scale` pixels per
    /// point. Call only when the items or the target change.
    pub fn prepare(&mut self, gpu: &Gpu, items: &[TextItem], size: [u32; 2], scale: f32) {
        if self.last.as_ref().is_some_and(|(i, s, k)| i == items && *s == size && *k == scale) {
            return;
        }
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        while self.buffers.len() < items.len() {
            self.buffers.push(Buffer::new_empty(Metrics::new(12.0, 15.0)));
        }

        // Shape every item and work out where its text block goes, in pixels.
        let mut placed = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            if item.text.trim().is_empty() || item.color[3] <= 0.0 || !item.size.is_finite() || item.size <= 0.0 {
                continue;
            }
            let family = &self.families[item.family as usize];
            let block = layout(&mut self.font_system, &mut self.buffers[i], item, family, scale);
            let [w, h] = block;
            let (x, y) = (item.pos[0] * scale, item.pos[1] * scale);
            let (left, top) = match item.anchor {
                Anchor::TopCenter => (x - w / 2.0, y),
                Anchor::TopLeft => (x, y),
                Anchor::TopRight => (x - w, y),
                Anchor::BottomLeft => (x, y - h),
                Anchor::BottomRight => (x - w, y - h),
                Anchor::Center => (x - w / 2.0, y - h / 2.0),
            };
            // Whole pixels keep the glyphs crisp.
            placed.push(Placed { item: i, left: left.round(), top: top.round(), w, h });
        }

        // Plates and glow sprites, in item order, under all text.
        let mut quads = Vec::new();
        let mut sprites = Vec::new();
        for p in &placed {
            let item = &items[p.item];
            match item.style {
                TextStyle::Minimal => {}
                TextStyle::Badge if item.accent[3] > 0.0 => quads.push(badge_quad(item, p, scale)),
                TextStyle::Badge => {}
                TextStyle::Glow if item.accent[3] > 0.0 => {
                    let buffer = &mut self.buffers[p.item];
                    if let Some(s) = glow_sprite(&mut self.font_system, &mut self.swash, buffer, p, scale) {
                        sprites.push((quads.len(), s));
                        quads.push(Quad {
                            rect: [0.0; 4],
                            uv: [0.0; 4],
                            fill: item.accent,
                            stroke: [0.0; 4],
                            shape: [0.0, 0.0, 1.0, 0.0],
                        });
                    }
                }
                TextStyle::Glow => {}
            }
        }
        // Rasterised glyph images are only needed while building sprites.
        self.swash.image_cache.clear();
        self.quads.upload(gpu, size, &mut quads, &sprites);

        // Glyphs.
        self.viewport.update(&gpu.queue, Resolution { width: size[0], height: size[1] });
        self.atlas.trim();
        let areas = placed.iter().map(|p| {
            let c = items[p.item].color;
            TextArea {
                buffer: &self.buffers[p.item],
                left: p.left,
                top: p.top,
                scale: 1.0,
                bounds: TextBounds::default(),
                default_color: Color::rgba(to_u8(c[0]), to_u8(c[1]), to_u8(c[2]), to_u8(c[3])),
                custom_glyphs: &[],
            }
        });
        if let Err(e) =
            self.glyphs.prepare(&gpu.device, &gpu.queue, &mut self.font_system, &mut self.atlas, &self.viewport, areas, &mut self.swash)
        {
            eprintln!("vw-render: text prepare failed: {e}");
        }
        self.buffers.truncate(items.len());
        self.last = Some((items.to_vec(), size, scale));
    }

    /// Draws what `prepare` uploaded into an open render pass.
    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.quads.render(pass);
        let _ = self.glyphs.render(&self.atlas, &self.viewport, pass);
    }
}

/// One shaped item's text block, in target pixels.
struct Placed {
    item: usize,
    left: f32,
    top: f32,
    w: f32,
    h: f32,
}

fn pick_family(fs: &FontSystem, names: &[&str], generic: Family<'_>) -> FamilyOwned {
    names
        .iter()
        .find(|name| fs.db().faces().any(|f| f.families.iter().any(|(n, _)| n == *name)))
        .map(|name| FamilyOwned::Name((*name).into()))
        .unwrap_or_else(|| FamilyOwned::new(generic))
}

/// Shapes `item` into `buffer` at pixel size and returns the block's [width, height]
/// in pixels.
fn layout(fs: &mut FontSystem, buffer: &mut Buffer, item: &TextItem, family: &FamilyOwned, scale: f32) -> [f32; 2] {
    let px = item.size * scale;
    let align = match item.anchor {
        Anchor::TopCenter | Anchor::Center => Align::Center,
        Anchor::TopRight | Anchor::BottomRight => Align::Right,
        Anchor::TopLeft | Anchor::BottomLeft => Align::Left,
    };
    let width = item.max_width.map(|w| (w * scale).max(px));
    buffer.set_metrics_and_size(Metrics::new(px, px * LINE_HEIGHT), width, None);
    let attrs = Attrs::new().family(family.as_family());
    buffer.set_text(&item.text, &attrs, Shaping::Advanced, Some(align));
    buffer.shape_until_scroll(fs, false);
    let measure = |b: &Buffer| {
        b.layout_runs().fold([0.0f32, 0.0f32], |[w, h], run| [w.max(run.line_w), h.max(run.line_top + run.line_height)])
    };
    let mut block = measure(buffer);
    // Align lines within the block itself, not within the wrap width, so anchors
    // place the visible text. Re-wrapping at the widest line's width gives the same
    // breaks.
    if align != Align::Left {
        buffer.set_size(Some(block[0] + 0.5), None);
        buffer.shape_until_scroll(fs, false);
        block = measure(buffer);
    }
    block
}

fn badge_quad(item: &TextItem, p: &Placed, scale: f32) -> Quad {
    let line = item.size * LINE_HEIGHT * scale;
    // One line: the font size plus padding, centred on the line box; more lines add
    // their line height.
    let h = p.h - line + (item.size + 2.0 * BADGE_PAD_Y) * scale;
    let w = p.w + 2.0 * BADGE_PAD_X * scale;
    let (cx, cy) = (p.left + p.w / 2.0, p.top + p.h / 2.0);
    let [r, g, b, _] = item.accent;
    // The web outline is the note colour at 25%; the plate colour here is already a
    // mix towards it, so brighten its hue to full value.
    let m = r.max(g).max(b);
    let stroke = if m > 0.0 { [r / m, g / m, b / m, BADGE_STROKE_ALPHA] } else { [1.0, 1.0, 1.0, BADGE_STROKE_ALPHA] };
    Quad {
        // One extra pixel each side for the anti-aliased edge and the outer stroke.
        rect: [cx - w / 2.0 - 1.0, cy - h / 2.0 - 1.0, w + 2.0, h + 2.0],
        uv: [0.0; 4],
        fill: item.accent,
        stroke,
        shape: [(BADGE_RADIUS * scale).min(h / 2.0), BADGE_STROKE * scale, 0.0, 0.0],
    }
}

/// A blurred coverage mask, `factor` target pixels per texel, placed at `origin`.
struct Sprite {
    origin: [f32; 2],
    size: [u32; 2],
    factor: u32,
    data: Vec<u8>,
}

/// Rasterises the item's glyphs into a coverage mask and blurs it into a halo.
fn glow_sprite(fs: &mut FontSystem, swash: &mut SwashCache, buffer: &mut Buffer, p: &Placed, scale: f32) -> Option<Sprite> {
    // One texel per point: halve the resolution on Retina.
    let f = (scale.floor() as u32).max(1);
    let sigma = GLOW_SIGMA * scale / f as f32;
    let pad = ((3.0 * sigma).ceil() as u32 + 1) * f;
    let w = (p.w.ceil() as u32 + 2 * pad).div_ceil(f) * f;
    let h = (p.h.ceil() as u32 + 2 * pad).div_ceil(f) * f;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 16 << 20 {
        return None;
    }
    let mut full = vec![0u8; (w * h) as usize];
    buffer.draw(fs, swash, Color::rgba(255, 255, 255, 255), |x, y, cw, ch, c| {
        for yy in y..y + ch as i32 {
            for xx in x..x + cw as i32 {
                let (px, py) = (xx + pad as i32, yy + pad as i32);
                if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                    let i = (py as u32 * w + px as u32) as usize;
                    full[i] = full[i].saturating_add(c.a());
                }
            }
        }
    });
    let (sw, sh) = (w / f, h / f);
    let mut mask = vec![0f32; (sw * sh) as usize];
    for y in 0..h {
        for x in 0..w {
            mask[((y / f) * sw + x / f) as usize] += f32::from(full[(y * w + x) as usize]);
        }
    }
    let norm = 1.0 / (255.0 * (f * f) as f32);
    mask.iter_mut().for_each(|v| *v *= norm);
    gaussian_blur(&mut mask, sw as usize, sh as usize, sigma);
    let data = mask.iter().map(|v| to_u8(v * GLOW_GAIN)).collect();
    Some(Sprite { origin: [p.left - pad as f32, p.top - pad as f32], size: [sw, sh], factor: f, data })
}

/// Three box blurs per axis, which approximates a Gaussian closely.
fn gaussian_blur(img: &mut [f32], w: usize, h: usize, sigma: f32) {
    let n = 3.0;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let m = ((12.0 * sigma * sigma - n * (wl * wl) as f32 - 4.0 * n * wl as f32 - 3.0 * n) / (-4.0 * wl as f32 - 4.0)).round() as i32;
    let mut tmp = vec![0f32; img.len()];
    for pass in 0..3 {
        let r = (if pass < m { wl } else { wl + 2 } - 1) as usize / 2;
        box_blur(img, &mut tmp, w, h, r, 1, w); // rows
        box_blur(&tmp, img, h, w, r, w, 1); // columns
    }
}

/// Blurs `lines` lines of `len` samples; sample `j` of line `i` is at `i * line + j * step`.
/// Samples outside the image count as zero.
fn box_blur(src: &[f32], dst: &mut [f32], len: usize, lines: usize, r: usize, step: usize, line: usize) {
    let inv = 1.0 / (2 * r + 1) as f32;
    for i in 0..lines {
        let at = |j: usize| i * line + j * step;
        let mut sum: f32 = (0..r.min(len)).map(|j| src[at(j)]).sum();
        for j in 0..len {
            if j + r < len {
                sum += src[at(j + r)];
            }
            dst[at(j)] = sum * inv;
            if j >= r {
                sum -= src[at(j - r)];
            }
        }
    }
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Quad {
    rect: [f32; 4],
    uv: [f32; 4],
    fill: [f32; 4],
    stroke: [f32; 4],
    shape: [f32; 4],
}

/// Plates and glow sprites: one instanced draw.
struct QuadLayer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    glow: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    instances: Option<wgpu::Buffer>,
    count: u32,
}

impl QuadLayer {
    fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text quads"),
            source: wgpu::ShaderSource::Wgsl(include_str!("text/quads.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text quads"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text quads"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let quad_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Quad>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4],
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("text quads"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[Some(quad_layout)], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("text quad params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("text glow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let glow = glow_texture(gpu, [1, 1]);
        let bind_group = bind(gpu, &layout, &params, &glow, &sampler);
        QuadLayer { pipeline, layout, params, sampler, glow, bind_group, instances: None, count: 0 }
    }

    /// Packs the sprites into the glow texture, fills in their quads' rects and
    /// UVs, and uploads the instances. `sprites` pairs a quad index with its sprite.
    fn upload(&mut self, gpu: &Gpu, size: [u32; 2], quads: &mut Vec<Quad>, sprites: &[(usize, Sprite)]) {
        let params: [f32; 4] = [size[0].max(1) as f32, size[1].max(1) as f32, 0.0, 0.0];
        gpu.queue.write_buffer(&self.params, 0, bytemuck::cast_slice(&params));

        // Shelf packing with a one-texel empty border around every sprite, so
        // bilinear sampling never picks up a neighbour.
        let max_dim = gpu.device.limits().max_texture_dimension_2d;
        let widest = sprites.iter().map(|(_, s)| s.size[0] + 2).max().unwrap_or(1);
        let atlas_w = widest.max(256).min(max_dim);
        let mut slots = Vec::with_capacity(sprites.len());
        let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
        for (_, s) in sprites {
            let (w, h) = (s.size[0] + 1, s.size[1] + 1);
            if x + w + 1 > atlas_w {
                x = 0;
                y += shelf;
                shelf = 0;
            }
            if w + 1 > atlas_w || y + h + 1 > max_dim {
                slots.push(None);
                continue;
            }
            slots.push(Some([x + 1, y + 1]));
            x += w;
            shelf = shelf.max(h);
        }
        let atlas_h = if sprites.is_empty() { 1 } else { (y + shelf + 1).next_multiple_of(16).min(max_dim) };
        let atlas_w = if sprites.is_empty() { 1 } else { atlas_w };

        let cur = [self.glow.width(), self.glow.height()];
        let needed = u64::from(atlas_w) * u64::from(atlas_h);
        let current = u64::from(cur[0]) * u64::from(cur[1]);
        if atlas_w > cur[0] || atlas_h > cur[1] || current > 4 * needed.max(256 * 16) {
            self.glow = glow_texture(gpu, [atlas_w, atlas_h]);
            self.bind_group = bind(gpu, &self.layout, &self.params, &self.glow, &self.sampler);
        }
        let (tw, th) = (self.glow.width() as f32, self.glow.height() as f32);
        if !sprites.is_empty() {
            let mut image = vec![0u8; (atlas_w * atlas_h) as usize];
            for ((_, s), slot) in sprites.iter().zip(&slots) {
                let Some([sx, sy]) = *slot else { continue };
                for row in 0..s.size[1] {
                    let src = (row * s.size[0]) as usize;
                    let dst = ((sy + row) * atlas_w + sx) as usize;
                    image[dst..dst + s.size[0] as usize].copy_from_slice(&s.data[src..src + s.size[0] as usize]);
                }
            }
            gpu.queue.write_texture(
                self.glow.as_image_copy(),
                &image,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(atlas_w), rows_per_image: Some(atlas_h) },
                wgpu::Extent3d { width: atlas_w, height: atlas_h, depth_or_array_layers: 1 },
            );
        }
        let mut dropped = Vec::new();
        for ((qi, s), slot) in sprites.iter().zip(&slots) {
            let Some([sx, sy]) = *slot else {
                dropped.push(*qi);
                continue;
            };
            let f = s.factor as f32;
            quads[*qi].rect = [s.origin[0], s.origin[1], s.size[0] as f32 * f, s.size[1] as f32 * f];
            quads[*qi].uv = [sx as f32 / tw, sy as f32 / th, (sx + s.size[0]) as f32 / tw, (sy + s.size[1]) as f32 / th];
        }
        for qi in dropped.into_iter().rev() {
            quads.remove(qi);
        }

        self.count = quads.len() as u32;
        let bytes: &[u8] = bytemuck::cast_slice(quads);
        if bytes.is_empty() {
            return;
        }
        if self.instances.as_ref().is_none_or(|b| b.size() < bytes.len() as u64) {
            self.instances = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("text quads"),
                size: (bytes.len() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        gpu.queue.write_buffer(self.instances.as_ref().unwrap(), 0, bytes);
    }

    fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(buf) = self.instances.as_ref().filter(|_| self.count > 0) else { return };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, buf.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}

fn glow_texture(gpu: &Gpu, size: [u32; 2]) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("text glow"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn bind(gpu: &Gpu, layout: &wgpu::BindGroupLayout, params: &wgpu::Buffer, glow: &wgpu::Texture, sampler: &wgpu::Sampler) -> wgpu::BindGroup {
    let view = glow.create_view(&Default::default());
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("text quads"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

#[cfg(test)]
mod tests;
