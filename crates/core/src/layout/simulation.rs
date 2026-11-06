//! Force simulation: a port of the d3-force setup the web version runs
//! (`layout-core.js` `createSimulation`), with the same forces, strengths, size tiers
//! and integration. Positions are not bit-identical to d3 (different starting points
//! and quadtree), but the forces and their balance are.

use super::quadtree::QuadTree;
use super::targets::compute_targets;
use super::{LayoutGraph, LayoutKind, LayoutOptions};

const MASSIVE: usize = 3000;
const LARGE: usize = 600;
const MEDIUM: usize = 200;
const SMALL: usize = 50;

/// The web worker's rule: settled once alpha stays below this for `IDLE_TICKS` ticks.
const IDLE_ALPHA: f64 = 0.02;
const IDLE_TICKS: u32 = 30;

/// d3's linear congruential generator, used for "jiggle" when points coincide.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = (1_664_525 * self.0 + 1_013_904_223) % 4_294_967_296;
        self.0 as f64 / 4_294_967_296.0
    }
    fn jiggle(&mut self) -> f64 {
        (self.next() - 0.5) * 1e-6
    }
}

struct LinkForce {
    distance: f64,
    strength: f64,
    /// Share of each link's correction applied to its target (by relative degree).
    bias: Vec<f64>,
}

struct ChargeForce {
    strength: f64,
    theta2: f64,
}

struct ClusterForce {
    strength: f64,
    keys: Vec<i32>,
    groups: usize,
    sx: Vec<f64>,
    sy: Vec<f64>,
    cnt: Vec<u32>,
}

struct TargetForce {
    tx: Vec<f64>,
    ty: Vec<f64>,
    sx: f64,
    sy: f64,
}

pub struct Simulation {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub vx: Vec<f64>,
    pub vy: Vec<f64>,
    alpha: f64,
    alpha_decay: f64,
    /// Fraction of velocity kept each tick (d3 stores 1 - velocityDecay).
    velocity_keep: f64,
    links: Vec<(u32, u32)>,
    radius: Vec<f64>,
    link: Option<LinkForce>,
    charge: Option<ChargeForce>,
    center: Option<(f64, f64)>,
    collide_pad: Option<f64>,
    cluster: Option<ClusterForce>,
    targets: Option<TargetForce>,
    /// When set, only these nodes move; the rest stay pinned but still exert forces.
    mobile: Option<Vec<bool>>,
    mobile_idx: Vec<u32>,
    rng: Lcg,
    idle_ticks: u32,
    ticks: u64,
    tree: QuadTree,
    scratch_x: Vec<f64>,
    scratch_y: Vec<f64>,
}

