//! Layout engine behaviour: deterministic, settles, saves and reuses its cache, and
//! relaxes only around an edit.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use vw_core::layout::{LayoutEngine, LayoutFrame, LayoutGraph, LayoutKind, LayoutOptions, Simulation, place, settle};
use vw_core::vault::{Graph, Vault, VaultOptions};

/// A vault of `n` notes in a few tagged clusters with cross links.
fn make_vault(root: &Path, n: usize) {
    for i in 0..n {
        let tag = format!("t{}", i % 5);
        let links: String = (1..4).map(|k| format!("[[N{}]] ", (i * 7 + k * 13) % n)).collect();
        fs::write(root.join(format!("N{i}.md")), format!("#{tag}\n{links}")).unwrap();
    }
}

fn graph_of(root: &Path) -> Graph {
    Vault::scan(root, VaultOptions::default()).graph()
}

fn opts() -> LayoutOptions {
    LayoutOptions { now_ms: 1.8e12, ..LayoutOptions::default() }
}

fn settled_frame(rx: &mpsc::Receiver<LayoutFrame>) -> (LayoutFrame, Duration, usize) {
    let start = Instant::now();
    let mut frames = 0;
    loop {
        let f = rx.recv_timeout(Duration::from_secs(20)).expect("layout frame");
        frames += 1;
        if f.settled {
            return (f, start.elapsed(), frames);
        }
    }
}

fn by_id(f: &LayoutFrame) -> HashMap<String, [f32; 2]> {
    f.ids.iter().cloned().zip(f.positions.iter().copied()).collect()
}

#[test]
fn same_graph_same_layout() {
    let dir = tempfile::tempdir().unwrap();
    make_vault(dir.path(), 200);
    let lg = LayoutGraph::from_graph(&graph_of(dir.path()));
    for kind in [LayoutKind::Force, LayoutKind::Rings, LayoutKind::Timeline, LayoutKind::Galaxy] {
        let o = LayoutOptions { kind, ..opts() };
        let run = || {
            let p = place(&lg, &HashMap::new(), &o);
            let mut sim = Simulation::new(&lg, &o, p.x, p.y, p.vx, p.vy, 1.0);
            settle(&mut sim, 600);
            assert!(sim.is_settled(), "{kind:?} did not settle");
            assert!(sim.x.iter().chain(&sim.y).all(|v| v.is_finite()), "{kind:?} produced NaN");
            (sim.x, sim.y)
        };
        assert_eq!(run(), run(), "{kind:?} is not deterministic");
    }
}

#[test]
fn engine_caches_and_relaxes_locally() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir(&vault).unwrap();
    make_vault(&vault, 400);
    let cache = dir.path().join("layout.json");

    // Cold start: lays out from scratch, then saves the cache.
    let (tx, rx) = mpsc::channel();
    let engine = LayoutEngine::spawn(opts(), Some(cache.clone()), Duration::from_millis(50), move |f| {
        let _ = tx.send(f);
    });
    engine.set_graph(graph_of(&vault));
    let (cold, took, frames) = settled_frame(&rx);
    eprintln!("cold layout: {} ms, {frames} frames", took.as_millis());
    assert!(cache.exists(), "cache not written");
    drop(engine);

    // Relaunch on the same vault: the first frame is the cached, settled layout.
    let (tx, rx) = mpsc::channel();
    let engine = LayoutEngine::spawn(opts(), Some(cache.clone()), Duration::from_millis(50), move |f| {
        let _ = tx.send(f);
    });
    engine.set_graph(graph_of(&vault));
    let (warm, took, frames) = settled_frame(&rx);
    eprintln!("relaunch: {} ms, {frames} frame(s)", took.as_millis());
    assert_eq!(frames, 1, "relaunch should publish one settled frame and do no work");
    assert_eq!(by_id(&cold), by_id(&warm));

    // An edit: one new note linked to N5. It appears next to N5 and the rest of the
    // graph barely moves.
    fs::write(vault.join("Fresh.md"), "#t1\n[[N5]]").unwrap();
    engine.set_graph(graph_of(&vault));
    let (edited, took, _) = settled_frame(&rx);
    eprintln!("edit relax: {} ms", took.as_millis());
    let before = by_id(&warm);
    let after = by_id(&edited);
    let dist = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let mut moved: Vec<f32> = before.iter().map(|(id, p)| dist(*p, after[id])).collect();
    moved.sort_by(f32::total_cmp);
    let median = moved[moved.len() / 2];
    let near = dist(after["Fresh"], after["N5"]);
    eprintln!("median move {median:.2} px, new note {near:.1} px from its link");
    assert!(median < 5.0, "existing notes moved a median {median} px");
    assert!(near < 120.0, "new note landed {near} px from its link");
}
