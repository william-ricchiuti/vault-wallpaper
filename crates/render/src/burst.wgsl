// Edit bursts: sprite images baked once into a small atlas (see burst.rs), which
// Core Animation then moves and fades. Positions are in atlas points; each sprite is
// centred in its own cell, and `progress` picks the frame for animated kinds.
//
// Output is premultiplied, as Core Animation expects.

struct BurstUniforms {
  origin: vec2<f32>,    // region top-left, screen points
  size: vec2<f32>,      // region size, points
  scale: f32,           // pixels per point
  _p0: f32,
  _p1: f32,
  _p2: f32,
};
@group(0) @binding(0) var<uniform> u: BurstUniforms;

struct Burst {
  @location(0) center: vec2<f32>,
  @location(1) core: f32,
  @location(2) reach: f32,
  @location(3) color: vec4<f32>,
  @location(4) progress: f32,
};

struct BurstOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) local: vec2<f32>,
  @location(1) @interpolate(flat) core: f32,
  @location(2) @interpolate(flat) reach: f32,
  @location(3) @interpolate(flat) color: vec4<f32>,
  @location(4) @interpolate(flat) progress: f32,
  @location(5) @interpolate(flat) seed: f32,
  @location(6) paper: vec2<f32>,
};

// Same hash as the scene shader, so the bleed's wobble lines up with the dot's.
fn hash2(p: vec2<f32>) -> f32 {
  return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn vnoise(p: vec2<f32>) -> f32 {
  let i = floor(p);
  let f = p - i;
  let w = f * f * (3.0 - 2.0 * f);
  let a = mix(hash2(i), hash2(i + vec2<f32>(1.0, 0.0)), w.x);
  let b = mix(hash2(i + vec2<f32>(0.0, 1.0)), hash2(i + vec2<f32>(1.0, 1.0)), w.x);
  return mix(a, b, w.y);
}

// How far past `reach` the ragged front can poke (keep in step with burst.rs).
const FRONT_SLACK: f32 = 1.2;

@vertex fn burst_vs(@builtin(vertex_index) v: u32, b: Burst) -> BurstOut {
  let corners = array<vec2<f32>, 6>(vec2(-1., -1.), vec2(1., -1.), vec2(-1., 1.), vec2(-1., 1.), vec2(1., -1.), vec2(1., 1.));
  let r = b.reach * FRONT_SLACK + 2.0 / u.scale;
  let off = corners[v] * r;
  let p = b.center + off - u.origin;
  var o: BurstOut;
  o.pos = vec4<f32>(p.x / u.size.x * 2.0 - 1.0, 1.0 - p.y / u.size.y * 2.0, 0.0, 1.0);
  o.local = off;
  o.core = b.core;
  o.reach = b.reach;
  o.color = b.color;
  o.progress = b.progress;
  o.seed = hash2(b.center) * 6.2831853;
  o.paper = b.center + off;
  return o;
}

// ---- ink bleed (Ink & Paper): fresh ink on the dot, and a wet front that spreads
// out through the paper fibres, slows, and soaks in. At progress 1 it is fully
// transparent, so the still scene underneath is the settled state. ----
@fragment fn ink_bleed_fs(in: BurstOut) -> @location(0) vec4<f32> {
  let p = clamp(in.progress, 0.0, 1.0);
  let d = length(in.local);
  let a = atan2(in.local.y, in.local.x);
  let s = in.seed;
  // The dot's own rim (as ink_fs draws it).
  let wob = sin(a * 2.0 + s) * 0.5 + sin(a * 3.0 + s * 1.7) * 0.3 + sin(a * 7.0 + s * 2.9) * 0.2;
  let core = in.core * (1.0 + wob * 0.07);
  let on_dot = clamp((core - d) * u.scale + 0.5, 0.0, 1.0);
  // The wet front: fast at first, then settling (ease-out), ragged where fibres wick.
  let spread = 1.0 - pow(1.0 - p, 3.0);
  let lump = 1.0 + 0.10 * (sin(a * 5.0 + s * 3.1) * 0.6 + sin(a * 11.0 + s * 0.7) * 0.4);
  let wick = 0.92 + 0.16 * vnoise(in.paper * 0.45);
  let front = mix(core, in.reach, spread) * lump * wick;
  let inside = clamp((front - d) * u.scale * 0.5 + 0.5, 0.0, 1.0);
  // A light wash behind the front, stronger near the dot, and the darker tide line
  // wet ink leaves at its edge.
  let wash = 0.22 * (1.0 - 0.65 * smoothstep(core, max(front, core + 1e-3), d));
  let w = max(1.1, 0.07 * in.reach);
  let tide = exp(-pow((d - front) / w, 2.0)) * 0.30;
  let fibre = 0.72 + 0.56 * hash2(floor(in.paper * 1.5));
  // Wet ink dries lighter: everything fades out as the bleed settles.
  let wet = pow(1.0 - p, 1.6);
  let fresh = 0.38 * on_dot;
  let alpha = clamp(in.color.a * wet * (fresh + (1.0 - on_dot) * (inside * wash + tide) * fibre), 0.0, 1.0);
  return vec4<f32>(in.color.rgb * alpha, alpha);
}

// ---- Pulse: light travelling out along links from a saved note. ----

// A ring of light opening out from the saved note, with a brief flash on the dot.
// Gone at progress 1.
@fragment fn pulse_ring_fs(in: BurstOut) -> @location(0) vec4<f32> {
  let p = clamp(in.progress, 0.0, 1.0);
  let d = length(in.local);
  let r = mix(in.core, in.reach, 1.0 - pow(1.0 - p, 3.0));
  let w = mix(1.2, 3.5, p);
  let fade = pow(1.0 - p, 1.5);
  let ring = exp(-pow((d - r) / w, 2.0)) * fade * 0.85;
  let flash = exp(-pow(d / (in.core * 1.6), 2.0)) * pow(1.0 - p, 3.0);
  let alpha = clamp(in.color.a * (ring + flash), 0.0, 1.0);
  let rgb = mix(in.color.rgb, vec3<f32>(1.0), flash * 0.6);
  return vec4<f32>(rgb * alpha, alpha);
}

// A travelling pulse: a bright head at the centre and a tail trailing towards -x,
// thinning and fading. Core Animation turns it along the link and moves it.
@fragment fn pulse_comet_fs(in: BurstOut) -> @location(0) vec4<f32> {
  let x = in.local.x;
  let y = in.local.y;
  let d = length(in.local);
  let head = exp(-pow(d / in.core, 2.0));
  let halo = exp(-pow(d / (in.core * 2.2), 2.0)) * 0.30;
  let t = clamp(-x / in.reach, 0.0, 1.0);
  let behind = select(0.0, 1.0, x < 0.0);
  let width = in.core * mix(0.75, 0.2, t);
  let tail = behind * pow(1.0 - t, 2.0) * exp(-pow(y / width, 2.0)) * 0.75;
  let alpha = clamp(in.color.a * max(head + halo, tail), 0.0, 1.0);
  let rgb = mix(in.color.rgb, vec3<f32>(1.0), head * 0.55);
  return vec4<f32>(rgb * alpha, alpha);
}

// A soft glint on a note a pulse reaches: a bright point in a small soft halo.
@fragment fn pulse_glint_fs(in: BurstOut) -> @location(0) vec4<f32> {
  let d = length(in.local);
  let point = exp(-pow(d / in.core, 2.0));
  let halo = exp(-pow(d / (in.reach * 0.42), 2.0)) * 0.32;
  let alpha = clamp(in.color.a * (point + halo), 0.0, 1.0);
  let rgb = mix(in.color.rgb, vec3<f32>(1.0), point * 0.5);
  return vec4<f32>(rgb * alpha, alpha);
}
