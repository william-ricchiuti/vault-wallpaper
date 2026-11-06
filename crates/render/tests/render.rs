//! Off-screen render of a tiny graph: background, a link and two notes land where and
//! in the colour they should.

use vw_core::layout::LayoutGraph;
use vw_render::{Framing, Gpu, SceneStyle, build_instances, render_rgba, tag_color};

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
    let positions = [[50.0, 60.0], [150.0, 60.0]];
    let style = SceneStyle { background_glow: [0.0; 4], link_opacity: 1.0, link_width: 2.0, ..SceneStyle::default() };
    let (nodes, edges) = build_instances(&graph, &positions, &Framing::IDENTITY, &style);
    let (w, h) = (200, 120);
    let rgba = render_rgba(&gpu, [w, h], 1.0, &style, &nodes, &edges);
    assert!(close(px(&rgba, w, 5, 5), style.background, 2), "background {:?}", px(&rgba, w, 5, 5));
    assert!(close(px(&rgba, w, 50, 60), tag_color("work"), 16), "tagged note {:?}", px(&rgba, w, 50, 60));
    assert!(close(px(&rgba, w, 150, 60), style.accent, 16), "untagged note {:?}", px(&rgba, w, 150, 60));
    assert!(close(px(&rgba, w, 100, 60), style.accent, 6), "link {:?}", px(&rgba, w, 100, 60));
    assert!(close(px(&rgba, w, 100, 90), style.background, 2), "empty space {:?}", px(&rgba, w, 100, 90));
}
