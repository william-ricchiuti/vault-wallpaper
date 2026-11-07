//! The settings page's backend: `parser.js`'s `createRequestHandler`, answered in-process.
//!
//! Every request from the page arrives here through the `vw://app` custom scheme. Routes,
//! status codes, error bodies and file formats match the web version, so `settings.html`
//! runs almost unchanged:
//!
//! | Request | Answer |
//! |---|---|
//! | `GET /`, `/settings.html` | the page |
//! | `GET /presets.json`, `/presets/<id>.jpg` | built-in presets and their thumbnails |
//! | `GET /themes/<id>.jpg` | native theme previews |
//! | `GET /api/config` | the config on disk, minus `vaultPath` |
//! | `POST /api/config` | validate a patch (all or nothing), save atomically, return the new config |
//! | `GET /api/defaults` | the default config, with motion off (as for a new install) |
//! | `GET /api/themes` | `[{ id, name, description, dark, native?, preview? }]` for each of [`config::THEMES`], native themes first |
//! | `GET /api/tags` | `{ tag: noteCount }`, supplied by the app ([`Backend::tags`]) |
//! | `GET/POST/DELETE /api/custom-presets` | the `custom-presets.json` sidecar |
//!
//! Not served: `/events` (SSE; the window pushes changes with `evaluate_script` instead),
//! `/api/state`, `/api/note-of-hour`, `/graph.json` and the wallpaper page itself.

use std::borrow::Cow;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use vw_core::config::{self, Config, ConfigError};
use wry::http::{Method, Request, Response, StatusCode, header};

const SETTINGS_HTML: &[u8] = include_bytes!("../../assets/settings.html");
const PRESETS_JSON: &[u8] = include_bytes!("../../assets/presets.json");
const THEMES_JSON: &str = include_str!("../../assets/themes.json");

macro_rules! thumbnails {
    ($($id:literal),* $(,)?) => {
        &[$(($id, include_bytes!(concat!("../../assets/presets/", $id, ".jpg")))),*]
    };
}

/// Preset thumbnails, 640x360 JPEG (converted from the web version's PNGs to keep the
/// binary small).
const THUMBNAILS: &[(&str, &[u8])] = thumbnails!(
    "abyss",
    "ambient",
    "aurora",
    "bauhaus",
    "blueprint",
    "botanical",
    "celestial",
    "confetti",
    "constellation",
    "contrast",
    "crystalline",
    "dense",
    "ink",
    "led",
    "library",
    "metro",
    "mist",
    "mycelium",
    "neon",
    "parchment",
    "phosphor",
    "pixel",
    "plain",
    "riso",
    "sketch",
    "stained-glass",
    "stitch",
    "survey",
    "synapse",
    "synthwave",
    "territories",
    "topographic",
    "vapor",
    "wash",
);

/// Native theme previews, 640x416 JPEG: the real-vault renders from
/// `cargo run --release -p vw-render --example render_vault -- <vault> out.png --theme <id> --size 1512 982 1`,
/// downscaled with `sips -Z 640 -s format jpeg -s formatOptions 78`. A theme listed in
/// `themes.json` without an image here gets no `preview`.
const THEME_PREVIEWS: &[(&str, &[u8])] = &[
    ("default", include_bytes!("../../assets/themes/default.jpg")),
    ("night-sky", include_bytes!("../../assets/themes/night-sky.jpg")),
    ("aurora-lite", include_bytes!("../../assets/themes/aurora-lite.jpg")),
    ("ink-paper", include_bytes!("../../assets/themes/ink-paper.jpg")),
    ("pulse", include_bytes!("../../assets/themes/pulse.jpg")),
];

/// What the handler reads and writes. Shared between the web view's protocol handler and
/// [`SettingsWindow`](super::SettingsWindow).
pub struct Backend {
    pub config_path: PathBuf,
    /// Tag name and note count, from the live graph. Empty until the app supplies them.
    pub tags: Vec<(String, usize)>,
}

type Reply = Response<Cow<'static, [u8]>>;

impl Backend {
    pub fn new(config_path: PathBuf) -> Self {
        Backend {
            config_path,
            tags: Vec::new(),
        }
    }

