//! Ink & Paper: the graph printed on warm off-white paper. Notes are ink dots that
//! bleed softly into the paper, links are fine ink lines of slightly uneven weight,
//! hubs are bolder. Tags tint the ink with at most three muted inks. No glow and no
//! ambient motion: everything is drawn once into the cached scene. Saving a note
//! sends a two-second ink bleed out from its dot (see [`burst_sprites`]), drawn over
//! the still scene and gone when it settles.

use std::collections::HashMap;

use vw_core::config::Config;
use vw_core::layout::LayoutGraph;

use crate::look::Look;
use crate::scene::{BgMode, EdgeStyle, INK_CORE, SceneData, SceneStyle, place_labels};
use crate::text::{FontFamily, TextStyle};
use crate::burst::BurstInstance;
use crate::{EdgeInstance, Framing, NodeInstance};

/// Warm off-white paper.
const PAPER: [f32; 3] = [0.949, 0.929, 0.882];
/// Carbon black, Prussian indigo, oxblood: muted, all dark enough to read as ink.
const INKS: [[f32; 3]; 3] = [[0.10, 0.095, 0.09], [0.16, 0.19, 0.30], [0.34, 0.13, 0.12]];
/// Dark ink for labels.
const LABEL_INK: [f32; 4] = [0.12, 0.11, 0.10, 0.82];

fn hash(bytes: impl IntoIterator<Item = u8>) -> u32 {
    bytes.into_iter().fold(2_166_136_261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619))
}

/// Inks by tag. The plate stays mostly black: only a few of the biggest tags print
/// in a colour ink, and never the very biggest, which carries the black.
struct Inks<'a> {
    coloured: HashMap<&'a str, [f32; 3]>,
}

impl<'a> Inks<'a> {
    /// Tags ranked 2nd to 5th by note count (with at least 3 notes) get indigo and
    /// oxblood in turn.
    fn new(tags: impl IntoIterator<Item = &'a str>) -> Self {
        let mut count: HashMap<&str, usize> = HashMap::new();
        for t in tags {
            *count.entry(t).or_default() += 1;
        }
        let mut ranked: Vec<(&str, usize)> = count.into_iter().filter(|&(_, c)| c >= 3).collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let coloured = ranked.iter().skip(1).take(4).enumerate().map(|(k, &(t, _))| (t, INKS[1 + k % 2])).collect();
        Inks { coloured }
    }

    fn get(&self, tag: Option<&str>) -> [f32; 3] {
        tag.and_then(|t| self.coloured.get(t).copied()).unwrap_or(INKS[0])
    }
}

/// 0..1 from a pair of indices: a stable per-link variation.
fn jitter(a: usize, b: usize) -> f32 {
    (hash(a.to_le_bytes().into_iter().chain(b.to_le_bytes())) % 1000) as f32 / 999.0
}

