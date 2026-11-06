//! The config store: the web version's `config.json` schema, typed.
//!
//! Key names (camelCase in JSON), defaults, ranges and accept/reject rules match
//! `parser.js` (`DEFAULTS`, `VALIDATORS`), so an existing config or custom preset imports
//! as it is. Four web-only keys (`port`, `refreshMs`, `renderer`, `fastHash`) are still
//! validated on input but never kept. `parity/config-js.js` checks this against
//! `parser.js` itself.

mod store;
mod validate;
mod watch;

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::layout::{LayoutKind, LayoutOptions};
use crate::vault::{GraphOptions, VaultOptions};

pub use store::{default_path, import_legacy, load, save};
pub use validate::{THEMES, is_hex_color, sanitize_file, sanitize_patch};
pub use watch::{ConfigWatcher, DEBOUNCE, watch};

/// Every setting. Colours stay `#rrggbb` strings, as stored; [`rgb`] decodes them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// `None` until the user picks a vault (the web version required one up front).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
    pub accent: String,
    pub background: String,
    pub link_opacity: f64,
    pub node_glow: bool,
    pub particles: bool,
    pub particle_speed: f64,
    pub particle_density: f64,
    pub motion_mode: MotionMode,
    pub cluster_by_tag: bool,
    pub cluster_halos: bool,
    pub background_gradient: bool,
    pub hub_labels: bool,
    pub hub_label_count: u32,
    pub label_min_importance: f64,
    pub edge_coloring: bool,
    pub depth_of_field: bool,
    pub note_flare: bool,
    pub auto_scale_large_vaults: bool,
    pub show_unresolved_links: bool,
    pub glow_intensity: f64,
    pub edge_style: EdgeStyle,
    pub node_color_mode: NodeColorMode,
    pub label_font: LabelFont,
    pub auto_theme: bool,
    pub light_accent: String,
    pub light_background: String,
    pub max_rendered_nodes: u32,
    pub ignore_paths: Vec<String>,
    pub tag_colors: BTreeMap<String, String>,
    pub glow_breathing: bool,
    pub glow_breathing_speed: f64,
    pub glow_breathing_depth: f64,
    pub ambient_particles: bool,
    pub ambient_particle_count: u32,
    pub ambient_particle_speed: f64,
    pub ambient_particle_size: f64,
    pub chromatic_bloom: bool,
    pub chromatic_bloom_intensity: f64,
    pub depth_parallax: bool,
    pub depth_parallax_strength: f64,
    pub depth_parallax_layers: u32,
    /// One of [`THEMES`].
    pub theme: String,
    pub label_style: LabelStyle,
    // Composition
    #[serde(with = "layout_name")]
    pub layout: LayoutKind,
    pub framing: Framing,
    pub zoom: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    pub safe_area: SafeArea,
    // Colour
    pub palette: Palette,
    pub recency_days: u32,
    pub time_of_day: TimeOfDay,
    pub tag_shapes: bool,
    // Texture
    pub grain: bool,
    pub vignette: bool,
    // Overlays and focus
    pub overlay: Overlay,
    pub label_source: LabelSource,
    pub focus_note: String,
    pub focus_hops: u32,
    pub rotate: Rotate,
    // Performance
    pub max_fps: u32,
    pub idle_fps: u32,
}

impl Default for Config {
    /// `DEFAULTS` from `parser.js`.
    fn default() -> Self {
        Config {
            vault_path: None,
            accent: "#7c5cff".into(),
            background: "#0a0a0f".into(),
            link_opacity: 0.18,
            node_glow: true,
            particles: true,
            particle_speed: 1.0,
            particle_density: 0.3,
            motion_mode: MotionMode::Balanced,
            cluster_by_tag: true,
            cluster_halos: true,
            background_gradient: true,
            hub_labels: false,
            hub_label_count: 5,
            label_min_importance: 0.22,
            edge_coloring: true,
            depth_of_field: true,
            note_flare: true,
            auto_scale_large_vaults: true,
            show_unresolved_links: true,
            glow_intensity: 1.0,
            edge_style: EdgeStyle::Line,
            node_color_mode: NodeColorMode::Tag,
            label_font: LabelFont::Sans,
            auto_theme: false,
            light_accent: "#39407a".into(),
            light_background: "#ece6d6".into(),
            max_rendered_nodes: 5000,
            ignore_paths: Vec::new(),
            tag_colors: BTreeMap::new(),
            glow_breathing: true,
            glow_breathing_speed: 1.0,
            glow_breathing_depth: 0.15,
            ambient_particles: false,
            ambient_particle_count: 40,
            ambient_particle_speed: 0.3,
            ambient_particle_size: 1.5,
            chromatic_bloom: true,
            chromatic_bloom_intensity: 0.4,
            depth_parallax: true,
            depth_parallax_strength: 0.5,
            depth_parallax_layers: 3,
            theme: "default".into(),
            label_style: LabelStyle::default(),
            layout: LayoutKind::Force,
            framing: Framing::Center,
            zoom: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            safe_area: SafeArea::default(),
            palette: Palette::Auto,
            recency_days: 7,
            time_of_day: TimeOfDay::Off,
            tag_shapes: false,
            grain: true,
            vignette: false,
            overlay: Overlay::None,
            label_source: LabelSource::Hubs,
            focus_note: String::new(),
            focus_hops: 2,
            rotate: Rotate::default(),
            max_fps: 30,
            idle_fps: 10,
        }
    }
}

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "lowercase")]
        pub enum $name { $($variant),+ }
    };
}

