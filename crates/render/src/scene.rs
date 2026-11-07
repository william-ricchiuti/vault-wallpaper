//! Turning layout positions, the look and the config into what the GPU draws.
//! Runs only when something changed, never per frame.

use vw_core::config::{self, Config};
use vw_core::layout::LayoutGraph;

use crate::look::Look;
use crate::text::{Anchor, FontFamily, TextItem, TextStyle};
use crate::{EdgeInstance, Framing, NodeInstance};

/// What the background shader draws under the gradient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BgMode {
    #[default]
    Plain = 0,
    Starfield = 1,
    Paper = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeStyle {
    #[default]
    Line,
    /// Gently bowed links.
    Curve,
    None,
}

/// Everything about the look that is not per note: colours, strengths, effects.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneStyle {
    pub background: [f32; 3],
    /// Radial gradient centre colour; alpha 1 = gradient on, 0 = flat background.
    pub background_glow: [f32; 4],
    pub accent: [f32; 3],
    pub edge_style: EdgeStyle,
    /// Glow strength in the composite (0 = no glow layer at all).
    pub glow: f32,
    /// Chromatic split of the glow, in glow texels (0 = off).
    pub chromatic: f32,
    pub vignette: f32,
    pub grain: f32,
    pub halos: bool,
    pub depth_of_field: bool,
    /// Glow breathing: depth 0..1 and period in seconds; `None` = still.
    pub breathing: Option<(f32, f32)>,
    /// Frames per second while something animates (0 = never animate).
    pub motion_fps: f32,
    pub labels: bool,
    pub label_font: FontFamily,
    pub label_style: TextStyle,
    /// Tone effects down on big vaults (`autoScaleLargeVaults`), as the web version does.
    pub auto_scale: bool,
    /// Badge plate opacity (`labelStyle.backgroundAlpha`, default 0.72).
    pub label_plate_alpha: f32,
    pub bg_mode: BgMode,
}

/// The web version's per-size tuning (`sceneTuning`): bigger vaults get subtler,
/// cheaper effects.
/// The parts of the web version's per-size tuning (`sceneTuning`) the renderer
/// applies itself; link opacity, halo strength and label counts come with the [`Look`].
#[derive(Debug, Clone, Copy)]
struct Tuning {
    glow_scale: f32,
    depth_of_field: bool,
}

fn tuning(style: &SceneStyle, nodes: usize, links: usize) -> Tuning {
    if !style.auto_scale {
        return Tuning { glow_scale: 1.0, depth_of_field: style.depth_of_field };
    }
    let (dense, huge) = (nodes > 350 || links > 1200, nodes > 900 || links > 3200);
    if nodes > 3000 || links > 10000 {
        return Tuning { glow_scale: 0.0, depth_of_field: false };
    }
    Tuning {
        glow_scale: if huge { 0.82 } else if dense { 0.92 } else { 1.0 },
        depth_of_field: style.depth_of_field && !huge,
    }
}

impl Default for SceneStyle {
    fn default() -> Self {
        style_from_config(&Config::default(), false)
    }
}

/// "#rrggbb" to 0..1 sRGB values.
pub fn parse_hex(s: &str) -> Option<[f32; 3]> {
    config::rgb(s).map(|[r, g, b]| [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0])
}

