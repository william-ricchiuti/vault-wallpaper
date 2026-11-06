//! Lays out a vault on this thread and reports how long it took.
//!   cargo run --release --example layout_bench -- <vault> [--kind force|rings|timeline|galaxy] [--no-cluster] [--out positions.json]
use std::collections::HashMap;
use std::time::Instant;
use vw_core::layout::{LayoutGraph, LayoutKind, LayoutOptions, Simulation, place, settle};
use vw_core::vault::{Vault, VaultOptions};
fn main() {
    let mut args = std::env::args().skip(1);
    let mut vault = None;
    let mut opts = LayoutOptions::default();
    let mut out = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--kind" => opts.kind = LayoutKind::parse(&args.next().unwrap()).expect("kind"),
            "--no-cluster" => opts.cluster_by_tag = false,
            "--out" => out = args.next(),
            _ => vault = Some(a),
        }
    }
    let graph = Vault::scan(vault.expect("vault path"), VaultOptions::default()).graph();
    let lg = LayoutGraph::from_graph(&graph);
    let p = place(&lg, &HashMap::new(), &opts);
    let t0 = Instant::now();
    let mut sim = Simulation::new(&lg, &opts, p.x, p.y, p.vx, p.vy, 1.0);
    let ticks = settle(&mut sim, 600);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!("rust {}: {} nodes, {} links: {ticks} ticks, {ms:.0} ms ({:.2} ms/tick)", opts.kind.name(), lg.len(), lg.links.len(), ms / ticks as f64);
    if let Some(out) = out {
        let map: HashMap<&str, [f64; 2]> = lg.id.iter().enumerate().map(|(i, id)| (id.as_str(), [sim.x[i], sim.y[i]])).collect();
        std::fs::write(out, serde_json::to_string(&map).unwrap()).unwrap();
    }
}
