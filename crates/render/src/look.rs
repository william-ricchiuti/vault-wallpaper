//! What each note and link looks like under the current config: colour, size,
//! importance and which notes get labels. A port of the web version's derived-visuals
//! logic (`refreshDerivedVisuals`, `nodeColor`, `edgeColor`, `buildSceneTuningRaw`,
//! `computeFocusSet`, `buildClusterHalos` and the label candidates in `index.html`),
//! recomputed only when the graph or config changes.
//!
//! Per-frame effects (pulse, depth of field, depth-parallax layers, flares, glow
//! breathing) stay in the renderer; this module produces the static base they modulate.

use std::collections::HashMap;
use vw_core::config::{Config, LabelSource, NodeColorMode, Palette, rgb};
use vw_core::layout::LayoutGraph;
use vw_core::vault::Graph;

/// Inputs besides the graph and config.
#[derive(Debug, Clone, Copy)]
pub struct LookContext {
    /// "Now" in ms since the epoch, for age and recency colouring.
    pub now_ms: f64,
    /// True when the light colours are in use (`autoTheme`, time of day).
    pub light: bool,
}

/// A note chosen for a label.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelPick {
    /// Index into the graph's nodes.
    pub node: u32,
    pub text: String,
    /// 0..1, for sizing and fading.
    pub importance: f32,
}

/// Per-note and per-link visuals, in the same order as `LayoutGraph`'s nodes and links.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Look {
    /// Straight (not premultiplied) sRGB colour and alpha per note.
    pub node_color: Vec<[f32; 4]>,
    /// Radius in points per note, before depth-of-field scaling.
    pub node_radius: Vec<f32>,
    /// 0..1: degree relative to the best-connected note.
    pub node_importance: Vec<f32>,
    /// Colour and alpha per link (alpha already includes `linkOpacity`).
    pub edge_color: Vec<[f32; 4]>,
    /// Cluster-halo group per note (`None` for notes that don't form a halo), and
    /// each group's colour. Centroids come from positions, so the renderer computes them.
    pub halo_group: Vec<Option<u32>>,
    pub halo_color: Vec<[f32; 3]>,
    /// Notes to label, most important first. These are candidates: the renderer
    /// draws them in order, skips any that land off-screen or overlap an earlier
    /// label, and stops after `label_count` drawn (the web version's `drawLabels`).
    pub labels: Vec<LabelPick>,
    /// Most labels to draw (`hubLabelCount`, lowered for large vaults). 0 when
    /// `hubLabels` is off.
    pub label_count: u32,
    /// Centre alpha of a cluster halo's radial gradient (0.11 for small vaults, less
    /// for big ones, 0 when halos are off). Stops: 1× at 0, 0.28× at 0.55, 0 at the rim.
    pub halo_alpha: f32,
    /// Multiplier on the halo radius:
    /// `max(56, (max(w, h) * 0.72 + sqrt(count) * 22) * halo_radius_boost)`.
    pub halo_radius_boost: f32,
}

/// The web version's named palettes (`PALETTES` in `renderer-core.js`).
const AURORA: [&str; 8] = ["#5ef2c8", "#7aa8ff", "#c77dff", "#ff7ab6", "#ffd479", "#6ee7ff", "#a3ff8f", "#ff9f6e"];
const RISO: [&str; 8] = ["#ff48b0", "#0078bf", "#ffe800", "#00a95c", "#ff6c2f", "#765ba7", "#ff665e", "#00838a"];
const METRO: [&str; 8] = ["#e3242b", "#0072ce", "#00a651", "#f6a800", "#f68b1f", "#8e3fbf", "#00b3c8", "#a05a2c"];
const PASTEL: [&str; 8] = ["#f2c6a0", "#cfe0b0", "#b9d3ea", "#ebc3d9", "#f3e3a4", "#cdc1e8", "#a9ddd6", "#e8bcae"];
const EARTH: [&str; 8] = ["#c8a165", "#8a9a5b", "#b5651d", "#6b8e8e", "#a0522d", "#d2b48c", "#556b2f", "#9c6644"];
const NEON: [&str; 8] = ["#7df9ff", "#ff5ce1", "#b18cff", "#ffd36b", "#6bffb5", "#ff8a6b", "#6bb5ff", "#ff6b9d"];
const VIRIDIS: [&str; 8] = ["#440154", "#46327e", "#365c8d", "#277f8e", "#1fa187", "#4ac16d", "#a0da39", "#fde725"];
const MONO: [&str; 8] = ["#f2f2f2", "#d0d0d0", "#b0b0b0", "#909090", "#e4e4e4", "#c0c0c0", "#a0a0a0", "#808080"];

