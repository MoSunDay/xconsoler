//! Persistent store: user aliases and history entries (JSON on disk).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::alias::{self, AliasDef};
use crate::keyspec;

/// Hard cap on stored history entries.
pub const MAX_HISTORY: usize = 10_000;

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
}

impl Default for Config {
    fn default() -> Self {
        Config {
            wake_key: keyspec::DEFAULT_SPEC.to_string(),
        }
    }
}

/// Persisted state. `aliases` holds user-defined aliases only; built-ins are
/// merged back in at load time (see [`merge_aliases`]).
#[derive(Debug, Default, Serialize, Deserialize)]
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
        Ok(store) => store,
        Err(_) => {
            let aside = sibling_path(path, ".corrupt");
            let _ = fs::rename(path, &aside);
            Store::default()
        }
    }
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

    fn user_def(name: &str, linux: Option<&str>) -> AliasDef {
        AliasDef {
            name: name.to_string(),
            shortcuts: vec![],
            linux: linux.map(|s| s.to_string()),
            macos: None,
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
        let over = user_def("browser", Some("echo replaced"));
        let extra = user_def("mine", Some("echo hi"));
        let merged = merge_aliases(&[over, extra]);

        assert_eq!(merged.len(), alias::defaults().len() + 1);
        let browser = merged.iter().find(|d| d.name == "browser").unwrap();
        assert_eq!(browser.linux.as_deref(), Some("echo replaced"));
        assert!(!browser.builtin);
        // built-in order is preserved, user additions go last
        assert_eq!(merged[0].name, "browser");
        assert_eq!(merged[1].name, "clipboard");
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
}