/// A stable colour per tag (hue from a hash of the name). Used when no look is
/// available yet; [`Look`] carries the real colours.
pub fn tag_color(tag: &str) -> [f32; 3] {
    let h = tag.bytes().fold(2_166_136_261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16_777_619));
    let (h, s, l) = ((h % 360) as f32 / 60.0, 0.62f32, 0.64f32);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
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

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// The style for a config. `light` selects the light colours.
pub fn style_from_config(cfg: &Config, light: bool) -> SceneStyle {
    let (bg, accent) = if light { (&cfg.light_background, &cfg.light_accent) } else { (&cfg.background, &cfg.accent) };
    let background = parse_hex(bg).unwrap_or([0.04, 0.04, 0.06]);
    let accent = parse_hex(accent).unwrap_or([0.49, 0.36, 1.0]);
    let centre = mix(background, accent, 0.15);
    let motion_fps = match cfg.motion_mode {
        config::MotionMode::Still => 0.0f32,
        config::MotionMode::Light => 4.0,
        config::MotionMode::Balanced => 8.0,
        config::MotionMode::Showcase => 15.0,
    }
    .min(cfg.max_fps as f32);
    SceneStyle {
        background,
        background_glow: [centre[0], centre[1], centre[2], if cfg.background_gradient { 1.0 } else { 0.0 }],
        accent,
        edge_style: match cfg.edge_style {
            config::EdgeStyle::Line => EdgeStyle::Line,
            config::EdgeStyle::Curve => EdgeStyle::Curve,
            config::EdgeStyle::None => EdgeStyle::None,
        },
        // Glow reads as light on dark backgrounds only; on light ones it muddies.
        glow: if cfg.node_glow && !light { cfg.glow_intensity as f32 } else { 0.0 },
        chromatic: if cfg.chromatic_bloom { cfg.chromatic_bloom_intensity as f32 * 4.0 } else { 0.0 },
        vignette: if cfg.vignette { 0.35 } else { 0.0 },
        grain: if cfg.grain { 0.5 } else { 0.0 },
        halos: cfg.cluster_halos,
        depth_of_field: cfg.depth_of_field,
        // The web version breathes over ~30 s at speed 1.
        breathing: (cfg.glow_breathing && motion_fps > 0.0)
            .then(|| (cfg.glow_breathing_depth as f32, 30.0 / cfg.glow_breathing_speed.max(0.05) as f32)),
        motion_fps,
        labels: cfg.hub_labels,
        label_font: match cfg.label_font {
            config::LabelFont::Mono => FontFamily::Mono,
            config::LabelFont::Serif => FontFamily::Serif,
            _ => FontFamily::Sans,
        },
        label_style: match cfg.label_style.mode {
            Some(config::LabelStyleMode::Glow) => TextStyle::Glow,
            Some(config::LabelStyleMode::Minimal) => TextStyle::Minimal,
            _ => TextStyle::Badge,
        },
        auto_scale: cfg.auto_scale_large_vaults,
        label_plate_alpha: cfg.label_style.background_alpha.unwrap_or(0.72) as f32,
        bg_mode: BgMode::Plain,
    }
}

impl SceneStyle {
    /// Glow multiplier at time `t` seconds (1 when not breathing).
    pub fn breath(&self, t: f64) -> f32 {
        match self.breathing {
            Some((depth, period)) => 1.0 - depth * 0.5 * (1.0 - (t * std::f64::consts::TAU / f64::from(period)).cos() as f32),
            None => 1.0,
        }
    }

    pub fn animates(&self) -> bool {
        self.breathing.is_some() && self.glow > 0.0 && self.motion_fps > 0.0
    }
}

/// Solid ink dot radius as a fraction of an ink instance's radius (see `ink_fs`).
pub const INK_CORE: f32 = 0.42;

/// Everything the cached scene draws, in screen points.
#[derive(Debug, Clone, Default)]
pub struct SceneData {
    pub halos: Vec<NodeInstance>,
    pub edges: Vec<EdgeInstance>,
    pub nodes: Vec<NodeInstance>,
    pub glow: Vec<NodeInstance>,
    /// Discs drawn with diffraction spikes (Night Sky hubs).
    pub stars: Vec<NodeInstance>,
    /// Ink dots that bleed into the paper (Ink & Paper). `radius` is the bleed's
    /// outer reach; the solid dot is [`INK_CORE`] of it.
    pub inks: Vec<NodeInstance>,
    /// Pen strokes that swell in the middle and thin toward their ends (Ink & Paper).
    pub ink_edges: Vec<EdgeInstance>,
    pub text: Vec<TextItem>,
}

