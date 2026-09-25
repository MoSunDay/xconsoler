//! Persistent store: user aliases and history entries (JSON on disk).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::alias::{self, AliasDef};
use crate::keyspec;

/// Hard cap on stored history entries. Only the newest entries are kept:
/// the store is trimmed after every load and every record, so the file
/// stays small and startup never pays for a long history.
pub const MAX_HISTORY: usize = 100;

/// One recorded execution. The input is stored base64-encoded so arbitrary
/// text (quotes, newlines, unicode) survives the JSON roundtrip untouched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub alias: String,
    pub input_b64: String,
    pub ts: u64,
}

impl HistoryEntry {
    /// Build an entry, base64-encoding the plain input.
    pub fn new(alias: &str, input: &str, ts: u64) -> Self {
        HistoryEntry {
            alias: alias.to_string(),
            input_b64: encode_b64(input),
            ts,
        }
    }

    /// Decode the stored input.
    pub fn input(&self) -> String {
        decode_b64(&self.input_b64)
    }

    /// Dedup key: `alias` + NUL + plain input.
    pub fn key(&self) -> String {
        format!("{}\u{0}{}", self.alias, self.input())
    }
}

/// User preferences persisted alongside the aliases/history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Wake-key spec string (`keyspec::DEFAULT_SPEC` when absent).
    pub wake_key: String,
    /// Command-palette key spec (`keyspec::DEFAULT_COMMAND_SPEC` when absent).
    #[serde(default = "default_command_key")]
    pub command_key: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            wake_key: keyspec::DEFAULT_SPEC.to_string(),
            command_key: default_command_key(),
        }
    }
}

/// Serde default for [`Config::command_key`]: the wake key's own default.
fn default_command_key() -> String {
    keyspec::DEFAULT_COMMAND_SPEC.to_string()
}

/// Persisted state. `aliases` holds user-defined aliases only; built-ins are
/// merged back in at load time (see [`merge_aliases`]).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Store {
    /// `#[serde(default)]`: stores written before the config existed load
    /// with the default wake key.
    #[serde(default)]
    pub config: Config,
    pub aliases: Vec<AliasDef>,
    pub history: Vec<HistoryEntry>,
}

/// Validate a wake-key spec and persist its canonical form
/// (`keyspec::describe`) into the store.
pub fn set_wake_key(store: &mut Store, spec: &str) -> Result<(), String> {
    let parsed = keyspec::parse(spec)?;
    store.config.wake_key = keyspec::describe(&parsed);
    Ok(())
}

/// Validate a command-palette key spec and persist its canonical form
/// (`keyspec::describe`) into the store.
pub fn set_command_key(store: &mut Store, spec: &str) -> Result<(), String> {
    let parsed = keyspec::parse(spec)?;
    store.config.command_key = keyspec::describe(&parsed);
    Ok(())
}

/// Config directory for xconsoler (falls back to `~/.xconsoler` when the
/// platform config dir is unknown).
pub fn config_dir() -> PathBuf {
    match dirs::config_dir() {
        Some(p) => p.join("xconsoler"),
        None => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".xconsoler")
        }
    }
}

/// Default store location: `<config_dir>/store.json`.
pub fn default_path() -> PathBuf {
    config_dir().join("store.json")
}

/// Load the store. Missing file or unreadable file yields a default store;
/// corrupt JSON is moved aside to `<path>.corrupt` before falling back.
pub fn load(path: &Path) -> Store {
    let data = match fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return Store::default(),
    };
    match serde_json::from_str::<Store>(&data) {
        Ok(mut store) => {
            trim_history(&mut store.history);
            store
        }
        Err(_) => {
            let aside = sibling_path(path, ".corrupt");
            let _ = fs::rename(path, &aside);
            Store::default()
        }
    }
}

/// Drop all but the newest [`MAX_HISTORY`] entries (history is newest first).
/// Applied on load and on record, so an oversized store shrinks the first
/// time it is read.
pub fn trim_history(history: &mut Vec<HistoryEntry>) {
    history.truncate(MAX_HISTORY);
}

/// Serialize pretty JSON and atomically replace `path` (tmp file + rename).
pub fn save(path: &Path, store: &Store) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(store)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    let tmp = sibling_path(path, ".tmp");
    fs::write(&tmp, json.as_bytes())
        .with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to move {} into place", tmp.display()))
}

/// Effective alias list: built-ins in their canonical order, overridden
/// in place by same-name user aliases, new user names appended at the end.
pub fn merge_aliases(user: &[AliasDef]) -> Vec<AliasDef> {
    let mut merged = alias::defaults();
    for u in user {
        match merged.iter().position(|d| d.name == u.name) {
            Some(i) => merged[i] = u.clone(),
            None => merged.push(u.clone()),
        }
    }
    merged
}