string_enum!(MotionMode { Still, Light, Balanced, Showcase });
string_enum!(EdgeStyle { Line, Curve, None });
string_enum!(NodeColorMode { Tag, Age, Folder, Recency });
string_enum!(LabelFont { Sans, Mono, Serif });
string_enum!(LabelStyleMode { Badge, Glow, Minimal, Inherit });
string_enum!(Framing { Center, Fill, Left, Right, None });
string_enum!(Palette { Auto, Aurora, Riso, Metro, Pastel, Earth, Neon, Viridis, Mono });
string_enum!(TimeOfDay { Off, Tint, Palette });
string_enum!(Overlay { None, Clock, Stats, Note });
string_enum!(LabelSource { Hubs, Recent });

/// Per-theme label overrides; an unset field means "the theme decides".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<LabelStyleMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_alpha: Option<f64>,
    /// `"accent"`, `"node"` or `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chromatic_split: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_style: Option<LabelFont>,
}

/// Screen-edge insets in points, kept clear of the graph (menu bar, dock, widgets).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeArea {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

/// Cycle through `themes` every `every_minutes`; an empty list means no rotation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rotate {
    pub themes: Vec<String>,
    pub every_minutes: u32,
}

impl Default for Rotate {
    fn default() -> Self {
        Rotate { themes: Vec::new(), every_minutes: 60 }
    }
}

/// A config plus the keys that were dropped on the way in.
#[derive(Debug, Clone)]
pub struct Sanitized {
    pub config: Config,
    /// Invalid keys that were left out. Empty means the web version would have accepted
    /// the file as it is.
    pub issues: Vec<Issue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub key: String,
    pub message: String,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Json(serde_json::Error),
    /// The root (of a file or a patch) is not a JSON object.
    NotObject,
    UnknownKey(String),
    Invalid { key: String, message: String },
}

impl ConfigError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, ConfigError::Io(e) if e.kind() == std::io::ErrorKind::NotFound)
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "{e}"),
            ConfigError::Json(e) => write!(f, "invalid JSON: {e}"),
            ConfigError::NotObject => f.write_str("config root must be a JSON object"),
            ConfigError::UnknownKey(k) => write!(f, "unknown config key: {k}"),
            ConfigError::Invalid { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io(e) => Some(e),
            ConfigError::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl Config {
    /// Parses a whole `config.json`, dropping invalid keys (see [`sanitize_file`]).
    pub fn from_json(text: &str) -> Result<Sanitized, ConfigError> {
        sanitize_file(&serde_json::from_str(text).map_err(ConfigError::Json)?)
    }

    /// Applies a settings-UI update. All or nothing: an unknown or invalid key leaves
    /// `self` untouched. Returns the keys that were set, so callers can tell whether the
    /// vault needs a rescan (`ignorePaths`, `showUnresolvedLinks`).
    pub fn apply_patch(&mut self, raw: &Value) -> Result<Vec<String>, ConfigError> {
        let clean = sanitize_patch(raw)?;
        let keys = clean.keys().cloned().collect();
        *self = validate::build(self.clone(), clean)?;
        Ok(keys)
    }

    pub fn vault_options(&self) -> VaultOptions {
        VaultOptions {
            ignore_paths: self.ignore_paths.clone(),
            graph: GraphOptions { show_unresolved_links: self.show_unresolved_links },
        }
    }

    /// Layout options for this config; the viewport and clock keep their defaults.
    pub fn layout_options(&self) -> LayoutOptions {
        LayoutOptions { kind: self.layout, cluster_by_tag: self.cluster_by_tag, ..LayoutOptions::default() }
    }
}

/// `#rrggbb` to bytes. `None` for anything else.
pub fn rgb(hex: &str) -> Option<[u8; 3]> {
    if !is_hex_color(hex) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(1)?, byte(3)?, byte(5)?])
}

/// `LayoutKind` lives in `layout`, which has no serde; (de)serialise it by name.
mod layout_name {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    use crate::layout::LayoutKind;

    pub fn serialize<S: Serializer>(kind: &LayoutKind, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(kind.name())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<LayoutKind, D::Error> {
        let name = String::deserialize(d)?;
        LayoutKind::parse(&name).ok_or_else(|| D::Error::custom(format!("unknown layout {name:?}")))
    }
}

#[cfg(test)]
mod tests;
