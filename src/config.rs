//! config.json: loading (BOM tolerant), the hand-editable one-line-per-rule
//! save format, atomic writes and the backup copy. Ported from status.py.

use crate::rules::{compile_idle_rules, compile_rules, IdleRule, Rule};
use anyhow::{anyhow, Context, Result};
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

/// The config as plain JSON data. Unknown keys are kept, exactly like the
/// Python version worked on dicts, so the editor never drops settings.
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub data: Map<String, Value>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        // utf-8-sig: accept files saved with a BOM (some Windows editors add one).
        let bytes = fs::read(path).with_context(|| format!("can't read {}", path.display()))?;
        let bytes: &[u8] = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            &bytes[3..]
        } else {
            &bytes
        };
        let data: Map<String, Value> = serde_json::from_slice(bytes)
            .with_context(|| format!("{} isn't valid JSON", path.display()))?;
        Ok(Config { data })
    }

    pub fn get_str(&self, key: &str) -> String {
        match self.data.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => String::new(),
        }
    }

    pub fn get_f64(&self, key: &str, default: f64) -> f64 {
        self.data.get(key).and_then(Value::as_f64).unwrap_or(default)
    }

    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.data.get(key).and_then(Value::as_bool).unwrap_or(default)
    }

    pub fn get_object(&self, key: &str) -> Option<&Map<String, Value>> {
        self.data.get(key).and_then(Value::as_object)
    }

    pub fn get_list(&self, key: &str) -> Vec<Value> {
        self.data
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }
}

/// A config with its rules compiled: what the presence loop reloads.
pub struct LoadedConfig {
    pub data: Map<String, Value>,
    pub rules: Vec<Rule>,
    pub idle_rules: Vec<IdleRule>,
}

impl LoadedConfig {
    pub fn load(path: &Path) -> Result<LoadedConfig> {
        let cfg = Config::load(path)?;
        let rules = compile_rules(&cfg.get_list("rules"))?;
        let idle_rules = compile_idle_rules(&cfg.get_list("idle_rules"))?;
        Ok(LoadedConfig { data: cfg.data, rules, idle_rules })
    }

    pub fn client_id(&self) -> String {
        match self.data.get("client_id") {
            Some(Value::String(s)) => s.trim().to_string(),
            _ => String::new(),
        }
    }

    pub fn poll_seconds(&self) -> f64 {
        self.data
            .get("poll_seconds")
            .and_then(Value::as_f64)
            .filter(|n| *n > 0.0)
            .unwrap_or(3.0)
    }
}

/// JSON with one line per setting and per rule, so it stays hand-editable.
pub fn format_config(map: &Map<String, Value>) -> String {
    fn value_str(v: &Value) -> String {
        let is_rule_list = v.as_array().map(|a| !a.is_empty() && a.iter().all(Value::is_object)) == Some(true);
        if is_rule_list {
            let items: Vec<String> = v
                .as_array()
                .unwrap()
                .iter()
                .map(|x| format!("    {}", serde_json::to_string(x).unwrap()))
                .collect();
            format!("[\n{}\n  ]", items.join(",\n"))
        } else {
            serde_json::to_string(v).unwrap()
        }
    }

    let mut items: Vec<String> = Vec::new();
    for (k, v) in map {
        if k == "rules" {
            continue;
        }
        items.push(format!("  {}: {}", serde_json::to_string(k).unwrap(), value_str(v)));
    }
    let rules = map.get("rules").cloned().unwrap_or(Value::Array(vec![]));
    items.push(format!("  \"rules\": {}", value_str(&rules)));
    format!("{{\n{}\n}}\n", items.join(",\n"))
}

/// Save config.json atomically; the previous version goes to config.backup.json.
pub fn write_config(path: &Path, map: &Map<String, Value>) -> Result<()> {
    let text = format_config(map);
    serde_json::from_str::<Value>(&text).map_err(|_| anyhow!("refusing to write unreadable config"))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f =
            fs::File::create(&tmp).with_context(|| format!("can't write {}", tmp.display()))?;
        f.write_all(text.as_bytes())?;
        f.flush()?;
    }
    if path.exists() {
        let backup = path.with_file_name("config.backup.json");
        let _ = fs::copy(path, &backup);
    }
    let mut last_err = None;
    for _ in 0..20 {
        // The running copy may be reading the file this very instant.
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
    let _ = fs::remove_file(&tmp);
    Err(anyhow!("{}", last_err.map(|e| e.to_string()).unwrap_or_default()))
}

/// Read the raw config as a Value map (for the editor's Reset button).
pub fn read_object(path: &Path) -> Result<Map<String, Value>> {
    Config::load(path).map(|c| c.data)
}

/// Where config.json lives next to the log.
pub fn app_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("DISCORD_STATUS_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs_fallback().unwrap_or_else(|| PathBuf::from("."))
        });
    base.join("DiscordStatus")
}

fn dirs_fallback() -> Option<PathBuf> {
    std::env::var("USERPROFILE").ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bom_tolerant_load() {
        let dir = std::env::temp_dir().join("discord-status-test-bom");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        fs::write(&path, b"\xEF\xBB\xBF{\"poll_seconds\": 7}").unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.get_f64("poll_seconds", 3.0), 7.0);
    }

    #[test]
    fn defaults_for_missing_keys() {
        let cfg = Config { data: Map::new() };
        assert_eq!(cfg.get_f64("poll_seconds", 3.0), 3.0);
        assert_eq!(cfg.get_str("client_id"), "");
        assert!(cfg.get_bool("notify", true));
    }

    #[test]
    fn one_line_per_rule_format() {
        let mut map = Map::new();
        map.insert("client_id".into(), json!("123"));
        map.insert(
            "rules".into(),
            json!([{"process": ["Code.exe"], "details": "coding"}]),
        );
        let text = format_config(&map);
        assert!(text.starts_with("{\n"));
        assert!(text.contains("\"client_id\": \"123\""));
        assert!(text.contains("    {\"details\":\"coding\",\"process\":[\"Code.exe\"]}"));
        assert!(text.trim_end().ends_with('}'));
        // Round trip.
        serde_json::from_str::<Value>(&text).unwrap();
    }

    #[test]
    fn write_and_backup() {
        let dir = std::env::temp_dir().join("discord-status-test-write");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut first = Map::new();
        first.insert("a".into(), json!(1));
        write_config(&path, &first).unwrap();
        let mut second = Map::new();
        second.insert("b".into(), json!(2));
        write_config(&path, &second).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.get_f64("b", 0.0), 2.0);
        let backup = dir.join("config.backup.json");
        assert!(backup.exists());
    }
}