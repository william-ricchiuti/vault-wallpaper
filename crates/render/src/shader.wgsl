// Graph renderer: background, links as anti-aliased quads, notes as anti-aliased
// discs. Positions arrive in logical points; `u.scale` converts to pixels so edges
// stay one pixel soft on any display.

struct Uniforms {
  size: vec2<f32>,      // viewport in points
  scale: f32,           // pixels per point
  bg_mode: f32,         // 0 = gradient/flat, 1 = starfield, 2 = paper
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
fn hash2(p: vec2<f32>) -> f32 {
  return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// Smooth value noise in 0..1 (bilinear between hashed lattice points).
fn vnoise(p: vec2<f32>) -> f32 {
  let i = floor(p);
  let f = p - i;
  let w = f * f * (3.0 - 2.0 * f);
  let a = mix(hash2(i), hash2(i + vec2<f32>(1.0, 0.0)), w.x);
  let b = mix(hash2(i + vec2<f32>(0.0, 1.0)), hash2(i + vec2<f32>(1.0, 1.0)), w.x);
  return mix(a, b, w.y);
}

// A sparse field of faint background stars, fixed in screen space (drawn once with
// the scene, so it costs nothing afterwards).
fn starfield(pt: vec2<f32>) -> f32 {
  var acc = 0.0;
  for (var layer = 0; layer < 2; layer++) {
    let cell_size = select(9.0, 23.0, layer == 1);
    let cell = floor(pt / cell_size);
    let h = hash2(cell + f32(layer) * 17.0);
    if (h > 0.9) {
      let off = vec2<f32>(hash2(cell + 3.1), hash2(cell + 7.7)) * cell_size;
      let d = length(pt - (cell * cell_size + off)) * u.scale;
      let r = 0.6 + hash2(cell + 1.3) * select(0.6, 1.4, layer == 1);
      acc += smoothstep(r + 0.8, r - 0.2, d) * (0.15 + 0.55 * hash2(cell + 9.2)) * (h - 0.9) * 10.0;
    }
  }
  return acc;
}

@fragment fn bg_fs(in: BgOut) -> @location(0) vec4<f32> {
  var base = u.background.rgb;
  if (u.bg_mode > 1.5) {
    // Paper: low-frequency mottling plus fine fibre noise.
    let pt = in.uv * u.size;
    let mottle = vnoise(pt / 90.0) * 0.6 + vnoise(pt / 23.0 + 5.3) * 0.4;
    let fibre = hash2(floor(pt * u.scale));
    base = base * (0.982 + mottle * 0.026 + fibre * 0.01);
  }
  if (u.background_glow.a <= 0.0) {
    if (u.bg_mode > 0.5 && u.bg_mode < 1.5) {
      base += vec3<f32>(starfield(in.uv * u.size));
    }
    return vec4<f32>(base, 1.0);
  }
  let px = (in.uv - 0.5) * u.size;
  let t = clamp(length(px) / (max(u.size.x, u.size.y) * 0.72), 0.0, 1.0);
  let rim = max(u.background.rgb - vec3<f32>(5.0 / 255.0), vec3<f32>(0.0));
  var c = select(mix(u.background.rgb, rim, (t - 0.5) * 2.0), mix(u.background_glow.rgb, u.background.rgb, t * 2.0), t < 0.5);
  if (u.bg_mode > 0.5 && u.bg_mode < 1.5) {
    c += vec3<f32>(starfield(in.uv * u.size));
  }
  c *= base / max(u.background.rgb, vec3<f32>(1e-4));
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

// ---- stars: a bright core with four diffraction spikes ----
@fragment fn star_fs(in: NodeOut) -> @location(0) vec4<f32> {
  let p = in.local / max(in.radius, 1e-3);
  let d = length(p);
  let core = exp(-d * d * 60.0);
  let spikes = (exp(-abs(p.y) * 70.0) + exp(-abs(p.x) * 70.0)) * (1.0 - smoothstep(0.0, 1.0, d));
  let a = clamp((core + spikes * 0.6) * in.color.a, 0.0, 1.0);
  return vec4<f32>(in.color.rgb, a);
}

// ---- aurora curtains (Aurora Lite): drawn into the low-resolution glow layer ----
// A curtain is a wavy, folded base line with vertical rays rising from it, bright at
// the base and fading upward, coloured from `color` (base) to `top`. Drawn once per
// scene change; the blur and the window server's breathing do the rest.
struct Curtain {
  @location(0) base: vec2<f32>,   // centre of the base line, in points
  @location(1) size: vec2<f32>,   // half width, height (points)
  @location(2) color: vec4<f32>,  // base colour, strength
  @location(3) top: vec4<f32>,    // colour at the top, seed
};
struct CurtainOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) local: vec2<f32>,  // offset from the base centre, in points (y down)
  @location(1) size: vec2<f32>,
  @location(2) color: vec4<f32>,
  @location(3) top: vec4<f32>,
};

fn hash1(x: f32) -> f32 {
  return fract(sin(x * 127.1) * 43758.5453);
}

fn vnoise(x: f32) -> f32 {
  let i = floor(x);
  let f = fract(x);
  return mix(hash1(i), hash1(i + 1.0), f * f * (3.0 - 2.0 * f));
}

@vertex fn curtain_vs(@builtin(vertex_index) v: u32, c: Curtain) -> CurtainOut {
  let corners = array<vec2<f32>, 6>(vec2(-1., 0.), vec2(1., 0.), vec2(-1., 1.), vec2(-1., 1.), vec2(1., 0.), vec2(1., 1.));
  let k = corners[v];
  let fold = c.size.y * 0.2;
  // From above the tallest ray down to below the lowest fold plus the soft lower edge.
  let y0 = -(c.size.y * 1.05 + fold);
  let y1 = fold + 10.0 + c.size.y * 0.06;
  let off = vec2<f32>(k.x * c.size.x, mix(y0, y1, k.y));
  var o: CurtainOut;
  o.pos = to_clip(c.base + off);
  o.local = off;
  o.size = c.size;
  o.color = c.color;
  o.top = c.top;
  return o;
}

@fragment fn curtain_fs(in: CurtainOut) -> @location(0) vec4<f32> {
  let hw = max(in.size.x, 1.0);
  let height = max(in.size.y, 1.0);
  let seed = in.top.a;
  let x = in.local.x;
  let s = x / hw;
  // Ends taper in strength and height.
  let env = smoothstep(1.0, 0.35, abs(s));
  // Folds: the base line wanders up and down across the curtain.
  let fold = (sin(s * 3.4 + seed) * 0.55 + sin(s * 7.3 + seed * 2.3) * 0.3 + sin(s * 13.0 + seed * 0.7) * 0.15) * height * 0.2;
  let h = fold - in.local.y;   // height above the base line
  // Rays: vertical striations of varying brightness and height.
  let r = vnoise(x / 11.0 + seed * 7.0) * 0.5 + vnoise(x / 29.0 + seed * 3.0) * 0.5;
  let rays = 0.15 + 0.85 * r * r * 1.6;
  let reach = height * (0.45 + 0.55 * vnoise(x / 70.0 + seed * 5.0)) * (0.45 + 0.55 * env);
  var profile: f32;
  if (h < 0.0) {
    let soft = 5.0 + height * 0.035;
    profile = exp(-(h * h) / (soft * soft));
  } else {
    let hn = h / reach;
    profile = (exp(-hn * 1.8) + exp(-h / (height * 0.035)) * 1.1) * (1.0 - smoothstep(0.55, 1.0, hn));
  }
  let rgb = mix(in.color.rgb, in.top.rgb, smoothstep(0.08, 0.65, max(h, 0.0) / height));
  let a = clamp(in.color.a * env * rays * profile, 0.0, 1.0);
  return vec4<f32>(rgb * a, a);
}

// ---- ink dots: a solid dot with a slightly ragged edge and a soft, fibrous bleed
// into the paper. `radius` is the bleed's reach; the dot is INK_CORE of it. ----
const INK_CORE: f32 = 0.42;

struct InkOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) local: vec2<f32>,
  @location(1) radius: f32,
  @location(2) color: vec4<f32>,
  @location(3) @interpolate(flat) seed: f32,
};