/// Base64-encode (standard alphabet) for storage.
pub fn encode_b64(s: &str) -> String {
    STANDARD.encode(s.as_bytes())
}

/// Base64-decode; on invalid input the original string is returned as-is.
pub fn decode_b64(s: &str) -> String {
    STANDARD
        .decode(s.as_bytes())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_else(|| s.to_string())
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn user_def(name: &str, linux: Option<&str>) -> AliasDef {
        AliasDef {
            name: name.to_string(),
            triggers: vec![],
            linux: linux.map(|s| s.to_string()),
            macos: None,
            shortcuts: BTreeMap::new(),
            builtin: false,
        }
    }

    #[test]
    fn b64_roundtrip() {
        let s = "hello <world> & 'quotes' --/+= \n\ttab";
        assert_eq!(decode_b64(&encode_b64(s)), s);
    }

    #[test]
    fn entry_encodes_input() {
        let e = HistoryEntry::new("browser", "https://example.com/?a=1&b='x'", 42);
        assert_eq!(e.input(), "https://example.com/?a=1&b='x'");
        assert_eq!(e.ts, 42);
        assert_eq!(e.key(), "browser\u{0}https://example.com/?a=1&b='x'");
        assert_ne!(e.input_b64, e.input());
    }

    #[test]
    fn default_path_lives_in_config_dir() {
        assert!(config_dir().to_string_lossy().contains("xconsoler"));
        assert!(default_path().starts_with(config_dir()));
        assert_eq!(default_path().file_name().unwrap(), "store.json");
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let mut store = Store::default();
        store.aliases.push(user_def("mine", Some("echo {input}")));
        store.history.push(HistoryEntry::new("browser", "a b c", 7));
        save(&path, &store).unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.aliases, store.aliases);
        assert_eq!(loaded.history, store.history);
        assert_eq!(loaded.history[0].input(), "a b c");
    }

    #[test]
    fn save_creates_missing_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/dir/store.json");
        save(&path, &Store::default()).unwrap();
        assert!(path.is_file());
    }

    #[test]
    fn load_trims_history_to_the_newest_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let mut store = Store::default();
        for i in 0..(MAX_HISTORY + 25) {
            store
                .history
                .push(HistoryEntry::new("browser", &format!("i{i}"), i as u64));
        }
        save(&path, &store).unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.history.len(), MAX_HISTORY);
        // newest-first order survives: the 25 oldest entries are gone
        assert_eq!(loaded.history[0].input(), "i0");
        assert_eq!(
            loaded.history.last().unwrap().input(),
            format!("i{}", MAX_HISTORY - 1)
        );
    }

    #[test]
    fn corrupt_file_moves_aside_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        fs::write(&path, "{ this is not valid json !!!").unwrap();

        let store = load(&path);
        assert!(store.aliases.is_empty());
        assert!(store.history.is_empty());
        assert!(!path.exists(), "corrupt file should have been renamed");
        assert!(dir.path().join("store.json.corrupt").is_file());
    }

    #[test]
    fn missing_file_yields_default() {
        let dir = tempfile::tempdir().unwrap();
        let store = load(&dir.path().join("nope.json"));
        assert!(store.aliases.is_empty());
        assert!(store.history.is_empty());
    }

    #[test]
    fn merge_overrides_builtin_and_appends_new() {
        let over = user_def("br", Some("echo replaced"));
        let extra = user_def("mine", Some("echo hi"));
        let merged = merge_aliases(&[over, extra]);

        assert_eq!(merged.len(), alias::defaults().len() + 1);
        let br = merged.iter().find(|d| d.name == "br").unwrap();
        assert_eq!(br.linux.as_deref(), Some("echo replaced"));
        assert!(!br.builtin);
        // built-in order is preserved, user additions go last
        assert_eq!(merged[0].name, "br");
        assert_eq!(merged[1].name, "cd");
        assert_eq!(merged.last().unwrap().name, "mine");
    }

    #[test]
    fn old_store_json_without_config_gets_default_wake_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let old = serde_json::json!({
            "aliases": [user_def("mine", Some("echo {input}"))],
            "history": []
        });
        fs::write(&path, old.to_string()).unwrap();

        let store = load(&path);
        assert_eq!(store.config, Config::default());
        assert_eq!(store.config.wake_key, keyspec::DEFAULT_SPEC);
        assert_eq!(store.aliases.len(), 1);
    }

    #[test]
    fn default_store_has_default_wake_key() {
        assert_eq!(Store::default().config.wake_key, "alt+d");
    }

    #[test]
    fn default_config_has_both_keys() {
        let config = Config::default();
        assert_eq!(config.wake_key, "alt+d");
        assert_eq!(config.command_key, "alt+d");
        assert_eq!(Store::default().config, config);
    }

    #[test]
    fn config_without_command_key_gets_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let old = serde_json::json!({
            "config": { "wake_key": "ctrl+g" },
            "aliases": [],
            "history": []
        });
        fs::write(&path, old.to_string()).unwrap();

        let store = load(&path);
        assert_eq!(store.config.wake_key, "ctrl+g");
        assert_eq!(store.config.command_key, "alt+d");
    }

    #[test]
    fn store_json_without_config_gets_both_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let old = serde_json::json!({ "aliases": [], "history": [] });
        fs::write(&path, old.to_string()).unwrap();

        let store = load(&path);
        assert_eq!(store.config.wake_key, "alt+d");
        assert_eq!(store.config.command_key, "alt+d");
    }

    /// On-disk compatibility: the old field names are still the JSON keys —
    /// `"shortcuts"` holds the trigger words and `"args"` holds the concrete
    /// key → value entries, so an existing store keeps loading (and saving)
    /// untouched.
    #[test]
    fn old_store_json_field_names_load_into_the_new_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let old = serde_json::json!({
            "aliases": [{
                "name": "mine",
                "shortcuts": ["tt", "tw"],
                "linux": "echo {input}",
                "macos": null,
                "args": { "here": "cd /tmp" },
                "builtin": false
            }],
            "history": []
        });
        fs::write(&path, old.to_string()).unwrap();

        let store = load(&path);
        let def = &store.aliases[0];
        assert_eq!(def.name, "mine");
        assert_eq!(def.triggers, vec!["tt".to_string(), "tw".to_string()]);
        assert_eq!(
            def.shortcuts.get("here").map(String::as_str),
            Some("cd /tmp")
        );

        // Saving again keeps the historical keys, so older builds can still
        // read what this one writes.
        save(&path, &store).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["aliases"][0]["shortcuts"][1], "tw");
        assert_eq!(json["aliases"][0]["args"]["here"], "cd /tmp");
        assert!(json["aliases"][0].get("triggers").is_none());
    }

    /// A store missing the map entirely (only triggers present) still loads:
    /// `args` carries `#[serde(default)]`.
    #[test]
    fn missing_args_key_defaults_to_no_shortcuts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let old = serde_json::json!({
            "aliases": [{
                "name": "mine",
                "shortcuts": ["tt"],
                "linux": "echo {input}",
                "macos": null,
                "builtin": false
            }],
            "history": []
        });
        fs::write(&path, old.to_string()).unwrap();

        let store = load(&path);
        assert_eq!(store.aliases[0].triggers, vec!["tt".to_string()]);
        assert!(store.aliases[0].shortcuts.is_empty());
    }

    #[test]
    fn set_wake_key_validates_and_normalizes() {
        let mut store = Store::default();
        assert!(set_wake_key(&mut store, "alt+J").is_ok());
        assert_eq!(store.config.wake_key, "alt+j");
        assert!(set_wake_key(&mut store, "ctrl+g").is_ok());
        assert_eq!(store.config.wake_key, "ctrl+g");
        let err = set_wake_key(&mut store, "alt+ctrl+x").unwrap_err();
        assert!(err.contains("alt+ctrl combo unsupported"));
        // rejected specs leave the previous value in place
        assert_eq!(store.config.wake_key, "ctrl+g");
    }

    #[test]
    fn set_wake_key_roundtrips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let mut store = Store::default();
        set_wake_key(&mut store, "alt+j").unwrap();
        save(&path, &store).unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.config.wake_key, "alt+j");
        assert_eq!(loaded.config, store.config);
    }

    #[test]
    fn set_command_key_validates_and_normalizes() {
        let mut store = Store::default();
        assert!(set_command_key(&mut store, " ctrl+O ").is_ok());
        assert_eq!(store.config.command_key, "ctrl+o");
        assert!(set_command_key(&mut store, "alt+J").is_ok());
        assert_eq!(store.config.command_key, "alt+j");
        // command-key writes leave the wake key alone
        assert_eq!(store.config.wake_key, "alt+d");
        let err = set_command_key(&mut store, "garbage").unwrap_err();
        assert!(err.contains("expected \"alt+<c>\" or \"ctrl+<c>\""));
        assert!(set_command_key(&mut store, "alt+ctrl+x").is_err());
        // rejected specs leave the previous value in place
        assert_eq!(store.config.command_key, "alt+j");
    }
}
