//! The watcher sees creates, edits, deletes and folder moves, ignores hidden files, and
//! publishes the change quickly. Prints how long each change took to arrive.

use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use vw_core::vault::{Graph, Vault, VaultOptions, watch};

fn wait_for(rx: &mpsc::Receiver<Graph>, what: &str, pred: impl Fn(&Graph) -> bool) -> Duration {
    let start = Instant::now();
    loop {
        let left = Duration::from_secs(5).saturating_sub(start.elapsed());
        match rx.recv_timeout(left) {
            Ok(g) if pred(&g) => {
                let took = start.elapsed();
                eprintln!("{what}: {} ms", took.as_millis());
                return took;
            }
            Ok(_) => {}
            Err(_) => panic!("timed out waiting for: {what}"),
        }
    }
}

fn has_node(g: &Graph, id: &str) -> bool {
    g.nodes.iter().any(|n| n.id == id && !n.ghost)
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn watcher_tracks_changes() {
    // tempdir lives under /var, which FSEvents reports as /private/var: this also
    // checks that event paths are mapped back onto the root.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "A.md", "#start");
    write(root, "folder/C.md", "[[A]]");

    let vault = Vault::scan(root, VaultOptions::default());
    assert_eq!(vault.len(), 2);
    let (tx, rx) = mpsc::channel();
    let _w = watch(vault, move |g| {
        let _ = tx.send(g);
    })
    .unwrap();
    // Let FSEvents start before the first change.
    std::thread::sleep(Duration::from_millis(300));

    let mut worst = Duration::ZERO;
    write(root, "B.md", "[[A]]");
    worst = worst.max(wait_for(&rx, "create note", |g| {
        has_node(g, "B") && g.links.iter().any(|l| l.source == "B" && l.target == "A")
    }));

    write(root, "A.md", "#start #added");
    worst = worst.max(wait_for(&rx, "edit tags", |g| {
        g.nodes.iter().any(|n| n.id == "A" && n.tags.as_deref() == Some(&["start".into(), "added".into()][..]))
    }));

    fs::remove_file(root.join("B.md")).unwrap();
    worst = worst.max(wait_for(&rx, "delete note", |g| !has_node(g, "B")));

    fs::rename(root.join("folder"), root.join("moved")).unwrap();
    worst = worst.max(wait_for(&rx, "rename folder", |g| {
        g.nodes.iter().any(|n| n.id == "C" && n.folder.as_deref() == Some("moved"))
    }));

    // Hidden folders never reach the graph.
    write(root, ".obsidian/workspace.md", "[[A]]");
    write(root, "Visible.md", "after the hidden one");
    worst = worst.max(wait_for(&rx, "hidden ignored", |g| {
        has_node(g, "Visible") && !has_node(g, "workspace")
    }));

    eprintln!("slowest change: {} ms", worst.as_millis());
    assert!(worst < Duration::from_secs(1), "changes took up to {worst:?}");
}
