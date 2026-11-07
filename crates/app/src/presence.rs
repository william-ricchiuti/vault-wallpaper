//! The Obsidian plugin's view of what you're doing, turned into a highlight.
//!
//! When the plugin reports an open note, that note and its direct neighbours are
//! drawn brighter (the open note also gets a label) and everything else slightly
//! dimmed. It is a change to the cached scene's [`Look`], redrawn once when the open
//! note changes, never per frame. A save reported by the plugin starts the theme's
//! edit burst for that note.
//!
//! With no plugin, or a state older than [`STALE_AFTER`](vw_core::presence::STALE_AFTER),
//! nothing here changes the picture.

use std::path::Path;
use std::time::{Duration, Instant};

use vw_core::layout::LayoutGraph;
use vw_core::presence::{self, PresenceState, PresenceWatcher, node_for_path};
use vw_core::vault::Graph;
use vw_render::{LabelPick, Look, SceneStyle};

/// A save older than this when the app first sees it is history, not an edit to animate.
const SAVE_RECENT_MS: f64 = 10_000.0;
const LABEL_MAX_CHARS: usize = 28;

/// What a new state changed.
#[derive(Debug, Default, PartialEq)]
pub struct Change {
    /// The highlight (open note or its links) is different: redraw the scene.
    pub highlight: bool,
    /// A note saved in Obsidian since the last state, as a vault-relative path.
    pub saved: Option<String>,
}

#[derive(Default)]
pub struct Presence {
    _watcher: Option<PresenceWatcher>,
    state: Option<PresenceState>,
    last_save: Option<(String, f64)>,
}

impl Presence {
    /// Starts watching the plugin's state file in the vault at `root`.
    pub fn start(root: &Path, on_state: impl Fn(Option<PresenceState>) + Send + 'static) -> Presence {
        match presence::watch(root, on_state) {
            Ok(w) => Presence { _watcher: Some(w), ..Presence::default() },
            Err(e) => {
                eprintln!("not watching the Obsidian plugin's state: {e}");
                Presence::default()
            }
        }
    }

    /// The open note and Obsidian's links for it, while the state is fresh.
    fn effective(&self, now_ms: f64) -> Option<(&str, &[String])> {
        let s = self.state.as_ref().filter(|s| s.is_fresh(now_ms))?;
        Some((s.active.as_deref()?, &s.links))
    }

    /// True while a fresh state from the plugin is in hand (Obsidian is open).
    pub fn live(&self, now_ms: f64) -> bool {
        self.state.as_ref().is_some_and(|s| s.is_fresh(now_ms))
    }

    /// The open note, while the state is fresh.
    pub fn active(&self, now_ms: f64) -> Option<&str> {
        self.effective(now_ms).map(|(a, _)| a)
    }

    pub fn update(&mut self, state: Option<PresenceState>, now_ms: f64) -> Change {
        let before = self.effective(now_ms).map(|(a, l)| (a.to_string(), l.to_vec()));
        let save = state.as_ref().filter(|s| s.is_fresh(now_ms)).and_then(|s| s.save()).map(|(p, t)| (p.to_string(), t));
        // A state that is already stale is the same as none (and needs no expiry).
        self.state = state.filter(|s| s.is_fresh(now_ms));
        let after = self.effective(now_ms).map(|(a, l)| (a.to_string(), l.to_vec()));
        let saved = match save {
            Some(s) if self.last_save.as_ref() != Some(&s) => {
                let recent = now_ms - s.1 < SAVE_RECENT_MS;
                self.last_save = Some(s.clone());
                recent.then_some(s.0)
            }
            _ => None,
        };
        Change { highlight: before != after, saved }
    }

    /// Drops a state that went stale; true when that removed a highlight.
    pub fn expire(&mut self, now_ms: f64) -> bool {
        if self.state.as_ref().is_some_and(|s| !s.is_fresh(now_ms)) {
            let had = self.state.as_ref().is_some_and(|s| s.active.is_some());
            self.state = None;
            return had;
        }
        false
    }