/// Fresh (green) to stale (red), for `nodeColorMode: age` without a palette.
const AGE_PALETTE: [&str; 5] = ["#78d97c", "#b8d960", "#e8c84a", "#e89a3c", "#d96b4a"];

/// How many label candidates to hand over; the web version caps recent mode at 60.
const MAX_LABEL_CANDIDATES: usize = 60;
const LABEL_MAX_CHARS: usize = 28;
const DAY_MS: f64 = 86_400_000.0;

type Rgb = [u8; 3];

fn palette_colors(p: Palette) -> Option<[Rgb; 8]> {
    let hexes = match p {
        Palette::Auto => return None,
        Palette::Aurora => AURORA,
        Palette::Riso => RISO,
        Palette::Metro => METRO,
        Palette::Pastel => PASTEL,
        Palette::Earth => EARTH,
        Palette::Neon => NEON,
        Palette::Viridis => VIRIDIS,
        Palette::Mono => MONO,
    };
    Some(hexes.map(|h| rgb(h).expect("palette colours are valid hex")))
}

/// The size-dependent part of `buildSceneTuningRaw` that affects the look.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Tuning {
    label_count: u32,
    label_min_importance: f64,
    edge_coloring: bool,
    link_opacity_scale: f64,
    halo_alpha: f64,
    halo_radius_boost: f64,
}

fn tuning(cfg: &Config, nodes: usize, links: usize) -> Tuning {
    let dense = nodes > 350 || links > 1200;
    let huge = nodes > 900 || links > 3200;
    let massive = nodes > 3000 || links > 10000;
    let ultra = nodes > 10000 || links > 40000;
    let count = cfg.hub_label_count;
    let min_imp = cfg.label_min_importance;
    let halos = |a: f64| if cfg.cluster_halos { a } else { 0.0 };
    if !cfg.auto_scale_large_vaults {
        return Tuning {
            label_count: count,
            label_min_importance: min_imp,
            edge_coloring: cfg.edge_coloring,
            link_opacity_scale: 1.0,
            halo_alpha: halos(0.11),
            halo_radius_boost: 1.0,
        };
    }
    if ultra {
        return Tuning {
            label_count: count.min(2),
            label_min_importance: min_imp.max(0.78),
            edge_coloring: false,
            link_opacity_scale: 0.32,
            halo_alpha: 0.0,
            halo_radius_boost: 0.0,
        };
    }
    if massive {
        return Tuning {
            label_count: count.min(3),
            label_min_importance: min_imp.max(0.64),
            edge_coloring: false,
            link_opacity_scale: 0.42,
            halo_alpha: 0.0,
            halo_radius_boost: 0.0,
        };
    }
    let pick = |h: f64, d: f64, s: f64| if huge { h } else if dense { d } else { s };
    Tuning {
        label_count: if huge { count.min(4) } else if dense { count.min(6) } else { count },
        label_min_importance: min_imp.max(pick(0.42, 0.28, 0.0)),
        edge_coloring: cfg.edge_coloring,
        link_opacity_scale: pick(0.78, 0.9, 1.0),
        halo_alpha: halos(pick(0.05, 0.08, 0.11)),
        halo_radius_boost: pick(0.85, 0.95, 1.0),
    }
}

