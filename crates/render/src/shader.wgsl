// Graph renderer: background, links as anti-aliased quads, notes as anti-aliased
// discs. Positions arrive in logical points; `u.scale` converts to pixels so edges
// stay one pixel soft on any display.

struct Uniforms {
  size: vec2<f32>,      // viewport in points
  scale: f32,           // pixels per point
  _pad: f32,
  background: vec4<f32>,
  background_glow: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

fn to_clip(p: vec2<f32>) -> vec4<f32> {
  return vec4<f32>(p.x / u.size.x * 2.0 - 1.0, 1.0 - p.y / u.size.y * 2.0, 0.0, 1.0);
}

// ---- background: flat colour with a soft lift toward the centre ----
struct BgOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex fn bg_vs(@builtin(vertex_index) i: u32) -> BgOut {
  let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
  var o: BgOut;
  o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
  o.uv = p;
  return o;
}

// The web version's radial gradient: accent-tinted centre, the background colour
// halfway out, slightly darker at the rim (radius 0.72 of the larger side).
@fragment fn bg_fs(in: BgOut) -> @location(0) vec4<f32> {
  if (u.background_glow.a <= 0.0) {
    return vec4<f32>(u.background.rgb, 1.0);
  }
  let px = (in.uv - 0.5) * u.size;
  let t = clamp(length(px) / (max(u.size.x, u.size.y) * 0.72), 0.0, 1.0);
  let rim = max(u.background.rgb - vec3<f32>(5.0 / 255.0), vec3<f32>(0.0));
  let c = select(mix(u.background.rgb, rim, (t - 0.5) * 2.0), mix(u.background_glow.rgb, u.background.rgb, t * 2.0), t < 0.5);
  return vec4<f32>(c, 1.0);
}

// ---- links ----
struct Edge {
  @location(0) a: vec2<f32>,
  @location(1) b: vec2<f32>,
  @location(2) color: vec4<f32>,
  @location(3) width: f32,
};
struct EdgeOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) across: f32,   // signed distance from the centre line, in points
  @location(1) half: f32,     // half the stroke width, in points
  @location(2) color: vec4<f32>,
};

@vertex fn edge_vs(@builtin(vertex_index) v: u32, e: Edge) -> EdgeOut {
  let corners = array<vec2<f32>, 6>(vec2(0., -1.), vec2(1., -1.), vec2(0., 1.), vec2(0., 1.), vec2(1., -1.), vec2(1., 1.));
  let c = corners[v];
  var dir = e.b - e.a;
  let len = length(dir);
  dir = select(vec2<f32>(1.0, 0.0), dir / len, len > 1e-4);
  let n = vec2<f32>(-dir.y, dir.x);
  let half = e.width * 0.5;
  let reach = half + 1.0 / u.scale;   // one pixel of feather
  var o: EdgeOut;
  o.pos = to_clip(mix(e.a, e.b, c.x) + n * c.y * reach);
  o.across = c.y * reach;
  o.half = half;
  o.color = e.color;
  return o;
}

@fragment fn edge_fs(in: EdgeOut) -> @location(0) vec4<f32> {
  // Thin lines fade instead of thinning below a pixel, like a canvas stroke.
  let cover = clamp((in.half - abs(in.across)) * u.scale + 0.5, 0.0, 1.0);
  let thin = min(1.0, in.half * 2.0 * u.scale);
  return vec4<f32>(in.color.rgb, in.color.a * cover * thin);
}

// ---- notes ----
struct Node {
  @location(0) center: vec2<f32>,
  @location(1) radius: f32,
  @location(2) color: vec4<f32>,
};
struct NodeOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) local: vec2<f32>,  // offset from the centre, in points
  @location(1) radius: f32,
  @location(2) color: vec4<f32>,
};

@vertex fn node_vs(@builtin(vertex_index) v: u32, n: Node) -> NodeOut {
  let corners = array<vec2<f32>, 6>(vec2(-1., -1.), vec2(1., -1.), vec2(-1., 1.), vec2(-1., 1.), vec2(1., -1.), vec2(1., 1.));
  let reach = n.radius + 1.0 / u.scale;
  let off = corners[v] * reach;
  var o: NodeOut;
  o.pos = to_clip(n.center + off);
  o.local = off;
  o.radius = n.radius;
  o.color = n.color;
  return o;
}

@fragment fn node_fs(in: NodeOut) -> @location(0) vec4<f32> {
  let cover = clamp((in.radius - length(in.local)) * u.scale + 0.5, 0.0, 1.0);
  return vec4<f32>(in.color.rgb, in.color.a * cover);
}

// ---- soft discs: cluster halos (alpha-blended into the scene) and node glow
// (added into the low-resolution glow layer) ----
fn soft(in: NodeOut) -> f32 {
  let t = length(in.local) / max(in.radius, 1e-3);
  return in.color.a * exp(-t * t * 3.0) * (1.0 - smoothstep(0.85, 1.0, t));
}

@fragment fn halo_fs(in: NodeOut) -> @location(0) vec4<f32> {
  return vec4<f32>(in.color.rgb, soft(in));
}

@fragment fn glow_fs(in: NodeOut) -> @location(0) vec4<f32> {
  let a = soft(in);
  return vec4<f32>(in.color.rgb * a, a);
}