@vertex fn ink_vs(@builtin(vertex_index) v: u32, n: Node) -> InkOut {
  let corners = array<vec2<f32>, 6>(vec2(-1., -1.), vec2(1., -1.), vec2(-1., 1.), vec2(-1., 1.), vec2(1., -1.), vec2(1., 1.));
  let reach = n.radius + 1.0 / u.scale;
  let off = corners[v] * reach;
  var o: InkOut;
  o.pos = to_clip(n.center + off);
  o.local = off;
  o.radius = n.radius;
  o.color = n.color;
  o.seed = hash2(n.center) * 6.2831853;
  return o;
}

@fragment fn ink_fs(in: InkOut) -> @location(0) vec4<f32> {
  let d = length(in.local);
  let a = atan2(in.local.y, in.local.x);
  let s = in.seed;
  // Low-order wobble around the rim: ink never prints a perfect circle.
  let wob = sin(a * 2.0 + s) * 0.5 + sin(a * 3.0 + s * 1.7) * 0.3 + sin(a * 7.0 + s * 2.9) * 0.2;
  let core = in.radius * INK_CORE * (1.0 + wob * 0.07);
  let cover = clamp((core - d) * u.scale + 0.5, 0.0, 1.0);
  // Bleed: falls off from the dot's edge, lumpier than the dot, broken up by fibres.
  let t = clamp((d - core) / max(in.radius - core, 1e-3), 0.0, 1.0);
  let lump = 1.0 + 0.45 * (sin(a * 5.0 + s * 3.1) * 0.6 + sin(a * 11.0 + s * 0.7) * 0.4);
  let fibre = 0.82 + 0.36 * hash2(floor(in.pos.xy / max(u.scale, 1.0)));
  let bleed = pow(1.0 - t, 2.6) * 0.26 * lump * fibre;
  // Ink pools a little toward the rim of the dot, so its centre reads a touch lighter.
  let pool = mix(0.88, 1.0, smoothstep(0.0, core, d));
  let alpha = cover * pool + (1.0 - cover) * bleed;
  return vec4<f32>(in.color.rgb, clamp(in.color.a * alpha, 0.0, 1.0));
}

