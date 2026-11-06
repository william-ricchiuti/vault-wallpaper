//! A port of `parser.js`'s `VALIDATORS`. Validators work on raw JSON values, the way the
//! JS ones work on whatever `JSON.parse` returned, so type confusions (`"5"` for `5`,
//! `null` for a missing key) are judged exactly as the web version judges them.

use serde_json::{Map, Value};

use super::{Config, ConfigError, Issue, Sanitized};

const MOTION_MODES: &[&str] = &["still", "light", "balanced", "showcase"];
const EDGE_STYLES: &[&str] = &["line", "curve", "none"];
const NODE_COLOR_MODES: &[&str] = &["tag", "age", "folder", "recency"];
const LABEL_FONTS: &[&str] = &["sans", "mono", "serif"];
const LABEL_STYLE_MODES: &[&str] = &["badge", "glow", "minimal", "inherit"];
const LAYOUTS: &[&str] = &["force", "rings", "timeline", "galaxy"];
const FRAMINGS: &[&str] = &["center", "fill", "left", "right", "none"];
const PALETTES: &[&str] = &["auto", "aurora", "riso", "metro", "pastel", "earth", "neon", "viridis", "mono"];
const TIME_OF_DAY: &[&str] = &["off", "tint", "palette"];
const OVERLAYS: &[&str] = &["none", "clock", "stats", "note"];
const LABEL_SOURCES: &[&str] = &["hubs", "recent"];
const RENDERERS: &[&str] = &["auto", "canvas", "webgl"];

/// Theme ids accepted: the web version's (its `themes/*.js` plus `default`), kept so old
/// configs and presets import, followed by the native app's own themes (designed for
/// lightness; the web version rejects these ids, an intentional difference).
pub const THEMES: &[&str] = &[
    "default",
    "aurora",
    "bauhaus",
    "celestial",
    "led",
    "metro",
    "mycelium",
    "phosphor",
    "pixel",
    "riso",
    "sketch",
    "stained-glass",
    "stitch",
    "survey",
    "synapse",
    "territories",
    "wash",
    // Native themes.
    "night-sky",
    "pulse",
    "aurora-lite",
    "ink-paper",
];

#[derive(Clone, Copy)]
enum Rule {
    Hex,
    Bool,
    Int(i64, i64),
    Num(f64, f64),
    Enum(&'static [&'static str]),
    Str(usize),
    IgnorePaths,
    TagColors,
    LabelStyle,
    SafeArea,
    Rotate,
}

use Rule::*;

