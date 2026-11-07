//! Pulse: a calm, still graph on dark slate (the default look, quieted: muted tag
//! colours, fine links, a little light on the hubs, no haze). Saving a note sends
//! light out from it along its links, hop by hop: a ring opens on the note, pulses
//! run to its neighbours, which glint, and dimmer pulses run one hop further. Then
//! everything is still again.
//!
//! Cost: nothing between edits. An edit bakes a small atlas of soft sprites (half
//! resolution) and the window server moves them; see [`crate::burst`].

use vw_core::config::Config;
use vw_core::layout::LayoutGraph;

use crate::burst::{BurstActor, BurstInstance, BurstKey, BurstKind, EditBurst};
use crate::look::Look;
use crate::scene::{BgMode, EdgeStyle, SceneData, SceneStyle, place_labels, tag_color};
use crate::text::{FontFamily, TextStyle};
use crate::{EdgeInstance, Framing, NodeInstance};

/// Seconds from save to still.
pub(super) const DURATION: f32 = 2.6;

const BACKGROUND: [f32; 3] = [0.030, 0.037, 0.052];
/// Notes and links: a cool, light grey, with only a hint of each tag's hue.
const SLATE: [f32; 3] = [0.66, 0.70, 0.77];
const TAG_HINT: f32 = 0.3;
/// Light is warm white tinted by the saved note's colour.
const LIGHT: [f32; 3] = [1.0, 0.96, 0.88];

/// The two hops: when pulses leave and arrive (0..1 of the burst), and how bright.
const HOPS: [([f32; 2], f32); 2] = [([0.05, 0.40], 1.0), ([0.43, 0.80], 0.65)];
/// Most pulses per hop, best-connected neighbours first.
const HOP_MAX: [usize; 2] = [20, 28];
/// Most second-hop pulses leaving one neighbour.
const FAN_MAX: usize = 6;

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

pub(super) fn style(_cfg: &Config, base: SceneStyle) -> SceneStyle {
    SceneStyle {
        background: BACKGROUND,
        background_glow: [0.052, 0.062, 0.084, 1.0],
        accent: SLATE,
        edge_style: EdgeStyle::Line,
        // Only hubs glow, faintly.
        glow: 0.55,
        chromatic: 0.0,
        vignette: 0.38,
        grain: 0.0,
        halos: false,
        depth_of_field: false,
        // Still between edits, whatever the motion setting: motion comes from saves.
        breathing: None,
        motion_fps: 0.0,
        labels: base.labels,
        label_font: FontFamily::Sans,
        label_style: TextStyle::Minimal,
        auto_scale: base.auto_scale,
        label_plate_alpha: 0.0,
        bg_mode: BgMode::Plain,
    }
}

/// A note's colour: slate with a hint of its tag's hue. Not the look's colours:
/// those default to the accent, which turns a big vault into a purple haze.
fn tint(graph: &LayoutGraph, _look: &Look, i: usize) -> [f32; 3] {
    match graph.tag.get(i).and_then(Option::as_deref) {
        Some(tag) => mix(SLATE, tag_color(tag), TAG_HINT),
        None => SLATE,
    }
}

