use std::path::PathBuf;

use super::*;

const BG: [u8; 4] = [16, 18, 28, 255];

fn gpu() -> Option<Gpu> {
    let g = Gpu::headless();
    if g.is_none() {
        eprintln!("no GPU adapter; skipping text test");
    }
    g
}

fn item(text: &str, pos: [f32; 2], anchor: Anchor, style: TextStyle) -> TextItem {
    TextItem {
        text: text.into(),
        pos,
        anchor,
        size: 14.0,
        color: [0.96, 0.97, 1.0, 0.88],
        family: FontFamily::Sans,
        style,
        accent: match style {
            TextStyle::Badge => [0.12, 0.15, 0.24, 0.72],
            _ => [0.49, 0.36, 1.0, 0.9],
        },
        max_width: None,
    }
}

/// Clears an off-screen target to `BG`, draws the prepared text, and reads it back.
fn draw(gpu: &Gpu, text: &mut TextRenderer, items: &[TextItem], size: [u32; 2], scale: f32) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("text test"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    text.prepare(gpu, items, size, scale);
    let view = texture.create_view(&Default::default());
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    {
        let c = |v: u8| f64::from(v) / 255.0;
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("text test"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: c(BG[0]), g: c(BG[1]), b: c(BG[2]), a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        text.render(&mut pass);
    }
    gpu.queue.submit([enc.finish()]);

    let row = size[0] * 4;
    let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("text readback"),
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

fn px(img: &[u8], size: [u32; 2], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * size[0] + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2], img[i + 3]]
}

/// Bounding box [x0, y0, x1, y1] of pixels that differ from the background by more
/// than `threshold` in any channel.
fn ink(img: &[u8], size: [u32; 2], threshold: u8) -> Option<[u32; 4]> {
    let mut b: Option<[u32; 4]> = None;
    for y in 0..size[1] {
        for x in 0..size[0] {
            let p = px(img, size, x, y);
            if (0..3).any(|c| p[c].abs_diff(BG[c]) > threshold) {
                let r = b.get_or_insert([x, y, x, y]);
                *r = [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)];
            }
        }
    }
    b
}

/// Pixels brighter than mid-grey: the glyphs themselves, not plates or halos.
fn bright(img: &[u8], size: [u32; 2]) -> Option<[u32; 4]> {
    let mut b: Option<[u32; 4]> = None;
    for y in 0..size[1] {
        for x in 0..size[0] {
            if px(img, size, x, y)[..3].iter().all(|&v| v > 150) {
                let r = b.get_or_insert([x, y, x, y]);
                *r = [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)];
            }
        }
    }
    b
}

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn fonts_resolve_to_system_families() {
    let Some(gpu) = gpu() else { return };
    let text = TextRenderer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm);
    let names = text.resolved_families();
    eprintln!("resolved fonts: {names:?}");
    if cfg!(target_os = "macos") {
        assert!(SANS.contains(&names[0].as_str()), "sans fell back: {names:?}");
        assert!(MONO.contains(&names[1].as_str()), "mono fell back: {names:?}");
        assert!(SERIF.contains(&names[2].as_str()), "serif fell back: {names:?}");
    }
}

