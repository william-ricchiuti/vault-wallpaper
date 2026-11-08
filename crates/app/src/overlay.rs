//! The glow overlay: the blurred glow, rendered once at a quarter of the screen's
//! resolution into its own Core Animation layer above the scene. Breathing is a
//! Core Animation opacity animation on that layer, run by the window server at a low
//! frame rate: the app draws nothing while the wallpaper breathes, and the window
//! server skips it entirely while the wallpaper is covered.
//!
//! Measured on 2026-11-06: breathing by redrawing frames cost ~216 MB of extra driver
//! memory and ~2.5% app CPU; this way costs neither. The window server does some work
//! while the layer breathes (noisy to measure, a few percent of a core at most);
//! stepping the opacity from the app at 8 fps measured worse, so this stays.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFData, CFRetained, CGAffineTransform};
use objc2_core_graphics::{CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo, kCGColorSpaceSRGB};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSPoint, NSRect, NSSize, NSString, NSValue};
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::{MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage};
use objc2_quartz_core::{
    CAAnimation, CAAnimationGroup, CABasicAnimation, CAFrameRateRange, CAKeyframeAnimation, CALayer, CAMediaTiming, CAMediaTimingFunction, CAMetalLayer,
    CATransaction, kCAAnimationDiscrete, kCAFillModeBoth, kCAFilterLinear, kCAMediaTimingFunctionEaseInEaseOut,
};
use vw_render::{BakedActor, BakedBurst, BurstActor, BurstRenderer, EditBurst, Gpu, SceneStyle, WallpaperRenderer};

const BREATH_KEY: &str = "vw-breath";

/// Format of the scene layer's surfaces ('BGRA' IOSurfaces).
pub const SCENE_LAYER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

/// The composited scene, shown as the contents of a plain layer at the bottom of the
/// window: one IOSurface the renderer draws into, replaced on every redraw.
///
/// Memory: a wgpu surface (CAMetalLayer) keeps every drawable it has handed out, so
/// after its second present the window holds two full-screen buffers for good
/// (~24 MB each on a 3024x1964 display). This layer holds one: a redraw makes a new
/// surface, draws into it, waits for the GPU, shows it and frees the old one.
/// Presenting also wakes the Metal driver's render pool just as a render pass does,
/// so skipping the present costs nothing extra (measured 2026-11-07).
pub struct SceneLayer {
    layer: Retained<CALayer>,
    shown: Option<CFRetained<IOSurfaceRef>>,
}

/// A surface being drawn for [`SceneLayer::show`].
pub struct SceneTarget {
    surface: CFRetained<IOSurfaceRef>,
    pub texture: wgpu::Texture,
}

impl SceneLayer {
    /// Adds the layer under everything else in `window`'s content view.
    pub fn new(window: &NSWindow) -> Option<Self> {
        let host = window.contentView()?.layer()?;
        let layer = CALayer::new();
        layer.setFrame(host.bounds());
        layer.setOpaque(true);
        layer.setContentsScale(window.backingScaleFactor());
        host.insertSublayer_atIndex(&layer, 0);
        Some(SceneLayer { layer, shown: None })
    }

    /// Keeps the layer's frame matched to its host after a resize.
    pub fn fit(&self, window: &NSWindow) {
        if let Some(host) = window.contentView().and_then(|v| v.layer()) {
            without_actions(|| {
                self.layer.setFrame(host.bounds());
                self.layer.setContentsScale(window.backingScaleFactor());
            });
        }
    }