/// Derives the look. `graph` and `layout` describe the same graph in the same node
/// order (`LayoutGraph::from_graph(graph)`).
pub fn derive_look(graph: &Graph, layout: &LayoutGraph, cfg: &Config, ctx: &LookContext) -> Look {
    let n = layout.len();
    let tune = tuning(cfg, n, layout.links.len());
    let (accent_hex, background_hex) = if ctx.light {
        (pick_hex(&cfg.light_accent, &cfg.accent), pick_hex(&cfg.light_background, &cfg.background))
    } else {
        (cfg.accent.as_str(), cfg.background.as_str())
    };
    let background = rgb(background_hex).unwrap_or([10, 10, 15]);
    let accent = ensure_contrast(rgb(accent_hex).unwrap_or([124, 92, 255]), background, 1.8);
    let palette = palette_colors(cfg.palette);

    // Importance: degree over the best-connected note's degree (at least 1).
    let max_deg = layout.degree.iter().copied().max().unwrap_or(0).max(1);
    let node_importance: Vec<f32> = layout.degree.iter().map(|&d| (d as f32 / max_deg as f32).min(1.0)).collect();
    // The default renderer's radius, which is also the layout's collision radius.
    let node_radius: Vec<f32> = (0..n)
        .map(|i| {
            let d = (layout.degree[i] as f32).sqrt();
            if layout.ghost[i] { 1.5 + d * 0.8 } else { 2.2 + d * 1.45 }
        })
        .collect();

    let folders: Vec<Option<&str>> = (0..n).map(|i| graph.nodes.get(i).and_then(|g| g.folder.as_deref())).collect();
    let tags: Vec<Option<&str>> = layout.tag.iter().map(|t| t.as_deref()).collect();
    let tag_rank = rank_map(tags.iter().zip(&layout.ghost).filter(|(_, g)| !**g).filter_map(|(t, _)| *t));
    let folder_rank = rank_map(folders.iter().zip(&layout.ghost).filter(|(_, g)| !**g).filter_map(|(f, _)| *f));
    let age = age_stats(&layout.mtime);
    let recency_days = if cfg.recency_days == 0 { 7.0 } else { f64::from(cfg.recency_days) };
    let focus = focus_set(graph, layout, cfg, ctx.now_ms);
    let in_focus = |i: usize| focus.as_ref().is_none_or(|f| f[i]);

    let mut folder_cache: HashMap<&str, Rgb> = HashMap::new();
    let mut rgb_base: Vec<Rgb> = Vec::with_capacity(n);
    let mut dim = vec![false; n];
    let mut node_color = Vec::with_capacity(n);
    for i in 0..n {
        let ghost = layout.ghost[i];
        let color = match cfg.node_color_mode {
            NodeColorMode::Age => {
                let b = age_bucket(age, layout.mtime[i]);
                match &palette {
                    Some(p) => p[b % p.len()],
                    None => rgb(AGE_PALETTE[b]).unwrap(),
                }
            }
            NodeColorMode::Folder => match (folders[i], &palette) {
                (Some(f), Some(p)) => p[folder_rank.get(f).copied().unwrap_or(0) % p.len()],
                (Some(f), None) => *folder_cache.entry(f).or_insert_with(|| folder_color(f)),
                (None, _) => accent,
            },
            // Tag and recency both colour by tag; recency also dims stale notes.
            NodeColorMode::Tag | NodeColorMode::Recency => {
                let custom = tags[i].and_then(|t| cfg.tag_colors.get(t)).and_then(|h| rgb(h));
                let from_palette = match (tags[i], &palette) {
                    (Some(t), Some(p)) => Some(p[tag_rank.get(t).copied().unwrap_or(0) % p.len()]),
                    _ => None,
                };
                custom.or(from_palette).unwrap_or(accent)
            }
        };
        dim[i] = cfg.node_color_mode == NodeColorMode::Recency
            && (ghost || node_days(layout.mtime[i], ctx.now_ms) >= recency_days);
        // Core alpha of the default renderer, without the per-frame pulse and
        // depth-of-field terms: brighter for hubs, ghosts at 35%, dimmed at 22%.
        let mut alpha = (0.55 + node_importance[i] * 0.35).min(1.0);
        if ghost {
            alpha *= 0.35;
        }
        if dim[i] {
            alpha *= 0.22;
        }
        if !in_focus(i) {
            alpha = 0.0;
        }
        rgb_base.push(color);
        node_color.push(rgba(color, alpha));
    }

    let edge_color = layout
        .links
        .iter()
        .map(|&(s, t)| {
            let (s, t) = (s as usize, t as usize);
            // Links take their source note's colour (the web version's fallback to
            // the target never triggers: every note has a colour).
            let color = if tune.edge_coloring { rgb_base[s] } else { accent };
            let importance = f64::from(node_importance[s].max(node_importance[t]));
            let mut alpha = cfg.link_opacity * tune.link_opacity_scale * (0.3 + importance * 0.7);
            if dim[s] && dim[t] {
                alpha *= 0.25;
            }
            if !(in_focus(s) && in_focus(t)) {
                alpha = 0.0;
            }
            rgba(color, alpha as f32)
        })
        .collect();

    // Cluster halos: one per tag with at least three (visible) notes, coloured like
    // the first note of that tag.
    let mut halo_group = vec![None; n];
    let mut halo_color = Vec::new();
    if tune.halo_alpha > 0.0 {
        let mut groups: HashMap<&str, (u32, usize)> = HashMap::new();
        let mut first_color: Vec<Rgb> = Vec::new();
        for i in 0..n {
            let Some(tag) = tags[i] else { continue };
            if !in_focus(i) {
                continue;
            }
            let next = first_color.len() as u32;
            let entry = groups.entry(tag).or_insert_with(|| {
                first_color.push(rgb_base[i]);
                (next, 0)
            });
            entry.1 += 1;
            halo_group[i] = Some(entry.0);
        }
        // Renumber so only groups of three or more remain, in first-seen order.
        let mut remap = vec![None; first_color.len()];
        let mut counts = vec![0usize; first_color.len()];
        for &(g, c) in groups.values() {
            counts[g as usize] = c;
        }
        for (g, &c) in counts.iter().enumerate() {
            if c >= 3 {
                remap[g] = Some(halo_color.len() as u32);
                halo_color.push(rgb_f32(first_color[g]));
            }
        }
        for h in &mut halo_group {
            *h = h.and_then(|g| remap[g as usize]);
        }
    }

    let label_count = if cfg.hub_labels { tune.label_count } else { 0 };
    let labels = if label_count == 0 {
        Vec::new()
    } else {
        let mut idx: Vec<usize> = (0..n).filter(|&i| !layout.ghost[i] && in_focus(i)).collect();
        match cfg.label_source {
            LabelSource::Recent => {
                idx.retain(|&i| layout.mtime[i] > 0.0);
                idx.sort_by(|&a, &b| layout.mtime[b].total_cmp(&layout.mtime[a]));
            }
            LabelSource::Hubs => {
                idx.retain(|&i| f64::from(node_importance[i]) >= tune.label_min_importance);
                idx.sort_by(|&a, &b| node_importance[b].total_cmp(&node_importance[a]));
            }
        }
        idx.truncate(MAX_LABEL_CANDIDATES);
        idx.into_iter()
            .map(|i| LabelPick { node: i as u32, text: label_text(graph, layout, i), importance: node_importance[i] })
            .collect()
    };

    Look {
        node_color,
        node_radius,
        node_importance,
        edge_color,
        halo_group,
        halo_color,
        labels,
        label_count,
        halo_alpha: tune.halo_alpha as f32,
        halo_radius_boost: tune.halo_radius_boost as f32,
    }
}