    /// `custom-presets.json` next to the config file, as on the web. For the default
    /// config that is `~/Library/Application Support/Vault Wallpaper/custom-presets.json`.
    pub fn presets_path(&self) -> PathBuf {
        let dir = self
            .config_path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        dir.join("custom-presets.json")
    }

    pub fn handle(&self, req: &Request<Vec<u8>>) -> Reply {
        let path = req.uri().path();
        let method = req.method();
        if method == Method::OPTIONS {
            return empty(StatusCode::NO_CONTENT);
        }
        match (method, path) {
            (&Method::GET, "/" | "/settings.html") => {
                asset("text/html; charset=utf-8", SETTINGS_HTML)
            }
            (&Method::GET, "/presets.json") => asset("application/json", PRESETS_JSON),
            (_, "/favicon.ico") => empty(StatusCode::NO_CONTENT),
            (&Method::GET, "/api/config") => match self.read_config() {
                Ok(c) => ok_json(&safe_config(&c)),
                Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
            },
            (&Method::POST, "/api/config") => self.post_config(req),
            (&Method::GET, "/api/defaults") => ok_json(&safe_config(&defaults())),
            (&Method::GET, "/api/themes") => ok_json(&themes()),
            (&Method::GET, "/api/tags") => ok_json(&self.tags_json()),
            (_, "/api/custom-presets") => self.custom_presets(req),
            (&Method::GET, p) => {
                let image = |dir: &str, list: &[(&str, &'static [u8])]| {
                    let id = p.strip_prefix(dir)?.strip_suffix(".jpg")?;
                    list.iter().find(|(n, _)| *n == id).map(|(_, b)| *b)
                };
                match image("/presets/", THUMBNAILS).or_else(|| image("/themes/", THEME_PREVIEWS)) {
                    Some(bytes) => asset("image/jpeg", bytes),
                    None => not_found(),
                }
            }
            _ => not_found(),
        }
    }

    fn read_config(&self) -> Result<Config, ConfigError> {
        config::load(&self.config_path).map(|s| s.config)
    }

    /// Re-reads the file on every save, so a hand edit made while the window is open is
    /// kept rather than overwritten by a stale copy.
    fn post_config(&self, req: &Request<Vec<u8>>) -> Reply {
        let body = match json_body(req) {
            Ok(v) => v,
            Err((status, message)) => return error(status, &message),
        };
        let mut cfg = match self.read_config() {
            Ok(c) => c,
            // Don't replace a file we can't read (e.g. a broken hand edit) with defaults.
            Err(e) => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("can't read the config file: {e}"),
                );
            }
        };
        if let Err(e) = cfg.apply_patch(&body) {
            return error(StatusCode::BAD_REQUEST, &patch_message(&e));
        }
        if let Err(e) = config::save(&self.config_path, &cfg) {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("save failed: {e}"),
            );
        }
        ok_json(&safe_config(&cfg))
    }

    fn tags_json(&self) -> Value {
        // TODO(app): the tag list comes from the live vault graph; the app supplies it
        // with `SettingsWindow::set_tags`. Until then this is `{}` and the page says
        // "No tags found in vault".
        Value::Object(
            self.tags
                .iter()
                .map(|(t, n)| (t.clone(), Value::from(*n)))
                .collect(),
        )
    }

    fn custom_presets(&self, req: &Request<Vec<u8>>) -> Reply {
        let path = self.presets_path();
        match *req.method() {
            Method::GET => ok_json(&Value::Array(read_presets(&path))),
            Method::POST => {
                let body = match json_body(req) {
                    Ok(v) => v,
                    Err((status, message)) => return error(status, &message),
                };
                let name = match body.get("name") {
                    Some(Value::String(s)) if s.encode_utf16().count() <= 60 => {
                        s.trim().to_string()
                    }
                    _ => {
                        return error(
                            StatusCode::BAD_REQUEST,
                            "name must be a string of at most 60 characters",
                        );
                    }
                };
                if name.is_empty() {
                    return error(StatusCode::BAD_REQUEST, "name is required");
                }
                let patch = match config::sanitize_patch(body.get("config").unwrap_or(&json!({}))) {
                    Ok(p) => p,
                    Err(e) => return error(StatusCode::BAD_REQUEST, &patch_message(&e)),
                };
                let current = self.read_config().unwrap_or_default();
                let pick = |key: &str, fallback: &str| {
                    patch
                        .get(key)
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(fallback)
                        .to_string()
                };
                let entry = json!({
                    "name": name,
                    "description": "Custom preset",
                    "accent": pick("accent", &current.accent),
                    "background": pick("background", &current.background),
                    "custom": true,
                    "config": Value::Object(patch.clone()),
                });
                let mut list = read_presets(&path);
                list.retain(|p| p.get("name").and_then(Value::as_str) != Some(name.as_str()));
                list.push(entry);
                self.write_presets(&path, list)
            }
            Method::DELETE => {
                let name = query_param(req.uri().query().unwrap_or(""), "name").unwrap_or_default();
                let mut list = read_presets(&path);
                list.retain(|p| p.get("name").and_then(Value::as_str) != Some(name.as_str()));
                self.write_presets(&path, list)
            }
            _ => error(StatusCode::METHOD_NOT_ALLOWED, "method not allowed"),
        }
    }

    fn write_presets(&self, path: &Path, list: Vec<Value>) -> Reply {
        let list = Value::Array(list);
        match write_atomic(path, &list) {
            Ok(()) => ok_json(&list),
            Err(e) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        }
    }
}

