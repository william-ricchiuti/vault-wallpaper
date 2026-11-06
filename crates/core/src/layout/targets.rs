//! Target slots for the non-force layouts (rings, timeline, galaxy): a port of
//! `computeTargets` in the web version's `layout-core.js`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::f64::consts::PI;

use super::{LayoutGraph, LayoutKind, LayoutOptions};

/// FNV-1a over UTF-16 code units mapped to [0, 1), as `hash01` in the web version.
pub(crate) fn hash01(s: &str) -> f64 {
    let mut h: u32 = 2_166_136_261;
    for u in s.encode_utf16() {
        h = (h ^ u32::from(u)).wrapping_mul(16_777_619);
    }
    f64::from(h % 100_000) / 100_000.0
}

/// Tags ordered by note count (most first, ties by name); untagged notes share the ""
/// bucket, always last. Ghosts are not counted.
fn tag_order(graph: &LayoutGraph) -> Vec<(&str, usize)> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut first_seen: Vec<&str> = Vec::new();
    for i in 0..graph.len() {
        if graph.ghost[i] {
            continue;
        }
        let t = graph.tag[i].as_deref().unwrap_or("");
        let c = counts.entry(t).or_insert(0);
        if *c == 0 {
            first_seen.push(t);
        }
        *c += 1;
    }
    let mut ordered: Vec<(&str, usize)> = first_seen.into_iter().map(|t| (t, counts[t])).collect();
    ordered.sort_by(|a, b| match (a.0.is_empty(), b.0.is_empty()) {
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => b.1.cmp(&a.1).then_with(|| a.0.encode_utf16().cmp(b.0.encode_utf16())),
    });
    ordered
}

/// Target positions around (0, 0), or `None` for the force layout.
pub(crate) fn compute_targets(kind: LayoutKind, graph: &LayoutGraph, opts: &LayoutOptions) -> Option<(Vec<f64>, Vec<f64>)> {
    if kind == LayoutKind::Force {
        return None;
    }
    let n = graph.len();
    let r_big = 80f64.max(opts.width.min(opts.height) * 0.42);
    let ordered = tag_order(graph);
    let rank: HashMap<&str, usize> = ordered.iter().enumerate().map(|(i, (t, _))| (*t, i)).collect();
    let mut tx = vec![0.0; n];
    let mut ty = vec![0.0; n];
    let tag_of = |i: usize| graph.tag[i].as_deref().unwrap_or("");

    if kind == LayoutKind::Timeline {
        let lanes = ordered.len().clamp(1, 12);
        let w = r_big * 2.4;
        let max_days = 3.0 * 365.0;
        for i in 0..n {
            let h = hash01(&graph.id[i]);
            if graph.ghost[i] {
                tx[i] = (h * 2.0 - 1.0) * w * 0.5;
                ty[i] = r_big * 0.98;
                continue;
            }
            let days = if graph.mtime[i] > 0.0 { ((opts.now_ms - graph.mtime[i]) / 86_400_000.0).max(0.0) } else { max_days };
            let u = 1.0 - (days.ln_1p() / max_days.ln_1p()).min(1.0);
            let lane = rank.get(tag_of(i)).copied().unwrap_or(0).min(lanes - 1);
            let ly = if lanes == 1 { 0.0 } else { lane as f64 / (lanes - 1) as f64 * 2.0 - 1.0 };
            tx[i] = -w * 0.5 + u * w;
            ty[i] = ly * r_big * 0.82 + (h - 0.5) * r_big * 0.1;
        }
        return Some((tx, ty));
    }

    // Notes grouped per tag, highest degree first (stable, like JavaScript's sort).
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for i in 0..n {
        if !graph.ghost[i] {
            groups.entry(tag_of(i)).or_default().push(i);
        }
    }
    for list in groups.values_mut() {
        list.sort_by(|&a, &b| graph.degree[b].cmp(&graph.degree[a]));
    }
    let total = groups.values().map(Vec::len).sum::<usize>().max(1) as f64;

    match kind {
        LayoutKind::Rings => {
            let mut acc = -PI / 2.0;
            for (t, _) in &ordered {
                let Some(list) = groups.get(t) else { continue };
                let span = list.len() as f64 / total * PI * 2.0;
                let pad = span * 0.06;
                for (r, &idx) in list.iter().enumerate() {
                    let id = &graph.id[idx];
                    let h = hash01(id);
                    let u = r as f64 / list.len() as f64;
                    let rad = if r == 0 { 0.2 } else { 0.32 + 0.68 * u.sqrt() + (hash01(&format!("{id}#")) - 0.5) * 0.05 };
                    let a = if r == 0 { acc + span / 2.0 } else { acc + pad + (span - 2.0 * pad) * h };
                    tx[idx] = a.cos() * rad * r_big;
                    ty[idx] = a.sin() * rad * r_big;
                }
                acc += span;
            }
        }
        LayoutKind::Galaxy => {
            let arms = ordered.len().clamp(2, 8);
            for (t, _) in &ordered {
                let Some(list) = groups.get(t) else { continue };
                let arm = rank.get(t).copied().unwrap_or(0) % arms;
                for (r, &idx) in list.iter().enumerate() {
                    let u = (r + 1) as f64 / (list.len() + 1) as f64;
                    let rad = if r == 0 { 0.07 } else { 0.12 + 0.9 * u.powf(0.75) };
                    let th = arm as f64 / arms as f64 * PI * 2.0
                        + rad * 2.7
                        + (hash01(&graph.id[idx]) - 0.5) * 0.42 * (1.1 - rad);
                    tx[idx] = th.cos() * rad * r_big;
                    ty[idx] = th.sin() * rad * r_big * 0.78;
                }
            }
        }
        LayoutKind::Force | LayoutKind::Timeline => unreachable!(),
    }
    let squash = if kind == LayoutKind::Galaxy { 0.78 } else { 1.0 };
    for i in 0..n {
        if !graph.ghost[i] {
            continue;
        }
        let id = &graph.id[i];
        let a = hash01(id) * PI * 2.0;
        let rr = 1.02 + hash01(&format!("{id}~")) * 0.12;
        tx[i] = a.cos() * rr * r_big;
        ty[i] = a.sin() * rr * r_big * squash;
    }
    Some((tx, ty))
}

#[cfg(test)]
mod tests {
    use super::hash01;

    #[test]
    fn hash01_matches_javascript() {
        // Values from LayoutCore.hash01 in the web version.
        assert_eq!(hash01("Note"), 0.88125);
        assert_eq!(hash01("日本語"), 0.93518);
        assert_eq!(hash01(""), 0.36261);
    }
}