/// A note's dot radius in points, from its importance.
fn core(importance: f32, ghost: bool) -> f32 {
    if ghost { 0.8 } else { 1.1 + importance.sqrt() * 3.4 }
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

    // Fine, cool links; dense vaults get fainter ones so the core doesn't fog over.
    let density = (800.0 / graph.links.len().max(1) as f32).sqrt().clamp(0.4, 1.0);
    for (k, &(s, t)) in graph.links.iter().enumerate() {
        let (s, t) = (s as usize, t as usize);
        if s >= n || t >= n || !visible(s) || !visible(t) || look.edge_color.get(k).is_some_and(|c| c[3] <= 0.0) {
            continue;
        }
        let lo = imp(s).min(imp(t)).sqrt();
        let [r, g, b] = mix(mix(tint(graph, look, s), tint(graph, look, t), 0.5), SLATE, 0.5);
        data.edges.push(EdgeInstance { a: screen[s], b: screen[t], color: [r, g, b, (0.11 + lo * 0.26) * density], width: 0.55 + lo * 0.6 });
    }

    // Small notes first so hubs draw on top.
    let mut order: Vec<usize> = (0..n).filter(|&i| visible(i)).collect();
    order.sort_by(|&a, &b| imp(a).total_cmp(&imp(b)));
    for i in order {
        let p = imp(i);
        let [r, g, b] = tint(graph, look, i);
        let radius = core(p, graph.ghost[i]);
        if graph.ghost[i] {
            data.nodes.push(NodeInstance { center: screen[i], radius, color: [r, g, b, 0.3] });
            continue;
        }
        data.nodes.push(NodeInstance { center: screen[i], radius, color: [r, g, b, 0.80 + p * 0.2] });
        if p > 0.12 {
            data.glow.push(NodeInstance { center: screen[i], radius: radius * 3.2 + 4.0, color: [r, g, b, 0.05 + p * 0.2] });
        }
    }

    if style.labels && look.label_count > 0 {
        let no_dof = |_: [f32; 2]| 0.0;
        data.text = place_labels(look, &screen, n, size, style, false, &no_dof);
        for t in &mut data.text {
            t.color = [0.80, 0.83, 0.89, 0.62];
            t.size *= 0.88;
        }
    }
    data
}