/// Every key the native config keeps, in `VALIDATORS` order.
const RULES: &[(&str, Rule)] = &[
    ("accent", Hex),
    ("background", Hex),
    ("linkOpacity", Num(0.0, 1.0)),
    ("nodeGlow", Bool),
    ("particles", Bool),
    ("particleSpeed", Num(0.1, 3.0)),
    ("particleDensity", Num(0.05, 1.0)),
    ("motionMode", Enum(MOTION_MODES)),
    ("clusterByTag", Bool),
    ("clusterHalos", Bool),
    ("backgroundGradient", Bool),
    ("hubLabels", Bool),
    ("hubLabelCount", Int(1, 50)),
    ("labelMinImportance", Num(0.0, 1.0)),
    ("edgeColoring", Bool),
    ("depthOfField", Bool),
    ("noteFlare", Bool),
    ("autoScaleLargeVaults", Bool),
    ("showUnresolvedLinks", Bool),
    ("glowIntensity", Num(0.0, 1.0)),
    ("edgeStyle", Enum(EDGE_STYLES)),
    ("nodeColorMode", Enum(NODE_COLOR_MODES)),
    ("labelFont", Enum(LABEL_FONTS)),
    ("autoTheme", Bool),
    ("lightAccent", Hex),
    ("lightBackground", Hex),
    ("maxRenderedNodes", Int(100, 100_000)),
    ("ignorePaths", IgnorePaths),
    ("tagColors", TagColors),
    ("glowBreathing", Bool),
    ("glowBreathingSpeed", Num(0.1, 3.0)),
    ("glowBreathingDepth", Num(0.0, 0.4)),
    ("ambientParticles", Bool),
    ("ambientParticleCount", Int(0, 300)),
    ("ambientParticleSpeed", Num(0.05, 2.0)),
    ("ambientParticleSize", Num(0.5, 4.0)),
    ("chromaticBloom", Bool),
    ("chromaticBloomIntensity", Num(0.0, 1.0)),
    ("depthParallax", Bool),
    ("depthParallaxStrength", Num(0.0, 1.0)),
    ("depthParallaxLayers", Int(2, 4)),
    ("theme", Enum(THEMES)),
    ("labelStyle", LabelStyle),
    ("layout", Enum(LAYOUTS)),
    ("framing", Enum(FRAMINGS)),
    ("zoom", Num(0.4, 3.0)),
    ("offsetX", Num(-1.0, 1.0)),
    ("offsetY", Num(-1.0, 1.0)),
    ("safeArea", SafeArea),
    ("palette", Enum(PALETTES)),
    ("recencyDays", Int(1, 365)),
    ("timeOfDay", Enum(TIME_OF_DAY)),
    ("tagShapes", Bool),
    ("grain", Bool),
    ("vignette", Bool),
    ("overlay", Enum(OVERLAYS)),
    ("labelSource", Enum(LABEL_SOURCES)),
    ("focusNote", Str(300)),
    ("focusHops", Int(1, 4)),
    ("rotate", Rotate),
    ("maxFps", Int(0, 60)),
    ("idleFps", Int(0, 30)),
];

/// Web-only keys still in `VALIDATORS`: checked so accept/reject matches, then dropped.
const DROPPED: &[(&str, Rule)] = &[("refreshMs", Int(1000, 60_000)), ("fastHash", Bool), ("renderer", Enum(RENDERERS))];

/// `port` is validated only for whole files (it is not in `VALIDATORS`, so a patch
/// rejects it as unknown).
const PORT: Rule = Int(1, 65_535);

fn rule(key: &str) -> Option<(Rule, bool)> {
    let find = |t: &[(&str, Rule)]| t.iter().find(|(k, _)| *k == key).map(|(_, r)| *r);
    find(RULES).map(|r| (r, true)).or_else(|| find(DROPPED).map(|r| (r, false)))
}

/// Like `sanitizePersistedConfig`, but forgiving: an invalid key is dropped (and
/// reported) instead of rejecting the whole file, so one bad hand edit doesn't cost every
/// other setting. Unknown keys are ignored, as on the web.
pub fn sanitize_file(raw: &Value) -> Result<Sanitized, ConfigError> {
    let obj = raw.as_object().ok_or(ConfigError::NotObject)?;
    let mut clean = Map::new();
    let mut issues = Vec::new();
    let mut vault_path = None;
    for (key, value) in obj {
        let checked = match key.as_str() {
            // Existence is not checked here: a vault on an unmounted drive is the app's
            // problem to report, not a reason to throw the setting away.
            "vaultPath" => match value.as_str() {
                Some(s) if !js_trim(s).is_empty() => {
                    vault_path = Some(s.to_string());
                    Ok(())
                }
                _ => Err("vaultPath must be a non-empty string".to_string()),
            },
            "port" => check(PORT, value, key).map(drop),
            _ => match rule(key) {
                Some((r, keep)) => check(r, value, key).map(|v| {
                    if keep {
                        clean.insert(key.clone(), v);
                    }
                }),
                None => Ok(()),
            },
        };
        if let Err(message) = checked {
            issues.push(Issue { key: key.clone(), message });
        }
    }
    let mut config = build(Config::default(), clean)?;
    config.vault_path = vault_path;
    Ok(Sanitized { config, issues })
}

