//! Themes: whole looks designed for the native app under the lightness rules (still
//! by default, expensive work done once, motion handed to the window server).
//!
//! A theme is three things:
//! - a style ([`Theme::style`]): colours and effects, starting from the config;
//! - a scene builder ([`Theme::build`]): what gets drawn into the cached scene;
//! - an ambient motion ([`Theme::ambient`]) the window server runs on the glow layer
//!   with no app frames, or none.
//!
//! The web version's theme ids map onto the default look (they were Canvas2D designs
//! that don't port cheaply); the native themes have ids of their own.

mod aurora_lite;
mod night_sky;

use vw_core::config::Config;
use vw_core::layout::LayoutGraph;

use crate::look::Look;
use crate::scene::{SceneData, SceneStyle, build_scene, style_from_config};
use crate::Framing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// The web version's default renderer and its presets.
    #[default]
    Default,
    /// Notes as stars, tags as constellations.
    NightSky,
    /// Soft aurora curtains rising over the biggest tag clusters.
    AuroraLite,
}

/// Motion the window server runs on its own (see the app's glow overlay).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ambient {
    None,
    /// The glow layer's opacity eases between 1 and `1 - depth` and back every
    /// `period` seconds, at no more than `fps` frames per second.
    Breathe { depth: f32, period: f32, fps: f32 },
}

impl Theme {
    pub fn from_id(id: &str) -> Theme {
        match id {
            "night-sky" => Theme::NightSky,
            "aurora-lite" => Theme::AuroraLite,
            _ => Theme::Default,
        }
    }

    pub fn from_config(cfg: &Config) -> Theme {
        Theme::from_id(&cfg.theme)
    }

    pub fn style(self, cfg: &Config, light: bool) -> SceneStyle {
        let base = style_from_config(cfg, light);
        match self {
            Theme::Default => base,
            Theme::NightSky => night_sky::style(cfg, base),
            Theme::AuroraLite => aurora_lite::style(cfg, base),
        }
    }

    pub fn build(
        self,
        graph: &LayoutGraph,
        look: &Look,
        positions: &[[f32; 2]],
        framing: &Framing,
        style: &SceneStyle,
        size: [f32; 2],
    ) -> SceneData {
        match self {
            Theme::Default => build_scene(graph, look, positions, framing, style, size),
            Theme::NightSky => night_sky::build(graph, look, positions, framing, style, size),
            Theme::AuroraLite => aurora_lite::build(graph, look, positions, framing, style, size),
        }
    }

    /// The window-server motion for this theme under `style` (none unless the
    /// config's motion setting allows it).
    pub fn ambient(self, style: &SceneStyle) -> Ambient {
        if !style.animates() {
            return Ambient::None;
        }
        match style.breathing {
            Some((depth, period)) => Ambient::Breathe { depth, period, fps: style.motion_fps },
            None => Ambient::None,
        }
    }
}
