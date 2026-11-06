//! Time from saving a note to receiving the updated graph, on any vault (the note is
//! created and then removed): `cargo run --release --example watch_bench -- <vault>`
use std::sync::mpsc;
use std::time::{Duration, Instant};
use vw_core::vault::{Vault, VaultOptions, watch};
fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("vault path"));
    let vault = Vault::scan(&root, VaultOptions::default());
    let n = vault.len();
    let (tx, rx) = mpsc::channel();
    let _w = watch(vault, move |g| {
        let _ = tx.send((Instant::now(), g.nodes.len()));
    })
    .expect("watch");
    std::thread::sleep(Duration::from_millis(500));
    while rx.try_recv().is_ok() {}
    let probe = root.join("zz-watch-bench-probe.md");
    let mut times = Vec::new();
    for i in 0..5 {
        let t0 = Instant::now();
        std::fs::write(&probe, format!("probe {i} [[Note 1]] #bench")).unwrap();
        let (t1, _) = rx.recv_timeout(Duration::from_secs(5)).expect("graph update");
        times.push((t1 - t0).as_millis());
        std::thread::sleep(Duration::from_millis(300));
        while rx.try_recv().is_ok() {}
    }
    std::fs::remove_file(&probe).unwrap();
    println!("{n} notes: save -> new graph {times:?} ms");
}