fn pick_hex<'a>(preferred: &'a str, fallback: &'a str) -> &'a str {
    if rgb(preferred).is_some() { preferred } else { fallback }
}

fn rgb_f32(c: Rgb) -> [f32; 3] {
    c.map(|v| f32::from(v) / 255.0)
}

fn rgba(c: Rgb, alpha: f32) -> [f32; 4] {
    let [r, g, b] = rgb_f32(c);
    [r, g, b, alpha.clamp(0.0, 1.0)]
}

/// Note title, truncated like the web version (28 characters with an ellipsis).
fn label_text(graph: &Graph, layout: &LayoutGraph, i: usize) -> String {
    let title = graph.nodes.get(i).and_then(|n| n.label.as_deref()).unwrap_or(&layout.id[i]);
    if title.chars().count() > LABEL_MAX_CHARS {
        let mut s: String = title.chars().take(LABEL_MAX_CHARS - 1).collect();
        s.push('…');
        s
    } else {
        title.to_string()
    }
}

/// Rank by frequency, most common first, ties by name (JavaScript string order).
fn rank_map<'a>(values: impl Iterator<Item = &'a str>) -> HashMap<&'a str, usize> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for v in values.filter(|v| !v.is_empty()) {
        *counts.entry(v).or_insert(0) += 1;
    }
    let mut list: Vec<(&str, usize)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.encode_utf16().cmp(b.0.encode_utf16())));
    list.into_iter().enumerate().map(|(i, (v, _))| (v, i)).collect()
}

/// Oldest and newest non-zero mtime, when they differ.
fn age_stats(mtime: &[f64]) -> Option<(f64, f64)> {
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for &t in mtime.iter().filter(|&&t| t != 0.0) {
        min = min.min(t);
        max = max.max(t);
    }
    (min.is_finite() && max > min).then_some((min, max))
}

/// 0 (newest fifth of the vault's time span) to 4 (oldest, or no mtime).
fn age_bucket(stats: Option<(f64, f64)>, mtime: f64) -> usize {
    let last = AGE_PALETTE.len() - 1;
    let Some((min, max)) = stats else { return last };
    if mtime == 0.0 {
        return last;
    }
    let t = (mtime - min) / (max - min);
    (((1.0 - t) * AGE_PALETTE.len() as f64).floor() as usize).min(last)
}

fn node_days(mtime: f64, now_ms: f64) -> f64 {
    if mtime == 0.0 { 9999.0 } else { ((now_ms - mtime) / DAY_MS).max(0.0) }
}

/// JavaScript's `hashStr`: `h = h * 31 + code unit`, wrapping in 32 bits.
fn hash_str(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_shl(5).wrapping_sub(h).wrapping_add(i32::from(c)))
}

/// Deterministic per-folder colour (`folderColor`): hue from the name's hash.
fn folder_color(folder: &str) -> Rgb {
    let hue = f64::from(hash_str(folder).rem_euclid(360));
    hsl_to_rgb(hue, 60.0, 64.0)
}

/// `hslToHex` from `renderer-core.js`, rounding each channel the same way.
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Rgb {
    let (s, l) = (s / 100.0, l / 100.0);
    let k = |n: f64| (n + h / 30.0) % 12.0;
    let a = s * l.min(1.0 - l);
    let f = |n: f64| {
        let c = l - a * (k(n) - 3.0).min(9.0 - k(n)).clamp(-1.0, 1.0);
        (255.0 * c).round() as u8
    };
    [f(0.0), f(8.0), f(4.0)]
}