/// Like `sanitizeConfigPatch`: any unknown or invalid key rejects the whole patch.
/// Returns the cleaned values of the keys the native app keeps.
pub fn sanitize_patch(raw: &Value) -> Result<Map<String, Value>, ConfigError> {
    let obj = raw.as_object().ok_or(ConfigError::NotObject)?;
    let mut clean = Map::new();
    for (key, value) in obj {
        let (r, keep) = rule(key).ok_or_else(|| ConfigError::UnknownKey(key.clone()))?;
        let v = check(r, value, key).map_err(|message| ConfigError::Invalid { key: key.clone(), message })?;
        if keep {
            clean.insert(key.clone(), v);
        }
    }
    Ok(clean)
}

/// Overlays cleaned values onto `base`. Validators guarantee the shapes, so a failure
/// here is a bug in this module, surfaced rather than hidden.
pub(super) fn build(base: Config, clean: Map<String, Value>) -> Result<Config, ConfigError> {
    let vault_path = base.vault_path.clone();
    let Value::Object(mut map) = serde_json::to_value(base).map_err(ConfigError::Json)? else {
        unreachable!("Config serializes to an object")
    };
    map.extend(clean);
    let mut config: Config = serde_json::from_value(Value::Object(map)).map_err(ConfigError::Json)?;
    config.vault_path = vault_path;
    Ok(config)
}

fn check(rule: Rule, v: &Value, key: &str) -> Result<Value, String> {
    match rule {
        Hex => hex(v, key),
        Bool => match v {
            Value::Bool(_) => Ok(v.clone()),
            _ => Err(format!("{key} must be a boolean")),
        },
        Int(min, max) => int(v, key, min, max),
        Num(min, max) => num(v, key, min, max),
        Enum(set) => one_of(v, key, set),
        Str(max) => match v.as_str() {
            // JS measures length in UTF-16 units, before trimming.
            Some(s) if s.encode_utf16().count() <= max => Ok(Value::from(js_trim(s))),
            _ => Err(format!("{key} must be a string of at most {max} characters")),
        },
        IgnorePaths => ignore_paths(v, key),
        TagColors => tag_colors(v),
        LabelStyle => label_style(v),
        SafeArea => safe_area(v, key),
        Rotate => rotate(v, key),
    }
}

fn hex(v: &Value, key: &str) -> Result<Value, String> {
    match v.as_str() {
        Some(s) if is_hex_color(s) => Ok(v.clone()),
        _ => Err(format!("{key} must be a 6-digit hex color like #7c5cff")),
    }
}

/// `/^#[0-9a-fA-F]{6}$/`; JS `$` without the `m` flag allows no trailing newline.
pub fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s.bytes().skip(1).all(|b| b.is_ascii_hexdigit())
}

fn int(v: &Value, key: &str, min: i64, max: i64) -> Result<Value, String> {
    // `Number.isInteger` accepts 5.0 and 1e3; normalise them to plain integers.
    match v.as_f64() {
        Some(f) if f.fract() == 0.0 && f >= min as f64 && f <= max as f64 => Ok(Value::from(f as i64)),
        _ => Err(format!("{key} must be an integer between {min} and {max}")),
    }
}

fn num(v: &Value, key: &str, min: f64, max: f64) -> Result<Value, String> {
    match v.as_f64() {
        Some(f) if f >= min && f <= max => Ok(v.clone()),
        _ => Err(format!("{key} must be a number between {min} and {max}")),
    }
}

fn one_of(v: &Value, key: &str, set: &[&str]) -> Result<Value, String> {
    match v.as_str() {
        Some(s) if set.contains(&s) => Ok(v.clone()),
        _ => Err(format!("{key} must be one of: {}", set.join(", "))),
    }
}

fn ignore_paths(v: &Value, key: &str) -> Result<Value, String> {
    let list = v.as_array().ok_or_else(|| format!("{key} must be an array of strings"))?;
    for (i, p) in list.iter().enumerate() {
        match p.as_str() {
            Some(s) if !js_trim(s).is_empty() => {}
            _ => return Err(format!("{key}[{i}] must be a non-empty string")),
        }
    }
    Ok(v.clone())
}