#[test]
fn each_style_draws_where_the_text_is_and_nowhere_else() {
    let Some(gpu) = gpu() else { return };
    let size = [480, 120];
    let scale = 2.0;
    let mut text = TextRenderer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm);
    for (style, name) in [(TextStyle::Badge, "badge"), (TextStyle::Glow, "glow"), (TextStyle::Minimal, "minimal")] {
        let items = [item("Vault Wallpaper · hubs", [120.0, 20.0], Anchor::TopCenter, style)];
        let img = draw(&gpu, &mut text, &items, size, scale);
        write_crate_png(&img, size, &format!("text-{name}.png"));

        let glyphs = bright(&img, size).unwrap_or_else(|| panic!("{name}: no glyph pixels"));
        // Centred on x = 120pt = 240px, top at 20pt = 40px.
        let cx = (glyphs[0] + glyphs[2]) / 2;
        assert!(cx.abs_diff(240) <= 4, "{name}: glyphs centred at {cx}, want 240 ({glyphs:?})");
        assert!(glyphs[1] >= 40 && glyphs[3] < 40 + 36, "{name}: glyph rows {glyphs:?}");

        let all = ink(&img, size, 2).unwrap();
        // Background stays untouched away from the label.
        for (x, y) in [(2, 2), (470, 110), (10, 60), (470, 10)] {
            assert_eq!(px(&img, size, x, y), BG, "{name}: pixel ({x},{y}) changed");
        }
        match style {
            TextStyle::Minimal => {
                // Nothing beyond the glyphs' own anti-aliasing.
                assert!(all[0] + 3 >= glyphs[0] && all[2] <= glyphs[2] + 3, "{name}: ink {all:?} vs glyphs {glyphs:?}");
            }
            TextStyle::Badge => {
                // The plate reaches 8pt left and right of the text and is lighter than the
                // background just outside the glyphs.
                assert!(glyphs[0] - all[0] >= 12 && all[2] - glyphs[2] >= 12, "{name}: plate {all:?} vs glyphs {glyphs:?}");
                let p = px(&img, size, all[0] + 10, (all[1] + all[3]) / 2);
                assert!(p[2] > BG[2] + 10, "{name}: plate pixel {p:?}");
            }
            TextStyle::Glow => {
                // A violet halo around the glyphs: blue exceeds red just outside them.
                assert!(glyphs[0] - all[0] >= 3 && all[3] - glyphs[3] >= 3, "{name}: halo {all:?} vs glyphs {glyphs:?}");
                let p = px(&img, size, glyphs[0] - 2, (glyphs[1] + glyphs[3]) / 2);
                assert!(p[2] > p[0] && p[2] > BG[2] + 5, "{name}: halo pixel {p:?}");
            }
        }
    }
}

#[test]
fn anchors_place_the_block() {
    let Some(gpu) = gpu() else { return };
    let size = [400, 200];
    let mut text = TextRenderer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm);
    let cases = [
        (Anchor::TopLeft, [20.0, 20.0]),
        (Anchor::TopRight, [380.0, 20.0]),
        (Anchor::BottomLeft, [20.0, 180.0]),
        (Anchor::BottomRight, [380.0, 180.0]),
        (Anchor::Center, [200.0, 100.0]),
    ];
    for (anchor, pos) in cases {
        let img = draw(&gpu, &mut text, &[item("Anchor 12:34", pos, anchor, TextStyle::Minimal)], size, 1.0);
        let b = bright(&img, size).unwrap_or_else(|| panic!("{anchor:?}: nothing drawn"));
        let (x, y) = (pos[0] as u32, pos[1] as u32);
        match anchor {
            Anchor::TopLeft => assert!(b[0].abs_diff(x) <= 3 && b[1] >= y && b[1] <= y + 8, "{anchor:?} {b:?}"),
            Anchor::TopRight => assert!(b[2].abs_diff(x) <= 3 && b[1] >= y && b[1] <= y + 8, "{anchor:?} {b:?}"),
            Anchor::BottomLeft => assert!(b[0].abs_diff(x) <= 3 && b[3] <= y && b[3] + 8 >= y, "{anchor:?} {b:?}"),
            Anchor::BottomRight => assert!(b[2].abs_diff(x) <= 3 && b[3] <= y && b[3] + 8 >= y, "{anchor:?} {b:?}"),
            Anchor::Center => {
                let (cx, cy) = ((b[0] + b[2]) / 2, (b[1] + b[3]) / 2);
                assert!(cx.abs_diff(x) <= 3 && cy.abs_diff(y) <= 4, "{anchor:?} {b:?}");
            }
            Anchor::TopCenter => unreachable!(),
        }
    }
}