    /// A new IOSurface of `size` pixels, wrapped as a wgpu render target.
    pub fn target(&self, gpu: &Gpu, size: [u32; 2]) -> Option<SceneTarget> {
        let [w, h] = size.map(|v| v.max(1));
        let keys = ["IOSurfaceWidth", "IOSurfaceHeight", "IOSurfaceBytesPerElement", "IOSurfacePixelFormat"].map(NSString::from_str);
        let values = [w, h, 4, u32::from_be_bytes(*b"BGRA")].map(NSNumber::new_u32);
        let props = NSDictionary::from_retained_objects(&keys.each_ref().map(|k| &**k), &values);
        // SAFETY: NSDictionary is toll-free bridged with CFDictionary, and the
        // properties are the documented CFString keys with CFNumber values.
        let surface = unsafe { IOSurfaceRef::new(&*(Retained::as_ptr(&props) as *const objc2_core_foundation::CFDictionary)) }?;
        // SAFETY: the device is wgpu's Metal device; the texture is created from it,
        // matches the descriptor below and is cleared by the first render pass.
        unsafe {
            let hal = gpu.device.as_hal::<wgpu::hal::api::Metal>()?;
            let desc = MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                w as usize,
                h as usize,
                false,
            );
            desc.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
            desc.setStorageMode(MTLStorageMode::Shared);
            let raw = hal.raw_device().newTextureWithDescriptor_iosurface_plane(&desc, &surface, 0)?;
            let extent = wgpu::hal::CopyExtent { width: w, height: h, depth: 1 };
            let hal_texture = wgpu::hal::metal::Device::texture_from_raw(raw, SCENE_LAYER_FORMAT, MTLTextureType::Type2D, 1, 1, extent, None);
            let texture = gpu.device.create_texture_from_hal::<wgpu::hal::api::Metal>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("scene layer"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: SCENE_LAYER_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
                wgpu::TextureUses::UNINITIALIZED,
            );
            Some(SceneTarget { surface, texture })
        }
    }

    /// Shows `target` once the GPU has finished drawing it, and frees the surface
    /// shown before.
    pub fn show(&mut self, gpu: &Gpu, target: SceneTarget) {
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let SceneTarget { surface, texture } = target;
        drop(texture);
        // SAFETY: IOSurfaceRef is toll-free bridged with the IOSurface class, which
        // layer contents accept.
        let contents: &AnyObject = unsafe { &*(CFRetained::as_ptr(&surface).as_ptr() as *const AnyObject) };
        without_actions(|| unsafe { self.layer.setContents(Some(contents)) });
        self.shown = Some(surface);
    }
}

pub struct GlowOverlay {
    layer: Retained<CAMetalLayer>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The breathing currently applied: (depth, period seconds, fps).
    breathing: Option<(f32, f32, f32)>,
}

impl GlowOverlay {
    /// Adds the overlay layer to `window`'s content view, above wgpu's own layer.
    /// Call after the base surface was created.
    pub fn new(gpu: &Gpu, window: &NSWindow, format: wgpu::TextureFormat) -> Option<Self> {
        let (layer, surface, config) = metal_layer(gpu, window, format)?;
        Some(GlowOverlay { layer, surface, config, breathing: None })
    }

    /// Redraws the overlay from the renderer's current glow layer. Call after
    /// `set_scene`. Hides the layer when there is no glow.
    pub fn draw(&mut self, gpu: &Gpu, renderer: &WallpaperRenderer, style: &SceneStyle) {
        let size = renderer.glow_size().filter(|_| renderer.has_glow() && style.glow > 0.0);
        self.layer.setHidden(size.is_none());
        let Some([w, h]) = size else { return };
        if self.config.width != w || self.config.height != h {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&gpu.device, &self.config);
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => {
                self.surface.configure(&gpu.device, &self.config);
                return;
            }
        };
        renderer.render_glow_overlay(gpu, &frame.texture.create_view(&Default::default()), style);
        gpu.queue.present(frame);
    }

    /// Keeps the layer's frame matched to its host after a resize.
    pub fn fit(&self, window: &NSWindow) {
        if let Some(host) = window.contentView().and_then(|v| v.layer()) {
            self.layer.setFrame(host.bounds());
        }
    }

    /// Starts, changes or stops the breathing animation. `None` stops it.
    pub fn set_breathing(&mut self, breathing: Option<(f32, f32, f32)>) {
        if self.breathing == breathing {
            return;
        }
        self.breathing = breathing;
        let key = NSString::from_str(BREATH_KEY);
        self.layer.removeAnimationForKey(&key);
        let Some((depth, period, fps)) = breathing else { return };
        let anim = CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str("opacity")));
        let (from, to) = (NSNumber::new_f32(1.0), NSNumber::new_f32((1.0 - depth).clamp(0.0, 1.0)));
        // SAFETY: opacity animates between numbers.
        unsafe {
            anim.setFromValue(Some(&from));
            anim.setToValue(Some(&to));
        }
        anim.setDuration(f64::from(period) / 2.0);
        anim.setAutoreverses(true);
        anim.setRepeatCount(f32::INFINITY);
        anim.setTimingFunction(Some(&CAMediaTimingFunction::functionWithName(unsafe { kCAMediaTimingFunctionEaseInEaseOut })));
        // Slow breathing needs few frames; ask the window server for no more.
        anim.setPreferredFrameRateRange(CAFrameRateRange { minimum: 1.0, maximum: fps, preferred: fps });
        self.layer.addAnimation_forKey(&anim, Some(&key));
    }
}