impl Simulation {
    /// Builds the simulation for `graph` starting from positions `x`, `y` (and
    /// velocities, zero for a cold start) at the given alpha.
    pub fn new(graph: &LayoutGraph, opts: &LayoutOptions, x: Vec<f64>, y: Vec<f64>, vx: Vec<f64>, vy: Vec<f64>, alpha: f64) -> Self {
        let n = graph.len();
        let massive = n > MASSIVE;
        let links = graph.links.clone();
        let link_force = |distance: f64, strength: f64| {
            let deg = &graph.degree;
            let bias = links
                .iter()
                .map(|&(s, t)| {
                    let (ds, dt) = (f64::from(deg[s as usize]), f64::from(deg[t as usize]));
                    ds / (ds + dt)
                })
                .collect();
            LinkForce { distance, strength, bias }
        };
        let mut sim = Simulation {
            x,
            y,
            vx,
            vy,
            alpha,
            alpha_decay: 0.0,
            velocity_keep: 0.0,
            radius: graph.radius.clone(),
            link: None,
            charge: None,
            center: None,
            collide_pad: None,
            cluster: None,
            targets: None,
            mobile: None,
            mobile_idx: (0..n as u32).collect(),
            rng: Lcg(1),
            idle_ticks: 0,
            ticks: 0,
            tree: QuadTree::new(),
            scratch_x: vec![0.0; n],
            scratch_y: vec![0.0; n],
            links: Vec::new(),
        };
        match compute_targets(opts.kind, graph, opts) {
            Some((tx, ty)) => {
                let sx = if opts.kind == LayoutKind::Timeline { 0.55 } else { 0.35 };
                sim.targets = Some(TargetForce { tx, ty, sx, sy: 0.35 });
                sim.link = Some(link_force(30.0, 0.02));
                sim.collide_pad = (!massive).then_some(1.2);
                sim.alpha_decay = if massive { 0.05 } else { 0.035 };
                sim.velocity_keep = 1.0 - 0.55;
            }
            None => {
                let charge = if massive {
                    -4.0
                } else if n > LARGE {
                    -10.0
                } else if n > MEDIUM {
                    -15.0
                } else if n > SMALL {
                    -25.0
                } else {
                    -30.0
                };
                let distance = if massive {
                    16.0
                } else if n > LARGE {
                    24.0
                } else if n > MEDIUM {
                    30.0
                } else {
                    40.0
                };
                sim.link = Some(link_force(distance, if massive { 0.15 } else { 0.3 }));
                let theta: f64 = if massive { 1.2 } else { 0.9 };
                sim.charge = Some(ChargeForce { strength: charge, theta2: theta * theta });
                sim.center = Some((0.0, 0.0));
                sim.collide_pad = (!massive).then_some(2.0);
                if opts.cluster_by_tag {
                    sim.cluster = Some(cluster_force(graph, 0.03));
                }
                sim.alpha_decay = if massive {
                    0.05
                } else if n > LARGE {
                    0.03
                } else {
                    0.02
                };
                sim.velocity_keep = 1.0
                    - if massive {
                        0.75
                    } else if n > LARGE {
                        0.68
                    } else {
                        0.6
                    };
            }
        }
        sim.links = links;
        sim
    }

    /// Restricts movement to the nodes marked true (a local relax after an edit).
    pub fn set_mobile(&mut self, mobile: Vec<bool>) {
        self.mobile_idx = mobile.iter().enumerate().filter(|(_, m)| **m).map(|(i, _)| i as u32).collect();
        self.mobile = Some(mobile);
    }

    fn is_mobile(&self, i: usize) -> bool {
        self.mobile.as_ref().is_none_or(|m| m[i])
    }

