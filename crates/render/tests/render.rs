//! Off-screen render of a tiny graph: background, a link and two notes land where and
//! in the colour they should.

use vw_core::layout::LayoutGraph;
use vw_render::{Framing, Gpu, Look, SceneStyle, build_scene, render_rgba, tag_color};

fn px(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * w + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

fn close(a: [u8; 3], b: [f32; 3], tol: i32) -> bool {
    a.iter().zip(b).all(|(&a, b)| (i32::from(a) - (b * 255.0).round() as i32).abs() <= tol)
}

#[test]
fn draws_notes_links_and_background() {
    let Some(gpu) = Gpu::headless() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let graph = LayoutGraph {
        id: vec!["a".into(), "b".into()],
        tag: vec![Some("work".into()), None],
        ghost: vec![false, false],
        mtime: vec![0.0, 0.0],
        links: vec![(0, 1)],
        degree: vec![1, 1],
        radius: vec![4.0, 4.0],
    };
    let positions = [[25.0, 30.0], [75.0, 30.0]];
    let style = SceneStyle { background_glow: [0.0; 4], glow: 0.0, vignette: 0.0, grain: 0.0, halos: false, depth_of_field: false, ..SceneStyle::default() };
    let [tr, tg, tb] = tag_color("work");
    let [ar, ag, ab] = style.accent;
    let look = Look {
        node_color: vec![[tr, tg, tb, 0.95], [ar, ag, ab, 0.95]],
        node_radius: vec![6.0, 6.0],
        // Importance 1 makes links full width (1 pt) and full alpha.
        node_importance: vec![1.0, 1.0],
        edge_color: vec![[ar, ag, ab, 1.0]],
        ..Look::default()
    };
    let (w, h) = (200, 120);
    let data = build_scene(&graph, &look, &positions, &Framing::IDENTITY, &style, [w as f32 / 2.0, h as f32 / 2.0]);
    let rgba = render_rgba(&gpu, [w, h], 2.0, &style, &data, 1.0);
    assert!(close(px(&rgba, w, 5, 5), style.background, 2), "background {:?}", px(&rgba, w, 5, 5));
    assert!(close(px(&rgba, w, 50, 60), tag_color("work"), 16), "tagged note {:?}", px(&rgba, w, 50, 60));
    assert!(close(px(&rgba, w, 150, 60), style.accent, 16), "untagged note {:?}", px(&rgba, w, 150, 60));
    assert!(close(px(&rgba, w, 100, 60), style.accent, 6), "link {:?}", px(&rgba, w, 100, 60));
    assert!(close(px(&rgba, w, 100, 90), style.background, 2), "empty space {:?}", px(&rgba, w, 100, 90));
}
