//! Graph layout: where every note sits. Runs off the render thread, caches positions
//! so a relaunch opens on the settled picture, and only relaxes the neighbourhood of
//! a change after an edit.
//!
//! Coordinates are in "layout pixels" around (0, 0), computed for a reference viewport
//! (`LayoutOptions::width`/`height`). The renderer frames them onto real screens.

mod cache;
mod engine;
mod quadtree;
mod simulation;
mod targets;

use std::collections::HashMap;

use crate::vault::Graph;

pub use cache::PositionCache;
pub use engine::{LayoutEngine, LayoutFrame};
pub use simulation::Simulation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutKind {
    #[default]
    Force,
    /// One sector per tag, hubs near the middle.
    Rings,
    /// Old notes on the left, today on the right, one lane per tag.
    Timeline,
    /// One spiral arm per tag.
    Galaxy,
}

impl LayoutKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "force" => Self::Force,
            "rings" => Self::Rings,
            "timeline" => Self::Timeline,
            "galaxy" => Self::Galaxy,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Force => "force",
            Self::Rings => "rings",
            Self::Timeline => "timeline",
            Self::Galaxy => "galaxy",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutOptions {
    pub kind: LayoutKind,
    /// Pull same-tag notes together (`clusterByTag`).
    pub cluster_by_tag: bool,
    /// Reference viewport the layout is computed for.
    pub width: f64,
    pub height: f64,
    /// "Now" for the timeline layout, in ms since the epoch.
    pub now_ms: f64,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
        LayoutOptions { kind: LayoutKind::Force, cluster_by_tag: true, width: 1920.0, height: 1080.0, now_ms }
    }
}

/// The graph reduced to what layout needs, with links as node indices.
#[derive(Debug, Clone, Default)]
pub struct LayoutGraph {
    pub id: Vec<String>,
    pub tag: Vec<Option<String>>,
    pub ghost: Vec<bool>,
    pub mtime: Vec<f64>,
    pub links: Vec<(u32, u32)>,
    pub degree: Vec<u32>,
    /// Node radius in layout pixels, from its degree (the web version's formula).
    pub radius: Vec<f64>,
}

impl LayoutGraph {
    pub fn from_graph(g: &Graph) -> Self {
        let index: HashMap<&str, u32> = g.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i as u32)).collect();
        let links: Vec<(u32, u32)> = g
            .links
            .iter()
            .filter_map(|l| Some((*index.get(l.source.as_str())?, *index.get(l.target.as_str())?)))
            .collect();
        let mut degree = vec![0u32; g.nodes.len()];
        for &(s, t) in &links {
            degree[s as usize] += 1;
            degree[t as usize] += 1;
        }
        let radius = g
            .nodes
            .iter()
            .zip(&degree)
            .map(|(n, &d)| {
                let d = f64::from(d).sqrt();
                if n.ghost { 1.5 + d * 0.8 } else { 2.2 + d * 1.45 }
            })
            .collect();
        LayoutGraph {
            id: g.nodes.iter().map(|n| n.id.clone()).collect(),
            tag: g.nodes.iter().map(|n| n.tag.clone()).collect(),
            ghost: g.nodes.iter().map(|n| n.ghost).collect(),
            mtime: g.nodes.iter().map(|n| n.mtime.unwrap_or(0.0)).collect(),
            links,
            degree,
            radius,
        }
    }

    pub fn len(&self) -> usize {
        self.id.len()
    }

    /// Per-node fingerprint of what affects its place: tag, ghost flag and the ids it
    /// links with. Comparing fingerprints tells which notes an edit touched.
    pub fn fingerprints(&self) -> Vec<u64> {
        use std::hash::{Hash, Hasher};
        let neighbours = self.neighbours();
        (0..self.len())
            .map(|i| {
                let mut ids: Vec<&str> = neighbours[i].iter().map(|&j| self.id[j as usize].as_str()).collect();
                ids.sort_unstable();
                let mut h = rustc_hash::FxHasher::default();
                (self.tag[i].as_deref(), self.ghost[i], ids).hash(&mut h);
                h.finish()
            })
            .collect()
    }

    /// Linked node indices per node (both directions).
    pub fn neighbours(&self) -> Vec<Vec<u32>> {
        let mut out = vec![Vec::new(); self.len()];
        for &(s, t) in &self.links {
            out[s as usize].push(t);
            out[t as usize].push(s);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.id.is_empty()
    }

}

/// Starting state for a simulation: positions and velocities, plus how many nodes
/// kept a previous position.
pub struct Placement {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub vx: Vec<f64>,
    pub vy: Vec<f64>,
    pub kept: usize,
}

/// A deterministic pair of numbers in [0, 1) for a node id, so new notes land in the
/// same spot every time.
fn id_noise(id: &str) -> (f64, f64) {
    let mut h = targets::hash01(id).to_bits() ^ 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        h = h.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = h;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    };
    (next(), next())
}

/// Positions for `graph`: previous positions where known; new notes next to the
/// notes they link to (with a little jitter), or scattered near the centre.
pub fn place(graph: &LayoutGraph, previous: &HashMap<String, [f64; 4]>, opts: &LayoutOptions) -> Placement {
    let n = graph.len();
    let mut p = Placement { x: vec![f64::NAN; n], y: vec![f64::NAN; n], vx: vec![0.0; n], vy: vec![0.0; n], kept: 0 };
    let mut known = vec![false; n];
    for (i, id) in graph.id.iter().enumerate() {
        if let Some(&[x, y, vx, vy]) = previous.get(id) {
            (p.x[i], p.y[i], p.vx[i], p.vy[i]) = (x, y, vx, vy);
            known[i] = true;
            p.kept += 1;
        }
    }
    let mut sum = vec![(0.0, 0.0, 0u32); n];
    for &(s, t) in &graph.links {
        let (s, t) = (s as usize, t as usize);
        if known[t] && !known[s] {
            sum[s].0 += p.x[t];
            sum[s].1 += p.y[t];
            sum[s].2 += 1;
        }
        if known[s] && !known[t] {
            sum[t].0 += p.x[s];
            sum[t].1 += p.y[s];
            sum[t].2 += 1;
        }
    }
    let spread = opts.width.min(opts.height) * 0.4;
    for (i, &was_known) in known.iter().enumerate() {
        if was_known {
            continue;
        }
        let (a, b) = id_noise(&graph.id[i]);
        let (sx, sy, c) = sum[i];
        if c > 0 {
            p.x[i] = sx / f64::from(c) + (a - 0.5) * 20.0;
            p.y[i] = sy / f64::from(c) + (b - 0.5) * 20.0;
        } else {
            p.x[i] = (a - 0.5) * spread;
            p.y[i] = (b - 0.5) * spread;
        }
    }
    p
}

/// Runs a simulation to rest on the calling thread. Returns the number of ticks.
pub fn settle(sim: &mut Simulation, max_ticks: u64) -> u64 {
    while !sim.is_settled() && sim.ticks() < max_ticks {
        sim.tick();
    }
    sim.ticks()
}
