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

/// Layout -> screen: `screen = (p - c) * s + t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Framing {
    pub sx: f32,
    pub sy: f32,
    pub cx: f32,
    pub cy: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Framing {
    pub const IDENTITY: Framing = Framing { sx: 1.0, sy: 1.0, cx: 0.0, cy: 0.0, tx: 0.0, ty: 0.0 };

    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        [(p[0] - self.cx) * self.sx + self.tx, (p[1] - self.cy) * self.sy + self.ty]
    }

    /// Fits `points` (layout coordinates) into a `width` x `height` point screen.
    pub fn fit(points: &[[f32; 2]], width: f32, height: f32, opts: &FramingOptions) -> Framing {
        let sa = opts.safe_area;
        let rx = sa.left + width * 0.05;
        let ry = sa.top + height * 0.075;
        let rw = (width - sa.left - sa.right - width * 0.1).max(50.0);
        let rh = (height - sa.top - sa.bottom - height * 0.15).max(50.0);
        if opts.mode == FramingMode::None {
            return Framing { tx: width / 2.0, ty: height / 2.0, ..Framing::IDENTITY };
        }
        let Some([x0, y0, x1, y1]) = percentile_bounds(points) else {
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
        Framing { sx: sx * opts.zoom, sy: sy * opts.zoom, cx: (x0 + x1) / 2.0, cy: (y0 + y1) / 2.0, tx: cx, ty: cy }
    }
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
}
