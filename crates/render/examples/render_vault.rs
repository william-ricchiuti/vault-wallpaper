//! Renders a vault off-screen to a PNG with a config or a web-version preset.
//!   cargo run --release -p vw-render --example render_vault -- <vault> <out.png>
//!       [--preset <name> --presets <presets.json>] [--config <config.json>] [--size W H SCALE]
//!       [--theme <id>] [--burst <t>]
//! `--burst t` overlays the theme's edit burst for the best-connected note at time t
//! (0..1), as the window server would show it.
use std::collections::HashMap;
use std::time::Instant;
use vw_core::config::Config;
use vw_core::layout::{LayoutGraph, Simulation, place, settle};
use vw_core::vault::Vault;
use vw_render::{BurstRenderer, EditBurst, Framing, FramingOptions, Gpu, LookContext, Theme, burst, derive_look, render_rgba, write_png};
fn main() {
    let mut it = std::env::args().skip(1);
    let vault = it.next().expect("vault");
    let out = it.next().expect("out.png");
    let (mut w, mut h, mut scale) = (3024u32, 1964u32, 2.0f32);
    let mut cfg = Config::default();
    let (mut preset, mut presets) = (None, None);
    let mut burst_t: Option<f32> = None;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--preset" => preset = it.next(),
            "--theme" => {
                cfg.apply_patch(&serde_json::json!({ "theme": it.next().unwrap() })).expect("theme");
            }
            "--presets" => presets = it.next(),
            "--burst" => burst_t = Some(it.next().unwrap().parse().unwrap()),
            "--config" => cfg = vw_core::config::load(std::path::Path::new(&it.next().unwrap())).expect("config").config,
            "--size" => {
                w = it.next().unwrap().parse().unwrap();
                h = it.next().unwrap().parse().unwrap();
                scale = it.next().unwrap().parse().unwrap();
            }
            other => panic!("unknown argument {other}"),
        }
    }
    if let Some(name) = preset {
        let list: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(presets.expect("--presets")).unwrap()).unwrap();
        let p = list.as_array().unwrap().iter().find(|p| p["name"] == name.as_str()).expect("preset not found");
        let issues = cfg.apply_patch(&p["config"]).expect("preset applies");
        for i in issues {
            eprintln!("preset note: {i}");
        }
    }
    let graph = Vault::scan(&vault, cfg.vault_options()).graph();
    let lg = LayoutGraph::from_graph(&graph);
    let opts = cfg.layout_options();
    let p = place(&lg, &HashMap::new(), &opts);
    let mut sim = Simulation::new(&lg, &opts, p.x, p.y, p.vx, p.vy, 1.0);
    settle(&mut sim, 600);
    let positions: Vec<[f32; 2]> = sim.x.iter().zip(&sim.y).map(|(&x, &y)| [x as f32, y as f32]).collect();
    let points = [w as f32 / scale, h as f32 / scale];
    let framing = Framing::fit_graph(&positions, &lg.degree, points[0], points[1], &FramingOptions::from_config(&cfg));
    let theme = Theme::from_config(&cfg);
    let style = theme.style(&cfg, false);
    let look = derive_look(&graph, &lg, &cfg, &LookContext { now_ms: 1.8e12, light: false });
    let data = theme.build(&lg, &look, &positions, &framing, &style, points);
    let gpu = Gpu::headless().expect("gpu");
    let t = Instant::now();
    let mut rgba = render_rgba(&gpu, [w, h], scale, &style, &data, 1.0);
    eprintln!("{} nodes, {} links, {} halos rendered at {w}x{h} in {} ms (incl. setup and readback)", data.nodes.len(), data.edges.len(), data.halos.len(), t.elapsed().as_millis());
    if let Some(t) = burst_t {
        let moving = Config { motion_mode: vw_core::config::MotionMode::Balanced, ..cfg.clone() };
        let edit = theme.edit_burst(&moving).unwrap_or(EditBurst { duration: 2.0, fps: 12.0, resolution: 1.0 });
        let hub = (0..lg.len()).filter(|&i| !lg.ghost[i]).max_by_key(|&i| lg.degree[i]).expect("a note");
        let actors = theme.burst_actors(&lg, &look, &positions, &framing, hub, &edit);
        if let Some(baked) = BurstRenderer::new(&gpu).bake(&gpu, &actors, scale * edit.resolution) {
            eprintln!("burst for {}: {} actors, atlas {}x{} ({} KB)", lg.id[hub], actors.len(), baked.size[0], baked.size[1], baked.bytes() / 1024);
            let over = burst::composite_at(&baked, t, [w, h], scale);
            for (d, o) in rgba.chunks_mut(4).zip(over.chunks(4)) {
                let a = f32::from(o[3]) / 255.0;
                for k in 0..3 {
                    d[k] = (f32::from(o[k]) + f32::from(d[k]) * (1.0 - a)).round().min(255.0) as u8;
                }
            }
        }
    }
    write_png(std::path::Path::new(&out), [w, h], &rgba).unwrap();
}