pub(super) fn style(_cfg: &Config, base: SceneStyle) -> SceneStyle {
    SceneStyle {
        background: PAPER,
        // A barely lighter centre, so the sheet reads as lit rather than flat.
        background_glow: [0.968, 0.952, 0.912, 1.0],
        accent: INKS[0],
        edge_style: EdgeStyle::Line,
        glow: 0.0,
        chromatic: 0.0,
        vignette: 0.04,
        grain: 0.35,
        halos: false,
        depth_of_field: false,
        breathing: None,
        motion_fps: 0.0,
        labels: base.labels,
        label_font: FontFamily::Serif,
        // Plain serif text; the "glow" is paper-coloured, a knockout that keeps
        // captions legible where they cross ink.
        label_style: TextStyle::Glow,
        auto_scale: base.auto_scale,
        label_plate_alpha: 0.0,
        bg_mode: BgMode::Paper,
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
    let tag = |i: usize| graph.tag[i].as_deref();
    let inks = Inks::new((0..n).filter(|&i| !graph.ghost[i]).filter_map(tag));
    let mut data = SceneData::default();

    // Dense vaults print with a lighter hand, or the core turns into a blot.
    let links = graph.links.len().max(1) as f32;
    let density = (600.0 / links).sqrt().clamp(0.22, 1.0);

    for (k, &(s, t)) in graph.links.iter().enumerate() {
        let (s, t) = (s as usize, t as usize);
        if s >= n || t >= n || !visible(s) || !visible(t) {
            continue;
        }
        if look.edge_color.get(k).is_some_and(|c| c[3] <= 0.0) {
            continue;
        }
        // Weight follows the weaker end: spokes from a hub out to single notes stay
        // fine, so hubs don't print as dark sunbursts; links between two
        // well-connected notes are the bold strokes.
        let (lo, hi) = (imp(s).min(imp(t)).sqrt(), imp(s).max(imp(t)).sqrt());
        let j = jitter(s, t);
        // Same tag: that tag's ink. Across tags: black.
        let [r, g, b] = if tag(s).is_some() && tag(s) == tag(t) { inks.get(tag(s)) } else { INKS[0] };
        data.ink_edges.push(EdgeInstance {
            a: screen[s],
            b: screen[t],
            color: [r, g, b, (0.15 + lo * 0.55 + hi * 0.08) * density * (0.75 + j * 0.5)],
            width: (0.5 + lo * 1.4 + hi * 0.3) * (0.7 + j * 0.6),
        });
    }

    // Small dots first so hubs print on top.
    let mut order: Vec<usize> = (0..n).filter(|&i| visible(i)).collect();
    order.sort_by(|&a, &b| imp(a).total_cmp(&imp(b)));
    for i in order {
        let p = imp(i);
        let [r, g, b] = inks.get(tag(i));
        if graph.ghost[i] {
            // Unresolved links: a faint pinprick, no bleed.
            data.nodes.push(NodeInstance { center: screen[i], radius: 0.8, color: [r, g, b, 0.35] });
            continue;
        }
        let (dot, alpha) = dot(p);
        data.inks.push(NodeInstance { center: screen[i], radius: dot / INK_CORE, color: [r, g, b, alpha] });
    }

    if style.labels && look.label_count > 0 {
        let no_dof = |_: [f32; 2]| 0.0;
        data.text = place_labels(look, &screen, n, size, style, false, &no_dof);
        for t in &mut data.text {
            t.color = LABEL_INK;
            t.accent = [PAPER[0], PAPER[1], PAPER[2], 0.92];
            t.size *= 0.9;
        }
    }
    data
}

/// A note's solid dot radius (points) and ink alpha, from its importance.
fn dot(importance: f32) -> (f32, f32) {
    (0.85 + importance.sqrt() * 5.6, 0.74 + importance.sqrt() * 0.24)
}

/// The ink bleed for each edited note: fresh ink on its dot and a wet front that
/// settles a little past the dot's printed bleed (at least 14 pt out, so small notes
/// show too).
pub(super) fn burst_sprites(
    graph: &LayoutGraph,
    look: &Look,
    positions: &[[f32; 2]],
    framing: &Framing,
    nodes: &[usize],
) -> Vec<BurstInstance> {
    let n = positions.len().min(graph.len());
    nodes
        .iter()
        .copied()
        .filter(|&i| i < n && !graph.ghost[i] && look.node_color.get(i).is_none_or(|c| c[3] > 0.0))
        .map(|i| {
            let (core, alpha) = dot(look.node_importance.get(i).copied().unwrap_or(0.0));
            let [r, g, b] = ink(graph.tag[i].as_deref());
            BurstInstance {
                center: framing.apply(positions[i]),
                core,
                reach: (core / INK_CORE * 1.9).max(core + 14.0),
                color: [r, g, b, alpha],
                progress: 0.0,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Ambient, Theme};

    #[test]
    fn ink_paper_is_still_and_light() {
        let cfg = Config { motion_mode: vw_core::config::MotionMode::Showcase, ..Config::default() };
        let theme = Theme::from_id("ink-paper");
        assert_eq!(theme, Theme::InkPaper);
        let style = theme.style(&cfg, false);
        assert_eq!(style.glow, 0.0);
        assert!(style.breathing.is_none());
        assert_eq!(theme.ambient(&style), Ambient::None);
        assert_eq!(style.bg_mode, BgMode::Paper);
    }

    #[test]
    fn colour_inks_go_to_a_few_big_tags() {
        let mut tags = Vec::new();
        for (k, name) in ["a", "b", "c", "d", "e", "f", "g"].iter().enumerate() {
            tags.extend(std::iter::repeat_n(*name, 20 - k));
        }
        tags.push("tiny");
        let inks = Inks::new(tags);
        assert_eq!(inks.get(None), INKS[0]);
        assert_eq!(inks.get(Some("a")), INKS[0], "the biggest tag stays black");
        assert_eq!(inks.get(Some("b")), INKS[1]);
        assert_eq!(inks.get(Some("c")), INKS[2]);
        assert_eq!(inks.get(Some("e")), INKS[2]);
        assert_eq!(inks.get(Some("f")), INKS[0]);
        assert_eq!(inks.get(Some("tiny")), INKS[0]);
    }

    #[test]
    fn edits_bleed_unless_still() {
        let theme = Theme::InkPaper;
        let still = Config { motion_mode: vw_core::config::MotionMode::Still, ..Config::default() };
        assert!(theme.edit_burst(&still).is_none());
        let light = Config { motion_mode: vw_core::config::MotionMode::Light, ..Config::default() };
        let burst = theme.edit_burst(&light).expect("bleeds in Light");
        assert_eq!(burst.kind, crate::BurstKind::InkBleed);
        assert_eq!(burst.duration, 2.0);
        assert!(burst.fps > 0.0 && burst.fps <= 20.0);
        let capped = Config { max_fps: 5, ..light.clone() };
        assert_eq!(theme.edit_burst(&capped).unwrap().fps, 5.0);
        // Themes without an edit effect stay still on save.
        assert!(Theme::NightSky.edit_burst(&light).is_none());
    }

    #[test]
    fn bleed_sits_on_the_printed_dot() {
        let graph = LayoutGraph {
            id: vec!["a".into(), "ghost".into(), "b".into()],
            tag: vec![None, None, Some("x".into())],
            ghost: vec![false, true, false],
            mtime: vec![0.0; 3],
            links: vec![],
            degree: vec![0; 3],
            radius: vec![3.0; 3],
        };
        let look = Look { node_importance: vec![0.0, 0.0, 1.0], ..Look::default() };
        let positions = [[10.0, 20.0], [30.0, 30.0], [50.0, 60.0]];
        let framing = Framing { tx: 5.0, ..Framing::IDENTITY };
        let style = Theme::InkPaper.style(&Config::default(), false);
        let scene = build(&graph, &look, &positions, &framing, &style, [200.0, 200.0]);
        let sprites = Theme::InkPaper.burst_sprites(&graph, &look, &positions, &framing, &[0, 1, 2, 9]);
        // Ghosts and out-of-range indices have no ink to bleed.
        assert_eq!(sprites.len(), 2);
        for s in &sprites {
            let ink = scene.inks.iter().find(|k| k.center == s.center).expect("matches a printed dot");
            assert!((s.core - ink.radius * INK_CORE).abs() < 1e-4);
            assert_eq!(s.color, ink.color);
            assert!(s.reach >= s.core + 14.0 && s.reach >= ink.radius);
            assert_eq!(s.progress, 0.0);
        }
    }
}