/// A transparent CAMetalLayer above the window's content (and above any layer added
/// before it), with a wgpu surface on it. The layer starts the size of its host.
fn metal_layer(
    gpu: &Gpu,
    window: &NSWindow,
    format: wgpu::TextureFormat,
) -> Option<(Retained<CAMetalLayer>, wgpu::Surface<'static>, wgpu::SurfaceConfiguration)> {
    let view = window.contentView()?;
    let host: Retained<CALayer> = view.layer()?;
    let layer = CAMetalLayer::new();
    layer.setFrame(host.bounds());
    layer.setMagnificationFilter(unsafe { kCAFilterLinear });
    layer.setOpaque(false);
    host.addSublayer(&layer);
    // SAFETY: the layer is a CAMetalLayer that outlives the surface: the host keeps it
    // as a sublayer, and the owning struct holds a reference to it too.
    let surface = unsafe {
        gpu.instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(Retained::as_ptr(&layer) as *mut c_void))
    }
    .ok()?;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: 1,
        height: 1,
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 1,
        // Non-opaque layer; the content is premultiplied, as Core Animation expects.
        alpha_mode: wgpu::CompositeAlphaMode::PostMultiplied,
        view_formats: vec![],
        color_space: Default::default(),
    };
    Some((layer, surface, config))
}

/// Runs `f` with Core Animation's implicit animations off, so frame and visibility
/// changes apply at once instead of fading over a quarter second.
fn without_actions(f: impl FnOnce()) {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    f();
    CATransaction::commit();
}

/// Edit bursts (Ink & Paper's bleed, Pulse's travelling light), played by the
/// window server. A burst is baked once into a small atlas ([`BurstRenderer::bake`]);
/// each actor becomes a small layer showing its strip of that atlas, with keyframe
/// animations for position, opacity and frame. The app presents no frames: measured
/// on 2026-11-06, presenting burst frames (even into a 150 px layer) kept ~210 MB of
/// driver memory dirty for the whole burst plus ~1 s; baking adds no time to the edit
/// redraw's own spike, and the burst then costs its atlas (~1-2 MB).
///
/// The container layer and the bake pipelines are created on the first burst, so a
/// wallpaper nobody edits under never pays for them; the pipelines are kept for
/// later bursts (dropping them after each bake measured no smaller).
pub struct BurstOverlay {
    layer: Retained<CALayer>,
    renderer: BurstRenderer,
    /// Running bursts' layers, by note id.
    running: Vec<(String, Retained<CALayer>)>,
}

impl BurstOverlay {
    pub fn new(gpu: &Gpu, window: &NSWindow) -> Option<Self> {
        let host = window.contentView()?.layer()?;
        let layer = CALayer::new();
        without_actions(|| {
            layer.setFrame(host.bounds());
            host.addSublayer(&layer);
        });
        Some(BurstOverlay { layer, renderer: BurstRenderer::new(gpu), running: Vec::new() })
    }

    /// Keeps the container matched to its host after a resize.
    pub fn fit(&self, window: &NSWindow) {
        if let Some(host) = window.contentView().and_then(|v| v.layer()) {
            without_actions(|| self.layer.setFrame(host.bounds()));
        }
    }