// ---- ink links: a pen stroke, thin at the ends and swelling to full width a little
// off-centre (the swell point varies per link) ----
struct InkEdgeOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) across: f32,
  @location(1) half: f32,
  @location(2) color: vec4<f32>,
  @location(3) along: f32,
  @location(4) @interpolate(flat) peak: f32,
};

@vertex fn ink_edge_vs(@builtin(vertex_index) v: u32, e: Edge) -> InkEdgeOut {
  let corners = array<vec2<f32>, 6>(vec2(0., -1.), vec2(1., -1.), vec2(0., 1.), vec2(0., 1.), vec2(1., -1.), vec2(1., 1.));
  let c = corners[v];
  var dir = e.b - e.a;
  let len = length(dir);
  dir = select(vec2<f32>(1.0, 0.0), dir / len, len > 1e-4);
  let n = vec2<f32>(-dir.y, dir.x);
  let half = e.width * 0.5;
  let reach = half + 1.0 / u.scale;
  var o: InkEdgeOut;
  o.pos = to_clip(mix(e.a, e.b, c.x) + n * c.y * reach);
  o.across = c.y * reach;
  o.half = half;
  o.color = e.color;
  o.along = c.x;
  o.peak = 0.38 + 0.24 * hash2(e.a * 0.37 + e.b * 0.61);
  return o;
}

@fragment fn ink_edge_fs(in: InkEdgeOut) -> @location(0) vec4<f32> {
  let span = select(1.0 - in.peak, in.peak, in.along < in.peak);
  let k = (in.along - in.peak) / span;
  let half = in.half * (0.35 + 0.65 * (1.0 - k * k));
  let cover = clamp((half - abs(in.across)) * u.scale + 0.5, 0.0, 1.0);
  let thin = min(1.0, half * 2.0 * u.scale);
  return vec4<f32>(in.color.rgb, in.color.a * cover * thin);
}