/// WCAG relative luminance.
fn luminance(c: Rgb) -> f64 {
    let lin = |v: u8| {
        let s = f64::from(v) / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    std::array::from_fn(|i| {
        let (x, y) = (f64::from(a[i]), f64::from(b[i]));
        (x + (y - x) * t).round().clamp(0.0, 255.0) as u8
    })
}

/// The accent's contrast floor (`ensureContrast`): push `fg` toward white on dark
/// backgrounds, black on light ones, in 5% steps until the ratio reaches `min`.
fn ensure_contrast(fg: Rgb, bg: Rgb, min: f64) -> Rgb {
    if contrast_ratio(fg, bg) >= min {
        return fg;
    }
    let target = if luminance(bg) < 0.4 { [255; 3] } else { [0; 3] };
    let mut t = 0.05;
    while t <= 1.0001 {
        let c = mix(fg, target, t);
        if contrast_ratio(c, bg) >= min {
            return c;
        }
        t += 0.05;
    }
    target
}

/// `computeFocusSet`: the notes within `focusHops` links of `focusNote` (matched
/// case-insensitively on id or title; `today` means today's `YYYY-MM-DD` note).
/// `None` when no focus is set or the note isn't found, so everything shows.
fn focus_set(graph: &Graph, layout: &LayoutGraph, cfg: &Config, now_ms: f64) -> Option<Vec<bool>> {
    let q = cfg.focus_note.trim();
    if q.is_empty() || layout.is_empty() {
        return None;
    }
    let want = if q.eq_ignore_ascii_case("today") { local_day_key(now_ms) } else { q.to_lowercase() };
    let root = (0..layout.len()).find(|&i| {
        let title = graph.nodes.get(i).and_then(|n| n.label.as_deref()).unwrap_or(&layout.id[i]);
        layout.id[i].to_lowercase() == want || title.to_lowercase() == want
    })?;
    let neighbours = layout.neighbours();
    let mut dist = vec![u32::MAX; layout.len()];
    dist[root] = 0;
    let mut queue = vec![root];
    let mut head = 0;
    while head < queue.len() {
        let u = queue[head];
        head += 1;
        if dist[u] >= cfg.focus_hops {
            continue;
        }
        for &v in &neighbours[u] {
            let v = v as usize;
            if dist[v] == u32::MAX {
                dist[v] = dist[u] + 1;
                queue.push(v);
            }
        }
    }
    Some(dist.into_iter().map(|d| d != u32::MAX).collect())
}

/// Local date of `now_ms` as `YYYY-MM-DD` (the web version's `dayKey`).
fn local_day_key(now_ms: f64) -> String {
    let secs = (now_ms / 1000.0).floor() as libc::time_t;
    // SAFETY: localtime_r only writes into the zeroed `tm` we own.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            return String::new();
        }
        tm
    };
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}


#[cfg(test)]
mod tests {
    use super::*;
    use vw_core::config::NodeColorMode;
    use vw_core::vault::{Link, Node};

    const NOW: f64 = 1_800_000_000_000.0;

    fn node(id: &str, tag: Option<&str>, folder: Option<&str>, mtime: Option<f64>) -> Node {
        Node {
            id: id.into(),
            label: None,
            tag: tag.map(Into::into),
            tags: None,
            mtime,
            folder: folder.map(Into::into),
            ghost: false,
        }
    }

    fn ghost(id: &str) -> Node {
        Node { ghost: true, ..node(id, None, None, None) }
    }

    fn link(s: &str, t: &str) -> Link {
        Link { source: s.into(), target: t.into() }
    }

    /// hub links to a1..a3 (#alpha) and b1 (#beta); b1 links to b2; c (no tag) to ghost g.
    fn sample() -> Graph {
        let day = DAY_MS;
        Graph {
            nodes: vec![
                node("hub", Some("alpha"), Some("Projects"), Some(NOW - day)),
                node("a1", Some("alpha"), Some("Projects"), Some(NOW - 2.0 * day)),
                node("a2", Some("alpha"), Some("Areas"), Some(NOW - 30.0 * day)),
                node("a3", Some("alpha"), None, Some(NOW - 40.0 * day)),
                node("b1", Some("beta"), Some("Areas"), Some(NOW - 100.0 * day)),
                node("b2", Some("beta"), None, Some(NOW - 3.0 * day)),
                node("c", None, None, None),
                ghost("g"),
            ],
            links: vec![
                link("hub", "a1"),
                link("hub", "a2"),
                link("hub", "a3"),
                link("hub", "b1"),
                link("b1", "b2"),
                link("c", "g"),
            ],
            ..Graph::default()
        }
    }

