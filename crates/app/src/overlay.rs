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
use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{
    CABasicAnimation, CAFrameRateRange, CALayer, CAMediaTiming, CAMediaTimingFunction, CAMetalLayer, CATransaction, kCAFilterLinear,
    kCAMediaTimingFunctionEaseInEaseOut,
};
use vw_render::{BurstInstance, BurstKind, BurstRenderer, Gpu, SceneStyle, WallpaperRenderer};

const BREATH_KEY: &str = "vw-breath";

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

/// Edit bursts (Ink & Paper's bleed, Pulse): a small layer placed over just the
/// bursts' bounding box, redrawn at a low frame rate while they run, hidden after.
/// Created on the first edit, so a wallpaper nobody edits under never pays for it.
pub struct BurstOverlay {
    surface: wgpu::Surface<'static>,
    layer: Retained<CAMetalLayer>,
    config: wgpu::SurfaceConfiguration,
    renderer: BurstRenderer,
    /// Where the layer sits: x, y, w, h in screen points (top-left origin).
    region: Option<[f32; 4]>,
    scale: f32,
}

/// Drawables larger than this (pixels) are given back when the bursts end.
const KEEP_PIXELS: u32 = 512 * 512;

impl BurstOverlay {
    pub fn new(gpu: &Gpu, window: &NSWindow, format: wgpu::TextureFormat) -> Option<Self> {
        let (layer, surface, config) = metal_layer(gpu, window, format)?;
        without_actions(|| layer.setHidden(true));
        Some(BurstOverlay { surface, layer, config, renderer: BurstRenderer::new(gpu, format), region: None, scale: 1.0 })
    }

    /// Places the layer over `region` (screen points, top-left origin) of `window`.
    pub fn set_region(&mut self, gpu: &Gpu, window: &NSWindow, region: [f32; 4], scale: f32) {
        let w = ((region[2] * scale).round() as u32).max(1);
        let h = ((region[3] * scale).round() as u32).max(1);
        if self.config.width != w || self.config.height != h {
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&gpu.device, &self.config);
        }
        if self.region != Some(region)
            && let Some(host) = window.contentView().and_then(|v| v.layer())
        {
            let [x, y, rw, rh] = region.map(f64::from);
            // AppKit's backing layers are y-up unless the view is flipped.
            let y = if host.isGeometryFlipped() { y } else { host.bounds().size.height - y - rh };
            without_actions(|| self.layer.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(rw, rh))));
        }
        self.region = Some(region);
        self.scale = scale;
    }

    /// Draws one frame of `sprites` and shows the layer.
    pub fn draw(&mut self, gpu: &Gpu, kind: BurstKind, sprites: &[BurstInstance]) {
        let Some(region) = self.region else { return };
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => {
                self.surface.configure(&gpu.device, &self.config);
                return;
            }
        };
        self.renderer.draw(gpu, &frame.texture.create_view(&Default::default()), region, self.scale, kind, sprites);
        gpu.queue.present(frame);
        if self.layer.isHidden() {
            without_actions(|| self.layer.setHidden(false));
        }
    }

    /// Hides the layer (the still scene below is the settled state) and gives back
    /// large drawables.
    pub fn hide(&mut self, gpu: &Gpu) {
        without_actions(|| self.layer.setHidden(true));
        if self.config.width * self.config.height > KEEP_PIXELS {
            self.config.width = 1;
            self.config.height = 1;
            self.surface.configure(&gpu.device, &self.config);
            self.region = None;
        }
    }
}
