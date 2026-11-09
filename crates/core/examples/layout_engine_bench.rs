//! Engine timings on a vault: cold layout, relaunch from cache, and one edit.
//!   cargo run --release --example layout_engine_bench -- <vault>
use std::sync::mpsc;
use std::time::{Duration, Instant};
use vw_core::layout::{LayoutEngine, LayoutOptions};
use vw_core::vault::{Vault, VaultOptions};
fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("vault"));
    let cache = std::env::temp_dir().join(format!("vw-layout-bench-{}.json", std::process::id()));
    let graph = Vault::scan(&root, VaultOptions::default()).graph();
    let (tx, rx) = mpsc::channel();
    let engine = LayoutEngine::spawn(LayoutOptions::default(), Some(cache.clone()), Duration::from_millis(100), move |f| { let _ = tx.send(f); }).expect("spawn layout thread");
    let wait = |rx: &mpsc::Receiver<vw_core::layout::LayoutFrame>, t: Instant, label: &str| {
        let mut frames = 0;
        loop { let f = rx.recv().unwrap(); frames += 1; if f.settled { println!("{label}: {} ms, {frames} frames", t.elapsed().as_millis()); break; } }
    };
    let t = Instant::now(); engine.set_graph(graph.clone()); wait(&rx, t, "cold layout");
    drop(engine);
    let (tx, rx) = mpsc::channel();
    let engine = LayoutEngine::spawn(LayoutOptions::default(), Some(cache.clone()), Duration::from_millis(100), move |f| { let _ = tx.send(f); }).expect("spawn layout thread");
    let t = Instant::now(); engine.set_graph(graph.clone()); wait(&rx, t, "relaunch from cache");
    let mut edited = graph.clone();
    let target = edited.nodes[0].id.clone();
    edited.nodes.push(vw_core::vault::Node { id: "zz-new-note".into(), label: None, tag: None, tags: None, mtime: Some(1.0), folder: None, ghost: false });
    edited.links.push(vw_core::vault::Link { source: "zz-new-note".into(), target });
    let t = Instant::now(); engine.set_graph(edited); wait(&rx, t, "one new note");
    let _ = std::fs::remove_file(&cache);
}
