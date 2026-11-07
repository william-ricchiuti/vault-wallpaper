//! Fitting the layout onto a screen: a port of the web version's
//! `computeFramingTarget`. Works in logical points; the renderer scales to pixels.

/// How the graph is placed on screen (`framing` in the config).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FramingMode {
    #[default]
    Center,
    /// Stretch (up to 1.6:1) to fill the safe area.
    Fill,
    /// Pulled into the left or right part of the screen.
    Left,
    Right,
    /// Layout coordinates used as screen points, unscaled.
    None,
}

impl FramingMode {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "center" => Self::Center,
            "fill" => Self::Fill,
            "left" => Self::Left,
            "right" => Self::Right,
            "none" => Self::None,
            _ => return None,
        })
    }
}

/// Insets in points kept free of the graph (menu bar, dock, icons).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SafeArea {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FramingOptions {
    pub mode: FramingMode,
    pub zoom: f32,
    /// -1..1, as a share of half the framing rectangle.
    pub offset_x: f32,
    pub offset_y: f32,
    pub safe_area: SafeArea,
}

impl Default for FramingOptions {
    fn default() -> Self {
        FramingOptions { mode: FramingMode::Center, zoom: 1.0, offset_x: 0.0, offset_y: 0.0, safe_area: SafeArea::default() }
    }
}

/// Layout -> screen. Inside the fitted core it is linear: `screen = (p - c) * s + t`.
/// Notes beyond the core (typically the ring of orphans around a real vault) are
/// pulled in radially so the farthest one lands on the edge of the framing
/// rectangle instead of shrinking the whole picture or falling off-screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Framing {
    pub sx: f32,
    pub sy: f32,
    pub cx: f32,
    pub cy: f32,
    pub tx: f32,
    pub ty: f32,
    /// Outer-ring compression; `None` keeps the mapping purely linear.
    pub outer: Option<OuterRing>,
}

/// Radial compression beyond the core, in core-normalised units: a point at
/// `q = ((x - cx) / hx, (y - cy) / hy)` with `|q| > 1` is moved to
/// `1 + (|q| - 1) * f`, where `f` is chosen per direction so that `|q| = r_max`
/// reaches the framing rectangle (`lim_*`, distances from the centre).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OuterRing {
    pub hx: f32,
    pub hy: f32,
    pub r_max: f32,
    /// Distances from the centre to the rectangle's right, left, bottom, top edges.
    pub lim_px: f32,
    pub lim_nx: f32,
    pub lim_py: f32,
    pub lim_ny: f32,
}

