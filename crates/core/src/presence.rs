//! What you're doing in Obsidian, from the optional companion plugin.
//!
//! The plugin (`obsidian-plugin/` in this repo) writes a small JSON file inside the
//! vault, [`STATE_PATH`], when the open note changes or a note is saved. The vault
//! reader skips hidden folders by design, so this file is watched on its own. There
//! is no port and no network: the file is the whole protocol.
//!
//! A state older than [`STALE_AFTER`] is ignored (Obsidian quit without saying so,
//! or the plugin was removed), and so is a missing or unreadable file: the wallpaper
//! then behaves exactly as it does without the plugin.

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use serde::Deserialize;

use crate::vault::Graph;

/// The plugin's folder, relative to the vault root.
pub const PLUGIN_DIR: &str = ".obsidian/plugins/vault-wallpaper";
/// The state file, relative to the vault root.
pub const STATE_PATH: &str = ".obsidian/plugins/vault-wallpaper/state.json";
/// The plugin refreshes the state every two minutes while Obsidian is open; older
/// than this means Obsidian is gone.
pub const STALE_AFTER: Duration = Duration::from_secs(5 * 60);
/// The state-file versions this reader understands.
pub const VERSION: u32 = 1;

/// The plugin's state file. Paths are vault-relative with "/" separators, as
/// Obsidian reports them ("Folder/Note.md").
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceState {
    pub version: u32,
    /// The note in the active editor, if any.
    #[serde(default)]
    pub active: Option<String>,
    /// Obsidian's resolved outgoing links of the active note.
    #[serde(default)]
    pub links: Vec<String>,
    /// The note last saved while open in Obsidian, and when (ms since the epoch).
    #[serde(default)]
    pub saved: Option<String>,
    #[serde(default)]
    pub saved_at: Option<f64>,
    /// When the plugin wrote this (ms since the epoch).
    pub updated: f64,
}

impl PresenceState {
    /// Parses the file's text. Unknown fields are ignored; an unknown version is an error.
    pub fn parse(text: &str) -> Result<PresenceState, String> {
        let s: PresenceState = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if s.version != VERSION {
            return Err(format!("unsupported state version {}", s.version));
        }
        if !s.updated.is_finite() {
            return Err("bad timestamp".into());
        }
        Ok(s)
    }

    /// When this state stops counting, in ms since the epoch.
    pub fn fresh_until_ms(&self) -> f64 {
        self.updated + STALE_AFTER.as_secs_f64() * 1000.0
    }

    /// True while the state is recent enough to act on. A timestamp a little in the
    /// future (clock skew) counts as fresh.
    pub fn is_fresh(&self, now_ms: f64) -> bool {
        now_ms < self.fresh_until_ms()
    }

    /// The save as a (path, time) pair, when there is one.
    pub fn save(&self) -> Option<(&str, f64)> {
        Some((self.saved.as_deref()?, self.saved_at?))
    }
}

/// Now in ms since the epoch.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// Reads the state file of the vault at `root`. `None` when it is missing or invalid.
pub fn read(root: &Path) -> Option<PresenceState> {
    let text = std::fs::read_to_string(root.join(STATE_PATH)).ok()?;
    match PresenceState::parse(&text) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("presence: ignoring {STATE_PATH}: {e}");
            None
        }
    }
}

/// The graph node for a vault-relative note path ("Folder/Note.md"), following the
/// reader's id rules: the basename, or for duplicated basenames the path without
/// ".md" ("./Note" at the root). Ghosts never match.
pub fn node_for_path(graph: &Graph, rel: &str) -> Option<usize> {
    let rel = rel.trim_start_matches('/');
    let stem = rel.strip_suffix(".md")?;
    let basename = stem.rsplit('/').next().unwrap_or(stem);
    let qualified = if stem.contains('/') { stem.to_string() } else { format!("./{stem}") };
    let find = |id: &str| graph.nodes.iter().position(|n| !n.ghost && n.id == id);
    find(&qualified).or_else(|| find(basename))
}

/// Keeps the watch alive; dropping it stops watching.
pub struct PresenceWatcher {
    _debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
    _worker: thread::JoinHandle<()>,
}

/// How long writes must be quiet before the file is re-read. The plugin writes once
/// per change already, so this only folds the temp-file-and-rename pair together.
const DEBOUNCE: Duration = Duration::from_millis(50);

/// Watches the state file of the vault at `root`. `on_state` runs on a background
/// thread: once at start with the current state (`None` if there is none), then each
/// time the file's content changes.
///
/// The plugin folder may not exist yet (plugin not installed), so this watches the
/// deepest folder of `.obsidian/plugins/vault-wallpaper` that exists, recursively when
/// it's an ancestor, and only re-reads for events inside the plugin folder.
pub fn watch(root: &Path, on_state: impl Fn(Option<PresenceState>) + Send + 'static) -> notify::Result<PresenceWatcher> {
    let root = root.to_path_buf();
    let target = root.join(PLUGIN_DIR);
    let (watched, mode) = match target.is_dir() {
        true => (target.clone(), RecursiveMode::NonRecursive),
        false => (deepest_existing(&root, &target), RecursiveMode::Recursive),
    };
    let (tx, rx) = mpsc::channel::<()>();
    let mut debouncer = new_debouncer(DEBOUNCE, None, move |result: DebounceEventResult| {
        if let Ok(events) = result
            && events.iter().any(|e| e.event.need_rescan() || e.event.paths.iter().any(|p| in_plugin_dir(p)))
        {
            let _ = tx.send(());
        }
    })?;
    debouncer.watch(&watched, mode)?;

    let worker = thread::spawn(move || {
        let path = root.join(STATE_PATH);
        let mut last = std::fs::read(&path).ok();
        on_state(last.as_deref().and_then(|_| read(&root)));
        while rx.recv().is_ok() {
            while rx.try_recv().is_ok() {}
            let now = std::fs::read(&path).ok();
            if now != last {
                last = now;
                on_state(last.as_deref().and_then(|_| read(&root)));
            }
        }
    });
    Ok(PresenceWatcher { _debouncer: debouncer, _worker: worker })
}

