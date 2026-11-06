//! The vault reader: scans an Obsidian vault into a [`Graph`] and keeps it current.

mod graph;
mod parse;
mod scan;
mod watch;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

pub use graph::{Graph, GraphOptions, Link, Node, materialize};
pub use parse::{extract_tags, extract_wikilinks, note_excerpt};
pub use scan::{Entry, has_hidden_segment, is_ignored, is_markdown, parse_entry};
pub use watch::{VaultWatcher, watch};

#[derive(Debug, Clone, Default)]
pub struct VaultOptions {
    /// Folder names or vault-relative paths to leave out (`ignorePaths`).
    pub ignore_paths: Vec<String>,
    pub graph: GraphOptions,
}

/// Every note in the vault, keyed by path.
pub struct Vault {
    root: PathBuf,
    opts: VaultOptions,
    entries: HashMap<PathBuf, Entry>,
}

impl Vault {
    /// Full scan of the vault, parsing notes in parallel.
    pub fn scan(root: impl Into<PathBuf>, opts: VaultOptions) -> Vault {
        let root = root.into();
        let files = scan::collect_markdown_files(&root, &root, &opts.ignore_paths);
        let entries = scan::parse_entries(&files).into_iter().map(|e| (e.path.clone(), e)).collect();
        Vault { root, opts, entries }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Paths of every note currently in the vault.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.entries.keys().map(PathBuf::as_path)
    }

    pub fn graph(&self) -> Graph {
        materialize(self.entries.values(), &self.root, &self.opts.graph)
    }

    fn wanted(&self, path: &Path) -> bool {
        is_markdown(path)
            && !has_hidden_segment(&self.root, path)
            && !is_ignored(&self.root, path, &self.opts.ignore_paths)
    }

    /// Brings the entries for these paths up to date with the disk, whatever happened
    /// to them (created, edited, deleted, renamed, or a folder moved). Returns true when
    /// anything that affects the graph changed.
    pub fn refresh_paths(&mut self, paths: &[PathBuf]) -> bool {
        let mut changed = false;
        let mut reparse: Vec<PathBuf> = Vec::new();
        // macOS reports a moved folder by its new name only, so a folder event or a
        // vanished path means some notes we hold may be gone without their own event.
        let mut prune = false;
        for path in paths {
            if path.strip_prefix(&self.root).is_err() {
                continue;
            }
            match std::fs::symlink_metadata(path) {
                Ok(meta) if meta.is_dir() => {
                    // A folder appeared or was moved in: drop what we had under it and
                    // rescan it, unless the folder itself is hidden or ignored.
                    prune = true;
                    changed |= self.remove_under(path);
                    let hidden = has_hidden_segment(&self.root, path)
                        || is_ignored(&self.root, path, &self.opts.ignore_paths);
                    if !hidden {
                        reparse.extend(scan::collect_markdown_files(&self.root, path, &self.opts.ignore_paths));
                    }
                }
                Ok(meta) if meta.is_file() => {
                    if self.wanted(path) {
                        reparse.push(path.clone());
                    }
                }
                // Gone (or a symlink, which is never followed): forget it and anything
                // that lived under it.
                _ => {
                    prune = true;
                    changed |= self.entries.remove(path).is_some();
                    changed |= self.remove_under(path);
                }
            }
        }
        if prune {
            changed |= self.prune_missing();
        }
        for entry in scan::parse_entries(&reparse) {
            let differs = self.entries.get(&entry.path).is_none_or(|prev| *prev != entry);
            if differs {
                self.entries.insert(entry.path.clone(), entry);
                changed = true;
            }
        }
        changed
    }

    /// Full rescan, for when the OS says events were dropped.
    pub fn rescan(&mut self) -> bool {
        let fresh = Vault::scan(self.root.clone(), self.opts.clone());
        let changed = fresh.entries != self.entries;
        self.entries = fresh.entries;
        changed
    }

    /// Drops entries whose file no longer exists.
    fn prune_missing(&mut self) -> bool {
        let missing: Vec<PathBuf> =
            self.entries.par_iter().filter(|(p, _)| !p.is_file()).map(|(p, _)| p.clone()).collect();
        for p in &missing {
            self.entries.remove(p);
        }
        !missing.is_empty()
    }

    fn remove_under(&mut self, dir: &Path) -> bool {
        let before = self.entries.len();
        self.entries.retain(|p, _| !(p != dir && p.starts_with(dir)));
        self.entries.len() != before
    }
}
