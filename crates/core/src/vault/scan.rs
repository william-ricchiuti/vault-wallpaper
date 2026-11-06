//! Finding and reading the vault's markdown files.
//!
//! Same rules as the web version's `collectMarkdownFiles` / `parseMarkdownEntry`: names
//! starting with "." are skipped at every level, `ignorePaths` entries match a whole
//! relative path, a path prefix, or any single folder name, only files ending in exactly
//! ".md" count, and symlinks are not followed (Node's `Dirent` reports them as neither
//! file nor directory).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rayon::prelude::*;

use super::parse::{extract_tags, extract_wikilinks};

/// One parsed note.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    /// File name without the ".md" suffix.
    pub basename: String,
    /// Modification time in milliseconds since the epoch, like Node's `stat.mtimeMs`.
    pub mtime_ms: f64,
    pub tags: Vec<String>,
    pub wikilinks: Vec<String>,
}

/// The path relative to the vault root, as a string with the platform separator,
/// or `None` when it is the root itself or outside it.
pub(crate) fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let rel = rel.to_str()?;
    if rel.is_empty() { None } else { Some(rel.to_string()) }
}

/// `isIgnoredPath`: true when a pattern equals the relative path, is a prefix folder of
/// it, or equals any one of its segments. Paths outside the vault count as ignored.
pub fn is_ignored(root: &Path, path: &Path, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let Some(rel) = relative(root, path) else { return true };
    let sep = std::path::MAIN_SEPARATOR;
    patterns.iter().any(|p| {
        rel == *p || rel.starts_with(&format!("{p}{sep}")) || rel.split(sep).any(|seg| seg == p)
    })
}

/// True when any segment of the path below the root starts with ".".
pub fn has_hidden_segment(root: &Path, path: &Path) -> bool {
    match relative(root, path) {
        None => true,
        Some(rel) => rel.split(std::path::MAIN_SEPARATOR).any(|seg| seg.starts_with('.')),
    }
}

pub fn is_markdown(path: &Path) -> bool {
    path.to_str().is_some_and(|p| p.ends_with(".md"))
}

/// All markdown files under `dir` (which is the root or a folder inside it).
pub fn collect_markdown_files(root: &Path, dir: &Path, patterns: &[String]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if is_ignored(root, &path, patterns) {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() && name.ends_with(".md") {
                files.push(path);
            }
        }
    }
    files
}

/// Reads and parses one note. `None` when it vanished or can't be read.
pub fn parse_entry(path: &Path) -> Option<Entry> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("note: skipped {} ({e})", path.display());
            }
            return None;
        }
    };
    let content = String::from_utf8_lossy(&bytes);
    let name = path.file_name()?.to_str()?;
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs() as f64 * 1000.0 + f64::from(d.subsec_nanos()) / 1e6);
    Some(Entry {
        path: path.to_path_buf(),
        basename: name.strip_suffix(".md").unwrap_or(name).to_string(),
        mtime_ms,
        tags: extract_tags(&content),
        wikilinks: extract_wikilinks(&content),
    })
}

/// Parses many files in parallel, dropping any that fail.
pub fn parse_entries(paths: &[PathBuf]) -> Vec<Entry> {
    paths.par_iter().filter_map(|p| parse_entry(p)).collect()
}
