//! Turning parsed notes into the graph: node ids, link resolution and ghost nodes.
//!
//! Mirrors `materializeGraph` in the web version's `parser.js`. One deliberate
//! difference: lookups use real maps, so notes named `toString` or links like
//! `[[constructor]]` work here, while they crash the JavaScript version.

use std::collections::BTreeMap;

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::path::Path;

use serde::{Serialize, Serializer};

use super::scan::{Entry, relative};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Node {
    pub id: String,
    /// The note's file name when it differs from the id (duplicate basenames get a
    /// folder-qualified id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// First tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// All tags, only present when there is more than one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "ms_like_js")]
    pub mtime: Option<f64>,
    /// Top-level folder under the vault; absent for notes at the root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// An unresolved link target with no note behind it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ghost: bool,
}

/// JavaScript prints whole-number doubles without ".0"; keep that so the output reads
/// like the web version's graph.json.
fn ms_like_js<S: Serializer>(v: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(x) if x.fract() == 0.0 && x.abs() < 9.0e15 => s.serialize_i64(*x as i64),
        Some(x) => s.serialize_f64(*x),
        None => s.serialize_none(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Link {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    /// Number of notes with at least one tag.
    pub tagged: usize,
    /// Every tag and how many notes carry it.
    pub discovered_tags: BTreeMap<String, usize>,
}

#[derive(Debug, Clone)]
pub struct GraphOptions {
    pub show_unresolved_links: bool,
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self { show_unresolved_links: true }
    }
}

/// JavaScript sorts strings by UTF-16 code units; Rust compares UTF-8 bytes. They only
/// differ for characters above U+FFFF, but parity is cheap.
fn cmp_utf16(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn rel_slash(root: &Path, path: &Path) -> String {
    relative(root, path).unwrap_or_default().replace(std::path::MAIN_SEPARATOR, "/")
}

/// `noteId`: the basename, or for duplicated basenames the vault-relative path without
/// ".md" ("./name" at the root).
fn note_id(entry: &Entry, root: &Path, duplicated: &HashSet<&str>) -> String {
    if !duplicated.contains(entry.basename.as_str()) {
        return entry.basename.clone();
    }
    let rel = rel_slash(root, &entry.path);
    let rel = rel.strip_suffix(".md").unwrap_or(&rel);
    if rel.contains('/') { rel.to_string() } else { format!("./{rel}") }
}

pub fn materialize<'a>(entries: impl IntoIterator<Item = &'a Entry>, root: &Path, opts: &GraphOptions) -> Graph {
    // Sorted by path so output is deterministic. Byte order, not JavaScript's UTF-16
    // order: they differ only for rare characters, and only in output order, which
    // nothing depends on.
    let mut raw: Vec<&Entry> = entries.into_iter().collect();
    raw.sort_unstable_by(|a, b| a.path.as_os_str().cmp(b.path.as_os_str()));
    let root_str = root.to_str().unwrap_or("").trim_end_matches(std::path::MAIN_SEPARATOR);
    let rel_of = |e: &Entry| -> String {
        let p = e.path.to_str().unwrap_or("");
        match p.strip_prefix(root_str) {
            Some(rest) => rest.trim_start_matches(std::path::MAIN_SEPARATOR).to_string(),
            None => relative(root, &e.path).unwrap_or_default(),
        }
    };

    let mut counts: HashMap<&str, usize> = HashMap::with_capacity_and_hasher(raw.len(), Default::default());
    for e in &raw {
        *counts.entry(e.basename.as_str()).or_default() += 1;
    }
    let duplicated: HashSet<&str> = counts.iter().filter(|(_, n)| **n > 1).map(|(b, _)| *b).collect();
    if !duplicated.is_empty() {
        let mut names: Vec<&str> = duplicated.iter().copied().collect();
        names.sort_by(|a, b| cmp_utf16(a, b));
        eprintln!(
            "note: {} duplicate basename{} resolved with folder prefix ({})",
            names.len(),
            if names.len() == 1 { "" } else { "s" },
            names.join(", ")
        );
    }

    // Everything below works on node indices; strings are only built for the output.
    let mut by_basename: HashMap<&str, Vec<u32>> = HashMap::with_capacity_and_hasher(raw.len(), Default::default());
    let mut by_lower: HashMap<String, Vec<u32>> = HashMap::with_capacity_and_hasher(raw.len(), Default::default());
    let mut by_rel: HashMap<String, u32> = HashMap::with_capacity_and_hasher(raw.len(), Default::default());
    let mut nodes: Vec<Node> = Vec::with_capacity(raw.len());
    let mut discovered_tags: BTreeMap<String, usize> = BTreeMap::new();
    let mut tagged = 0;
    let sep = std::path::MAIN_SEPARATOR;
    for (i, e) in raw.iter().enumerate() {
        let i = i as u32;
        let id = note_id(e, root, &duplicated);
        by_basename.entry(e.basename.as_str()).or_default().push(i);
        by_lower.entry(e.basename.to_lowercase()).or_default().push(i);
        let rel = rel_of(e);
        let folder = match rel.find(sep) {
            Some(idx) if idx > 0 => Some(rel[..idx].to_string()),
            _ => None,
        };
        let rel = if sep == '/' { rel } else { rel.replace(sep, "/") };
        by_rel.insert(rel.strip_suffix(".md").unwrap_or(&rel).to_lowercase(), i);
        if !e.tags.is_empty() {
            tagged += 1;
            for t in &e.tags {
                *discovered_tags.entry(t.clone()).or_default() += 1;
            }
        }
        nodes.push(Node {
            label: (e.basename != id).then(|| e.basename.clone()),
            id,
            tag: e.tags.first().cloned(),
            tags: (e.tags.len() > 1).then(|| e.tags.clone()),
            mtime: (e.mtime_ms != 0.0).then_some(e.mtime_ms),
            folder,
            ghost: false,
        });
    }

    // [[link]] resolution order: exact basename, then a vault-relative path
    // ([[Folder/Note]]), then the last path segment, then case-insensitively.
    let resolve = |target: &str| -> Option<&[u32]> {
        if let Some(v) = by_basename.get(target) {
            return Some(v);
        }
        if target.contains('/') {
            if let Some(i) = by_rel.get(&target.trim_start_matches('/').to_lowercase()) {
                return Some(std::slice::from_ref(i));
            }
            let last = &target[target.rfind('/').unwrap() + 1..];
            if let Some(v) = by_basename.get(last) {
                return Some(v);
            }
            if let Some(v) = by_lower.get(&last.to_lowercase()) {
                return Some(v);
            }
        }
        by_lower.get(&target.to_lowercase()).map(Vec::as_slice)
    };

    // Ids of notes and ghosts, for links that name an id directly (and ghost reuse).
    let mut by_id: HashMap<String, u32> =
        nodes.iter().enumerate().map(|(i, n)| (n.id.clone(), i as u32)).collect();
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    let mut seen: HashSet<(u32, u32)> = HashSet::default();
    for (source, e) in raw.iter().enumerate() {
        let source = source as u32;
        for target in &e.wikilinks {
            let resolved: &[u32] = match resolve(target) {
                Some(v) => v,
                None => match by_id.get(target.as_str()) {
                    Some(i) => std::slice::from_ref(i),
                    None => &[],
                },
            };
            if resolved.is_empty() {
                if opts.show_unresolved_links && !target.is_empty() {
                    let ghost = nodes.len() as u32;
                    by_id.insert(target.clone(), ghost);
                    nodes.push(Node {
                        id: target.clone(),
                        label: None,
                        tag: None,
                        tags: None,
                        mtime: None,
                        folder: None,
                        ghost: true,
                    });
                    if seen.insert((source, ghost)) {
                        pairs.push((source, ghost));
                    }
                }
                continue;
            }
            for &t in resolved {
                if t != source && seen.insert((source, t)) {
                    pairs.push((source, t));
                }
            }
        }
    }
    let links = pairs
        .into_iter()
        .map(|(s, t)| Link { source: nodes[s as usize].id.clone(), target: nodes[t as usize].id.clone() })
        .collect();

    Graph { nodes, links, tagged, discovered_tags }
}

impl Graph {
    /// Notes edited between `old` and `self`: real notes (not ghosts) that are new or
    /// whose modification time moved forward. This is the edit signal themes react to
    /// (Ink & Paper's bleed, Pulse); it costs one pass over the notes, once per
    /// watcher batch.
    pub fn edited_since(&self, old: &Graph) -> Vec<String> {
        let before: HashMap<&str, Option<f64>> = old.nodes.iter().map(|n| (n.id.as_str(), n.mtime)).collect();
        self.nodes
            .iter()
            .filter(|n| !n.ghost)
            .filter(|n| match before.get(n.id.as_str()) {
                None => true,
                Some(prev) => match (prev, n.mtime) {
                    (Some(a), Some(b)) => b > *a,
                    (None, Some(_)) => true,
                    _ => false,
                },
            })
            .map(|n| n.id.clone())
            .collect()
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    fn node(id: &str, mtime: Option<f64>, ghost: bool) -> Node {
        Node { id: id.into(), label: None, tag: None, tags: None, mtime, folder: None, ghost }
    }

    fn graph(nodes: Vec<Node>) -> Graph {
        Graph { nodes, ..Graph::default() }
    }

    #[test]
    fn edits_are_newer_or_new_notes() {
        let old = graph(vec![node("a", Some(1.0), false), node("b", Some(1.0), false), node("g", None, true)]);
        let new = graph(vec![
            node("a", Some(2.0), false),
            node("b", Some(1.0), false),
            node("c", Some(5.0), false),
            node("g", None, true),
            node("h", None, true),
        ]);
        assert_eq!(new.edited_since(&old), vec!["a".to_string(), "c".to_string()]);
        assert!(new.edited_since(&new).is_empty());
        // Going back in time (a restore) is not an edit we animate.
        assert!(old.edited_since(&new).is_empty());
    }
}
