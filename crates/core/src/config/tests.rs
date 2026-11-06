use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;

fn file(v: Value) -> Sanitized {
    sanitize_file(&v).unwrap()
}

fn dropped(v: Value) -> Vec<String> {
    file(v).issues.into_iter().map(|i| i.key).collect()
}

fn patch_ok(v: Value) -> bool {
    sanitize_patch(&v).is_ok()
}

#[test]
fn defaults_round_trip_through_json() {
    let text = serde_json::to_string(&Config::default()).unwrap();
    let back = Config::from_json(&text).unwrap();
    assert!(back.issues.is_empty());
    assert_eq!(back.config, Config::default());
}

#[test]
fn every_enum_value_is_accepted_and_typed() {
    for (key, set) in validate::enum_sets() {
        for v in set {
            let mut c = Config::default();
            c.apply_patch(&json!({ key: v })).unwrap_or_else(|e| panic!("{key}={v}: {e}"));
            assert_eq!(serde_json::to_value(&c).unwrap()[key], json!(v));
        }
        assert!(!patch_ok(json!({ key: "nope" })), "{key}");
    }
}

#[test]
fn whole_file_drops_only_bad_keys() {
    let s = file(json!({
        "vaultPath": "/v", "accent": "#fff", "zoom": 2, "hubLabelCount": 5.0,
        "port": 0, "refreshMs": 5000, "fastHash": false, "renderer": "webgl", "mystery": 1
    }));
    let keys: Vec<_> = s.issues.iter().map(|i| i.key.as_str()).collect();
    assert_eq!(keys, ["accent", "port"]);
    assert_eq!(s.config.accent, "#7c5cff");
    assert_eq!(s.config.zoom, 2.0);
    assert_eq!(s.config.hub_label_count, 5);
    assert_eq!(s.config.vault_path.as_deref(), Some("/v"));
    // Web-only keys never come back out.
    let out = serde_json::to_value(&s.config).unwrap();
    for k in ["port", "refreshMs", "fastHash", "renderer", "mystery"] {
        assert!(out.get(k).is_none(), "{k}");
    }
}

#[test]
fn file_root_and_vault_path() {
    assert!(matches!(sanitize_file(&json!([])), Err(ConfigError::NotObject)));
    assert!(file(json!({})).config.vault_path.is_none());
    assert_eq!(dropped(json!({ "vaultPath": "  " })), ["vaultPath"]);
    assert_eq!(dropped(json!({ "vaultPath": 3 })), ["vaultPath"]);
}

#[test]
fn patch_is_all_or_nothing() {
    let mut c = Config::default();
    let err = c.apply_patch(&json!({ "zoom": 2, "accent": "red" })).unwrap_err();
    assert!(matches!(err, ConfigError::Invalid { ref key, .. } if key == "accent"));
    assert_eq!(c, Config::default());
    // Not settable from the settings UI, as on the web.
    for k in ["vaultPath", "port", "mystery"] {
        assert!(matches!(sanitize_patch(&json!({ k: 1 })), Err(ConfigError::UnknownKey(_))), "{k}");
    }
    // Web-only keys are validated, then dropped.
    assert!(!patch_ok(json!({ "refreshMs": 10 })));
    let keys = c.apply_patch(&json!({ "refreshMs": 2000, "ignorePaths": ["x"] })).unwrap();
    assert_eq!(keys, ["ignorePaths"]);
    assert_eq!(c.ignore_paths, ["x"]);
}

#[test]
fn scalar_rules_match_the_web() {
    assert!(patch_ok(json!({ "accent": "#7C5CFF" })));
    assert!(!patch_ok(json!({ "accent": "#7c5cffaa" })));
    assert!(!patch_ok(json!({ "accent": "#7c5cf\n" })));
    assert!(!patch_ok(json!({ "linkOpacity": "0.5" })));
    assert!(!patch_ok(json!({ "linkOpacity": 1.01 })));
    assert!(!patch_ok(json!({ "nodeGlow": 1 })));
    assert!(patch_ok(json!({ "maxRenderedNodes": 1e3 })));
    assert!(!patch_ok(json!({ "hubLabelCount": 5.5 })));
    assert!(!patch_ok(json!({ "accent": null })));
}

#[test]
fn nested_rules_match_the_web() {
    let mut c = Config::default();
    c.apply_patch(&json!({ "labelStyle": { "mode": "glow", "glowColor": "node", "extra": 1 } })).unwrap();
    assert_eq!(c.label_style.mode, Some(LabelStyleMode::Glow));
    assert_eq!(serde_json::to_value(&c.label_style).unwrap(), json!({ "mode": "glow", "glowColor": "node" }));
    assert!(!patch_ok(json!({ "labelStyle": { "backgroundAlpha": 0.95 } })));
    assert!(!patch_ok(json!({ "labelStyle": { "mode": null } })));

    c.apply_patch(&json!({ "safeArea": { "top": 24 } })).unwrap();
    assert_eq!(c.safe_area, SafeArea { top: 24, ..SafeArea::default() });
    assert!(!patch_ok(json!({ "safeArea": { "middle": 1 } })));
    assert!(!patch_ok(json!({ "safeArea": { "top": 1001 } })));

    c.apply_patch(&json!({ "rotate": { "themes": ["riso", "wash"] } })).unwrap();
    assert_eq!(c.rotate, Rotate { themes: vec!["riso".into(), "wash".into()], every_minutes: 60 });
    assert!(!patch_ok(json!({ "rotate": { "themes": ["nope"] } })));
    assert!(!patch_ok(json!({ "rotate": { "themes": vec!["default"; 33] } })));
    assert!(!patch_ok(json!({ "rotate": { "everyMinutes": 0 } })));

    assert!(!patch_ok(json!({ "tagColors": { " ": "#ffffff" } })));
    assert!(!patch_ok(json!({ "tagColors": { "a": "red" } })));
    assert!(!patch_ok(json!({ "ignorePaths": ["ok", " "] })));
}