    fn look_with(g: &Graph, cfg: &Config, light: bool) -> Look {
        derive_look(g, &LayoutGraph::from_graph(g), cfg, &LookContext { now_ms: NOW, light })
    }

    fn look(cfg: &Config) -> Look {
        look_with(&sample(), cfg, false)
    }

    fn hex(h: &str) -> [f32; 3] {
        rgb_f32(rgb(h).unwrap())
    }

    fn rgb3(c: [f32; 4]) -> [f32; 3] {
        [c[0], c[1], c[2]]
    }

    const ACCENT: &str = "#7c5cff";

    #[test]
    fn default_tag_mode_uses_accent_for_everyone() {
        let l = look(&Config::default());
        assert_eq!(l.node_color.len(), 8);
        for c in &l.node_color {
            assert_eq!(rgb3(*c), hex(ACCENT));
        }
        assert_eq!(l.edge_color.len(), 6);
    }

    #[test]
    fn palette_colours_tags_by_frequency_rank() {
        let cfg = Config { palette: Palette::Riso, ..Config::default() };
        let l = look(&cfg);
        // alpha (4 notes) ranks 0, beta (2) ranks 1; untagged notes keep the accent.
        assert_eq!(rgb3(l.node_color[0]), hex(RISO[0]));
        assert_eq!(rgb3(l.node_color[4]), hex(RISO[1]));
        assert_eq!(rgb3(l.node_color[6]), hex(ACCENT));
        assert_eq!(rgb3(l.node_color[7]), hex(ACCENT));
        let cfg = Config { palette: Palette::Viridis, ..Config::default() };
        assert_eq!(rgb3(look(&cfg).node_color[0]), hex(VIRIDIS[0]));
    }

    #[test]
    fn tag_colors_override_palette() {
        let mut cfg = Config { palette: Palette::Neon, ..Config::default() };
        cfg.tag_colors.insert("beta".into(), "#123456".into());
        cfg.tag_colors.insert("alpha".into(), "not a colour".into());
        let l = look(&cfg);
        assert_eq!(rgb3(l.node_color[4]), hex("#123456"));
        assert_eq!(rgb3(l.node_color[5]), hex("#123456"));
        // An invalid override falls through to the palette.
        assert_eq!(rgb3(l.node_color[0]), hex(NEON[0]));
    }

    #[test]
    fn age_mode_buckets_by_vault_time_span() {
        let cfg = Config { node_color_mode: NodeColorMode::Age, ..Config::default() };
        let l = look(&cfg);
        // Newest (hub) is fresh green, oldest (b1) and notes without mtime are stale red.
        assert_eq!(rgb3(l.node_color[0]), hex(AGE_PALETTE[0]));
        assert_eq!(rgb3(l.node_color[4]), hex(AGE_PALETTE[4]));
        assert_eq!(rgb3(l.node_color[6]), hex(AGE_PALETTE[4]));
        assert_eq!(rgb3(l.node_color[7]), hex(AGE_PALETTE[4]));
        // a3 at 40 days of a 1..100 day span: t = 0.606, bucket floor(0.394 * 5) = 1.
        assert_eq!(rgb3(l.node_color[3]), hex(AGE_PALETTE[1]));
        let cfg = Config { palette: Palette::Metro, ..cfg };
        assert_eq!(rgb3(look(&cfg).node_color[4]), hex(METRO[4]));
    }

    #[test]
    fn folder_mode_hashes_or_uses_palette() {
        let cfg = Config { node_color_mode: NodeColorMode::Folder, ..Config::default() };
        let l = look(&cfg);
        assert_eq!(rgb3(l.node_color[0]), rgb_f32(folder_color("Projects")));
        assert_eq!(rgb3(l.node_color[0]), rgb3(l.node_color[1]));
        assert_ne!(rgb3(l.node_color[0]), rgb3(l.node_color[2]));
        assert_eq!(rgb3(l.node_color[3]), hex(ACCENT));
        // Areas (2 notes) and Projects (2 notes) tie on count; ties go by name.
        let cfg = Config { palette: Palette::Earth, ..cfg };
        let l = look(&cfg);
        assert_eq!(rgb3(l.node_color[2]), hex(EARTH[0]));
        assert_eq!(rgb3(l.node_color[0]), hex(EARTH[1]));
    }

    #[test]
    fn folder_colour_matches_the_web_version() {
        // hashStr('Projects') and hslToHex(hue, 60, 64), worked out in node.
        assert_eq!(hash_str("Projects"), -934_052_710);
        assert_eq!(folder_color("Projects"), rgb("#dac86c").unwrap());
        assert_eq!(hash_str(""), 0);
        assert_eq!(hsl_to_rgb(0.0, 60.0, 64.0), [218, 108, 108]);
        assert_eq!(hsl_to_rgb(120.0, 60.0, 64.0), [108, 218, 108]);
    }

