//! Renders a vault off-screen to a PNG: scan, lay out (or use the cache), frame, draw.
//!   cargo run --release -p vw-render --example render_vault -- <vault> <out.png> [width height scale]
use std::collections::HashMap;
use std::time::Instant;
use vw_core::layout::{LayoutGraph, LayoutOptions, Simulation, place, settle};
use vw_core::vault::{Vault, VaultOptions};
use vw_render::{Framing, FramingOptions, Gpu, SceneStyle, build_instances, render_rgba, write_png};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (w, h, scale): (u32, u32, f32) = if a.len() >= 5 { (a[2].parse().unwrap(), a[3].parse().unwrap(), a[4].parse().unwrap()) } else { (3024, 1964, 2.0) };
    let graph = Vault::scan(&a[0], VaultOptions::default()).graph();
    let lg = LayoutGraph::from_graph(&graph);
    let opts = LayoutOptions::default();
    let p = place(&lg, &HashMap::new(), &opts);
    let mut sim = Simulation::new(&lg, &opts, p.x, p.y, p.vx, p.vy, 1.0);
    settle(&mut sim, 600);
    let positions: Vec<[f32; 2]> = sim.x.iter().zip(&sim.y).map(|(&x, &y)| [x as f32, y as f32]).collect();
    let (pw, ph) = (w as f32 / scale, h as f32 / scale);
    let framing = Framing::fit(&positions, pw, ph, &FramingOptions::default());
    let style = SceneStyle::default();
    let (nodes, edges) = build_instances(&lg, &positions, &framing, &style);
    let gpu = Gpu::headless().expect("gpu");
    let t = Instant::now();
    let rgba = render_rgba(&gpu, [w, h], scale, &style, &nodes, &edges);
    eprintln!("{} nodes, {} links rendered at {w}x{h} in {} ms (incl. pipeline setup and readback)", nodes.len(), edges.len(), t.elapsed().as_millis());
    write_png(std::path::Path::new(&a[1]), [w, h], &rgba).unwrap();
}
