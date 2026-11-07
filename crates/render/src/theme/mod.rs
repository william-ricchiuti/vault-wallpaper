//! Themes: whole looks designed for the native app under the lightness rules (still
//! by default, expensive work done once, motion handed to the window server).
//!
//! A theme is three things:
//! - a style ([`Theme::style`]): colours and effects, starting from the config;
//! - a scene builder ([`Theme::build`]): what gets drawn into the cached scene;
//! - an ambient motion ([`Theme::ambient`]) the window server runs on the glow layer
//!   with no app frames, or none;
//! - optionally, an edit burst ([`Theme::edit_burst`], [`Theme::burst_actors`]): a
//!   short animation over the still scene when a note is saved, baked once and
//!   played by the window server (see [`crate::burst`]).
//!
//! The web version's theme ids map onto the default look (they were Canvas2D designs
//! that don't port cheaply); the native themes have ids of their own.

mod aurora_lite;
mod ink_paper;
mod night_sky;
mod pulse;

use vw_core::config::Config;
use vw_core::layout::LayoutGraph;

use crate::burst::{BurstActor, EditBurst};
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
    /// A printed plate: ink on warm paper, fully still.
    InkPaper,
    /// A calm, still graph; a saved note sends light out along its links.
    Pulse,
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
            "ink-paper" => Theme::InkPaper,
            "pulse" => Theme::Pulse,
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
            Theme::InkPaper => ink_paper::style(cfg, base),
            Theme::Pulse => pulse::style(cfg, base),
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
            Theme::InkPaper => ink_paper::build(graph, look, positions, framing, style, size),
            Theme::Pulse => pulse::build(graph, look, positions, framing, style, size),
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

    /// What this theme does when a note is saved, under the config's motion setting.
    /// `None`: nothing animates (the scene simply redraws once with the edit).
    pub fn edit_burst(self, cfg: &Config) -> Option<EditBurst> {
        let fps = burst_fps(cfg)?;
        match self {
            Theme::InkPaper => Some(EditBurst { duration: 2.0, fps, resolution: 1.0 }),
            // Soft light: baked at half resolution, scaled up by the window server.
            Theme::Pulse => Some(EditBurst { duration: pulse::DURATION, fps, resolution: 0.5 }),
            Theme::Default | Theme::NightSky | Theme::AuroraLite => None,
        }
    }

    /// The burst for the edited note `node` (an index into `graph`), in the same
    /// screen points as [`Theme::build`]. Empty when the note has nothing to show.
    pub fn burst_actors(
        self,
        graph: &LayoutGraph,
        look: &Look,
        positions: &[[f32; 2]],
        framing: &Framing,
        node: usize,
        burst: &EditBurst,
    ) -> Vec<BurstActor> {
        match self {
            Theme::InkPaper => ink_paper::burst_actors(graph, look, positions, framing, node, burst),
            Theme::Pulse => pulse::burst_actors(graph, look, positions, framing, node, burst),
            Theme::Default | Theme::NightSky | Theme::AuroraLite => Vec::new(),
        }
    }
}

/// Frame-rate cap for edit bursts: none in Still, low otherwise. Bursts are short and
/// rare, so they can afford a few more frames than ambient motion, but are designed
/// to read well at these rates.
pub fn burst_fps(cfg: &Config) -> Option<f32> {
    let fps: f32 = match cfg.motion_mode {
        vw_core::config::MotionMode::Still => return None,
        vw_core::config::MotionMode::Light => 8.0,
        vw_core::config::MotionMode::Balanced => 12.0,
        vw_core::config::MotionMode::Showcase => 20.0,
    };
    let fps = fps.min(cfg.max_fps as f32);
    (fps > 0.0).then_some(fps)
}