    #[test]
    fn recency_mode_dims_stale_notes_and_ghosts() {
        let cfg = Config { node_color_mode: NodeColorMode::Recency, recency_days: 7, ..Config::default() };
        let l = look(&cfg);
        let fresh = l.node_color[1][3]; // a1, 2 days old
        let stale = l.node_color[2][3]; // a2, 30 days old
        assert!((stale - (0.55 + l.node_importance[2] * 0.35) * 0.22).abs() < 1e-6);
        assert!(fresh > 0.55);
        assert!((l.node_color[7][3] - (0.55 + l.node_importance[7] * 0.35) * 0.35 * 0.22).abs() < 1e-6);
        // Colour still comes from the tag.
        assert_eq!(rgb3(l.node_color[1]), hex(ACCENT));
        // A link between two stale notes is dimmed to a quarter; c–g are both stale.
        let plain = look(&Config::default());
        assert!((l.edge_color[5][3] - plain.edge_color[5][3] * 0.25).abs() < 1e-6);
    }

    #[test]
    fn ghosts_are_faint_and_small() {
        let l = look(&Config::default());
        let g = 7;
        assert!((l.node_color[g][3] - (0.55 + l.node_importance[g] * 0.35) * 0.35).abs() < 1e-6);
        assert!((l.node_radius[g] - 2.3).abs() < 1e-6); // 1.5 + sqrt(1) * 0.8
        assert!((l.node_radius[6] - 3.65).abs() < 1e-6); // 2.2 + sqrt(1) * 1.45
        assert!(l.labels.iter().all(|p| p.node != g as u32));
    }

    #[test]
    fn importance_and_radius_follow_degree() {
        let l = look(&Config::default());
        assert_eq!(l.node_importance[0], 1.0); // hub, degree 4
        assert_eq!(l.node_importance[1], 0.25);
        assert_eq!(l.node_importance[4], 0.5);
        assert!((l.node_radius[0] - (2.2 + 2.0 * 1.45)).abs() < 1e-6);
        assert!((l.node_color[0][3] - 0.9).abs() < 1e-6);
    }

