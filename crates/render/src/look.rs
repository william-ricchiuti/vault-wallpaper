//! What each note and link looks like under the current config: colour, size,
//! importance and which notes get labels. A port of the web version's derived-visuals
//! logic (`refreshDerivedVisuals`, `nodeColor`, `edgeColor`, label candidates in
//! `index.html`), recomputed only when the graph or config changes.
//!
//! STUB: the signatures are fixed (the renderer is built against them); the body is
//! being written in a parallel session.

use vw_core::config::Config;
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
    /// Notes to label, most important first.
    pub labels: Vec<LabelPick>,
}

/// Derives the look. `graph` and `layout` describe the same graph in the same node
/// order (`LayoutGraph::from_graph(graph)`).
pub fn derive_look(graph: &Graph, layout: &LayoutGraph, cfg: &Config, ctx: &LookContext) -> Look {
    let _ = (graph, cfg, ctx);
    let n = layout.len();
    Look {
        node_color: vec![[0.49, 0.36, 1.0, 0.95]; n],
        node_radius: layout.degree.iter().map(|&d| (1.5 + (d as f32).sqrt() * 1.05) * 1.2).collect(),
        node_importance: vec![0.0; n],
        edge_color: vec![[0.49, 0.36, 1.0, 0.18]; layout.links.len()],
        halo_group: vec![None; n],
        halo_color: Vec::new(),
        labels: Vec::new(),
    }
}