/// Builds the scene for one screen of `width` x `height` points.
pub fn build_scene(
    graph: &LayoutGraph,
    look: &Look,
    positions: &[[f32; 2]],
    framing: &Framing,
    style: &SceneStyle,
    size: [f32; 2],
) -> SceneData {
    let n = positions.len().min(graph.len());
    let screen: Vec<[f32; 2]> = positions[..n].iter().map(|p| framing.apply(*p)).collect();
    let centre = [size[0] / 2.0, size[1] / 2.0];
    let reach = (centre[0] * centre[0] + centre[1] * centre[1]).sqrt();
    // 0 at the centre, 1 at the corners: drives depth of field.
    let dist = |p: [f32; 2]| ((p[0] - centre[0]).powi(2) + (p[1] - centre[1]).powi(2)).sqrt() / reach;
    let importance = |i: usize| look.node_importance.get(i).copied().unwrap_or(0.0);
    let tune = tuning(style, n, graph.links.len());
    let mut data = SceneData::default();

    if style.halos && look.halo_alpha > 0.0 {
        data.halos = halos(look, &screen, n);
    }

    if style.edge_style != EdgeStyle::None {
        for (k, &(s, t)) in graph.links.iter().enumerate() {
            let (s, t) = (s as usize, t as usize);
            if s >= n || t >= n {
                continue;
            }
            let Some(&[r, g, b, a]) = look.edge_color.get(k) else { continue };
            let imp = importance(s).max(importance(t));
            // The look's alpha already carries linkOpacity, size scaling and importance.
            let mut alpha = a;
            let mut width = 0.4 + imp * 0.6;
            if tune.depth_of_field {
                let m = 1.0 - (dist(screen[s]) + dist(screen[t])) / 2.0 * 0.5;
                alpha *= m;
                width *= m;
            }
            let color = [r, g, b, alpha];
            let (pa, pb) = (screen[s], screen[t]);
            if style.edge_style == EdgeStyle::Curve {
                push_curve(&mut data.edges, pa, pb, color, width);
            } else {
                data.edges.push(EdgeInstance { a: pa, b: pb, color, width });
            }
        }
    }

    // Small notes first so hubs draw on top.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| importance(a).total_cmp(&importance(b)));
    for i in order {
        let Some(&color) = look.node_color.get(i) else { continue };
        let mut radius = look.node_radius.get(i).copied().unwrap_or(2.0);
        let mut alpha = color[3];
        let mut dof_alpha = 1.0;
        if tune.depth_of_field {
            let d = dist(screen[i]);
            radius *= 1.0 - d * 0.3;
            dof_alpha = 1.0 - d * 0.4;
            alpha *= dof_alpha;
        }
        data.nodes.push(NodeInstance { center: screen[i], radius, color: [color[0], color[1], color[2], alpha] });
        if style.glow > 0.0 && tune.glow_scale > 0.0 && !graph.ghost[i] {
            // The web version's glow sprite: a soft disc a little bigger than the
            // note, faint unless the note is important; overall strength comes from
            // the composite's glow intensity.
            let imp = importance(i);
            data.glow.push(NodeInstance {
                center: screen[i],
                radius: (radius + 3.0 + radius.sqrt()) * tune.glow_scale,
                color: [color[0], color[1], color[2], (0.08 + imp * 0.15) * dof_alpha],
            });
        }
    }

    if style.labels && look.label_count > 0 {
        data.text = place_labels(look, &screen, n, size, style, tune.depth_of_field, &dist);
    }
    data
}

