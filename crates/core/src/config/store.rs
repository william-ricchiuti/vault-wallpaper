//! Reading and writing `config.json`.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::{Config, ConfigError, Sanitized};

/// `~/Library/Application Support/Vault Wallpaper/config.json`, or `None` without `$HOME`.
pub fn default_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join("Library/Application Support/Vault Wallpaper/config.json"))
}

/// Loads the config at `path`. A missing file is a first run and gives the defaults;
/// anything else that stops the file being read is an error.
pub fn load(path: &Path) -> Result<Sanitized, ConfigError> {
    match fs::read_to_string(path) {
        Ok(text) => Config::from_json(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Sanitized { config: Config::default(), issues: Vec::new() }),
        Err(e) => Err(ConfigError::Io(e)),
    }
}

/// Reads an old web-version `config.json`. Its web-only keys are dropped, and so is any
/// key that has since become invalid, so one stale value doesn't block the import.
pub fn import_legacy(path: &Path) -> Result<Sanitized, ConfigError> {
    Config::from_json(&fs::read_to_string(path).map_err(ConfigError::Io)?)
}

/// Writes `config` atomically: a temp file in the same folder, synced, then renamed over
/// the old one, so a crash or a reader (the watcher, a text editor) never sees half a file.
pub fn save(path: &Path, config: &Config) -> io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "config path has no file name"))?;
    let tmp = dir.join(format!(".{}.tmp-{}", name.to_string_lossy(), std::process::id()));
    let mut text = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    text.push('\n');
    let written = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}