/// The deepest existing folder between `root` and `target` (`root` itself at worst).
fn deepest_existing(root: &Path, target: &Path) -> PathBuf {
    let mut dir = target.to_path_buf();
    while !dir.is_dir() && dir != root {
        if !dir.pop() {
            return root.to_path_buf();
        }
    }
    dir
}

/// True for paths inside a `vault-wallpaper` folder (the plugin's), whatever the
/// event's spelling of the vault root (FSEvents reports real paths).
fn in_plugin_dir(p: &Path) -> bool {
    p.components().any(|c| matches!(c, Component::Normal(s) if s == "vault-wallpaper"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{Vault, VaultOptions};

    const T: f64 = 1_762_400_000_000.0;

    fn state(json: &str) -> PresenceState {
        PresenceState::parse(json).unwrap()
    }

    #[test]
    fn parses_the_plugin_format() {
        let s = state(
            r#"{"version":1,"active":"Folder/Note.md","links":["Other.md"],"saved":"Folder/Note.md","savedAt":1762400000500,"updated":1762400000000,"future":true}"#,
        );
        assert_eq!(s.active.as_deref(), Some("Folder/Note.md"));
        assert_eq!(s.links, vec!["Other.md".to_string()]);
        assert_eq!(s.save(), Some(("Folder/Note.md", 1_762_400_000_500.0)));
        assert_eq!(s.updated, T);
    }

    #[test]
    fn nothing_open_and_missing_fields() {
        let s = state(r#"{"version":1,"active":null,"saved":null,"savedAt":null,"updated":1762400000000}"#);
        assert_eq!(s.active, None);
        assert!(s.links.is_empty());
        assert_eq!(s.save(), None);
        let s = state(r#"{"version":1,"updated":1762400000000}"#);
        assert_eq!(s.active, None);
    }

    #[test]
    fn rejects_bad_files() {
        assert!(PresenceState::parse("").is_err());
        assert!(PresenceState::parse("{\"version\":1,").is_err());
        assert!(PresenceState::parse(r#"{"version":2,"updated":1}"#).is_err());
        assert!(PresenceState::parse(r#"{"version":1}"#).is_err());
        assert!(PresenceState::parse(r#"{"version":1,"active":5,"updated":1}"#).is_err());
    }

    #[test]
    fn staleness() {
        let s = state(r#"{"version":1,"active":"A.md","updated":1762400000000}"#);
        assert!(s.is_fresh(T));
        assert!(s.is_fresh(T - 30_000.0), "a clock slightly behind still counts");
        assert!(s.is_fresh(T + 4.0 * 60_000.0));
        assert!(!s.is_fresh(T + 5.0 * 60_000.0));
        assert!(!s.is_fresh(T + 3_600_000.0));
        assert_eq!(s.fresh_until_ms(), T + 300_000.0);
    }

    #[test]
    fn maps_paths_to_nodes_like_the_reader() {
        let dir = tempfile::tempdir().unwrap();
        let w = |rel: &str, body: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        w("Note.md", "[[Missing]]");
        w("a/Note.md", "");
        w("b/Unique.md", "[[Note]]");
        let g = Vault::scan(dir.path(), VaultOptions::default()).graph();
        let id = |rel: &str| node_for_path(&g, rel).map(|i| g.nodes[i].id.as_str());
        assert_eq!(id("Note.md"), Some("./Note"));
        assert_eq!(id("a/Note.md"), Some("a/Note"));
        assert_eq!(id("b/Unique.md"), Some("Unique"));
        assert_eq!(id("Missing.md"), None, "ghosts don't match");
        assert_eq!(id("c/Absent.md"), None);
        assert_eq!(id("b/Unique.canvas"), None);
    }

    #[test]
    fn reads_and_watches_the_state_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        assert_eq!(read(&root), None);
        let (tx, rx) = mpsc::channel();
        // The plugin folder doesn't exist yet: the watch starts on the root.
        let _w = watch(&root, move |s| tx.send(s).unwrap()).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), None);
        std::thread::sleep(Duration::from_millis(200));
        let plugin = root.join(PLUGIN_DIR);
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("state.json.tmp"), r#"{"version":1,"active":"A.md","updated":1762400000000}"#).unwrap();
        std::fs::rename(plugin.join("state.json.tmp"), plugin.join("state.json")).unwrap();
        let got = rx.recv_timeout(Duration::from_secs(5)).expect("a state after the write");
        assert_eq!(got.and_then(|s| s.active).as_deref(), Some("A.md"));
        assert_eq!(read(&root).and_then(|s| s.active).as_deref(), Some("A.md"));
        std::fs::remove_file(plugin.join("state.json")).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).expect("a state after the delete"), None);
    }
}
