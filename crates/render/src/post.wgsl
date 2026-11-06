// Post passes: blur the low-resolution glow layer, then composite scene + glow +
// vignette + grain onto the screen. The composite is the only pass that runs per
// animated frame (glow breathing); everything it reads is cached.

struct BgOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex fn full_vs(@builtin(vertex_index) i: u32) -> BgOut {
  let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
  var o: BgOut;
  o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
  o.uv = vec2<f32>(p.x, 1.0 - p.y);
  return o;
}

// ---- separable blur (9 taps folded into 5 bilinear reads) ----
struct Blur { dir: vec2<f32>, _pad: vec2<f32> };   // texel step, already divided by size
@group(0) @binding(0) var blur_src: texture_2d<f32>;
@group(0) @binding(1) var blur_smp: sampler;
@group(0) @binding(2) var<uniform> blur: Blur;

@fragment fn blur_fs(in: BgOut) -> @location(0) vec4<f32> {
  let o1 = blur.dir * 1.3846153846;
  let o2 = blur.dir * 3.2307692308;
  var c = textureSample(blur_src, blur_smp, in.uv) * 0.2270270270;
  c += textureSample(blur_src, blur_smp, in.uv + o1) * 0.3162162162;
  c += textureSample(blur_src, blur_smp, in.uv - o1) * 0.3162162162;
  c += textureSample(blur_src, blur_smp, in.uv + o2) * 0.0702702703;
  c += textureSample(blur_src, blur_smp, in.uv - o2) * 0.0702702703;
  return c;
}

// ---- composite ----
struct Comp {
  glow: f32,          // glow strength, breathing applied
  chroma: f32,        // chromatic split in glow texels (0 = off)
  vignette: f32,      // 0..1
  grain: f32,         // 0..1
  size: vec2<f32>,    // target size in pixels
  glow_texel: vec2<f32>,
};
@group(0) @binding(0) var scene_tex: texture_2d<f32>;
@group(0) @binding(1) var glow_tex: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;
@group(0) @binding(3) var<uniform> comp: Comp;

fn hash(p: vec2<f32>) -> f32 {
  return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

@fragment fn composite_fs(in: BgOut) -> @location(0) vec4<f32> {
  var c = textureSample(scene_tex, smp, in.uv).rgb;
  if (comp.glow > 0.0) {
    var g: vec3<f32>;
    if (comp.chroma > 0.0) {
      let off = (in.uv - 0.5) * comp.glow_texel * comp.chroma;
      g = vec3<f32>(
        textureSample(glow_tex, smp, in.uv + off).r,
        textureSample(glow_tex, smp, in.uv).g,
        textureSample(glow_tex, smp, in.uv - off).b);
    } else {
      g = textureSample(glow_tex, smp, in.uv).rgb;
    }
    // Screen-style add: bright where glow is, never past white.
    c = 1.0 - (1.0 - c) * (1.0 - clamp(g * comp.glow, vec3<f32>(0.0), vec3<f32>(1.0)));
  }
  if (comp.vignette > 0.0) {
    let d = length((in.uv - 0.5) * vec2<f32>(comp.size.x / comp.size.y, 1.0));
    c *= 1.0 - comp.vignette * smoothstep(0.35, 0.95, d);
  }
  if (comp.grain > 0.0) {
    // Static grain: the same pattern every frame, so it never forces redraws.
    c += (hash(floor(in.uv * comp.size)) - 0.5) * comp.grain * 0.06;
  }
  return vec4<f32>(c, 1.0);
}