impl Framing {
    pub const IDENTITY: Framing = Framing { sx: 1.0, sy: 1.0, cx: 0.0, cy: 0.0, tx: 0.0, ty: 0.0, outer: None };

    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        let (mut dx, mut dy) = (p[0] - self.cx, p[1] - self.cy);
        if let Some(o) = &self.outer {
            let (qx, qy) = (dx / o.hx, dy / o.hy);
            let r = qx.hypot(qy);
            if r > 1.0 && r.is_finite() {
                let (ux, uy) = (qx / r, qy / r);
                let lx = if ux >= 0.0 { o.lim_px } else { o.lim_nx };
                let ly = if uy >= 0.0 { o.lim_py } else { o.lim_ny };
                // Where this direction leaves the rectangle, in core units.
                let lim = (lx / ux.abs().max(1e-6)).min(ly / uy.abs().max(1e-6)).max(1.0);
                let f = ((lim - 1.0) / (o.r_max - 1.0).max(1e-6)).clamp(0.0, 1.0);
                let k = (1.0 + (r - 1.0) * f) / r;
                dx *= k;
                dy *= k;
            }
        }
        [dx * self.sx + self.tx, dy * self.sy + self.ty]
    }

    /// Fits `points` (layout coordinates) into a `width` x `height` point screen,
    /// treating every point alike.
    pub fn fit(points: &[[f32; 2]], width: f32, height: f32, opts: &FramingOptions) -> Framing {
        Self::fit_graph(points, &[], width, height, opts)
    }

    /// Like [`Framing::fit`], but sizes the picture to the linked notes (`degree > 0`)
    /// so a wide ring of orphans doesn't shrink the core; the ring is pulled in to
    /// the screen edges. `degree` may be empty (all points weigh the same).
    pub fn fit_graph(points: &[[f32; 2]], degree: &[u32], width: f32, height: f32, opts: &FramingOptions) -> Framing {
        let sa = opts.safe_area;
        let rx = sa.left + width * 0.05;
        let ry = sa.top + height * 0.075;
        let rw = (width - sa.left - sa.right - width * 0.1).max(50.0);
        let rh = (height - sa.top - sa.bottom - height * 0.15).max(50.0);
        if opts.mode == FramingMode::None {
            return Framing { tx: width / 2.0, ty: height / 2.0, ..Framing::IDENTITY };
        }
        let Some([x0, y0, x1, y1]) = core_bounds(points, degree) else {
            return Framing { tx: rx + rw / 2.0, ty: ry + rh / 2.0, ..Framing::IDENTITY };
        };
        let (gw, gh) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
        let (mut cx, mut cy) = (rx + rw / 2.0, ry + rh / 2.0);
        let (mut sx, mut sy) = match opts.mode {
            FramingMode::Fill => {
                let (mut sx, mut sy) = (rw / gw, rh / gh);
                let ratio = sx / sy;
                if ratio > 1.6 {
                    sx = sy * 1.6;
                } else if ratio < 1.0 / 1.6 {
                    sy = sx * 1.6;
                }
                (sx, sy)
            }
            FramingMode::Center => {
                let s = (rw / gw).min(rh / gh) * 0.82;
                (s, s)
            }
            FramingMode::Left | FramingMode::Right => {
                let s = (rw * 0.58 / gw).min(rh / gh);
                cx = rx + rw * if opts.mode == FramingMode::Left { 0.3 } else { 0.7 };
                (s, s)
            }
            FramingMode::None => unreachable!(),
        };
        // Small vaults: don't blow a handful of notes up across the whole screen.
        const MAX_SCALE: f32 = 2.2;
        if sx > MAX_SCALE || sy > MAX_SCALE {
            let f = MAX_SCALE / sx.max(sy);
            sx *= f;
            sy *= f;
        }
        cx += opts.offset_x * rw * 0.5;
        cy += opts.offset_y * rh * 0.5;
        let (ccx, ccy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (hx, hy) = (gw / 2.0, gh / 2.0);
        // Farthest point from the core, in core units; the ring is squeezed (before
        // zoom, so zooming in still zooms) so that it lands on the rectangle.
        let r_max = points
            .iter()
            .map(|p| ((p[0] - ccx) / hx).hypot((p[1] - ccy) / hy))
            .filter(|r| r.is_finite())
            .fold(0.0f32, f32::max);
        let outer = (r_max > 1.0).then(|| OuterRing {
            hx,
            hy,
            r_max,
            lim_px: (rx + rw - cx) / (hx * sx),
            lim_nx: (cx - rx) / (hx * sx),
            lim_py: (ry + rh - cy) / (hy * sy),
            lim_ny: (cy - ry) / (hy * sy),
        });
        Framing { sx: sx * opts.zoom, sy: sy * opts.zoom, cx: ccx, cy: ccy, tx: cx, ty: cy, outer }
    }
}

/// The extent the picture is sized to. For graphs of more than 40 notes where a
/// meaningful share is linked, that's the linked notes only; otherwise all of them.
fn core_bounds(points: &[[f32; 2]], degree: &[u32]) -> Option<[f32; 4]> {
    if points.len() > 40 && degree.len() == points.len() {
        let linked: Vec<[f32; 2]> = points.iter().zip(degree).filter(|(_, d)| **d > 0).map(|(p, _)| *p).collect();
        if linked.len() >= 20 && linked.len() * 10 >= points.len() * 3 {
            return percentile_bounds(&linked);
        }
    }
    percentile_bounds(points)
}

/// 2nd–98th percentile bounds (all points for small graphs), so far-flung orphans
/// don't shrink the whole picture. Samples big graphs.
fn percentile_bounds(points: &[[f32; 2]]) -> Option<[f32; 4]> {
    let step = (points.len() / 4000).max(1);
    let mut xs: Vec<f32> = points.iter().step_by(step).map(|p| p[0]).filter(|v| v.is_finite()).collect();
    let mut ys: Vec<f32> = points.iter().step_by(step).map(|p| p[1]).filter(|v| v.is_finite()).collect();
    if xs.is_empty() || ys.is_empty() {
        return None;
    }
    xs.sort_by(f32::total_cmp);
    ys.sort_by(f32::total_cmp);
    let (lo, hi) = if xs.len() > 40 { (0.02, 0.98) } else { (0.0, 1.0) };
    let q = |a: &[f32], f: f32| a[((f * (a.len() - 1) as f32).floor() as usize).min(a.len() - 1)];
    Some([q(&xs, lo), q(&ys, lo), q(&xs, hi), q(&ys, hi)])
}