/// The web version's drawLabels: candidates in order, skipping any off-screen or
/// overlapping an earlier label, until `label_count` are placed. Text sits 16 pt
/// below the note's centre plus its radius.
pub(crate) fn place_labels(
    look: &Look,
    screen: &[[f32; 2]],
    n: usize,
    size: [f32; 2],
    style: &SceneStyle,
    dof: bool,
    dist: &dyn Fn([f32; 2]) -> f32,
) -> Vec<TextItem> {
    let mut out = Vec::new();
    let bg = style.background;
    let light_bg = 0.2126 * bg[0] + 0.7152 * bg[1] + 0.0722 * bg[2] > 0.4;
    let text_color = if light_bg { [30.0 / 255.0, 30.0 / 255.0, 35.0 / 255.0, 0.85] } else { [245.0 / 255.0, 247.0 / 255.0, 1.0, 0.88] };
    let mut placed: Vec<[f32; 4]> = Vec::new();
    for pick in &look.labels {
        if placed.len() >= look.label_count as usize {
            break;
        }
        let i = pick.node as usize;
        if i >= n {
            continue;
        }
        let dof_alpha = if dof { 1.0 - dist(screen[i]) * 0.4 } else { 1.0 };
        if dof_alpha <= 0.12 {
            continue;
        }
        let r = look.node_radius.get(i).copied().unwrap_or(2.0);
        let font = (11.0 + pick.importance * 5.0).max(10.0);
        // Width estimate for overlap tests; the text renderer measures exactly.
        let w = pick.text.chars().count() as f32 * font * 0.55;
        let (x, y) = (screen[i][0], screen[i][1] + r + 16.0);
        let rect = [x - w / 2.0 - 8.0, y - font / 2.0 - 5.0, x + w / 2.0 + 8.0, y + font / 2.0 + 5.0];
        let off = rect[0] < 10.0 || rect[1] < 10.0 || rect[2] > size[0] - 10.0 || rect[3] > size[1] - 10.0;
        let overlaps = placed.iter().any(|p| !(rect[2] < p[0] || p[2] < rect[0] || rect[3] < p[1] || p[3] < rect[1]));
        if off || overlaps {
            continue;
        }
        placed.push(rect);
        let [cr, cg, cb, _] = look.node_color.get(i).copied().unwrap_or([1.0; 4]);
        // Badge plate: the background tinted 12% toward the note's colour.
        let plate = [bg[0] + (cr - bg[0]) * 0.12, bg[1] + (cg - bg[1]) * 0.12, bg[2] + (cb - bg[2]) * 0.12, style.label_plate_alpha * dof_alpha];
        let accent = if style.label_style == TextStyle::Glow { [cr, cg, cb, 0.9 * dof_alpha] } else { plate };
        out.push(TextItem {
            text: pick.text.clone(),
            pos: [x, y - font * 0.625],
            anchor: Anchor::TopCenter,
            size: font,
            color: [text_color[0], text_color[1], text_color[2], text_color[3] * dof_alpha],
            family: style.label_font,
            style: style.label_style,
            accent,
            max_width: None,
        });
    }
    out
}

/// The web version's cluster halos: one soft disc per tag group of 3 or more notes,
/// at the group's centroid, sized by its spread and count.
fn halos(look: &Look, screen: &[[f32; 2]], n: usize) -> Vec<NodeInstance> {
    let groups = look.halo_color.len();
    if groups == 0 {
        return Vec::new();
    }
    // count, sum x, sum y, min x, max x, min y, max y
    let mut acc = vec![(0u32, 0.0f32, 0.0f32, f32::MAX, f32::MIN, f32::MAX, f32::MIN); groups];
    for (i, p) in screen.iter().enumerate().take(n) {
        let Some(Some(g)) = look.halo_group.get(i) else { continue };
        let a = &mut acc[*g as usize];
        a.0 += 1;
        a.1 += p[0];
        a.2 += p[1];
        a.3 = a.3.min(p[0]);
        a.4 = a.4.max(p[0]);
        a.5 = a.5.min(p[1]);
        a.6 = a.6.max(p[1]);
    }
    let alpha = look.halo_alpha;
    let mut out: Vec<NodeInstance> = acc
        .iter()
        .enumerate()
        .filter(|(_, a)| a.0 >= 3)
        .map(|(g, a)| {
            let (w, h) = (a.4 - a.3, a.6 - a.5);
            let radius = 56f32.max((w.max(h) * 0.72 + (a.0 as f32).sqrt() * 22.0) * look.halo_radius_boost);
            let [r, gg, b] = look.halo_color[g];
            NodeInstance { center: [a.1 / a.0 as f32, a.2 / a.0 as f32], radius, color: [r, gg, b, alpha] }
        })
        .collect();
    // Big halos first, so small ones stay visible on top.
    out.sort_by(|a, b| b.radius.total_cmp(&a.radius));
    out
}

/// A quadratic curve bowed to one side by a tenth of its length, as short segments.
fn push_curve(out: &mut Vec<EdgeInstance>, a: [f32; 2], b: [f32; 2], color: [f32; 4], width: f32) {
    const SEGMENTS: usize = 6;
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let c = [(a[0] + b[0]) / 2.0 - dy * 0.1, (a[1] + b[1]) / 2.0 + dx * 0.1];
    let at = |t: f32| {
        let u = 1.0 - t;
        [u * u * a[0] + 2.0 * u * t * c[0] + t * t * b[0], u * u * a[1] + 2.0 * u * t * c[1] + t * t * b[1]]
    };
    let mut prev = a;
    for k in 1..=SEGMENTS {
        let p = at(k as f32 / SEGMENTS as f32);
        out.push(EdgeInstance { a: prev, b: p, color, width });
        prev = p;
    }
}