fn tag_colors(v: &Value) -> Result<Value, String> {
    let map = v.as_object().ok_or("tagColors must be an object of tag -> hex color")?;
    for (tag, color) in map {
        if js_trim(tag).is_empty() {
            return Err("tagColors keys must be non-empty strings".into());
        }
        hex(color, &format!("tagColors.{tag}"))?;
    }
    Ok(v.clone())
}

/// Unknown sub-keys are dropped silently, as on the web.
fn label_style(v: &Value) -> Result<Value, String> {
    let map = v.as_object().ok_or("labelStyle must be an object")?;
    let mut out = Map::new();
    if let Some(m) = map.get("mode") {
        out.insert("mode".into(), one_of(m, "labelStyle.mode", LABEL_STYLE_MODES)?);
    }
    if let Some(a) = map.get("backgroundAlpha") {
        out.insert("backgroundAlpha".into(), num(a, "labelStyle.backgroundAlpha", 0.0, 0.9)?);
    }
    if let Some(g) = map.get("glowColor") {
        match g.as_str() {
            Some("accent" | "node") => {}
            Some(s) if is_hex_color(s) => {}
            _ => return Err(r#"labelStyle.glowColor must be "accent", "node", or a hex color"#.into()),
        }
        out.insert("glowColor".into(), g.clone());
    }
    if let Some(c) = map.get("chromaticSplit") {
        out.insert("chromaticSplit".into(), check(Bool, c, "labelStyle.chromaticSplit")?);
    }
    if let Some(f) = map.get("fontStyle") {
        out.insert("fontStyle".into(), one_of(f, "labelStyle.fontStyle", LABEL_FONTS)?);
    }
    Ok(Value::Object(out))
}

const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

fn safe_area(v: &Value, key: &str) -> Result<Value, String> {
    let map = v.as_object().ok_or_else(|| format!("{key} must be an object with top, right, bottom, left"))?;
    let mut out = Map::new();
    for side in SIDES {
        let value = map.get(side).cloned().unwrap_or(Value::from(0));
        out.insert(side.into(), int(&value, &format!("{key}.{side}"), 0, 1000)?);
    }
    if let Some(k) = map.keys().find(|k| !SIDES.contains(&k.as_str())) {
        return Err(format!("{key}.{k} is not a known side"));
    }
    Ok(Value::Object(out))
}

fn rotate(v: &Value, key: &str) -> Result<Value, String> {
    let map = v
        .as_object()
        .ok_or_else(|| format!(r#"{key} must be an object like {{ "themes": [], "everyMinutes": 60 }}"#))?;
    let themes = match map.get("themes") {
        None => Vec::new(),
        Some(Value::Array(list)) if list.len() <= 32 => list
            .iter()
            .enumerate()
            .map(|(i, t)| one_of(t, &format!("{key}.themes[{i}]"), THEMES))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(format!("{key}.themes must be an array of up to 32 theme ids")),
    };
    let every = match map.get("everyMinutes") {
        None => Value::from(60),
        Some(m) => int(m, &format!("{key}.everyMinutes"), 1, 1440)?,
    };
    if let Some(k) = map.keys().find(|k| *k != "themes" && *k != "everyMinutes") {
        return Err(format!("{key}.{k} is not a known field"));
    }
    Ok(serde_json::json!({ "themes": themes, "everyMinutes": every }))
}

/// `String.prototype.trim`: JS whitespace differs from Rust's (`U+FEFF` yes, `U+0085` no).
pub(super) fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| {
        matches!(c, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}')
            || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
    })
}

#[cfg(test)]
pub(super) fn enum_sets() -> Vec<(&'static str, &'static [&'static str])> {
    RULES
        .iter()
        .filter_map(|(k, r)| match r {
            Enum(set) => Some((*k, *set)),
            _ => None,
        })
        .collect()
}
