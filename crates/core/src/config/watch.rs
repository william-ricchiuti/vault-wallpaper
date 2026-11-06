//! Picking up hand edits to `config.json`.
//!
//! Editors and [`save`](super::save) replace the file by rename, which swaps its inode, so
//! the folder is watched rather than the file. Debounced like the vault watcher, so a
//! write-then-rename save is one reload.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};

use super::{Config, ConfigError, save};

/// How long the file must be quiet before it is reloaded.
pub const DEBOUNCE: Duration = Duration::from_millis(100);

/// Keeps the watch alive; dropping it stops watching.
pub struct ConfigWatcher {
    path: PathBuf,
    last: Arc<Mutex<Config>>,
    _debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
    _worker: thread::JoinHandle<()>,
}

impl ConfigWatcher {
    /// Saves `config` without it coming back through the callback as an "edit".
    pub fn save(&self, config: &Config) -> std::io::Result<()> {
        *self.last.lock().unwrap() = config.clone();
        save(&self.path, config)
    }
}

/// Watches the config at `path`, starting from `current`. `on_change` runs on a
/// background thread with each new config that differs from the last one seen. An edit
/// that isn't valid JSON is logged and ignored, as on the web; invalid keys are dropped.
pub fn watch(path: &Path, current: Config, on_change: impl Fn(Config) + Send + 'static) -> notify::Result<ConfigWatcher> {
    let path = path.to_path_buf();
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
    std::fs::create_dir_all(&dir).map_err(notify::Error::io)?;
    let name: OsString = path.file_name().map(Into::into).unwrap_or_default();
    let (tx, rx) = mpsc::channel::<()>();

    let mut debouncer = new_debouncer(DEBOUNCE, None, move |result: DebounceEventResult| match result {
        Ok(events) => {
            let ours = events
                .iter()
                .any(|e| e.event.need_rescan() || e.event.paths.iter().any(|p| p.file_name() == Some(name.as_os_str())));
            if ours {
                let _ = tx.send(());
            }
        }
        Err(errors) => {
            for e in errors {
                eprintln!("config watcher error: {e}");
            }
        }
    })?;
    debouncer.watch(&dir, RecursiveMode::NonRecursive)?;

    let last = Arc::new(Mutex::new(current));
    let worker = {
        let (path, last) = (path.clone(), last.clone());
        thread::spawn(move || {
            while rx.recv().is_ok() {
                while rx.try_recv().is_ok() {}
                // A deleted file is not an edit back to the defaults.
                let loaded = match std::fs::read_to_string(&path).map_err(ConfigError::Io).and_then(|t| Config::from_json(&t)) {
                    Ok(l) => l,
                    Err(e) if e.is_not_found() => continue,
                    Err(e) => {
                        eprintln!("{}: change ignored: {e}", path.display());
                        continue;
                    }
                };
                for issue in &loaded.issues {
                    eprintln!("{}: {} dropped: {}", path.display(), issue.key, issue.message);
                }
                let mut seen = last.lock().unwrap();
                if *seen == loaded.config {
                    continue; // our own save, or a touch with no change
                }
                *seen = loaded.config.clone();
                drop(seen);
                on_change(loaded.config);
            }
        })
    };
    Ok(ConfigWatcher { path, last, _debouncer: debouncer, _worker: worker })
}
