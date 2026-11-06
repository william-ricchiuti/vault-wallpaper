//! Prints the graph for a vault as JSON, for the parity test and for poking at vaults.
//!
//!   vw-dump <vault> [--ignore <path>]... [--no-unresolved] [--timing] [--excerpts]

use std::time::Instant;

use vw_core::vault::{GraphOptions, Vault, VaultOptions, note_excerpt};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut vault_path = None;
    let mut opts = VaultOptions::default();
    let mut timing = false;
    let mut excerpts = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--ignore" => opts.ignore_paths.push(args.next().expect("--ignore needs a value")),
            "--no-unresolved" => opts.graph = GraphOptions { show_unresolved_links: false },
            "--timing" => timing = true,
            "--excerpts" => excerpts = true,
            _ => vault_path = Some(a),
        }
    }
    let Some(vault_path) = vault_path else {
        eprintln!("usage: vw-dump <vault> [--ignore <path>]... [--no-unresolved] [--timing]");
        std::process::exit(2);
    };
    let t0 = Instant::now();
    let vault = Vault::scan(vault_path, opts);
    let t1 = Instant::now();
    let graph = vault.graph();
    let t2 = Instant::now();
    if timing {
        eprintln!(
            "scan+parse {:.1} ms, graph {:.1} ms: {} notes, {} nodes, {} links",
            (t1 - t0).as_secs_f64() * 1000.0,
            (t2 - t1).as_secs_f64() * 1000.0,
            vault.len(),
            graph.nodes.len(),
            graph.links.len()
        );
    }
    if excerpts {
        // Path -> excerpt for every note, for the excerpt half of the parity test.
        let mut out = std::collections::BTreeMap::new();
        for path in vault.paths() {
            let content = String::from_utf8_lossy(&std::fs::read(path).expect("read")).into_owned();
            out.insert(path.to_string_lossy().into_owned(), note_excerpt(&content, 220));
        }
        println!("{}", serde_json::to_string(&out).expect("serialize"));
        return;
    }
    println!("{}", serde_json::to_string(&graph).expect("serialize"));
}
