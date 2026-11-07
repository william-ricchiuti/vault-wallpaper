//! Displays: which monitors are attached and where, how a change in that list maps
//! onto wallpaper windows, and span mode (one picture across every display).

use vw_render::{Framing, FramingOptions};
use winit::event_loop::ActiveEventLoop;
use winit::platform::macos::MonitorHandleExtMacOS;

/// One display, in AppKit points with a top-left origin (global coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geom {
    /// `CGDirectDisplayID`: stable while the display stays attached.
    pub id: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub scale: f64,
}

impl Geom {
    pub fn rect(&self) -> [f32; 4] {
        [self.x as f32, self.y as f32, self.w as f32, self.h as f32]
    }
}

/// The attached displays, as winit sees them now.
pub fn monitors(event_loop: &ActiveEventLoop) -> Vec<Geom> {
    event_loop
        .available_monitors()
        .map(|m| {
            let scale = m.scale_factor();
            let pos = m.position().to_logical::<f64>(scale);
            let size = m.size().to_logical::<f64>(scale);
            Geom { id: m.native_id(), x: pos.x, y: pos.y, w: size.width, h: size.height, scale }
        })
        .collect()
}

/// Span mode is opt-in for now: `VW_SPAN=1`.
pub fn span_from_env() -> bool {
    std::env::var("VW_SPAN").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    Add(Geom),
    Remove(u32),
    /// Same display, new position, size or scale.
    Update(Geom),
}

/// What turns the wallpaper windows for `old` into windows for `new`, matched by
/// display id.
pub fn diff(old: &[Geom], new: &[Geom]) -> Vec<Change> {
    let mut out = Vec::new();
    for o in old {
        if !new.iter().any(|n| n.id == o.id) {
            out.push(Change::Remove(o.id));
        }
    }
    for n in new {
        match old.iter().find(|o| o.id == n.id) {
            Some(o) if o != n => out.push(Change::Update(*n)),
            Some(_) => {}
            None => out.push(Change::Add(*n)),
        }
    }
    out
}

/// The smallest rectangle (x, y, w, h) holding every rectangle.
pub fn union(rects: impl IntoIterator<Item = [f32; 4]>) -> Option<[f32; 4]> {
    rects.into_iter().fold(None, |acc, [x, y, w, h]| {
        Some(match acc {
            None => [x, y, w, h],
            Some([ax, ay, aw, ah]) => {
                let (x0, y0) = (ax.min(x), ay.min(y));
                let (x1, y1) = ((ax + aw).max(x + w), (ay + ah).max(y + h));
                [x0, y0, x1 - x0, y1 - y0]
            }
        })
    })
}

/// `framing` fitted to the span rectangle, moved so `screen` (same coordinates)
/// shows its own part of it. The mapping is linear past the outer ring, so a
/// translation is exact.
pub fn slice(framing: Framing, span: [f32; 4], screen: [f32; 4]) -> Framing {
    Framing { tx: framing.tx - (screen[0] - span[0]), ty: framing.ty - (screen[1] - span[1]), ..framing }
}

/// The framing for one screen (`screen` is x, y, w, h in points): fitted to the
/// screen alone, or, in span mode, to the span rectangle and sliced.
pub fn screen_framing(positions: &[[f32; 2]], degree: &[u32], opts: &FramingOptions, screen: [f32; 4], span: Option<[f32; 4]>) -> Framing {
    match span {
        Some(u) => slice(Framing::fit_graph(positions, degree, u[2], u[3], opts), u, screen),
        None => Framing::fit_graph(positions, degree, screen[2], screen[3], opts),
    }
}

/// How screens are framed: the framing options, plus the span rectangle when
/// spanning.
#[derive(Clone, Copy)]
pub struct Fit<'a> {
    pub opts: &'a FramingOptions,
    pub span: Option<[f32; 4]>,
}

