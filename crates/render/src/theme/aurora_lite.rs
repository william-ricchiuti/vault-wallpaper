//! Aurora Lite: soft curtains of coloured light rise above the biggest tag clusters,
//! like an aurora over a dark landscape. Notes are small bright points under them and
//! links are barely there.
//!
//! Lightness: the curtains live in the quarter-resolution glow layer (already blurred
//! and composited), are drawn once per scene change, and with motion on they breathe
//! through the window server's opacity animation of that layer. No app frames.

use std::collections::HashMap;

use vw_core::config::{Config, MotionMode};
use vw_core::layout::LayoutGraph;

use crate::look::Look;
use crate::scene::{BgMode, EdgeStyle, SceneData, SceneStyle, place_labels};
use crate::text::{FontFamily, TextStyle};
use crate::{CurtainInstance, EdgeInstance, Framing, NodeInstance};

/// A calm aurora palette: (base colour, colour at the curtain's top).
const PALETTE: [([f32; 3], [f32; 3]); 5] = [
    // Green rising into violet.
    ([0.24, 0.95, 0.55], [0.42, 0.36, 0.90]),
    // Teal into blue-violet.
    ([0.16, 0.78, 0.80], [0.34, 0.42, 0.98]),
    // Violet into magenta.
    ([0.58, 0.44, 1.00], [0.86, 0.38, 0.84]),
    // Mint into teal.
    ([0.46, 1.00, 0.70], [0.18, 0.62, 0.86]),
    // A little magenta, into violet.
    ([0.88, 0.34, 0.74], [0.48, 0.32, 0.92]),
];
/// Order the biggest clusters take colours in: magenta comes up rarely.
const RANK_ORDER: [usize; 8] = [0, 2, 1, 3, 2, 4, 1, 0];
/// The most curtains: past this the sky turns to mud.
const MAX_CURTAINS: usize = 7;

fn hash(s: &str) -> u32 {
    s.bytes().fold(2_166_136_261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619))
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

pub(super) fn style(cfg: &Config, base: SceneStyle) -> SceneStyle {
    // A slow, shallow swell of the whole sky; still unless the config asks for motion.
    let breathing = (cfg.motion_mode != MotionMode::Still).then_some((0.4, 38.0));
    SceneStyle {
        background: [0.010, 0.020, 0.032],
        background_glow: [0.022, 0.050, 0.068, 1.0],
        accent: [0.30, 0.95, 0.62],
        edge_style: EdgeStyle::Line,
        glow: 1.0,
        chromatic: 0.0,
        vignette: 0.5,
        grain: 0.0,
        halos: false,
        depth_of_field: false,
        breathing,
        motion_fps: base.motion_fps.max(if breathing.is_some() { 6.0 } else { 0.0 }),
        labels: base.labels,
        label_font: FontFamily::Sans,
        label_style: TextStyle::Minimal,
        auto_scale: base.auto_scale,
        label_plate_alpha: 0.0,
        bg_mode: BgMode::Plain,
    }
}

/// One tag cluster on screen.
#[derive(Debug, Clone)]
struct Cluster {
    tag_hash: u32,
    count: f32,
    mean: [f32; 2],
    spread: [f32; 2],
}