    #[test]
    fn edges_take_source_colour_or_accent() {
        let mut cfg = Config::default();
        cfg.tag_colors.insert("beta".into(), "#00ff00".into());
        let l = look(&cfg);
        // b1 -> b2 takes b1's colour; hub -> b1 takes the hub's.
        assert_eq!(rgb3(l.edge_color[4]), hex("#00ff00"));
        assert_eq!(rgb3(l.edge_color[3]), hex(ACCENT));
        // alpha = linkOpacity * (0.3 + max importance * 0.7)
        assert!((l.edge_color[0][3] - 0.18).abs() < 1e-6);
        assert!((l.edge_color[4][3] - 0.18 * (0.3 + 0.5 * 0.7)).abs() < 1e-6);
        let cfg = Config { edge_coloring: false, link_opacity: 0.5, ..cfg };
        let l = look(&cfg);
        assert_eq!(rgb3(l.edge_color[4]), hex(ACCENT));
        assert!((l.edge_color[0][3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn light_mode_switches_accent() {
        let cfg = Config::default();
        let dark = look_with(&sample(), &cfg, false);
        let light = look_with(&sample(), &cfg, true);
        assert_eq!(rgb3(dark.node_color[0]), hex(ACCENT));
        assert_eq!(rgb3(light.node_color[0]), hex("#39407a"));
        assert_eq!(rgb3(light.edge_color[0]), hex("#39407a"));
    }

    #[test]
    fn accent_gets_a_contrast_floor() {
        let cfg = Config { accent: "#111111".into(), background: "#0a0a0f".into(), ..Config::default() };
        let c = rgb3(look(&cfg).node_color[6]);
        assert_ne!(c, hex("#111111"));
        let c = [(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8];
        assert!(contrast_ratio(c, [10, 10, 15]) >= 1.8);
    }

    #[test]
    fn labels_off_by_default() {
        let l = look(&Config::default());
        assert!(l.labels.is_empty());
        assert_eq!(l.label_count, 0);
    }

    #[test]
    fn hub_labels_most_important_first() {
        let mut g = sample();
        g.nodes[0].label = Some("The Hub".into());
        let cfg = Config { hub_labels: true, hub_label_count: 2, label_min_importance: 0.3, ..Config::default() };
        let l = look_with(&g, &cfg, false);
        assert_eq!(l.label_count, 2);
        let picks: Vec<(u32, &str)> = l.labels.iter().map(|p| (p.node, p.text.as_str())).collect();
        // hub (1.0), b1 (0.5); notes at 0.25 are under the floor.
        assert_eq!(picks, vec![(0, "The Hub"), (4, "b1")]);
        assert_eq!(l.labels[0].importance, 1.0);
    }

    #[test]
    fn recent_labels_newest_first() {
        let cfg = Config { hub_labels: true, label_source: LabelSource::Recent, ..Config::default() };
        let l = look(&cfg);
        let order: Vec<u32> = l.labels.iter().map(|p| p.node).collect();
        // Ghosts and notes without mtime are left out.
        assert_eq!(order, vec![0, 1, 5, 2, 3, 4]);
    }

    #[test]
    fn long_titles_are_truncated() {
        let mut g = sample();
        g.nodes[0].label = Some("a".repeat(40));
        let cfg = Config { hub_labels: true, ..Config::default() };
        let l = look_with(&g, &cfg, false);
        assert_eq!(l.labels[0].text.chars().count(), 28);
        assert!(l.labels[0].text.ends_with('…'));
    }

    #[test]
    fn large_vaults_get_fewer_labels_and_no_halos() {
        let n = 12_000;
        let g = Graph {
            nodes: (0..n).map(|i| node(&format!("n{i}"), Some("t"), None, None)).collect(),
            links: (1..n).map(|i| link("n0", &format!("n{i}"))).collect(),
            ..Graph::default()
        };
        let cfg = Config { hub_labels: true, hub_label_count: 5, ..Config::default() };
        let l = look_with(&g, &cfg, false);
        assert_eq!(l.label_count, 2);
        assert_eq!(l.labels.len(), 1); // only n0 clears the 0.78 floor
        assert_eq!(l.halo_alpha, 0.0);
        assert!(l.halo_color.is_empty() && l.halo_group.iter().all(Option::is_none));
        assert!((l.edge_color[0][3] - 0.18 * 0.32).abs() < 1e-6);
        let cfg = Config { auto_scale_large_vaults: false, ..cfg };
        let l = look_with(&g, &cfg, false);
        assert_eq!(l.label_count, 5);
        assert!((l.halo_alpha - 0.11).abs() < 1e-6);
    }

    #[test]
    fn halos_group_tags_with_three_or_more_notes() {
        let mut cfg = Config::default();
        cfg.tag_colors.insert("alpha".into(), "#ff0000".into());
        let l = look(&cfg);
        assert!((l.halo_alpha - 0.11).abs() < 1e-6);
        assert_eq!(l.halo_color, vec![hex("#ff0000")]);
        assert_eq!(&l.halo_group[..4], &[Some(0); 4]);
        assert!(l.halo_group[4..].iter().all(Option::is_none)); // beta has only two notes
        let cfg = Config { cluster_halos: false, ..cfg };
        let l = look(&cfg);
        assert_eq!(l.halo_alpha, 0.0);
        assert!(l.halo_color.is_empty());
    }

    #[test]
    fn focus_hides_notes_beyond_the_hops() {
        let cfg = Config { focus_note: "B2".into(), focus_hops: 1, hub_labels: true, ..Config::default() };
        let l = look(&cfg);
        let visible: Vec<usize> = (0..8).filter(|&i| l.node_color[i][3] > 0.0).collect();
        assert_eq!(visible, vec![4, 5]);
        let lit: Vec<usize> = (0..6).filter(|&i| l.edge_color[i][3] > 0.0).collect();
        assert_eq!(lit, vec![4]);
        assert!(l.labels.iter().all(|p| p.node == 4 || p.node == 5));
        let cfg = Config { focus_hops: 2, ..cfg };
        assert!(look(&cfg).node_color[0][3] > 0.0);
        // An unknown note means no focus at all.
        let cfg = Config { focus_note: "nope".into(), ..cfg };
        assert!(look(&cfg).node_color.iter().all(|c| c[3] > 0.0));
    }

    #[test]
    fn focus_today_finds_the_daily_note() {
        let key = local_day_key(NOW);
        assert_eq!(key.len(), 10);
        let mut g = sample();
        g.nodes.push(node(&key, None, None, None));
        let cfg = Config { focus_note: "today".into(), focus_hops: 1, ..Config::default() };
        let l = look_with(&g, &cfg, false);
        let visible: Vec<usize> = (0..g.nodes.len()).filter(|&i| l.node_color[i][3] > 0.0).collect();
        assert_eq!(visible, vec![8]);
    }

    #[test]
    fn empty_graph() {
        let l = look_with(&Graph::default(), &Config { hub_labels: true, ..Config::default() }, false);
        assert!(l.node_color.is_empty() && l.labels.is_empty() && l.halo_color.is_empty());
    }
}
