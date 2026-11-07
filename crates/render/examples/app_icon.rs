//! Draws the app icon: the menu-bar glyph (three linked dots, see `crates/app/src/menu.rs`)
//! lit up on a dark macOS-style rounded square. Pure CPU, no GPU needed.
//!
//!   cargo run --release -p vw-render --example app_icon -- packaging/icon/icon-1024.png
//!
//! `packaging/build-app.sh` turns the PNG into `AppIcon.icns`.

const S: usize = 1024;

/// The menu glyph's dots in its 36-pixel grid: (x, y, radius).
const GLYPH: [(f32, f32, f32); 3] = [(9.0, 25.0, 5.0), (27.0, 25.0, 5.0), (18.0, 9.0, 6.0)];
const LINKS: [(usize, usize); 3] = [(0, 1), (0, 2), (1, 2)];

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn hex(h: u32) -> [f32; 3] {
    [
        ((h >> 16) & 255) as f32 / 255.0,
        ((h >> 8) & 255) as f32 / 255.0,
        (h & 255) as f32 / 255.0,
    ]
}

/// Composites `c` at coverage `a` over `dst` (premultiplied colour).
fn over(dst: &mut [f32; 4], c: [f32; 3], a: f32) {
    let a = a.clamp(0.0, 1.0);
    for i in 0..3 {
        dst[i] = dst[i] * (1.0 - a) + c[i] * a;
    }
    dst[3] = dst[3] + a * (1.0 - dst[3]);
}

/// Additive light (for glow): brightens without changing coverage.
fn add(dst: &mut [f32; 4], c: [f32; 3], a: f32) {
    for i in 0..3 {
        dst[i] += c[i] * a.max(0.0);
    }
}

fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((p.0 - a.0 - dx * t).powi(2) + (p.1 - a.1 - dy * t).powi(2)).sqrt()
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "packaging/icon/icon-1024.png".into());

    // Apple's icon grid: an 824-point body centred on the 1024 canvas.
    let (c, half, n) = (512.0f32, 412.0f32, 5.0f32);
    // Glyph: 36-grid -> icon space, centred slightly above the middle.
    let k = 17.0f32;
    let (gx, gy) = (18.0f32, 17.0f32);
    let to_icon = |x: f32, y: f32| (c + (x - gx) * k, c + (y - gy) * k + 8.0);
    let dots: Vec<((f32, f32), f32)> = GLYPH
        .iter()
        .map(|&(x, y, r)| (to_icon(x, y), r * k))
        .collect();

    let top = hex(0x2a1d63);
    let bottom = hex(0x0c0a1e);
    let violet = hex(0x7c5cff);
    let lilac = hex(0xc9bcff);
    let teal = hex(0x5ce1e6);
    let pink = hex(0xff7ab8);
    let node_colors = [teal, pink, lilac];

    // A few faint background notes, so it reads as a bigger graph behind the glyph.
    let faint: [(f32, f32, f32); 7] = [
        (250.0, 300.0, 9.0),
        (780.0, 270.0, 11.0),
        (300.0, 760.0, 8.0),
        (760.0, 780.0, 10.0),
        (190.0, 540.0, 7.0),
        (840.0, 520.0, 8.0),
        (520.0, 860.0, 7.0),
    ];

    let mut img = vec![0u8; S * S * 4];
    for y in 0..S {
        for x in 0..S {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let mut px = [0.0f32; 4];

            // Soft drop shadow under the body.
            let (sx, sy) = ((p.0 - c).abs() / half, (p.1 - c - 12.0).abs() / half);
            let sd = (sx.powf(n) + sy.powf(n)).powf(1.0 / n) * half - half;
            over(
                &mut px,
                [0.0; 3],
                0.22 * (1.0 - (sd / 18.0).clamp(0.0, 1.0)),
            );

            // Body: superellipse, analytic anti-aliasing.
            let (ux, uy) = ((p.0 - c).abs() / half, (p.1 - c).abs() / half);
            let d = (ux.powf(n) + uy.powf(n)).powf(1.0 / n) * half - half;
            let body = (0.5 - d).clamp(0.0, 1.0);
            if body > 0.0 {
                let t = ((p.1 - (c - half)) / (2.0 * half)).clamp(0.0, 1.0);
                let mut col = mix(top, bottom, t);
                // Violet bloom behind the glyph.
                let r = ((p.0 - c).powi(2) + (p.1 - c + 20.0).powi(2)).sqrt();
                let bloom = (-(r / 300.0).powi(2)).exp() * 0.55;
                col = mix(col, violet, bloom * 0.6);
                let mut layer = [col[0], col[1], col[2], 1.0];

                // Faint background graph.
                for (i, &(fx, fy, fr)) in faint.iter().enumerate() {
                    let nearest = dots[i % 3].0;
                    let ld = seg_dist(p, (fx, fy), nearest);
                    over(&mut layer, lilac, 0.10 * (2.0 - ld).clamp(0.0, 1.0));
                    let dd = ((p.0 - fx).powi(2) + (p.1 - fy).powi(2)).sqrt();
                    over(&mut layer, lilac, 0.35 * (fr + 0.5 - dd).clamp(0.0, 1.0));
                }

                // Links of the glyph.
                for &(i, j) in &LINKS {
                    let ld = seg_dist(p, dots[i].0, dots[j].0);
                    add(&mut layer, violet, 0.35 * (-(ld / 40.0).powi(2)).exp());
                    over(
                        &mut layer,
                        mix(lilac, [1.0; 3], 0.3),
                        0.9 * (13.0 + 0.5 - ld).clamp(0.0, 1.0),
                    );
                }
                // Nodes: glow, then a solid disc with a bright core.
                for (idx, &((nx, ny), r)) in dots.iter().enumerate() {
                    let dd = ((p.0 - nx).powi(2) + (p.1 - ny).powi(2)).sqrt();
                    let col = node_colors[idx];
                    add(
                        &mut layer,
                        col,
                        0.55 * (-((dd - r).max(0.0) / 55.0).powi(2)).exp(),
                    );
                    let disc = (r + 0.5 - dd).clamp(0.0, 1.0);
                    let core = mix(
                        col,
                        [1.0; 3],
                        (1.0 - dd / r).clamp(0.0, 1.0).powf(1.5) * 0.85,
                    );
                    over(&mut layer, core, disc);
                }
                // Thin top highlight on the rim.
                let rim = (1.0 - (-d / 3.0).clamp(0.0, 1.0)) * (1.0 - t).powi(3) * 0.25;
                over(&mut layer, [1.0; 3], rim);
                over(
                    &mut px,
                    [layer[0].min(1.0), layer[1].min(1.0), layer[2].min(1.0)],
                    body,
                );
            }

            let i = (y * S + x) * 4;
            let a = px[3].clamp(0.0, 1.0);
            for ch in 0..3 {
                // Store straight (non-premultiplied) colour.
                let v = if a > 0.0 {
                    (px[ch] / a.max(1e-4)).min(1.0)
                } else {
                    0.0
                };
                img[i + ch] = (v * 255.0 + 0.5) as u8;
            }
            img[i + 3] = (a * 255.0 + 0.5) as u8;
        }
    }

    let file = std::fs::File::create(&out).expect("create output");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), S as u32, S as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&img))
        .expect("write png");
    println!("wrote {out}");
}