#[test]
fn focus_note_trims_and_counts_like_js() {
    let mut c = Config::default();
    c.apply_patch(&json!({ "focusNote": "\u{feff}  Home \u{85}" })).unwrap();
    assert_eq!(c.focus_note, "Home \u{85}");
    // 150 emoji are 300 UTF-16 units: the limit exactly.
    assert!(patch_ok(json!({ "focusNote": "😀".repeat(150) })));
    assert!(!patch_ok(json!({ "focusNote": "😀".repeat(151) })));
}

#[test]
fn mappings() {
    let mut c = Config::default();
    c.apply_patch(&json!({ "layout": "galaxy", "clusterByTag": false, "ignorePaths": ["t"], "showUnresolvedLinks": false }))
        .unwrap();
    let l = c.layout_options();
    assert_eq!((l.kind, l.cluster_by_tag), (LayoutKind::Galaxy, false));
    let v = c.vault_options();
    assert_eq!(v.ignore_paths, ["t"]);
    assert!(!v.graph.show_unresolved_links);
    assert_eq!(rgb("#7c5cFF"), Some([0x7c, 0x5c, 0xff]));
    assert_eq!(rgb("7c5cff"), None);
}

#[test]
fn load_save_and_import() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/config.json");
    let first = load(&path).unwrap();
    assert_eq!(first.config, Config::default());
    assert!(import_legacy(&path).unwrap_err().is_not_found());

    let mut c = Config { vault_path: Some("/Users/me/Vault".into()), ..Config::default() };
    c.apply_patch(&json!({ "theme": "riso", "tagColors": { "work": "#ff0000" } })).unwrap();
    save(&path, &c).unwrap();
    assert_eq!(load(&path).unwrap().config, c);
    // No temp file left behind.
    assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);

    let legacy = dir.path().join("old.json");
    std::fs::write(&legacy, r#"{"vaultPath":"/v","port":3000,"renderer":"webgl","theme":"gone","grain":false}"#).unwrap();
    let s = import_legacy(&legacy).unwrap();
    assert_eq!(s.issues.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["theme"]);
    assert!(!s.config.grain);
    assert_eq!(s.config.theme, "default");

    std::fs::write(&path, "{ not json").unwrap();
    assert!(matches!(load(&path), Err(ConfigError::Json(_))));
}

#[test]
fn watcher_reloads_hand_edits_but_not_its_own_saves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    save(&path, &Config::default()).unwrap();
    let (tx, rx) = mpsc::channel();
    let w = watch(&path, Config::default(), move |c| {
        let _ = tx.send(c);
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(300));

    std::fs::write(&path, r#"{"zoom": 2, "accent": "bad"}"#).unwrap();
    let got = rx.recv_timeout(Duration::from_secs(5)).expect("hand edit");
    assert_eq!((got.zoom, got.accent.as_str()), (2.0, "#7c5cff"));

    // Broken JSON is ignored; the next good edit still arrives.
    std::fs::write(&path, "{").unwrap();
    std::thread::sleep(Duration::from_millis(400));
    std::fs::write(&path, r#"{"zoom": 3}"#).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).expect("second edit").zoom, 3.0);

    let ours = Config { grain: false, ..Config::default() };
    w.save(&ours).unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(800)).is_err(), "own save echoed back");
}

/// Driven by `parity/config-js.js`: reads cases from `VW_CONFIG_CASES`, writes the Rust
/// verdicts to `VW_CONFIG_OUT`.
#[test]
#[ignore = "run by parity/config-js.js"]
fn parity_dump() {
    let cases: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(std::env::var("VW_CONFIG_CASES").unwrap()).unwrap()).unwrap();
    let out: Vec<Value> = cases
        .iter()
        .map(|case| {
            let input = &case["input"];
            match case["kind"].as_str().unwrap() {
                "file" => match sanitize_file(input) {
                    Ok(s) => json!({
                        "ok": true,
                        "issues": s.issues.iter().map(|i| &i.key).collect::<Vec<_>>(),
                        "value": s.config,
                    }),
                    Err(e) => json!({ "ok": false, "error": e.to_string() }),
                },
                _ => match sanitize_patch(input) {
                    Ok(p) => json!({ "ok": true, "value": p }),
                    Err(e) => json!({ "ok": false, "error": e.to_string() }),
                },
            }
        })
        .collect();
    std::fs::write(std::env::var("VW_CONFIG_OUT").unwrap(), serde_json::to_string(&out).unwrap()).unwrap();
}
