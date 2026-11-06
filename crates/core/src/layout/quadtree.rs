//! A flat quadtree over node positions, rebuilt each tick, for Barnes-Hut repulsion and
//! collision. Cells are squares; a leaf holds one point, or several when they coincide
//! (or the depth limit is hit).

const NONE: u32 = u32::MAX;
const MAX_DEPTH: u32 = 48;

#[derive(Clone, Copy)]
pub(crate) struct Cell {
    pub x0: f64,
    pub y0: f64,
    pub size: f64,
    pub children: [u32; 4],
    /// Range into `QuadTree::order` for leaves.
    pub start: u32,
    pub len: u32,
    /// Number of points below this cell, and their centroid.
    pub count: u32,
    pub cx: f64,
    pub cy: f64,
    /// Largest radius below this cell (collision only).
    pub rmax: f64,
}

impl Cell {
    pub fn is_leaf(&self) -> bool {
        self.children == [NONE; 4]
    }

    pub fn children(&self) -> impl Iterator<Item = u32> + '_ {
        self.children.iter().copied().filter(|&c| c != NONE)
    }
}

pub(crate) struct QuadTree {
    pub cells: Vec<Cell>,
    /// Point indices, grouped so each leaf's points are contiguous.
    pub order: Vec<u32>,
    scratch: Vec<u32>,
}

impl QuadTree {
    pub fn new() -> Self {
        QuadTree { cells: Vec::new(), order: Vec::new(), scratch: Vec::new() }
    }

    /// Rebuilds over points (`px[i]`, `py[i]`), reusing allocations. `radius` fills
    /// `rmax` when given.
    pub fn build(&mut self, px: &[f64], py: &[f64], radius: Option<&[f64]>) {
        let n = px.len();
        self.cells.clear();
        self.order.clear();
        self.order.extend(0..n as u32);
        if n == 0 {
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for i in 0..n {
            x0 = x0.min(px[i]);
            y0 = y0.min(py[i]);
            x1 = x1.max(px[i]);
            y1 = y1.max(py[i]);
        }
        let size = (x1 - x0).max(y1 - y0).max(1e-9) * (1.0 + 1e-9);
        self.scratch.resize(n, 0);
        let mut order = std::mem::take(&mut self.order);
        self.split(&mut order, 0, n, x0, y0, size, 0, px, py);
        self.order = order;
        self.accumulate(0, px, py, radius);
    }

    #[allow(clippy::too_many_arguments)]
    fn split(&mut self, order: &mut [u32], start: usize, end: usize, x0: f64, y0: f64, size: f64, depth: u32, px: &[f64], py: &[f64]) -> u32 {
        let id = self.cells.len() as u32;
        self.cells.push(Cell {
            x0,
            y0,
            size,
            children: [NONE; 4],
            start: start as u32,
            len: (end - start) as u32,
            count: (end - start) as u32,
            cx: 0.0,
            cy: 0.0,
            rmax: 0.0,
        });
        let pts = &order[start..end];
        let first = pts[0] as usize;
        let coincident = pts.iter().all(|&i| px[i as usize] == px[first] && py[i as usize] == py[first]);
        if pts.len() == 1 || coincident || depth >= MAX_DEPTH {
            return id;
        }
        // Counting partition of the points into the four quadrants.
        let half = size / 2.0;
        let (mx, my) = (x0 + half, y0 + half);
        let quad = |i: u32| -> usize {
            let i = i as usize;
            usize::from(px[i] >= mx) | (usize::from(py[i] >= my) << 1)
        };
        let mut counts = [0usize; 4];
        for &i in pts {
            counts[quad(i)] += 1;
        }
        let mut offsets = [0usize; 4];
        for q in 1..4 {
            offsets[q] = offsets[q - 1] + counts[q - 1];
        }
        let mut cursor = offsets;
        for &i in pts {
            let q = quad(i);
            self.scratch[cursor[q]] = i;
            cursor[q] += 1;
        }
        order[start..end].copy_from_slice(&self.scratch[..end - start]);
        let mut children = [NONE; 4];
        for q in 0..4 {
            if counts[q] == 0 {
                continue;
            }
            let (cx0, cy0) = (if q & 1 == 1 { mx } else { x0 }, if q & 2 == 2 { my } else { y0 });
            let s = start + offsets[q];
            children[q] = self.split(order, s, s + counts[q], cx0, cy0, half, depth + 1, px, py);
        }
        self.cells[id as usize].children = children;
        id
    }

    /// Post-order pass filling centroids and (optionally) the largest radius.
    fn accumulate(&mut self, id: u32, px: &[f64], py: &[f64], radius: Option<&[f64]>) {
        let cell = self.cells[id as usize];
        if cell.is_leaf() {
            let (mut sx, mut sy, mut r) = (0.0, 0.0, 0.0f64);
            for &i in &self.order[cell.start as usize..(cell.start + cell.len) as usize] {
                sx += px[i as usize];
                sy += py[i as usize];
                if let Some(rad) = radius {
                    r = r.max(rad[i as usize]);
                }
            }
            let c = &mut self.cells[id as usize];
            c.cx = sx / f64::from(cell.len);
            c.cy = sy / f64::from(cell.len);
            c.rmax = r;
            return;
        }
        let (mut sx, mut sy, mut r) = (0.0, 0.0, 0.0f64);
        for child in cell.children() {
            self.accumulate(child, px, py, radius);
            let c = self.cells[child as usize];
            sx += c.cx * f64::from(c.count);
            sy += c.cy * f64::from(c.count);
            r = r.max(c.rmax);
        }
        let c = &mut self.cells[id as usize];
        c.cx = sx / f64::from(c.count);
        c.cy = sy / f64::from(c.count);
        c.rmax = r;
    }

    pub fn leaf_points(&self, cell: &Cell) -> &[u32] {
        &self.order[cell.start as usize..(cell.start + cell.len) as usize]
    }
}
