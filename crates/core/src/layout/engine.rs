//! The layout thread. Owns the simulation, takes new graphs and options, and
//! publishes positions. It runs at utility priority (efficiency cores on Apple
//! Silicon), works as fast as it can while the layout moves, and sleeps otherwise.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use super::{LayoutGraph, LayoutOptions, PositionCache, Simulation, place};
use crate::vault::Graph;

/// Positions published by the engine.
#[derive(Debug, Clone)]
pub struct LayoutFrame {
    /// Increases each time the node set changes, so the receiver knows `ids` changed.
    pub generation: u64,
    pub ids: Arc<[String]>,
    /// [x, y] per id, in layout pixels around (0, 0).
    pub positions: Vec<[f32; 2]>,
    pub alpha: f64,
    /// True for the final frame of a run (nothing will move until the next change).
    pub settled: bool,
}

enum Cmd {
    Graph(Box<Graph>),
    Options(LayoutOptions),
}

pub struct LayoutEngine {
    tx: Sender<Cmd>,
    _thread: thread::JoinHandle<()>,
}

/// Alpha for a run: a full layout for a new graph, a gentle relax when most notes
/// kept their place (the web version's thresholds).
const COLD_ALPHA: f64 = 1.0;
const WARM_ALPHA: f64 = 0.25;
const OPTIONS_ALPHA: f64 = 0.8;
const MAX_TICKS: u64 = 600;

impl LayoutEngine {
    /// Starts the layout thread. `publish_every` limits how often moving frames are
    /// sent (the settled frame is always sent). `cache` is where positions are saved.
    /// Fails only if the OS won't start a thread.
    pub fn spawn(
        opts: LayoutOptions,
        cache: Option<PathBuf>,
        publish_every: Duration,
        on_frame: impl Fn(LayoutFrame) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let thread = thread::Builder::new().name("vw-layout".into()).spawn(move || {
            lower_priority();
            Worker::new(opts, cache, publish_every, Box::new(on_frame)).run(rx);
        })?;
        Ok(LayoutEngine { tx, _thread: thread })
    }

    pub fn set_graph(&self, graph: Graph) {
        let _ = self.tx.send(Cmd::Graph(Box::new(graph)));
    }

    pub fn set_options(&self, opts: LayoutOptions) {
        let _ = self.tx.send(Cmd::Options(opts));
    }
}

struct Worker {
    opts: LayoutOptions,
    cache_path: Option<PathBuf>,
    cache: Option<PositionCache>,
    publish_every: Duration,
    on_frame: Box<dyn Fn(LayoutFrame) + Send>,
    graph: LayoutGraph,
    /// Fingerprints of `graph`, by id.
    fingerprints: HashMap<String, u64>,
    ids: Arc<[String]>,
    sim: Option<Simulation>,
    generation: u64,
    running: bool,
    last_publish: Instant,
}

impl Worker {
    fn new(opts: LayoutOptions, cache_path: Option<PathBuf>, publish_every: Duration, on_frame: Box<dyn Fn(LayoutFrame) + Send>) -> Self {
        let cache = cache_path.as_deref().and_then(PositionCache::load);
        Worker {
            opts,
            cache_path,
            cache,
            publish_every,
            on_frame,
            graph: LayoutGraph::default(),
            fingerprints: HashMap::new(),
            ids: Arc::from(Vec::new()),
            sim: None,
            generation: 0,
            running: false,
            last_publish: Instant::now(),
        }
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        loop {
            // Idle: block until there is something to do. Running: take commands
            // without waiting, between batches of ticks.
            let cmd = if self.running {
                match rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            } else {
                match rx.recv_timeout(Duration::from_secs(3600)) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            };
            match cmd {
                Some(Cmd::Graph(g)) => self.load_graph(&g),
                Some(Cmd::Options(o)) => self.set_options(o),
                None => self.step(),
            }
        }
    }

    /// Current positions and velocities by id, from the running simulation.
    fn current(&self) -> HashMap<String, [f64; 4]> {
        let Some(sim) = &self.sim else { return HashMap::new() };
        self.graph.id.iter().enumerate().map(|(i, id)| (id.clone(), [sim.x[i], sim.y[i], sim.vx[i], sim.vy[i]])).collect()
    }

