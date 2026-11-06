//! Saved positions, so a relaunch opens on the settled graph instead of laying it
//! out again.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const VERSION: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PositionCache {
    pub version: u32,
    /// Layout kind and clustering the positions were computed with.
    pub layout: String,
    pub cluster_by_tag: bool,
    /// Node id -> [x, y].
    pub positions: HashMap<String, [f32; 2]>,
    /// Node id -> `LayoutGraph::fingerprints` entry, to tell what changed while the
    /// app was closed.
    pub fingerprints: HashMap<String, u64>,
}

impl PositionCache {
    /// Reads a cache file. A missing, unreadable or old-format file yields `None`.
    pub fn load(path: &Path) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        let cache: PositionCache = serde_json::from_slice(&bytes).ok()?;
        (cache.version == VERSION).then_some(cache)
    }

    /// Writes atomically (temp file + rename) so a crash never leaves half a file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp: PathBuf = path.with_extension("tmp");
        let mut out = self.clone();
        out.version = VERSION;
        fs::write(&tmp, serde_json::to_vec(&out).map_err(std::io::Error::other)?)?;
        fs::rename(tmp, path)
    }
}