impl Fit<'_> {
    pub fn framing(&self, positions: &[[f32; 2]], degree: &[u32], screen: [f32; 4]) -> Framing {
        screen_framing(positions, degree, self.opts, screen, self.span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(id: u32, x: f64, y: f64, w: f64, h: f64) -> Geom {
        Geom { id, x, y, w, h, scale: 2.0 }
    }

    #[test]
    fn diff_adds_removes_and_updates_by_id() {
        let old = [g(1, 0.0, 0.0, 1512.0, 982.0), g(2, 1512.0, 0.0, 2560.0, 1440.0)];
        let new = [g(1, 0.0, 0.0, 1728.0, 1117.0), g(3, -1920.0, 0.0, 1920.0, 1080.0)];
        let d = diff(&old, &new);
        assert_eq!(d, vec![Change::Remove(2), Change::Update(new[0]), Change::Add(new[1])]);
        assert!(diff(&new, &new).is_empty());
        let scaled = [Geom { scale: 1.0, ..new[0] }, new[1]];
        assert_eq!(diff(&new, &scaled), vec![Change::Update(scaled[0])]);
    }

    #[test]
    fn union_covers_side_by_side_and_offset_displays() {
        assert_eq!(union([]), None);
        assert_eq!(union([[0.0, 0.0, 100.0, 50.0]]), Some([0.0, 0.0, 100.0, 50.0]));
        // A second display to the left and lower than the main one.
        assert_eq!(union([[0.0, 0.0, 100.0, 50.0], [-80.0, 20.0, 80.0, 60.0]]), Some([-80.0, 0.0, 180.0, 80.0]));
    }

    fn sample() -> (Vec<[f32; 2]>, Vec<u32>) {
        let mut p = Vec::new();
        for i in 0..40 {
            let a = i as f32 * 0.7;
            p.push([a.cos() * (100.0 + i as f32 * 7.0), a.sin() * (60.0 + i as f32 * 3.0)]);
        }
        (p.clone(), vec![1; p.len()])
    }

    #[test]
    fn slices_tile_the_span_picture() {
        let (pos, deg) = sample();
        let opts = FramingOptions::default();
        let left = [0.0, 0.0, 1512.0, 982.0];
        let right = [1512.0, -200.0, 2560.0, 1440.0];
        let span = union([left, right]).unwrap();
        let whole = Framing::fit_graph(&pos, &deg, span[2], span[3], &opts);
        for screen in [left, right] {
            let f = screen_framing(&pos, &deg, &opts, screen, Some(span));
            for p in &pos {
                let w = whole.apply(*p);
                let s = f.apply(*p);
                // The screen's local point plus its offset in the span is the span point.
                assert!((s[0] + screen[0] - span[0] - w[0]).abs() < 1e-3);
                assert!((s[1] + screen[1] - span[1] - w[1]).abs() < 1e-3);
            }
        }
        // Each note lands on exactly one screen or in the gap the span leaves.
        let on = |f: &Framing, sc: [f32; 4], p: [f32; 2]| {
            let q = f.apply(p);
            q[0] >= 0.0 && q[1] >= 0.0 && q[0] < sc[2] && q[1] < sc[3]
        };
        let (fl, fr) = (screen_framing(&pos, &deg, &opts, left, Some(span)), screen_framing(&pos, &deg, &opts, right, Some(span)));
        for p in &pos {
            assert!(!(on(&fl, left, *p) && on(&fr, right, *p)), "a note on two screens at once");
        }
    }

    #[test]
    fn without_span_each_screen_fits_alone() {
        let (pos, deg) = sample();
        let opts = FramingOptions::default();
        let screen = [1512.0, 0.0, 2560.0, 1440.0];
        let f = screen_framing(&pos, &deg, &opts, screen, None);
        assert_eq!(f, Framing::fit_graph(&pos, &deg, 2560.0, 1440.0, &opts));
    }

    #[test]
    fn single_display_span_is_the_plain_fit() {
        let (pos, deg) = sample();
        let opts = FramingOptions::default();
        let screen = [0.0, 0.0, 1512.0, 982.0];
        let f = screen_framing(&pos, &deg, &opts, screen, union([screen]));
        assert_eq!(f, Framing::fit_graph(&pos, &deg, 1512.0, 982.0, &opts));
    }
}