    /// When the current state goes stale, so the main loop can wake once for it.
    pub fn wake_at(&self, now_ms: f64) -> Option<Instant> {
        let s = self.state.as_ref()?;
        let left = (s.fresh_until_ms() - now_ms).max(0.0);
        Some(Instant::now() + Duration::from_secs_f64(left / 1000.0 + 0.05))
    }

    /// Applies the highlight to a freshly derived look, if a note is open.
    pub fn apply(&self, look: &mut Look, graph: &Graph, layout: &LayoutGraph, style: &SceneStyle, now_ms: f64) {
        let Some((active, links)) = self.effective(now_ms) else { return };
        let Some(i) = node_for_path(graph, active) else { return };
        let extra: Vec<usize> = links.iter().filter_map(|p| node_for_path(graph, p)).collect();
        highlight(look, graph, layout, i, &extra, light(style));
    }

    /// Labels must be on for the open note's label to show; with hub labels off, the
    /// look carries only that one.
    pub fn adjust_style(&self, style: &mut SceneStyle, now_ms: f64) {
        if self.active(now_ms).is_some() {
            style.labels = true;
        }
    }
}

impl crate::App {
    /// A new state from the plugin: redraw the scene if the highlight moved, and start
    /// the edit burst of a note saved in Obsidian.
    pub(crate) fn on_presence(&mut self, state: Option<PresenceState>) {
        let now = presence::now_ms();
        let change = self.presence.update(state, now);
        if change.highlight {
            eprintln!("[{:.0} ms] obsidian: open note {}", self.ms(), self.presence.active(now).unwrap_or("(none)"));
            self.refresh_highlight();
        }
        if let Some(path) = change.saved {
            let id = self.shown.as_ref().and_then(|sh| node_for_path(&sh.graph, &path).map(|i| sh.graph.nodes[i].id.clone()));
            eprintln!("[{:.0} ms] obsidian: saved {path}", self.ms());
            if let Some(id) = id {
                self.burst_notes(vec![id]);
            }
        }
    }

    /// Drops the highlight once the state goes stale; returns when to check next.
    pub(crate) fn step_presence(&mut self) -> Option<Instant> {
        let now = presence::now_ms();
        if self.presence.expire(now) {
            eprintln!("[{:.0} ms] obsidian: state is stale; highlight off", self.ms());
            self.refresh_highlight();
        }
        self.presence.wake_at(now)
    }

    /// Re-derives the shown look (and the label switch) for the current highlight and
    /// redraws the cached scene once.
    fn refresh_highlight(&mut self) {
        self.apply_style();
        if let Some(s) = self.shown.take() {
            let look = self.look_for(&s.graph, &s.layout);
            self.shown = Some(crate::Shown { look, ..s });
        }
        self.invalidate_scene();
    }
}

fn light(style: &SceneStyle) -> bool {
    let bg = style.background;
    0.2126 * bg[0] + 0.7152 * bg[1] + 0.0722 * bg[2] > 0.4
}

/// Brightens note `active` and its direct neighbours (graph links both ways, plus
/// `extra`), labels `active`, and dims everything else a little. On a light
/// background "brighter" means darker ink.
pub fn highlight(look: &mut Look, graph: &Graph, layout: &LayoutGraph, active: usize, extra: &[usize], light: bool) {
    let n = look.node_color.len();
    if active >= n {
        return;
    }
    let mut near = vec![false; n];
    for &(a, b) in &layout.links {
        let (a, b) = (a as usize, b as usize);
        if a == active && b < n {
            near[b] = true;
        } else if b == active && a < n {
            near[a] = true;
        }
    }
    for &e in extra {
        if e < n {
            near[e] = true;
        }
    }
    near[active] = false;
    let toward = if light { 0.0 } else { 1.0 };
    let lift = |c: &mut [f32; 4], amount: f32| {
        for ch in c.iter_mut().take(3) {
            *ch += (toward - *ch) * amount;
        }
    };
    for (i, c) in look.node_color.iter_mut().enumerate() {
        if i == active {
            lift(c, 0.35);
            c[3] = 1.0;
            if let Some(r) = look.node_radius.get_mut(i) {
                *r = (*r * 1.5).max(*r + 3.0);
            }
        } else if near[i] {
            lift(c, 0.15);
            // Out-of-focus notes (alpha 0) stay hidden.
            if c[3] > 0.0 {
                c[3] = (c[3] * 1.3 + 0.15).min(1.0);
            }
            if let Some(r) = look.node_radius.get_mut(i) {
                *r *= 1.15;
            }
        } else {
            c[3] *= 0.55;
        }
    }
    for (k, &(a, b)) in layout.links.iter().enumerate() {
        let Some(c) = look.edge_color.get_mut(k) else { break };
        if a as usize == active || b as usize == active {
            if c[3] > 0.0 || near[a as usize] || near[b as usize] {
                c[3] = (c[3] * 1.8).clamp(0.35, 1.0);
            }
        } else {
            c[3] *= 0.55;
        }
    }
    // The open note's label goes first, so it's never the one crowded out.
    let importance = look.node_importance.get(active).copied().unwrap_or(0.0).max(0.8);
    let pick = LabelPick { node: active as u32, text: label_text(graph, layout, active), importance };
    look.labels.retain(|p| p.node != active as u32);
    if look.label_count == 0 {
        look.labels.clear();
    }
    look.labels.insert(0, pick);
    look.label_count += 1;
}

