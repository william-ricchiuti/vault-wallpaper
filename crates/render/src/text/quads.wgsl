// Plates (Badge) and glow sprites (Glow) drawn under the text. Positions are in
// target pixels; colours are straight sRGB with alpha, blended like the web canvas.

struct Params {
    screen: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var glow_tex: texture_2d<f32>;
@group(0) @binding(2) var glow_sampler: sampler;

struct Quad {
    @location(0) rect: vec4<f32>,   // x, y, w, h in pixels
    @location(1) uv: vec4<f32>,     // u0, v0, u1, v1 (glow only)
    @location(2) fill: vec4<f32>,
    @location(3) stroke: vec4<f32>,
    @location(4) shape: vec4<f32>,  // radius px, stroke width px, kind (0 plate, 1 glow), unused
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) fill: vec4<f32>,
    @location(4) stroke: vec4<f32>,
    @location(5) shape: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, q: Quad) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let c = corners[vi];
    let px = q.rect.xy + c * q.rect.zw;
    var out: VsOut;
    out.pos = vec4(px.x / params.screen.x * 2.0 - 1.0, 1.0 - px.y / params.screen.y * 2.0, 0.0, 1.0);
    out.half_size = q.rect.zw * 0.5;
    out.local = (c - 0.5) * q.rect.zw;
    out.uv = mix(q.uv.xy, q.uv.zw, c);
    out.fill = q.fill;
    out.stroke = q.stroke;
    out.shape = q.shape;
    return out;
}

fn rounded_box(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half_size + vec2(r);
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs(v: VsOut) -> @location(0) vec4<f32> {
    if v.shape.z > 0.5 {
        let a = textureSample(glow_tex, glow_sampler, v.uv).r * v.fill.a;
        return vec4(v.fill.rgb, a);
    }
    // The plate rect carries a 1px margin so the anti-aliased edge and the outer
    // half of the stroke fit; the path itself is half_size - 1.
    let d = rounded_box(v.local, v.half_size - vec2(1.0), v.shape.x);
    let fill_cov = clamp(0.5 - d, 0.0, 1.0) * v.fill.a;
    let sw = v.shape.y;
    let stroke_cov = clamp(sw * 0.5 + 0.5 - abs(d), 0.0, 1.0) * v.stroke.a;
    // Stroke over fill, straight alpha.
    let a = stroke_cov + fill_cov * (1.0 - stroke_cov);
    if a <= 0.0 {
        discard;
    }
    let rgb = (v.stroke.rgb * stroke_cov + v.fill.rgb * fill_cov * (1.0 - stroke_cov)) / a;
    return vec4(rgb, a);
}
