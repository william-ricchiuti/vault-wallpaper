//! The glow overlay: the blurred glow, rendered once at a quarter of the screen's
//! resolution into its own Core Animation layer above the scene. Breathing is a
//! Core Animation opacity animation on that layer, run by the window server at a low
//! frame rate: the app draws nothing while the wallpaper breathes, and the window
//! server skips it entirely while the wallpaper is covered.
//!
//! Measured on 2026-11-06: breathing by redrawing frames cost ~216 MB of extra driver
//! memory and ~2.5% CPU; this way costs neither.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use objc2_foundation::{NSNumber, NSString};
use objc2_quartz_core::{
    CABasicAnimation, CAFrameRateRange, CALayer, CAMediaTiming, CAMediaTimingFunction, CAMetalLayer, CATransaction, kCAFilterLinear,
    kCAMediaTimingFunctionEaseInEaseOut,
};
use vw_render::{Gpu, SceneStyle, WallpaperRenderer};

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
        let view = window.contentView()?;
        let host: Retained<CALayer> = view.layer()?;
        let layer = CAMetalLayer::new();
        layer.setFrame(host.bounds());
        layer.setMagnificationFilter(unsafe { kCAFilterLinear });
        layer.setOpaque(false);
        host.addSublayer(&layer);
        // SAFETY: the layer is a CAMetalLayer that outlives the surface (both live in
        // this struct, and the surface is dropped first: fields drop in order).
        let surface = unsafe {
            gpu.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(Retained::as_ptr(&layer) as *mut c_void))
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

    /// Sets the layer's opacity directly, with no implicit animation: the window
    /// server recomposites once for this change and then rests.
    pub fn set_opacity(&self, opacity: f32) {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.layer.setOpacity(opacity);
        CATransaction::commit();
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