fn label_text(graph: &Graph, layout: &LayoutGraph, i: usize) -> String {
    let title = graph.nodes.get(i).and_then(|n| n.label.as_deref()).or(layout.id.get(i).map(String::as_str)).unwrap_or("");
    if title.chars().count() > LABEL_MAX_CHARS {
        let mut s: String = title.chars().take(LABEL_MAX_CHARS - 1).collect();
        s.push('…');
        s
    } else {
        title.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vw_core::config::Config;
    use vw_core::vault::{Link, Node};
    use vw_render::{LookContext, derive_look};

    const T: f64 = 1_762_400_000_000.0;

    fn node(id: &str) -> Node {
        Node { id: id.into(), label: None, tag: None, tags: None, mtime: None, folder: None, ghost: false }
    }

    /// A - B - C, and D alone.
    fn graph() -> Graph {
        Graph {
            nodes: ["A", "B", "C", "D"].into_iter().map(node).collect(),
            links: vec![Link { source: "A".into(), target: "B".into() }, Link { source: "B".into(), target: "C".into() }],
            ..Graph::default()
        }
    }

    fn state(json: &str) -> Option<PresenceState> {
        Some(PresenceState::parse(json).unwrap())
    }

    #[test]
    fn highlights_the_note_and_its_neighbours() {
        let g = graph();
        let layout = LayoutGraph::from_graph(&g);
        let cfg = Config::default();
        let base = derive_look(&g, &layout, &cfg, &LookContext { now_ms: T, light: false });
        let mut look = base.clone();
        let b = layout.id.iter().position(|x| x == "B").unwrap();
        let a = layout.id.iter().position(|x| x == "A").unwrap();
        let d = layout.id.iter().position(|x| x == "D").unwrap();
        highlight(&mut look, &g, &layout, b, &[], false);
        assert_eq!(look.node_color[b][3], 1.0);
        assert!(look.node_radius[b] > base.node_radius[b]);
        assert!(look.node_color[b][0] > base.node_color[b][0] || base.node_color[b][0] >= 1.0, "brighter on dark");
        assert!(look.node_color[a][3] > base.node_color[a][3], "neighbour brighter");
        assert!(look.node_color[d][3] < base.node_color[d][3], "others dimmed");
        for (k, &(x, y)) in layout.links.iter().enumerate() {
            assert!(look.edge_color[k][3] >= base.edge_color[k][3], "links of B ({x}-{y}) not dimmed");
        }
        assert_eq!(look.labels[0].node, b as u32);
        assert_eq!(look.labels[0].text, "B");
        assert_eq!(look.label_count, base.label_count + 1);
    }

    #[test]
    fn extra_links_and_light_backgrounds() {
        let g = graph();
        let layout = LayoutGraph::from_graph(&g);
        let base = derive_look(&g, &layout, &Config::default(), &LookContext { now_ms: T, light: false });
        let mut look = base.clone();
        let a = layout.id.iter().position(|x| x == "A").unwrap();
        let d = layout.id.iter().position(|x| x == "D").unwrap();
        highlight(&mut look, &g, &layout, a, &[d], true);
        assert!(look.node_color[d][3] > base.node_color[d][3], "Obsidian's link counts as a neighbour");
        assert!(look.node_color[a][0] <= base.node_color[a][0], "darker ink on light");
    }

    #[test]
    fn labels_only_the_open_note_when_hub_labels_are_off() {
        let g = graph();
        let layout = LayoutGraph::from_graph(&g);
        let cfg = Config { hub_labels: false, ..Config::default() };
        let mut look = derive_look(&g, &layout, &cfg, &LookContext { now_ms: T, light: false });
        assert_eq!(look.label_count, 0);
        highlight(&mut look, &g, &layout, 0, &[], false);
        assert_eq!(look.label_count, 1);
        assert_eq!(look.labels.len(), 1);
    }

    #[test]
    fn changes_and_staleness() {
        let mut p = Presence::default();
        let ch = p.update(state(r#"{"version":1,"active":"B.md","updated":1762400000000}"#), T);
        assert_eq!(ch, Change { highlight: true, saved: None });
        assert_eq!(p.active(T), Some("B.md"));
        // A heartbeat: same note, newer timestamp. No redraw.
        let ch = p.update(state(r#"{"version":1,"active":"B.md","updated":1762400120000}"#), T + 120_000.0);
        assert_eq!(ch, Change::default());
        // A save of the open note.
        let ch = p.update(
            state(r#"{"version":1,"active":"B.md","saved":"B.md","savedAt":1762400121000,"updated":1762400121000}"#),
            T + 121_000.0,
        );
        assert_eq!(ch, Change { highlight: false, saved: Some("B.md".into()) });
        // The same save again is not a new one.
        let ch = p.update(
            state(r#"{"version":1,"active":"A.md","saved":"B.md","savedAt":1762400121000,"updated":1762400122000}"#),
            T + 122_000.0,
        );
        assert_eq!(ch, Change { highlight: true, saved: None });
        // Five minutes of silence: stale, highlight gone.
        let later = T + 122_000.0 + 300_000.0;
        assert_eq!(p.active(later), None);
        assert!(!p.live(later));
        assert!(p.expire(later));
        assert!(!p.expire(later));
        assert!(p.wake_at(later).is_none());
    }

    #[test]
    fn a_stale_file_at_startup_changes_nothing() {
        let mut p = Presence::default();
        let ch = p.update(
            state(r#"{"version":1,"active":"B.md","saved":"B.md","savedAt":1762400000000,"updated":1762400000000}"#),
            T + 3_600_000.0,
        );
        assert_eq!(ch, Change::default());
        assert_eq!(p.active(T + 3_600_000.0), None);
        // An old save in a fresh file isn't animated either.
        let mut p = Presence::default();
        let ch = p.update(
            state(r#"{"version":1,"active":"B.md","saved":"B.md","savedAt":1762399000000,"updated":1762400000000}"#),
            T,
        );
        assert_eq!(ch, Change { highlight: true, saved: None });
        // Plugin removed: no state at all.
        assert_eq!(p.update(None, T), Change { highlight: true, saved: None });
        assert_eq!(p.active(T), None);
    }

    #[test]
    fn apply_does_nothing_without_a_match() {
        let g = graph();
        let layout = LayoutGraph::from_graph(&g);
        let base = derive_look(&g, &layout, &Config::default(), &LookContext { now_ms: T, light: false });
        let style = vw_render::style_from_config(&Config::default(), false);
        let mut p = Presence::default();
        p.update(state(r#"{"version":1,"active":"Elsewhere/Gone.md","updated":1762400000000}"#), T);
        let mut look = base.clone();
        p.apply(&mut look, &g, &layout, &style, T);
        assert_eq!(look, base);
        p.update(state(r#"{"version":1,"active":"C.md","updated":1762400000000}"#), T);
        p.apply(&mut look, &g, &layout, &style, T);
        assert_ne!(look, base);
    }
}
