//! "Import Web Version Settings…": reads the web version's `config.json` (and the
//! `custom-presets.json` next to it) into this app's config.
//!
//! The web version keeps its config next to wherever it runs: `config.json` in a git
//! checkout, or `obsidian-live-wallpaper.config.json` in the folder `npx` ran from. There
//! is no fixed path to look in, so the user picks the file.

use std::fs;
use std::path::{Path, PathBuf};

use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use serde_json::Value;
use vw_core::config::{self, Config};

/// Asks for the old file, imports it and saves over `config_path`. Returns `true` if
/// the config was written (the config watcher then restyles and rescans).
pub fn run(config_path: &Path, current: &Config) -> bool {
    let Some(file) = FileDialog::new()
        .set_title("Choose the web version's config.json")
        .add_filter("JSON", &["json"])
        .pick_file()
    else {
        return false;
    };
    let imported = match config::import_legacy(&file) {
        Ok(s) => s,
        Err(e) => {
            message(
                MessageLevel::Error,
                "Could not import that file",
                &format!("{}: {e}", file.display()),
            );
            return false;
        }
    };
    if config_path.exists() {
        let answer = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title("Replace your settings?")
            .set_description(format!(
                "Your current settings will be replaced by the ones in {}.",
                file.display()
            ))
            .set_buttons(MessageButtons::OkCancel)
            .show();
        if answer != MessageDialogResult::Ok {
            return false;
        }
    }
    let cfg = merge(imported.config, current);
    if let Err(e) = config::save(config_path, &cfg) {
        message(
            MessageLevel::Error,
            "Could not save the settings",
            &e.to_string(),
        );
        return false;
    }
    let mut presets = import_presets(&file, config_path);
    if presets.is_none()
        && let Some(old) = blocked_presets(&file)
    {
        // In the App Sandbox only the picked file can be read, not the presets next to
        // it: one more pick grants that file too.
        let mut dialog = FileDialog::new()
            .set_title("Also import your custom presets: select custom-presets.json and click Open")
            .add_filter("JSON", &["json"]);
        if let Some(dir) = old.parent() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(picked) = dialog.pick_file() {
            presets = import_presets_file(&picked, config_path);
        }
    }
    let mut notes = Vec::new();
    if let Some(n) = presets.filter(|n| *n > 0) {
        notes.push(format!("{n} custom preset(s) added."));
    }
    if !imported.issues.is_empty() {
        let keys: Vec<&str> = imported.issues.iter().map(|i| i.key.as_str()).collect();
        notes.push(format!(
            "Skipped (invalid or web-only): {}.",
            keys.join(", ")
        ));
    }
    if cfg
        .vault_path
        .as_deref()
        .is_none_or(|p| !Path::new(p).is_dir())
    {
        notes.push(
            "The vault folder in that file wasn't found; use Choose Vault… to pick it.".into(),
        );
    }
    message(MessageLevel::Info, "Settings imported", &notes.join("\n"));
    true
}

/// The imported config, keeping the current vault when the old file has none.
pub fn merge(mut imported: Config, current: &Config) -> Config {
    if imported.vault_path.as_deref().is_none_or(str::is_empty) {
        imported.vault_path = current.vault_path.clone();
    }
    imported
}

/// Adds the old `custom-presets.json` (if any) to ours; a preset whose name we already
/// have keeps our version. Returns how many were added, `None` if there was nothing to read.
fn import_presets(old_config: &Path, config_path: &Path) -> Option<usize> {
    import_presets_file(&old_config.parent()?.join("custom-presets.json"), config_path)
}

/// The web version's `custom-presets.json` next to `old_config`, if it is there but can't
/// be read (the sandbox allows looking at it, not reading it).
fn blocked_presets(old_config: &Path) -> Option<PathBuf> {
    let old = old_config.parent()?.join("custom-presets.json");
    let denied = old.is_file() && fs::File::open(&old).is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied);
    denied.then_some(old)
}