/// `buildSafeConfig`: everything but `vaultPath` (and `port`, which the native config
/// doesn't have).
fn safe_config(cfg: &Config) -> Value {
    let mut v = serde_json::to_value(cfg).unwrap_or(Value::Object(Map::new()));
    if let Some(o) = v.as_object_mut() {
        o.remove("vaultPath");
    }
    v
}

/// The default config for "Reset to defaults": `parser.js`'s `DEFAULTS`, except that
/// motion starts off, as it does for a new install (lightness first).
fn defaults() -> Config {
    Config {
        motion_mode: config::MotionMode::Still,
        ..Config::default()
    }
}

/// The theme list in `themes.json` order (native themes first, then the web version's,
/// whose names and descriptions come from its `themes/*.js` headers), limited to the ids
/// the native config accepts. Any accepted id missing from the file is appended bare.
/// The page shows the native ones; web ids only render the default look natively.
fn themes() -> Value {
    let all: Vec<Value> = serde_json::from_str(THEMES_JSON).unwrap_or_default();
    let id_of = |t: &Value| t.get("id").and_then(Value::as_str).map(str::to_owned);
    let mut out: Vec<Value> = all
        .into_iter()
        .filter(|t| id_of(t).is_some_and(|id| config::THEMES.contains(&id.as_str())))
        .map(|mut t| {
            let id = id_of(&t).unwrap_or_default();
            if let Some(o) = t.as_object_mut()
                && !THEME_PREVIEWS.iter().any(|(n, _)| *n == id)
            {
                o.remove("preview");
            }
            t
        })
        .collect();
    for id in config::THEMES {
        if !out.iter().any(|t| id_of(t).as_deref() == Some(id)) {
            out.push(json!({ "id": id, "name": id, "description": "", "dark": true }));
        }
    }
    Value::Array(out)
}

fn read_presets(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| match v {
            Value::Array(a) => Some(a),
            _ => None,
        })
        .unwrap_or_default()
}

/// `writeConfigFile`: 2-space JSON, temp file then rename.
fn write_atomic(path: &Path, value: &Value) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let text = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    let written = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// A JSON request body. Like the web version: 415 unless the header is exactly
/// `application/json`, 400 if the body doesn't parse.
fn json_body(req: &Request<Vec<u8>>) -> Result<Value, (StatusCode, String)> {
    let ct = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    if ct != Some("application/json") {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json".into(),
        ));
    }
    serde_json::from_slice(req.body())
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid JSON: {e}")))
}

fn patch_message(e: &ConfigError) -> String {
    match e {
        ConfigError::NotObject => "request body must be a JSON object".to_string(),
        other => other.to_string(),
    }
}

