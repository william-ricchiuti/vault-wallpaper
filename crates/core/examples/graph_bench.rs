//! Times graph building on an already-scanned vault: `cargo run --release --example graph_bench -- <vault> [runs]`
use std::time::Instant;
use vw_core::vault::{Vault, VaultOptions};
fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("vault path");
    let runs: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(20);
    let vault = Vault::scan(path, VaultOptions::default());
    let mut best = f64::MAX;
    for _ in 0..runs {
        let t = Instant::now();
        let g = vault.graph();
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(g);
    }
    println!("graph: best of {runs} = {best:.1} ms for {} notes", vault.len());
}