fn import_presets_file(old: &Path, config_path: &Path) -> Option<usize> {
    let ours = presets_path(config_path);
    if old == ours {
        return None;
    }
    let incoming = read_array(old)?;
    let mut list = read_array(&ours).unwrap_or_default();
    let added = merge_presets(&mut list, incoming);
    if added > 0 {
        let mut text = serde_json::to_string_pretty(&Value::Array(list)).ok()?;
        text.push('\n');
        let tmp = ours.with_extension("json.tmp");
        fs::write(&tmp, text)
            .and_then(|()| fs::rename(&tmp, &ours))
            .ok()?;
    }
    Some(added)
}

fn presets_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .join("custom-presets.json")
}

fn read_array(path: &Path) -> Option<Vec<Value>> {
    match serde_json::from_str(&fs::read_to_string(path).ok()?).ok()? {
        Value::Array(a) => Some(a),
        _ => None,
    }
}

/// Appends each named preset from `incoming` that `list` doesn't have yet.
pub fn merge_presets(list: &mut Vec<Value>, incoming: Vec<Value>) -> usize {
    let mut added = 0;
    for p in incoming {
        let Some(name) = p.get("name").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        if list
            .iter()
            .any(|q| q.get("name").and_then(Value::as_str) == Some(name.as_str()))
        {
            continue;
        }
        list.push(p);
        added += 1;
    }
    added
}

fn message(level: MessageLevel, title: &str, text: &str) {
    MessageDialog::new()
        .set_level(level)
        .set_title(title)
        .set_description(text)
        .set_buttons(MessageButtons::Ok)
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_current_vault_when_import_has_none() {
        let current = Config {
            vault_path: Some("/v".into()),
            ..Config::default()
        };
        let imported = config::Config::from_json(r##"{"theme":"aurora","accent":"#ff0000"}"##)
            .unwrap()
            .config;
        let cfg = merge(imported, &current);
        assert_eq!(cfg.vault_path.as_deref(), Some("/v"));
        assert_eq!(cfg.theme, "aurora");
    }

    #[test]
    fn imported_vault_wins() {
        let current = Config {
            vault_path: Some("/v".into()),
            ..Config::default()
        };
        let imported = config::Config::from_json(r#"{"vaultPath":"/old"}"#)
            .unwrap()
            .config;
        assert_eq!(
            merge(imported, &current).vault_path.as_deref(),
            Some("/old")
        );
    }

    #[test]
    fn web_only_keys_are_reported_not_fatal() {
        let s =
            config::Config::from_json(r#"{"port":3000,"refreshMs":500,"theme":"riso"}"#).unwrap();
        assert_eq!(s.config.theme, "riso");
    }

    #[test]
    fn presets_merge_by_name() {
        let mut list = vec![json!({"name": "Mine", "accent": "#000000"})];
        let added = merge_presets(
            &mut list,
            vec![
                json!({"name": "Mine", "accent": "#ffffff"}),
                json!({"name": "Old"}),
                json!({"x": 1}),
            ],
        );
        assert_eq!(added, 1);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["accent"], "#000000");
    }

    #[test]
    fn import_file_end_to_end() {
        let dir = std::env::temp_dir().join(format!("vw-import-{}", std::process::id()));
        let (old, new) = (dir.join("web"), dir.join("native"));
        fs::create_dir_all(&old).unwrap();
        fs::write(
            old.join("config.json"),
            r#"{"vaultPath":"/notes","theme":"wash","port":3001}"#,
        )
        .unwrap();
        fs::write(
            old.join("custom-presets.json"),
            r#"[{"name":"Dusk","custom":true,"config":{}}]"#,
        )
        .unwrap();
        let s = config::import_legacy(&old.join("config.json")).unwrap();
        let cfg = merge(s.config, &Config::default());
        config::save(&new.join("config.json"), &cfg).unwrap();
        assert_eq!(
            import_presets(&old.join("config.json"), &new.join("config.json")),
            Some(1)
        );
        // Readable (not sandboxed) or missing: nothing to ask for.
        assert_eq!(blocked_presets(&old.join("config.json")), None);
        assert_eq!(blocked_presets(&dir.join("nowhere/config.json")), None);
        let back = config::load(&new.join("config.json")).unwrap().config;
        assert_eq!(back.vault_path.as_deref(), Some("/notes"));
        assert_eq!(back.theme, "wash");
        assert_eq!(
            read_array(&new.join("custom-presets.json")).unwrap().len(),
            1
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
