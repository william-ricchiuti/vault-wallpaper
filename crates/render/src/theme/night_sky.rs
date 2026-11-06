//! Night Sky: notes are stars, links within a tag are constellation lines, and a faint
//! starfield fills the background. Hubs get diffraction spikes. With motion on, the
//! stars' glow twinkles slowly (window-server breathing, no app frames).

use vw_core::config::{Config, MotionMode};
use vw_core::layout::LayoutGraph;

use crate::look::Look;
use crate::scene::{BgMode, EdgeStyle, SceneData, SceneStyle, place_labels};
use crate::text::{FontFamily, TextStyle};
use crate::{EdgeInstance, Framing, NodeInstance};

/// Star colours by temperature: white, blue-white, pale yellow, warm, blue.
const TEMPERATURES: [[f32; 3]; 5] = [
    [1.0, 1.0, 1.0],
    [0.80, 0.87, 1.0],
    [1.0, 0.95, 0.80],
    [1.0, 0.83, 0.69],
    [0.72, 0.81, 1.0],
];

fn temperature(tag: Option<&str>) -> [f32; 3] {
    let Some(tag) = tag else { return TEMPERATURES[0] };
    let h = tag.bytes().fold(2_166_136_261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619));
    TEMPERATURES[(h % TEMPERATURES.len() as u32) as usize]
}

pub(super) fn style(cfg: &Config, base: SceneStyle) -> SceneStyle {
    // Twinkling is slow and shallow; still unless the config asks for motion.
    let breathing = (cfg.motion_mode != MotionMode::Still).then_some((0.35, 46.0));
    SceneStyle {
        background: [0.016, 0.024, 0.055],
        background_glow: [0.05, 0.08, 0.17, 1.0],
        accent: [0.81, 0.85, 1.0],
        edge_style: EdgeStyle::Line,
        glow: 0.9,
        chromatic: 0.0,
        vignette: 0.45,
        grain: 0.0,
        halos: false,
        depth_of_field: false,
        breathing,
        motion_fps: base.motion_fps.max(if breathing.is_some() { 6.0 } else { 0.0 }),
        labels: base.labels,
        label_font: FontFamily::Serif,
        label_style: TextStyle::Minimal,
        auto_scale: base.auto_scale,
        label_plate_alpha: 0.0,
        bg_mode: BgMode::Starfield,
    }
}

pub(super) fn build(
    graph: &LayoutGraph,
    look: &Look,
    positions: &[[f32; 2]],
    framing: &Framing,
    style: &SceneStyle,
    size: [f32; 2],
) -> SceneData {
    let n = positions.len().min(graph.len());
    let screen: Vec<[f32; 2]> = positions[..n].iter().map(|p| framing.apply(*p)).collect();
    let imp = |i: usize| look.node_importance.get(i).copied().unwrap_or(0.0);
    let visible = |i: usize| look.node_color.get(i).is_none_or(|c| c[3] > 0.0);
    let mut data = SceneData::default();

    // Constellations: only links between notes of the same tag, faint and thin.
    for &(s, t) in &graph.links {
        let (s, t) = (s as usize, t as usize);
        if s >= n || t >= n || graph.tag[s].is_none() || graph.tag[s] != graph.tag[t] || !visible(s) || !visible(t) {
            continue;
        }
        let i = imp(s).max(imp(t));
        data.edges.push(EdgeInstance {
            a: screen[s],
            b: screen[t],
            color: [0.56, 0.64, 0.86, 0.10 + i * 0.25],
            width: 0.5 + i * 0.5,
        });
    }

    // Dim stars first so bright ones draw on top.
    let mut order: Vec<usize> = (0..n).filter(|&i| visible(i)).collect();
    order.sort_by(|&a, &b| imp(a).total_cmp(&imp(b)));
    for i in order {
        let [r, g, b] = temperature(graph.tag[i].as_deref());
        let p = imp(i);
        if graph.ghost[i] {
            data.nodes.push(NodeInstance { center: screen[i], radius: 0.6, color: [r, g, b, 0.25] });
            continue;
        }
        let core = 0.7 + p.sqrt() * 2.6;
        data.nodes.push(NodeInstance { center: screen[i], radius: core, color: [r, g, b, 0.75 + p * 0.25] });
        data.glow.push(NodeInstance { center: screen[i], radius: core * 4.0 + 3.0, color: [r, g, b, 0.10 + p * 0.35] });
        if p > 0.22 {
            data.stars.push(NodeInstance { center: screen[i], radius: 8.0 + p * 26.0, color: [r, g, b, 0.55 + p * 0.4] });
        }
    }

    if style.labels && look.label_count > 0 {
        let no_dof = |_: [f32; 2]| 0.0;
        data.text = place_labels(look, &screen, n, size, style, false, &no_dof);
        for t in &mut data.text {
            t.color = [0.86, 0.89, 1.0, 0.72];
            t.size *= 0.92;
        }
    }
    data
}