    /// Bakes `actors` and starts them for note `id`, `elapsed` seconds into `burst`
    /// (0 for a fresh burst; more when a running burst is rebuilt after the scene
    /// moved). `scale` is the screen's pixels per point. Returns the atlas bytes.
    pub fn play(&mut self, gpu: &Gpu, id: &str, actors: &[BurstActor], burst: &EditBurst, elapsed: f32, scale: f32) -> usize {
        self.remove(id);
        let Some(baked) = self.renderer.bake(gpu, actors, scale * burst.resolution) else { return 0 };
        let group = CALayer::new();
        let height = self.layer.bounds().size.height;
        // The container itself is not flipped; whether y runs down is the host's.
        let flipped = self.layer.superlayer().is_some_and(|h| h.isGeometryFlipped());
        // Screen points (top-left origin) to the container's coordinates.
        let point = |p: [f32; 2]| {
            let y = if flipped { f64::from(p[1]) } else { height - f64::from(p[1]) };
            NSPoint::new(f64::from(p[0]), y)
        };
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
        without_actions(|| {
            group.setFrame(self.layer.bounds());
            for b in &baked.actors {
                let a = &b.actor;
                let Some(image) = strip_image(&baked, b, space.as_deref()) else { continue };
                let layer = CALayer::new();
                let side = f64::from(b.cell) / f64::from(baked.scale);
                layer.setBounds(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(side, side)));
                layer.setPosition(point(a.keys[0].at));
                // SAFETY: a CGImage is valid layer contents.
                unsafe { layer.setContents(Some(image.as_ref())) };
                layer.setContentsRect(frame_rect(0, a.frames));
                layer.setMinificationFilter(unsafe { kCAFilterLinear });
                layer.setMagnificationFilter(unsafe { kCAFilterLinear });
                // The model value: invisible once the animation is gone.
                layer.setOpacity(0.0);
                let turn = if flipped { a.angle } else { -a.angle };
                if turn != 0.0 {
                    let (s, c) = (f64::from(turn).sin(), f64::from(turn).cos());
                    layer.setAffineTransform(CGAffineTransform { a: c, b: s, c: -s, d: c, tx: 0.0, ty: 0.0 });
                }
                layer.addAnimation_forKey(&actor_animation(a, burst, elapsed, &point), Some(&NSString::from_str("vw-burst")));
                group.addSublayer(&layer);
            }
            self.layer.addSublayer(&group);
        });
        self.running.push((id.to_owned(), group));
        baked.bytes()
    }

    /// Removes note `id`'s burst, if it runs.
    pub fn remove(&mut self, id: &str) {
        let (gone, keep): (Vec<_>, Vec<_>) = self.running.drain(..).partition(|(r, _)| r == id);
        self.running = keep;
        without_actions(|| gone.iter().for_each(|(_, l)| l.removeFromSuperlayer()));
    }

    /// Removes every burst but those for which `keep` says yes.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        let ids: Vec<String> = self.running.iter().filter(|(id, _)| !keep(id)).map(|(id, _)| id.clone()).collect();
        for id in ids {
            self.remove(&id);
        }
    }

    pub fn clear(&mut self) {
        self.retain(|_| false);
    }
}

/// The unit-space rectangle of frame `k` of `n` in an actor's strip.
fn frame_rect(k: u32, n: u32) -> NSRect {
    let n = f64::from(n.max(1));
    NSRect::new(NSPoint::new(f64::from(k) / n, 0.0), NSSize::new(1.0 / n, 1.0))
}

