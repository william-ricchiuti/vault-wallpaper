//! Text on the wallpaper: hub labels and overlays (clock, vault stats, note of the
//! hour). Laid out and uploaded only when the text or positions change; drawing it
//! is part of the cached scene, so it costs nothing per animated frame.
//!
//! STUB: the signatures are fixed (the renderer is built against them); the body is
//! being written in a parallel session.

use crate::Gpu;

/// Font family, matching the config's `labelFont`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontFamily {
    #[default]
    Sans,
    Mono,
    Serif,
}

/// How a label is dressed, matching `labelStyle.mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextStyle {
    /// Text on a rounded translucent plate.
    #[default]
    Badge,
    /// Text with a soft coloured glow behind it.
    Glow,
    /// Plain text.
    Minimal,
}

/// Where `pos` sits relative to the text block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    /// Centred horizontally, top edge at `pos` (labels under a note).
    #[default]
    TopCenter,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextItem {
    pub text: String,
    /// Position in points.
    pub pos: [f32; 2],
    pub anchor: Anchor,
    /// Font size in points.
    pub size: f32,
    /// Straight sRGB colour and alpha.
    pub color: [f32; 4],
    pub family: FontFamily,
    pub style: TextStyle,
    /// Plate colour for `Badge`, glow colour for `Glow` (alpha = strength).
    pub accent: [f32; 4],
    /// Wrap width in points (`None` = one line).
    pub max_width: Option<f32>,
}

/// GPU text renderer for one target format.
pub struct TextRenderer {
    _private: (),
}

impl TextRenderer {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
        let _ = (gpu, format);
        TextRenderer { _private: () }
    }

    /// Lays out and uploads `items` for a target of `size` pixels at `scale` pixels per
    /// point. Call only when the items or the target change.
    pub fn prepare(&mut self, gpu: &Gpu, items: &[TextItem], size: [u32; 2], scale: f32) {
        let _ = (gpu, items, size, scale);
    }

    /// Draws what `prepare` uploaded into an open render pass.
    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        let _ = pass;
    }
}
