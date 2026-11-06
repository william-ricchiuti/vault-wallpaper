//! Draws a laid-out graph to a PNG, for eyeballing layouts side by side.
//!   cargo run --release --example draw_layout -- <graph.json from vw-dump> <positions.json> <out.png>
use std::collections::HashMap;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let graph: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&a[0]).unwrap()).unwrap();
    let pos: HashMap<String, [f64; 2]> = serde_json::from_str(&std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    let (w, h) = (1400usize, 900usize);
    let mut img = vec![[8.0f32, 8.0, 14.0]; w * h];
    let pts: Vec<[f64; 2]> = pos.values().copied().collect();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &pts { x0 = x0.min(p[0]); y0 = y0.min(p[1]); x1 = x1.max(p[0]); y1 = y1.max(p[1]); }
    let s = ((w as f64 - 60.0) / (x1 - x0)).min((h as f64 - 60.0) / (y1 - y0));
    let map = |p: [f64; 2]| ((p[0] - (x0 + x1) / 2.0) * s + w as f64 / 2.0, (p[1] - (y0 + y1) / 2.0) * s + h as f64 / 2.0);
    let mut blend = |x: i64, y: i64, c: [f32; 3], a: f32| {
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 { return; }
        let px = &mut img[y as usize * w + x as usize];
        for k in 0..3 { px[k] += (c[k] - px[k]) * a; }
    };
    let color = |tag: Option<&str>| -> [f32; 3] {
        let Some(t) = tag else { return [150.0, 150.0, 170.0] };
        let hsh = t.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
        let hue = (hsh % 360) as f32 / 60.0;
        let x = 1.0 - (hue % 2.0 - 1.0).abs();
        let (r, g, b) = match hue as u32 { 0 => (1.0, x, 0.0), 1 => (x, 1.0, 0.0), 2 => (0.0, 1.0, x), 3 => (0.0, x, 1.0), 4 => (x, 0.0, 1.0), _ => (1.0, 0.0, x) };
        [80.0 + r * 175.0, 80.0 + g * 175.0, 80.0 + b * 175.0]
    };
    let nodes = graph["nodes"].as_array().unwrap();
    let tag_of: HashMap<&str, Option<&str>> = nodes.iter().map(|n| (n["id"].as_str().unwrap(), n["tag"].as_str())).collect();
    for l in graph["links"].as_array().unwrap() {
        let (Some(&p), Some(&q)) = (pos.get(l["source"].as_str().unwrap()), pos.get(l["target"].as_str().unwrap())) else { continue };
        let ((ax, ay), (bx, by)) = (map(p), map(q));
        let steps = ((bx - ax).abs().max((by - ay).abs()) as usize).max(1);
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            blend((ax + (bx - ax) * t) as i64, (ay + (by - ay) * t) as i64, [120.0, 110.0, 200.0], 0.08);
        }
    }
    let mut deg: HashMap<&str, f64> = HashMap::new();
    for l in graph["links"].as_array().unwrap() {
        *deg.entry(l["source"].as_str().unwrap()).or_default() += 1.0;
        *deg.entry(l["target"].as_str().unwrap()).or_default() += 1.0;
    }
    for (id, &p) in &pos {
        let (cx, cy) = map(p);
        let r = (1.0 + deg.get(id.as_str()).copied().unwrap_or(0.0).sqrt() * 0.5).min(7.0);
        let c = color(tag_of.get(id.as_str()).copied().flatten());
        let ri = r.ceil() as i64;
        for dy in -ri..=ri { for dx in -ri..=ri {
            let d = ((dx * dx + dy * dy) as f64).sqrt();
            if d <= r { blend(cx as i64 + dx, cy as i64 + dy, c, if d > r - 1.0 { 0.5 } else { 0.9 }); }
        }}
    }
    let bytes: Vec<u8> = img.iter().flat_map(|p| p.map(|v| v.clamp(0.0, 255.0) as u8)).collect();
    let f = std::fs::File::create(&a[2]).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.write_header().unwrap().write_image_data(&bytes).unwrap();
}