/// `URLSearchParams.get`: the first `key=value`, percent- and `+`-decoded.
fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (decode(k) == key).then(|| decode(v))
    })
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
                {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn respond(status: StatusCode, content_type: &str, body: Cow<'static, [u8]>) -> Reply {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .expect("static headers are valid")
}

fn asset(content_type: &str, bytes: &'static [u8]) -> Reply {
    respond(StatusCode::OK, content_type, Cow::Borrowed(bytes))
}

fn ok_json(v: &Value) -> Reply {
    respond(
        StatusCode::OK,
        "application/json",
        Cow::Owned(serde_json::to_vec(v).unwrap_or_default()),
    )
}

fn error(status: StatusCode, message: &str) -> Reply {
    respond(
        status,
        "application/json",
        Cow::Owned(serde_json::to_vec(&json!({ "error": message })).unwrap_or_default()),
    )
}

fn empty(status: StatusCode) -> Reply {
    respond(status, "text/plain", Cow::Borrowed(&[]))
}

fn not_found() -> Reply {
    respond(
        StatusCode::NOT_FOUND,
        "text/plain",
        Cow::Borrowed(b"Not found"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_backend(tag: &str) -> Backend {
        let dir =
            std::env::temp_dir().join(format!("vw-settings-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Backend::new(dir.join("config.json"))
    }

    fn req(method: &str, uri: &str, body: Option<Value>) -> Request<Vec<u8>> {
        let mut b = Request::builder().method(method).uri(uri);
        if body.is_some() {
            b = b.header("Content-Type", "application/json");
        }
        b.body(
            body.map(|v| serde_json::to_vec(&v).unwrap())
                .unwrap_or_default(),
        )
        .unwrap()
    }

    fn body(r: &Reply) -> Value {
        serde_json::from_slice(r.body()).unwrap()
    }

    #[test]
    fn serves_page_presets_and_thumbnails() {
        let b = temp_backend("assets");
        assert_eq!(
            b.handle(&req("GET", "vw://app/settings.html", None))
                .status(),
            200
        );
        let presets = body(&b.handle(&req("GET", "vw://app/presets.json", None)));
        for p in presets.as_array().unwrap() {
            let thumb = p["thumbnail"].as_str().unwrap();
            let r = b.handle(&req("GET", &format!("vw://app{thumb}"), None));
            assert_eq!(r.status(), 200, "{thumb}");
        }
        assert_eq!(
            b.handle(&req("GET", "vw://app/presets/nope.jpg", None))
                .status(),
            404
        );
        assert_eq!(
            b.handle(&req("GET", "vw://app/graph.json", None)).status(),
            404
        );
    }

    #[test]
    fn config_round_trip_and_validation() {
        let b = temp_backend("config");
        let cfg = body(&b.handle(&req("GET", "vw://app/api/config", None)));
        assert!(cfg.get("vaultPath").is_none());
        assert_eq!(cfg["accent"], Config::default().accent);

        let r = b.handle(&req(
            "POST",
            "vw://app/api/config",
            Some(json!({ "accent": "#123456", "particles": true })),
        ));
        assert_eq!(r.status(), 200);
        assert_eq!(body(&r)["accent"], "#123456");
        let on_disk = config::load(&b.config_path).unwrap().config;
        assert_eq!(on_disk.accent, "#123456");
        assert!(on_disk.particles);

        let r = b.handle(&req(
            "POST",
            "vw://app/api/config",
            Some(json!({ "accent": "red", "particles": false })),
        ));
        assert_eq!(r.status(), 400);
        assert!(body(&r)["error"].as_str().unwrap().contains("hex"));
        let r = b.handle(&req(
            "POST",
            "vw://app/api/config",
            Some(json!({ "bogus": 1 })),
        ));
        assert_eq!(body(&r)["error"], "unknown config key: bogus");
        assert!(
            config::load(&b.config_path).unwrap().config.particles,
            "rejected patch must not save"
        );

        let mut no_ct = req("POST", "vw://app/api/config", Some(json!({})));
        no_ct.headers_mut().remove("Content-Type");
        assert_eq!(b.handle(&no_ct).status(), 415);
        let _ = fs::remove_dir_all(b.config_path.parent().unwrap());
    }

    #[test]
    fn every_builtin_preset_applies() {
        let b = temp_backend("presets-apply");
        let presets = body(&b.handle(&req("GET", "vw://app/presets.json", None)));
        for p in presets.as_array().unwrap() {
            let mut patch = p["config"].clone();
            patch.as_object_mut().unwrap().remove("tagColors");
            let r = b.handle(&req("POST", "vw://app/api/config", Some(patch)));
            assert_eq!(
                r.status(),
                200,
                "{}: {}",
                p["name"],
                String::from_utf8_lossy(r.body())
            );
        }
        let _ = fs::remove_dir_all(b.config_path.parent().unwrap());
    }

    #[test]
    fn defaults_reset_is_accepted() {
        let b = temp_backend("defaults");
        let defaults = body(&b.handle(&req("GET", "vw://app/api/defaults", None)));
        assert_eq!(defaults["motionMode"], "still", "reset keeps motion off");
        assert_eq!(
            b.handle(&req("POST", "vw://app/api/config", Some(defaults)))
                .status(),
            200
        );
        let _ = fs::remove_dir_all(b.config_path.parent().unwrap());
    }

    #[test]
    fn custom_presets_crud() {
        let b = temp_backend("custom");
        assert_eq!(
            body(&b.handle(&req("GET", "vw://app/api/custom-presets", None))),
            json!([])
        );
        let r = b.handle(&req(
            "POST",
            "vw://app/api/custom-presets",
            Some(json!({ "name": " My preset ", "config": { "accent": "#abcdef" } })),
        ));
        assert_eq!(r.status(), 200);
        let list = body(&r);
        assert_eq!(list[0]["name"], "My preset");
        assert_eq!(list[0]["accent"], "#abcdef");
        assert_eq!(list[0]["background"], Config::default().background);
        assert_eq!(list[0]["custom"], true);
        let r = b.handle(&req(
            "POST",
            "vw://app/api/custom-presets",
            Some(json!({ "name": "", "config": {} })),
        ));
        assert_eq!(body(&r)["error"], "name is required");
        let r = b.handle(&req(
            "POST",
            "vw://app/api/custom-presets",
            Some(json!({ "name": "x", "config": { "nope": 1 } })),
        ));
        assert_eq!(r.status(), 400);

        let r = b.handle(&req(
            "DELETE",
            "vw://app/api/custom-presets?name=My%20preset",
            None,
        ));
        assert_eq!(body(&r), json!([]));
        assert_eq!(read_presets(&b.presets_path()), Vec::<Value>::new());
        let _ = fs::remove_dir_all(b.config_path.parent().unwrap());
    }

    #[test]
    fn themes_cover_the_config_ids() {
        let t = themes();
        let list = t.as_array().unwrap();
        let mut ids: Vec<&str> = list.iter().map(|t| t["id"].as_str().unwrap()).collect();
        // Native first, the default look leading.
        assert_eq!(&ids[..5], ["default", "night-sky", "pulse", "aurora-lite", "ink-paper"]);
        assert!(list[..5].iter().all(|t| t["native"] == true));
        assert!(list[5..].iter().all(|t| t.get("native").is_none()));
        assert!(list.iter().all(|t| t["description"].as_str().is_some_and(|d| !d.is_empty())));
        ids.sort_unstable();
        let mut want = config::THEMES.to_vec();
        want.sort_unstable();
        assert_eq!(ids, want);
    }

    #[test]
    fn theme_previews_are_served() {
        let b = temp_backend("previews");
        for t in themes().as_array().unwrap() {
            match t["preview"].as_str() {
                Some(p) => {
                    let r = b.handle(&req("GET", &format!("vw://app{p}"), None));
                    assert_eq!(r.status(), 200, "{p}");
                    assert_eq!(&r.body()[..2], [0xff, 0xd8], "{p} is a JPEG");
                }
                // Every native theme has a preview.
                None => assert!(t.get("native").is_none(), "{}", t["id"]),
            }
        }
        assert_eq!(
            b.handle(&req("GET", "vw://app/themes/nope.jpg", None)).status(),
            404
        );
    }

    #[test]
    fn tags_and_query_decoding() {
        let mut b = temp_backend("tags");
        assert_eq!(
            body(&b.handle(&req("GET", "vw://app/api/tags", None))),
            json!({})
        );
        b.tags = vec![("work".into(), 3)];
        assert_eq!(
            body(&b.handle(&req("GET", "vw://app/api/tags", None))),
            json!({ "work": 3 })
        );
        assert_eq!(
            query_param("a=1&name=caf%C3%A9+bar", "name").as_deref(),
            Some("café bar")
        );
        assert_eq!(query_param("name=100%", "name").as_deref(), Some("100%"));
    }
}
