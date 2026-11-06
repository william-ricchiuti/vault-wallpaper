//! Turning layout positions and graph data into GPU instances.

use vw_core::layout::LayoutGraph;

use crate::{EdgeInstance, Framing, NodeInstance};

/// Colours and strengths for the plain look. Phase 4 grows this into the full
/// default renderer's options.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneStyle {
    pub background: [f32; 3],
    /// Centre lift colour, and its strength in alpha (0 = flat background).
    pub background_glow: [f32; 4],
    pub accent: [f32; 3],
    pub link_opacity: f32,
    /// Stroke width of links, in points.
    pub link_width: f32,
}

impl Default for SceneStyle {
    fn default() -> Self {
        // The web version's defaults: accent #7c5cff on #0a0a0f, links at 0.18.
        let background = parse_hex("#0a0a0f").unwrap();
        SceneStyle {
            background,
            background_glow: [0.07, 0.06, 0.12, 0.6],
            accent: parse_hex("#7c5cff").unwrap(),
            link_opacity: 0.18,
            link_width: 0.8,
        }
    }
}

/// "#rrggbb" to 0..1 sRGB values.
pub fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0])
}

/// A stable colour per tag: hue from a hash of the name, fixed saturation and
/// lightness so every tag reads well on a dark background.
pub fn tag_color(tag: &str) -> [f32; 3] {
    let h = tag.bytes().fold(2_166_136_261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619));
    hsl_to_rgb((h % 360) as f32, 0.62, 0.64)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

/// Builds instances for one screen. `positions` are layout coordinates in the same
/// order as `graph`'s nodes.
pub fn build_instances(graph: &LayoutGraph, positions: &[[f32; 2]], framing: &Framing, style: &SceneStyle) -> (Vec<NodeInstance>, Vec<EdgeInstance>) {
    let screen: Vec<[f32; 2]> = positions.iter().map(|p| framing.apply(*p)).collect();
    let [ar, ag, ab] = style.accent;
    let edges = graph
        .links
        .iter()
        .filter_map(|&(s, t)| {
            let (a, b) = (*screen.get(s as usize)?, *screen.get(t as usize)?);
            Some(EdgeInstance { a, b, color: [ar, ag, ab, style.link_opacity], width: style.link_width })
        })
        .collect();
    // Radius from degree, like the web version's renderer (not the layout's
    // collision radius).
    let mut order: Vec<usize> = (0..screen.len()).collect();
    // Small notes first so hubs draw on top.
    order.sort_by_key(|&i| graph.degree[i]);
    let nodes = order
        .into_iter()
        .map(|i| {
            let d = (graph.degree[i] as f32).sqrt();
            let (radius, color) = if graph.ghost[i] {
                (1.3, [0.55, 0.55, 0.62, 0.45])
            } else {
                let [r, g, b] = graph.tag[i].as_deref().map_or(style.accent, tag_color);
                ((1.5 + d * 1.05) * 1.2, [r, g, b, 0.95])
            };
            NodeInstance { center: screen[i], radius, color }
        })
        .collect();
    (nodes, edges)
}