/// One actor's strip of frames as its own image (copied out of the atlas, so the
/// image's layout never depends on how Core Graphics orders sub-rectangles).
fn strip_image(baked: &BakedBurst, b: &BakedActor, space: Option<&CGColorSpace>) -> Option<CFRetained<CGImage>> {
    let (w, h) = (b.cell * b.actor.frames.max(1), b.cell);
    let mut bytes = Vec::with_capacity((w * h * 4) as usize);
    for y in b.origin[1]..b.origin[1] + h {
        let start = ((y * baked.size[0] + b.origin[0]) * 4) as usize;
        bytes.extend_from_slice(baked.rgba.get(start..start + (w * 4) as usize)?);
    }
    let data = CFData::from_bytes(&bytes);
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    // SAFETY: the provider holds w * h tightly packed premultiplied RGBA8 pixels.
    unsafe {
        CGImage::new(
            w as usize,
            h as usize,
            8,
            32,
            (w * 4) as usize,
            space,
            CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0),
            Some(&provider),
            std::ptr::null(),
            true,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
}

/// Key times must not go backwards; equal ones are nudged apart.
fn key_times(times: impl IntoIterator<Item = f32>) -> Retained<NSArray<NSNumber>> {
    let mut last = -1.0f64;
    let v: Vec<Retained<NSNumber>> = times
        .into_iter()
        .map(|t| {
            let t = f64::from(t).clamp(0.0, 1.0).max(last.min(1.0));
            last = t;
            NSNumber::new_f64(t)
        })
        .collect();
    NSArray::from_retained_slice(&v)
}

fn objects<T: objc2::Message>(v: Vec<Retained<T>>) -> Retained<NSArray> {
    // SAFETY: every Objective-C object is an AnyObject.
    let v: Vec<Retained<AnyObject>> = v.into_iter().map(|o| unsafe { Retained::cast_unchecked(o) }).collect();
    NSArray::from_retained_slice(&v)
}

/// Position, opacity and frame keyframes for one actor, grouped to the burst's
/// length and frame-rate cap.
fn actor_animation(a: &BurstActor, burst: &EditBurst, elapsed: f32, point: &dyn Fn([f32; 2]) -> NSPoint) -> Retained<CAAnimationGroup> {
    let rate = CAFrameRateRange { minimum: 1.0, maximum: burst.fps, preferred: burst.fps };
    let keyframes = |path: &str, values: Retained<NSArray>, times: Retained<NSArray<NSNumber>>, discrete: bool| {
        let k = CAKeyframeAnimation::animationWithKeyPath(Some(&NSString::from_str(path)));
        if discrete {
            k.setCalculationMode(unsafe { kCAAnimationDiscrete });
        }
        // SAFETY: values match the animated property's type.
        unsafe { k.setValues(Some(&values)) };
        k.setKeyTimes(Some(&times));
        k.setDuration(f64::from(burst.duration));
        k.setPreferredFrameRateRange(rate);
        Retained::into_super(Retained::into_super(k))
    };
    let mut parts: Vec<Retained<CAAnimation>> = Vec::new();
    let opacity = a.keys.iter().map(|k| NSNumber::new_f32(k.opacity)).collect();
    parts.push(keyframes("opacity", objects(opacity), key_times(a.keys.iter().map(|k| k.t)), false));
    if a.keys.windows(2).any(|w| w[0].at != w[1].at) {
        // SAFETY: NSValue wraps a point by value.
        let at = a.keys.iter().map(|k| unsafe { NSValue::valueWithPoint(point(k.at)) }).collect();
        parts.push(keyframes("position", objects(at), key_times(a.keys.iter().map(|k| k.t)), false));
    }
    if a.frames > 1 {
        // Frame k shows from t0 + k * step (frame 0 from the start), the last one to
        // the end: discrete key times have one more entry than values.
        let n = a.frames;
        let [t0, t1] = a.play;
        let step = (t1 - t0) / (n - 1) as f32;
        // SAFETY: NSValue wraps a rect by value.
        let rects = (0..n).map(|k| unsafe { NSValue::valueWithRect(frame_rect(k, n)) }).collect();
        let times = std::iter::once(0.0).chain((1..n).map(|k| t0 + k as f32 * step)).chain(std::iter::once(1.0));
        parts.push(keyframes("contentsRect", objects(rects), key_times(times), true));
    }
    let group = CAAnimationGroup::animation();
    group.setAnimations(Some(&NSArray::from_retained_slice(&parts)));
    group.setDuration(f64::from(burst.duration));
    group.setTimeOffset(f64::from(elapsed));
    group.setFillMode(unsafe { kCAFillModeBoth });
    group.setRemovedOnCompletion(false);
    group.setPreferredFrameRateRange(rate);
    group
}