    pub fn alpha(&self) -> f64 {
        self.alpha
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn is_settled(&self) -> bool {
        self.x.is_empty() || self.idle_ticks >= IDLE_TICKS
    }

    /// One step: decay alpha, apply forces in the web version's order, integrate.
    pub fn tick(&mut self) {
        self.alpha += (0.0 - self.alpha) * self.alpha_decay;
        let alpha = self.alpha;
        if self.targets.is_some() {
            self.apply_targets(alpha);
            self.apply_links(alpha);
            self.apply_collide();
        } else {
            self.apply_links(alpha);
            self.apply_charge(alpha);
            self.apply_center();
            self.apply_collide();
            self.apply_cluster(alpha);
        }
        for i in 0..self.x.len() {
            if !self.is_mobile(i) {
                self.vx[i] = 0.0;
                self.vy[i] = 0.0;
                continue;
            }
            self.vx[i] *= self.velocity_keep;
            self.vy[i] *= self.velocity_keep;
            self.x[i] += self.vx[i];
            self.y[i] += self.vy[i];
        }
        self.ticks += 1;
        if self.alpha < IDLE_ALPHA {
            self.idle_ticks += 1;
        } else {
            self.idle_ticks = 0;
        }
    }

    fn apply_targets(&mut self, alpha: f64) {
        let Some(t) = &self.targets else { return };
        for i in 0..self.x.len() {
            self.vx[i] += (t.tx[i] - self.x[i]) * t.sx * alpha;
            self.vy[i] += (t.ty[i] - self.y[i]) * t.sy * alpha;
        }
    }

    fn apply_links(&mut self, alpha: f64) {
        let Some(f) = &self.link else { return };
        for (k, &(s, t)) in self.links.iter().enumerate() {
            let (s, t) = (s as usize, t as usize);
            let mut x = self.x[t] + self.vx[t] - self.x[s] - self.vx[s];
            let mut y = self.y[t] + self.vy[t] - self.y[s] - self.vy[s];
            if x == 0.0 {
                x = self.rng.jiggle();
            }
            if y == 0.0 {
                y = self.rng.jiggle();
            }
            let mut l = (x * x + y * y).sqrt();
            l = (l - f.distance) / l * alpha * f.strength;
            x *= l;
            y *= l;
            let b = f.bias[k];
            self.vx[t] -= x * b;
            self.vy[t] -= y * b;
            self.vx[s] += x * (1.0 - b);
            self.vy[s] += y * (1.0 - b);
        }
    }

    /// Barnes-Hut many-body repulsion (d3 `forceManyBody`, distanceMin 1, no max).
    fn apply_charge(&mut self, alpha: f64) {
        let Some(f) = &self.charge else { return };
        let (strength, theta2) = (f.strength, f.theta2);
        self.tree.build(&self.x, &self.y, None);
        let tree = &self.tree;
        let mut stack: Vec<u32> = Vec::with_capacity(64);
        for k in 0..self.mobile_idx.len() {
            let i = self.mobile_idx[k] as usize;
            let (xi, yi) = (self.x[i], self.y[i]);
            let (mut fx, mut fy) = (0.0, 0.0);
            stack.clear();
            stack.push(0);
            while let Some(id) = stack.pop() {
                let cell = &tree.cells[id as usize];
                let mut x = cell.cx - xi;
                let mut y = cell.cy - yi;
                let mut l = x * x + y * y;
                // Far enough: treat the whole cell as one body at its centroid.
                if cell.size * cell.size / theta2 < l {
                    if x == 0.0 {
                        x = self.rng.jiggle();
                        l += x * x;
                    }
                    if y == 0.0 {
                        y = self.rng.jiggle();
                        l += y * y;
                    }
                    if l < 1.0 {
                        l = l.sqrt();
                    }
                    let w = strength * f64::from(cell.count) * alpha / l;
                    fx += x * w;
                    fy += y * w;
                    continue;
                }
                if !cell.is_leaf() {
                    stack.extend(cell.children());
                    continue;
                }
                for &j in tree.leaf_points(cell) {
                    let j = j as usize;
                    if j == i {
                        continue;
                    }
                    let mut x = self.x[j] - xi;
                    let mut y = self.y[j] - yi;
                    let mut l = x * x + y * y;
                    if x == 0.0 {
                        x = self.rng.jiggle();
                        l += x * x;
                    }
                    if y == 0.0 {
                        y = self.rng.jiggle();
                        l += y * y;
                    }
                    if l < 1.0 {
                        l = l.sqrt();
                    }
                    let w = strength * alpha / l;
                    fx += x * w;
                    fy += y * w;
                }
            }
            self.vx[i] += fx;
            self.vy[i] += fy;
        }
    }

    fn apply_center(&mut self) {
        let Some((cx, cy)) = self.center else { return };
        if self.mobile.is_some() {
            // Re-centring would drag the pinned nodes along.
            return;
        }
        let n = self.x.len() as f64;
        let sx = self.x.iter().sum::<f64>() / n - cx;
        let sy = self.y.iter().sum::<f64>() / n - cy;
        self.x.iter_mut().for_each(|v| *v -= sx);
        self.y.iter_mut().for_each(|v| *v -= sy);
    }

    /// d3 `forceCollide` (strength 1, one iteration) on predicted positions.
    fn apply_collide(&mut self) {
        let Some(pad) = self.collide_pad else { return };
        let n = self.x.len();
        for i in 0..n {
            self.scratch_x[i] = self.x[i] + self.vx[i];
            self.scratch_y[i] = self.y[i] + self.vy[i];
        }
        let radii: Vec<f64> = self.radius.iter().map(|r| r + pad).collect();
        self.tree.build(&self.scratch_x, &self.scratch_y, Some(&radii));
        let tree = &self.tree;
        let mut stack: Vec<u32> = Vec::with_capacity(64);
        for k in 0..self.mobile_idx.len() {
            let i = self.mobile_idx[k] as usize;
            let ri = radii[i];
            let ri2 = ri * ri;
            let xi = self.x[i] + self.vx[i];
            let yi = self.y[i] + self.vy[i];
            stack.clear();
            stack.push(0);
            while let Some(id) = stack.pop() {
                let cell = &tree.cells[id as usize];
                let r = ri + cell.rmax;
                if cell.x0 > xi + r || cell.x0 + cell.size < xi - r || cell.y0 > yi + r || cell.y0 + cell.size < yi - r {
                    continue;
                }
                if !cell.is_leaf() {
                    stack.extend(cell.children());
                    continue;
                }
                for &j in tree.leaf_points(cell) {
                    let j = j as usize;
                    // Each pair once: mobile pairs from the lower index, pinned
                    // partners from the mobile side.
                    if j == i || (j < i && self.mobile.as_ref().is_none_or(|m| m[j])) {
                        continue;
                    }
                    let rj = radii[j];
                    let r = ri + rj;
                    let mut x = xi - self.x[j] - self.vx[j];
                    let mut y = yi - self.y[j] - self.vy[j];
                    let mut l = x * x + y * y;
                    if l < r * r {
                        if x == 0.0 {
                            x = self.rng.jiggle();
                            l += x * x;
                        }
                        if y == 0.0 {
                            y = self.rng.jiggle();
                            l += y * y;
                        }
                        let d = l.sqrt();
                        let k = (r - d) / d;
                        x *= k;
                        y *= k;
                        let rj2 = rj * rj;
                        let share = rj2 / (ri2 + rj2);
                        self.vx[i] += x * share;
                        self.vy[i] += y * share;
                        self.vx[j] -= x * (1.0 - share);
                        self.vy[j] -= y * (1.0 - share);
                    }
                }
            }
        }
    }

    /// The web version's tag-cluster force: pull each tagged node toward its tag's
    /// centroid.
    fn apply_cluster(&mut self, alpha: f64) {
        let Some(c) = &mut self.cluster else { return };
        c.sx.iter_mut().for_each(|v| *v = 0.0);
        c.sy.iter_mut().for_each(|v| *v = 0.0);
        c.cnt.iter_mut().for_each(|v| *v = 0);
        for i in 0..self.x.len() {
            let k = c.keys[i];
            if k < 0 {
                continue;
            }
            let k = k as usize;
            c.sx[k] += self.x[i];
            c.sy[k] += self.y[i];
            c.cnt[k] += 1;
        }
        let s = c.strength * alpha;
        for i in 0..self.x.len() {
            let k = c.keys[i];
            if k < 0 || c.cnt[k as usize] < 2 {
                continue;
            }
            let k = k as usize;
            let n = f64::from(c.cnt[k]);
            self.vx[i] += (c.sx[k] / n - self.x[i]) * s;
            self.vy[i] += (c.sy[k] / n - self.y[i]) * s;
        }
        debug_assert!(c.groups == c.sx.len());
    }
}

fn cluster_force(graph: &LayoutGraph, strength: f64) -> ClusterForce {
    let mut ids: std::collections::HashMap<&str, i32> = std::collections::HashMap::new();
    let keys: Vec<i32> = graph
        .tag
        .iter()
        .map(|t| match t {
            Some(t) => {
                let next = ids.len() as i32;
                *ids.entry(t.as_str()).or_insert(next)
            }
            None => -1,
        })
        .collect();
    let groups = ids.len();
    ClusterForce { strength, keys, groups, sx: vec![0.0; groups], sy: vec![0.0; groups], cnt: vec![0; groups] }
}
