//! Watching the vault and publishing a fresh [`Graph`] after edits.
//!
//! FSEvents batches changes; the debouncer adds a short quiet window on top so a burst
//! (a save that writes a temp file and renames it, a sync client landing many files)
//! becomes one rebuild.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};

use super::{Graph, Vault};

/// How long the vault must be quiet before a batch of changes is applied.
pub const DEBOUNCE: Duration = Duration::from_millis(100);

/// Keeps the watch alive; dropping it stops watching.
pub struct VaultWatcher {
    _debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
    _worker: thread::JoinHandle<()>,
}

/// Starts watching `vault`. `on_graph` runs on a background thread with a new graph
/// each time a batch of changes alters it.
pub fn watch(mut vault: Vault, on_graph: impl Fn(Graph) + Send + 'static) -> notify::Result<VaultWatcher> {
    // FSEvents reports real paths (/private/var/... for /var/...), so map event paths
    // back onto the root the vault was opened with.
    let root = vault.root().to_path_buf();
    let real_root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
    // `None` asks for a full rescan; `Some` carries changed paths.
    let (tx, rx) = mpsc::channel::<Option<Vec<PathBuf>>>();

    let mut debouncer = new_debouncer(DEBOUNCE, None, move |result: DebounceEventResult| match result {
        Ok(events) => {
            if events.iter().any(|e| e.event.need_rescan()) {
                let _ = tx.send(None);
                return;
            }
            let mut paths: Vec<PathBuf> = events
                .into_iter()
                .flat_map(|e| e.event.paths)
                .map(|p| rebase(&p, &real_root, &root))
                .collect();
            paths.sort();
            paths.dedup();
            if std::env::var_os("VW_WATCH_DEBUG").is_some() {
                eprintln!("watch batch: {paths:?}");
            }
            if !paths.is_empty() {
                let _ = tx.send(Some(paths));
            }
        }
        Err(errors) => {
            for e in errors {
                eprintln!("watcher error: {e}");
            }
        }
    })?;
    debouncer.watch(vault.root(), RecursiveMode::Recursive)?;

    let worker = thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            // Fold in anything that queued up while the previous batch was applied.
            let mut rescan = first.is_none();
            let mut paths = first.unwrap_or_default();
            while let Ok(more) = rx.try_recv() {
                match more {
                    Some(p) => paths.extend(p),
                    None => rescan = true,
                }
            }
            let changed = if rescan { vault.rescan() } else { vault.refresh_paths(&paths) };
            if changed {
                on_graph(vault.graph());
            }
        }
    });
    Ok(VaultWatcher { _debouncer: debouncer, _worker: worker })
}

fn rebase(path: &Path, real_root: &Path, root: &Path) -> PathBuf {
    match path.strip_prefix(real_root) {
        Ok(rest) => root.join(rest),
        Err(_) => path.to_path_buf(),
    }
}