/// Tag clusters of three or more visible notes, biggest first. Grouped by tag here
/// rather than from the look's halo groups, which big vaults turn off.
fn clusters<'g>(graph: &'g LayoutGraph, screen: &[[f32; 2]], visible: &dyn Fn(usize) -> bool) -> (Vec<Cluster>, HashMap<&'g str, usize>) {
    // count, sum x, sum y, sum x², sum y²
    let mut acc: HashMap<&'g str, (f32, f32, f32, f32, f32)> = HashMap::new();
    for (i, p) in screen.iter().enumerate() {
        let Some(tag) = graph.tag[i].as_deref() else { continue };
        if graph.ghost[i] || !visible(i) {
            continue;
        }
        let a = acc.entry(tag).or_default();
        *a = (a.0 + 1.0, a.1 + p[0], a.2 + p[1], a.3 + p[0] * p[0], a.4 + p[1] * p[1]);
    }
    let mut out: Vec<(&'g str, Cluster)> = acc
        .into_iter()
        .filter(|(_, a)| a.0 >= 3.0)
        .map(|(tag, (c, sx, sy, sxx, syy))| {
            let mean = [sx / c, sy / c];
            let spread = [(sxx / c - mean[0] * mean[0]).max(0.0).sqrt(), (syy / c - mean[1] * mean[1]).max(0.0).sqrt()];
            (tag, Cluster { tag_hash: hash(tag), count: c, mean, spread })
        })
        .collect();
    // Biggest first; ties broken by name so the order is stable.
    out.sort_by(|a, b| b.1.count.total_cmp(&a.1.count).then_with(|| a.0.cmp(b.0)));
    let rank = out.iter().enumerate().map(|(k, (tag, _))| (*tag, k)).collect();
    (out.into_iter().map(|(_, c)| c).collect(), rank)
}

/// The notes' left and right halves as stand-in clusters (for vaults with few tags).
fn halves(pts: &[[f32; 2]]) -> Vec<Cluster> {
    if pts.len() < 2 {
        return Vec::new();
    }
    let mut xs: Vec<f32> = pts.iter().map(|p| p[0]).collect();
    xs.sort_by(f32::total_cmp);
    let mid = xs[xs.len() / 2];
    let side = |left: bool| {
        let part: Vec<&[f32; 2]> = pts.iter().filter(|p| (p[0] < mid) == left).collect();
        let c = part.len() as f32;
        let mean = [part.iter().map(|p| p[0]).sum::<f32>() / c, part.iter().map(|p| p[1]).sum::<f32>() / c];
        let var = |k: usize| part.iter().map(|p| (p[k] - mean[k]).powi(2)).sum::<f32>() / c;
        Cluster { tag_hash: if left { 7 } else { 1013 }, count: c, mean, spread: [var(0).sqrt(), var(1).sqrt()] }
    };
    let mut out = vec![side(false), side(true)];
    out.retain(|c| c.count >= 1.0);
    out
}

/// A curtain before it becomes an instance.
#[derive(Debug, Clone)]
struct Plan {
    base: [f32; 2],
    half_width: f32,
    height: f32,
    weight: f32,
    colour: usize,
    seed: f32,
}

/// Curtains over the biggest clusters. A cluster whose curtain would mostly sit on
/// an earlier one widens that one instead of stacking more light on it.
fn plan_curtains(clusters: &[Cluster], size: [f32; 2]) -> Vec<Plan> {
    let Some(biggest) = clusters.first() else { return Vec::new() };
    let mut plans: Vec<Plan> = Vec::new();
    for c in clusters.iter().take(MAX_CURTAINS * 3) {
        if plans.len() >= MAX_CURTAINS {
            break;
        }
        let weight = (c.count / biggest.count).sqrt();
        // Small clusters only get a curtain when they're a real share of the vault.
        if weight < 0.3 && !plans.is_empty() {
            break;
        }
        let half_width = (c.spread[0] * 2.0 + size[0] * 0.1).min(size[0] * 0.36);
        let height = (c.spread[1] * 1.5 + size[1] * 0.24).min(size[1] * 0.6);
        let base = [c.mean[0], c.mean[1] + c.spread[1] * 0.6];
        if let Some(p) = plans.iter_mut().find(|p| {
            (p.base[0] - base[0]).abs() < p.half_width.min(half_width) * 0.3 && (p.base[1] - base[1]).abs() < p.height.min(height) * 0.2
        }) {
            // Nearly the same place: strengthen the earlier curtain instead.
            p.half_width = p.half_width.max(half_width);
            p.height = p.height.max(height);
            p.weight = (p.weight + weight * 0.3).min(1.0);
            continue;
        }
        plans.push(Plan {
            base,
            half_width,
            height,
            weight,
            colour: RANK_ORDER[plans.len() % RANK_ORDER.len()],
            seed: (c.tag_hash % 997) as f32 * 0.37,
        });
    }
    plans
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

    let (mut clusters, rank) = clusters(graph, &screen, &visible);
    if clusters.len() < 2 {
        // Few or no tags: the whole graph still gets its aurora, as two curtains.
        let pts: Vec<[f32; 2]> = (0..n).filter(|&i| !graph.ghost[i] && visible(i)).map(|i| screen[i]).collect();
        clusters.extend(halves(&pts));
    }
    let plans = plan_curtains(&clusters, size);
    for p in &plans {
        let (base, top) = PALETTE[p.colour];
        data.curtains.push(CurtainInstance {
            base: p.base,
            size: [p.half_width, p.height],
            color: [base[0], base[1], base[2], 0.17 + 0.18 * p.weight],
            top: [top[0], top[1], top[2], p.seed],
        });
    }
    // Each tag's colour: the curtain colours for the biggest clusters in rank order,
    // the rest of the palette (no magenta) by name.
    let tag_colour = |tag: Option<&str>| -> [f32; 3] {
        let Some(tag) = tag else { return [0.80, 0.88, 0.94] };
        match rank.get(tag) {
            Some(&k) if k < RANK_ORDER.len() => PALETTE[RANK_ORDER[k]].0,
            _ => PALETTE[(hash(tag) % 4) as usize].0,
        }
    };

    // Links: barely there.
    for &(s, t) in &graph.links {
        let (s, t) = (s as usize, t as usize);
        if s >= n || t >= n || !visible(s) || !visible(t) {
            continue;
        }
        let i = imp(s).max(imp(t));
        data.edges.push(EdgeInstance { a: screen[s], b: screen[t], color: [0.55, 0.80, 0.82, 0.025 + i * 0.05], width: 0.5 });
    }

    // Notes: small bright points, tinted by their cluster. Dim ones first.
    let mut order: Vec<usize> = (0..n).filter(|&i| visible(i)).collect();
    order.sort_by(|&a, &b| imp(a).total_cmp(&imp(b)));
    for i in order {
        let p = imp(i);
        let [r, g, b] = mix(tag_colour(graph.tag[i].as_deref()), [1.0, 1.0, 1.0], 0.55);
        if graph.ghost[i] {
            data.nodes.push(NodeInstance { center: screen[i], radius: 0.6, color: [r, g, b, 0.22] });
            continue;
        }
        let core = 0.65 + p.sqrt() * 2.0;
        data.nodes.push(NodeInstance { center: screen[i], radius: core, color: [r, g, b, 0.7 + p * 0.3] });
        if p > 0.12 {
            data.glow.push(NodeInstance { center: screen[i], radius: core * 3.5 + 4.0, color: [r, g, b, 0.10 + p * 0.3] });
        }
    }

    if style.labels && look.label_count > 0 {
        let no_dof = |_: [f32; 2]| 0.0;
        data.text = place_labels(look, &screen, n, size, style, false, &no_dof);
        for t in &mut data.text {
            t.color = [0.86, 0.96, 0.94, 0.70];
            t.size *= 0.92;
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster(count: f32, x: f32, y: f32, spread: f32) -> Cluster {
        Cluster { tag_hash: count as u32, count, mean: [x, y], spread: [spread, spread] }
    }

    #[test]
    fn caps_curtains_and_skips_small_clusters() {
        let mut cs: Vec<Cluster> = (0..20).map(|k| cluster(100.0 - k as f32, 100.0 + k as f32 * 300.0, 400.0, 20.0)).collect();
        cs.push(cluster(2.0, 50.0, 50.0, 5.0));
        let plans = plan_curtains(&cs, [8000.0, 1000.0]);
        assert_eq!(plans.len(), MAX_CURTAINS);
    }

    #[test]
    fn overlapping_clusters_merge_into_one_curtain() {
        let cs = vec![cluster(100.0, 500.0, 400.0, 60.0), cluster(80.0, 520.0, 410.0, 50.0)];
        let plans = plan_curtains(&cs, [1500.0, 1000.0]);
        assert_eq!(plans.len(), 1);
        let apart = vec![cluster(100.0, 300.0, 400.0, 60.0), cluster(80.0, 1100.0, 400.0, 50.0)];
        assert_eq!(plan_curtains(&apart, [1500.0, 1000.0]).len(), 2);
    }

    #[test]
    fn halves_split_the_notes() {
        let pts: Vec<[f32; 2]> = (0..10).map(|k| [k as f32 * 10.0, 50.0]).collect();
        let h = halves(&pts);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].count + h[1].count, 10.0);
        assert!(h[0].mean[0] > h[1].mean[0]);
        assert!(halves(&pts[..1]).is_empty());
    }

    #[test]
    fn no_clusters_no_curtains() {
        assert!(plan_curtains(&[], [1500.0, 1000.0]).is_empty());
    }
}