impl FramingOptions {
    /// Framing as the config sets it (`framing`, `zoom`, `offsetX/Y`, `safeArea`).
    pub fn from_config(cfg: &vw_core::config::Config) -> Self {
        use vw_core::config::Framing as F;
        let sa = &cfg.safe_area;
        FramingOptions {
            mode: match cfg.framing {
                F::Center => FramingMode::Center,
                F::Fill => FramingMode::Fill,
                F::Left => FramingMode::Left,
                F::Right => FramingMode::Right,
                F::None => FramingMode::None,
            },
            zoom: cfg.zoom as f32,
            offset_x: cfg.offset_x as f32,
            offset_y: cfg.offset_y as f32,
            safe_area: SafeArea { top: sa.top as f32, right: sa.right as f32, bottom: sa.bottom as f32, left: sa.left as f32 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_fits_inside_the_margins() {
        let pts: Vec<[f32; 2]> = (0..100).map(|i| [(i % 10) as f32 * 10.0 - 45.0, (i / 10) as f32 * 10.0 - 45.0]).collect();
        let f = Framing::fit(&pts, 1000.0, 800.0, &FramingOptions::default());
        for p in &pts {
            let [x, y] = f.apply(*p);
            assert!((50.0..=950.0).contains(&x) && (60.0..=740.0).contains(&y), "{x},{y}");
        }
    }

    #[test]
    fn small_graphs_are_not_blown_up() {
        let pts = [[0.0, 0.0], [10.0, 0.0]];
        let f = Framing::fit(&pts, 3000.0, 2000.0, &FramingOptions::default());
        assert!(f.sx <= 2.2 + 1e-6);
    }

    /// A dense linked core plus a wide ring of orphans, like a real vault.
    fn core_and_ring() -> (Vec<[f32; 2]>, Vec<u32>) {
        let mut pts = Vec::new();
        let mut deg = Vec::new();
        for i in 0..400 {
            pts.push([(i % 20) as f32 * 10.0 - 95.0, (i / 20) as f32 * 10.0 - 95.0]);
            deg.push(3);
        }
        for i in 0..120 {
            let a = i as f32 * 0.37;
            let r = 400.0 + (i % 7) as f32 * 15.0;
            pts.push([r * a.cos(), r * a.sin()]);
            deg.push(0);
        }
        (pts, deg)
    }

    #[test]
    fn orphan_ring_does_not_shrink_the_core() {
        let (pts, deg) = core_and_ring();
        let opts = FramingOptions::default();
        let plain = Framing::fit(&pts, 1512.0, 982.0, &opts);
        let core = Framing::fit_graph(&pts, &deg, 1512.0, 982.0, &opts);
        assert!(core.sx > plain.sx * 1.8, "core {} vs plain {}", core.sx, plain.sx);
        // Every note, ring included, stays inside the framing rectangle.
        for p in &pts {
            let [x, y] = core.apply(*p);
            assert!((75.6 - 0.5..=1436.4 + 0.5).contains(&x) && (73.6 - 0.5..=908.4 + 0.5).contains(&y), "{p:?} -> {x},{y}");
        }
        // The core itself is mapped linearly.
        let [x, y] = core.apply([50.0, -20.0]);
        assert!((x - ((50.0 - core.cx) * core.sx + core.tx)).abs() < 1e-3);
        assert!((y - ((-20.0 - core.cy) * core.sy + core.ty)).abs() < 1e-3);
    }

    #[test]
    fn ring_keeps_its_order_outwards() {
        let (pts, deg) = core_and_ring();
        let f = Framing::fit_graph(&pts, &deg, 1512.0, 982.0, &FramingOptions::default());
        let d = |p: [f32; 2]| {
            let [x, y] = f.apply(p);
            (x - f.tx).hypot(y - f.ty)
        };
        for r in [120.0, 200.0, 300.0, 400.0, 480.0] {
            assert!(d([r, 0.0]) < d([r + 10.0, 0.0]), "r = {r}");
            assert!(d([0.0, r]) < d([0.0, r + 10.0]), "r = {r}");
        }
    }

    #[test]
    fn zoom_still_zooms_the_whole_picture() {
        let (pts, deg) = core_and_ring();
        let base = Framing::fit_graph(&pts, &deg, 1512.0, 982.0, &FramingOptions::default());
        let zoomed = Framing::fit_graph(&pts, &deg, 1512.0, 982.0, &FramingOptions { zoom: 2.0, ..Default::default() });
        for p in [[30.0, 40.0], [400.0, 0.0], [-300.0, 280.0]] {
            let (a, b) = (base.apply(p), zoomed.apply(p));
            assert!(((b[0] - zoomed.tx) - 2.0 * (a[0] - base.tx)).abs() < 1e-2);
            assert!(((b[1] - zoomed.ty) - 2.0 * (a[1] - base.ty)).abs() < 1e-2);
        }
    }

    #[test]
    fn small_or_mostly_orphan_graphs_use_every_note() {
        let (pts, _) = core_and_ring();
        // Too few linked notes to be "the core": falls back to all of them.
        let mut deg = vec![0; pts.len()];
        deg[..10].iter_mut().for_each(|d| *d = 1);
        let opts = FramingOptions::default();
        assert_eq!(Framing::fit_graph(&pts, &deg, 1512.0, 982.0, &opts), Framing::fit(&pts, 1512.0, 982.0, &opts));
        // A small fixture-sized graph is framed as before, degrees or not.
        let small: Vec<[f32; 2]> = pts[..30].to_vec();
        let deg = vec![1; 30];
        assert_eq!(Framing::fit_graph(&small, &deg, 1512.0, 982.0, &opts), Framing::fit(&small, 1512.0, 982.0, &opts));
    }
}