#[test]
fn max_width_wraps_and_centres_lines() {
    let Some(gpu) = gpu() else { return };
    let size = [400, 200];
    let mut text = TextRenderer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm);
    let mut it = item("The note of the hour wraps onto several lines", [200.0, 20.0], Anchor::TopCenter, TextStyle::Minimal);
    let one = bright(&draw(&gpu, &mut text, std::slice::from_ref(&it), size, 1.0), size).unwrap();
    it.max_width = Some(120.0);
    let img = draw(&gpu, &mut text, std::slice::from_ref(&it), size, 1.0);
    write_crate_png(&img, size, "text-wrapped.png");
    let wrapped = bright(&img, size).unwrap();
    assert!(wrapped[2] - wrapped[0] <= 122, "wrapped width {wrapped:?}");
    assert!(wrapped[3] - wrapped[1] > 2 * (one[3] - one[1]), "wrapped {wrapped:?} vs one line {one:?}");
    let cx = (wrapped[0] + wrapped[2]) / 2;
    assert!(cx.abs_diff(200) <= 4, "wrapped block centred at {cx} ({wrapped:?})");
}

#[test]
fn scale_renders_at_pixel_size_and_repeat_prepare_is_stable() {
    let Some(gpu) = gpu() else { return };
    let mut text = TextRenderer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm);
    let items = [item("Retina", [10.0, 10.0], Anchor::TopLeft, TextStyle::Glow)];
    let a = draw(&gpu, &mut text, &items, [200, 60], 1.0);
    let b = draw(&gpu, &mut text, &items, [400, 120], 2.0);
    let (ba, bb) = (bright(&a, [200, 60]).unwrap(), bright(&b, [400, 120]).unwrap());
    let (wa, wb) = (ba[2] - ba[0], bb[2] - bb[0]);
    assert!(wb.abs_diff(2 * wa) <= 3, "1x width {wa}, 2x width {wb}");
    // Same inputs again: prepare does nothing, and the frame is identical.
    let c = draw(&gpu, &mut text, &items, [400, 120], 2.0);
    assert_eq!(b, c);
}

#[test]
fn empty_and_many_items_are_fine() {
    let Some(gpu) = gpu() else { return };
    let size = [640, 400];
    let mut text = TextRenderer::new(&gpu, wgpu::TextureFormat::Rgba8Unorm);
    let img = draw(&gpu, &mut text, &[], size, 2.0);
    assert!(ink(&img, size, 0).is_none());
    let mut items = Vec::new();
    for i in 0..40 {
        let style = [TextStyle::Badge, TextStyle::Glow, TextStyle::Minimal][i % 3];
        let family = [FontFamily::Sans, FontFamily::Mono, FontFamily::Serif][i % 3];
        let mut it = item(&format!("Hub note {i}"), [20.0 + (i % 4) as f32 * 75.0, 12.0 + (i / 4) as f32 * 18.0], Anchor::TopLeft, style);
        it.family = family;
        it.size = 11.0;
        items.push(it);
    }
    let img = draw(&gpu, &mut text, &items, size, 2.0);
    write_crate_png(&img, size, "text-many.png");
    assert!(bright(&img, size).is_some());
    // Glow sprites stay small: 14 halos fit a narrow, short texture.
    let [w, h] = [text.quads.glow.width(), text.quads.glow.height()];
    assert!(w * h <= 512 * 512, "glow texture {w}x{h}");
    // Back to nothing: the frame is clean again.
    let img = draw(&gpu, &mut text, &[], size, 2.0);
    assert!(ink(&img, size, 0).is_none());
}

#[test]
fn blur_spreads_and_keeps_energy() {
    let (w, h) = (31, 31);
    let mut img = vec![0f32; w * h];
    img[15 * w + 15] = 1.0;
    gaussian_blur(&mut img, w, h, 3.0);
    let sum: f32 = img.iter().sum();
    assert!((sum - 1.0).abs() < 0.01, "sum {sum}");
    assert!(img[15 * w + 15] < 0.05 && img[15 * w + 18] > 0.005, "centre {} side {}", img[15 * w + 15], img[15 * w + 18]);
    // Symmetric.
    assert!((img[15 * w + 12] - img[15 * w + 18]).abs() < 1e-6 && (img[12 * w + 15] - img[18 * w + 15]).abs() < 1e-6);
}

fn write_crate_png(img: &[u8], size: [u32; 2], name: &str) {
    crate::write_png(&out_dir().join(name), size, img).unwrap();
}