    fn load_graph(&mut self, g: &Graph) {
        let graph = LayoutGraph::from_graph(g);
        let fingerprints = graph.fingerprints();
        let mut previous = self.current();
        let mut previous_prints = &self.fingerprints;
        if previous.is_empty()
            && let Some(c) = self.cache.as_ref().filter(|c| self.cache_matches(c))
        {
            previous = c
                .positions
                .iter()
                .map(|(id, &[x, y])| (id.clone(), [f64::from(x), f64::from(y), 0.0, 0.0]))
                .collect();
            previous_prints = &c.fingerprints;
        }
        // Notes whose links, tag or existence changed since the positions we have.
        let changed: Vec<bool> = graph
            .id
            .iter()
            .zip(&fingerprints)
            .map(|(id, fp)| !previous.contains_key(id) || previous_prints.get(id) != Some(fp))
            .collect();
        let n_changed = changed.iter().filter(|c| **c).count();
        let present: std::collections::HashSet<&str> = graph.id.iter().map(String::as_str).collect();
        let removed = previous.keys().any(|id| !present.contains(id.as_str()));

        let placement = place(&graph, &previous, &self.opts);
        self.generation += 1;
        self.ids = Arc::from(graph.id.clone());
        let sim = |alpha| Simulation::new(&graph, &self.opts, placement.x.clone(), placement.y.clone(), placement.vx.clone(), placement.vy.clone(), alpha);
        let (sim, running) = if placement.kept == 0 {
            (sim(COLD_ALPHA), true)
        } else if n_changed == 0 && !removed {
            // Same graph as the positions we hold: show them, no work.
            (sim(0.0), false)
        } else {
            // Let the changed notes and their direct neighbours settle; pin the rest so
            // the wallpaper doesn't shift on every save. A big change relaxes everything.
            let mut mobile = changed.clone();
            for (i, ns) in graph.neighbours().iter().enumerate() {
                if changed[i] {
                    for &j in ns {
                        mobile[j as usize] = true;
                    }
                }
            }
            let n_mobile = mobile.iter().filter(|m| **m).count();
            let mut s = sim(WARM_ALPHA);
            if n_mobile * 5 <= graph.len() {
                s.set_mobile(mobile);
            } else if placement.kept * 5 < graph.len() * 4 {
                s = sim(COLD_ALPHA);
            }
            (s, true)
        };
        self.sim = Some(sim);
        self.graph = graph;
        self.fingerprints = self.graph.id.iter().cloned().zip(fingerprints).collect();
        self.running = running;
        self.publish(!running);
    }

    fn cache_matches(&self, c: &PositionCache) -> bool {
        c.layout == self.opts.kind.name() && c.cluster_by_tag == self.opts.cluster_by_tag
    }

    fn set_options(&mut self, opts: LayoutOptions) {
        let layout_changed = opts.kind != self.opts.kind || opts.cluster_by_tag != self.opts.cluster_by_tag;
        self.opts = opts;
        if !layout_changed || self.graph.is_empty() {
            return;
        }
        let previous = self.current();
        let placement = place(&self.graph, &previous, &self.opts);
        self.sim = Some(Simulation::new(&self.graph, &self.opts, placement.x, placement.y, placement.vx, placement.vy, OPTIONS_ALPHA));
        self.running = true;
    }

    /// A batch of ticks (about 8 ms of work), then publish if due.
    fn step(&mut self) {
        let Some(sim) = &mut self.sim else {
            self.running = false;
            return;
        };
        let start = Instant::now();
        while !sim.is_settled() && sim.ticks() < MAX_TICKS && start.elapsed() < Duration::from_millis(8) {
            sim.tick();
        }
        if sim.is_settled() || sim.ticks() >= MAX_TICKS {
            self.running = false;
            // Save first, so whoever reacts to the settled frame finds the cache on disk.
            self.save_cache();
            self.publish(true);
        } else if self.last_publish.elapsed() >= self.publish_every {
            self.publish(false);
        }
    }

    fn publish(&mut self, settled: bool) {
        let Some(sim) = &self.sim else { return };
        let positions = sim.x.iter().zip(&sim.y).map(|(&x, &y)| [x as f32, y as f32]).collect();
        self.last_publish = Instant::now();
        (self.on_frame)(LayoutFrame {
            generation: self.generation,
            ids: self.ids.clone(),
            positions,
            alpha: sim.alpha(),
            settled,
        });
    }

    fn save_cache(&mut self) {
        let (Some(path), Some(sim)) = (&self.cache_path, &self.sim) else { return };
        let cache = PositionCache {
            version: 0,
            layout: self.opts.kind.name().to_string(),
            cluster_by_tag: self.opts.cluster_by_tag,
            positions: self.graph.id.iter().enumerate().map(|(i, id)| (id.clone(), [sim.x[i] as f32, sim.y[i] as f32])).collect(),
            fingerprints: self.fingerprints.clone(),
        };
        if let Err(e) = cache.save(path) {
            eprintln!("layout cache: could not save {} ({e})", path.display());
        }
        self.cache = Some(cache);
    }
}

/// Utility QoS: macOS schedules the thread on efficiency cores where it can and below
/// anything the user is interacting with.
fn lower_priority() {
    #[cfg(target_os = "macos")]
    // SAFETY: plain libc call on the current thread with a valid QoS class.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}