/// Keys for a pulse leaving `a` at `t0` and reaching `b` at `t1`, at `peak` opacity:
/// it fades in as it leaves and out as it arrives.
fn travel(a: [f32; 2], b: [f32; 2], [t0, t1]: [f32; 2], peak: f32) -> Vec<BurstKey> {
    let at = |t: f32| {
        let f = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
        [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
    };
    let (fade_in, fade_out) = (t0 + 0.04, t1 - 0.06);
    vec![
        BurstKey { t: 0.0, at: a, opacity: 0.0 },
        BurstKey { t: t0, at: a, opacity: 0.0 },
        BurstKey { t: fade_in, at: at(fade_in), opacity: peak },
        BurstKey { t: fade_out, at: at(fade_out), opacity: peak },
        BurstKey { t: t1, at: b, opacity: 0.0 },
        BurstKey { t: 1.0, at: b, opacity: 0.0 },
    ]
}

/// Keys for a glint at `at` that lights up at `t` and fades over `fade`.
fn glint(at: [f32; 2], t: f32, fade: f32, peak: f32) -> Vec<BurstKey> {
    let end = (t + fade).min(0.995);
    vec![
        BurstKey { t: 0.0, at, opacity: 0.0 },
        BurstKey { t: (t - 0.03).max(0.0), at, opacity: 0.0 },
        BurstKey { t: (t + 0.03).min(end), at, opacity: peak },
        BurstKey { t: end, at, opacity: 0.0 },
        BurstKey { t: 1.0, at, opacity: 0.0 },
    ]
}

pub(super) fn burst_actors(
    graph: &LayoutGraph,
    look: &Look,
    positions: &[[f32; 2]],
    framing: &Framing,
    node: usize,
    burst: &EditBurst,
) -> Vec<BurstActor> {
    let n = positions.len().min(graph.len());
    let visible = |i: usize| i < n && look.node_color.get(i).is_none_or(|c| c[3] > 0.0);
    if !visible(node) {
        return Vec::new();
    }
    let imp = |i: usize| look.node_importance.get(i).copied().unwrap_or(0.0);
    let at = |i: usize| framing.apply(positions[i]);
    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); n];
    // The links the scene draws, and only those.
    for (k, &(s, t)) in graph.links.iter().enumerate() {
        let (s, t) = (s as usize, t as usize);
        if s != t && visible(s) && visible(t) && look.edge_color.get(k).is_none_or(|c| c[3] > 0.0) {
            neighbours[s].push(t);
            neighbours[t].push(s);
        }
    }
    let by_importance = |v: &mut Vec<usize>| {
        v.sort_unstable();
        v.dedup();
        v.sort_by(|&a, &b| imp(b).total_cmp(&imp(a)).then(a.cmp(&b)));
    };

    let light = mix(tint(graph, look, node), LIGHT, 0.6);
    let colour = |a: f32| [light[0], light[1], light[2], a];
    let origin = at(node);
    let c0 = core(imp(node), graph.ghost[node]);
    let mut actors = Vec::new();

    // The ring opening on the saved note, and a glow that lingers on it.
    let ring = BurstInstance { center: [0.0; 2], core: c0, reach: c0 + 34.0, color: colour(0.9), progress: 0.0 };
    let ring_play = [0.0, 0.45];
    let frames = ((ring_play[1] * burst.duration * burst.fps).round() as u32).max(2) + 1;
    actors.push(BurstActor::fixed(BurstKind::PulseRing, ring, frames, ring_play, origin));
    let dot = BurstInstance { center: [0.0; 2], core: c0 * 0.9 + 1.2, reach: c0 * 3.0 + 10.0, color: colour(1.0), progress: 0.0 };
    actors.push(BurstActor { keys: glint(origin, 0.02, 0.8, 1.0), ..BurstActor::fixed(BurstKind::PulseGlint, dot, 1, [0.0, 1.0], origin) });

    // Hop by hop: pulses from each lit note to its next neighbours.
    let mut lit = vec![node];
    let mut seen = vec![node];
    for (hop, &(window, peak)) in HOPS.iter().enumerate() {
        let mut next: Vec<(usize, usize)> = Vec::new();
        for &from in &lit {
            let mut to: Vec<usize> = neighbours[from].iter().copied().filter(|t| !seen.contains(t)).collect();
            by_importance(&mut to);
            let fan = if hop == 0 { HOP_MAX[0] } else { FAN_MAX };
            for t in to.into_iter().take(fan) {
                if !next.iter().any(|&(_, x)| x == t) {
                    next.push((from, t));
                }
            }
        }
        next.sort_by(|a, b| imp(b.1).total_cmp(&imp(a.1)).then(a.1.cmp(&b.1)));
        next.truncate(HOP_MAX[hop]);
        for &(from, to) in &next {
            let (a, b) = (at(from), at(to));
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 4.0 {
                continue;
            }
            let comet = BurstInstance { center: [0.0; 2], core: 2.0, reach: (len * 0.45).clamp(8.0, 22.0), color: colour(1.0), progress: 0.0 };
            actors.push(BurstActor {
                angle: dy.atan2(dx),
                keys: travel(a, b, window, peak),
                ..BurstActor::fixed(BurstKind::PulseComet, comet, 1, [0.0, 1.0], a)
            });
            let c = core(imp(to), graph.ghost[to]);
            let spark = BurstInstance { center: [0.0; 2], core: c * 0.8 + 1.0, reach: c * 2.5 + 8.0, color: colour(1.0), progress: 0.0 };
            let fade = if hop == 0 { 0.32 } else { 0.18 };
            actors.push(BurstActor { keys: glint(b, window[1], fade, peak * 0.9), ..BurstActor::fixed(BurstKind::PulseGlint, spark, 1, [0.0, 1.0], b) });
        }
        seen.extend(next.iter().map(|&(_, t)| t));
        lit = next.into_iter().map(|(_, t)| t).collect();
    }
    actors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Ambient, Theme};
    use vw_core::config::MotionMode;

    /// a - b - c - d, plus a ghost e off b. Positions on a line, 50 pt apart.
    fn chain() -> (LayoutGraph, Look, Vec<[f32; 2]>) {
        let graph = LayoutGraph {
            id: ["a", "b", "c", "d", "e"].map(String::from).to_vec(),
            tag: vec![None; 5],
            ghost: vec![false, false, false, false, true],
            mtime: vec![0.0; 5],
            links: vec![(0, 1), (1, 2), (2, 3), (1, 4)],
            degree: vec![1, 3, 2, 1, 1],
            radius: vec![3.0; 5],
        };
        let look = Look { node_color: vec![[0.5, 0.6, 0.9, 1.0]; 5], node_importance: vec![0.2, 1.0, 0.5, 0.1, 0.0], ..Look::default() };
        let positions = (0..5).map(|i| [i as f32 * 50.0, 100.0]).collect();
        (graph, look, positions)
    }

    #[test]
    fn pulse_is_still_and_bursts_only_with_motion() {
        let theme = Theme::from_id("pulse");
        assert_eq!(theme, Theme::Pulse);
        let showcase = Config { motion_mode: MotionMode::Showcase, ..Config::default() };
        assert_eq!(theme.ambient(&theme.style(&showcase, false)), Ambient::None);
        let still = Config { motion_mode: MotionMode::Still, ..Config::default() };
        assert!(theme.edit_burst(&still).is_none());
        let burst = theme.edit_burst(&Config { motion_mode: MotionMode::Balanced, ..Config::default() }).unwrap();
        assert!(burst.resolution < 1.0 && burst.duration > 1.0 && burst.duration < 4.0);
    }

    #[test]
    fn light_travels_two_hops_and_settles() {
        let (graph, look, positions) = chain();
        let burst = EditBurst { duration: DURATION, fps: 12.0, resolution: 0.5 };
        let actors = burst_actors(&graph, &look, &positions, &Framing::IDENTITY, 0, &burst);
        let comets: Vec<&BurstActor> = actors.iter().filter(|a| a.kind == BurstKind::PulseComet).collect();
        // Hop 1: a -> b. Hop 2: b -> c and b -> e (the ghost), never back to a; d is three hops out.
        assert_eq!(comets.len(), 3);
        let ends: Vec<[f32; 2]> = comets.iter().map(|a| a.keys.last().unwrap().at).collect();
        assert!(ends.contains(&[50.0, 100.0]) && ends.contains(&[100.0, 100.0]) && ends.contains(&[200.0, 100.0]));
        assert!(!ends.contains(&[150.0, 100.0]) && !ends.contains(&[0.0, 100.0]));
        // The first pulse leaves a, is between a and b halfway through its hop, and
        // points along the link.
        let first = comets.iter().find(|a| a.keys.last().unwrap().at == [50.0, 100.0]).unwrap();
        assert_eq!(first.angle, 0.0);
        let (mid, op) = first.at(0.22);
        assert!(mid[0] > 10.0 && mid[0] < 40.0 && op > 0.9);
        // Second-hop pulses wait for the first to arrive and are dimmer.
        let second = comets.iter().find(|a| a.keys.last().unwrap().at == [100.0, 100.0]).unwrap();
        assert_eq!(second.at(0.3).1, 0.0);
        assert!(second.at(0.6).1 > 0.0 && second.at(0.6).1 < op);
        // Every actor ends transparent: faded out, or on its empty last frame.
        for a in &actors {
            let ended = a.at(1.0).1 == 0.0 || (a.frames > 1 && a.frame_at(1.0) == a.frames - 1);
            assert!(ended, "{:?} still showing", a.kind);
            assert!(a.keys.windows(2).all(|k| k[0].t <= k[1].t));
        }
    }

    #[test]
    fn a_lonely_note_still_rings() {
        let (mut graph, look, positions) = chain();
        graph.links.clear();
        let burst = EditBurst { duration: DURATION, fps: 8.0, resolution: 0.5 };
        let actors = burst_actors(&graph, &look, &positions, &Framing::IDENTITY, 3, &burst);
        assert_eq!(actors.len(), 2);
        assert!(burst_actors(&graph, &look, &positions, &Framing::IDENTITY, 99, &burst).is_empty());
    }
}
