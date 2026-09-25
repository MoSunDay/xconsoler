//! Core store tests: roundtrips, history trimming, config keys.

use super::*;
use std::collections::BTreeMap;

pub(super) fn user_def(name: &str, linux: Option<&str>) -> AliasDef {
    AliasDef {
        name: name.to_string(),
        triggers: vec![],
        linux: linux.map(|s| s.to_string()),
        macos: None,
        shortcuts: BTreeMap::new(),
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
fn default_path_lives_under_home() {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()));
    assert_eq!(config_dir(), home.join("xconsoler"));
    assert_eq!(default_path(), home.join("xconsoler/store.json"));
    assert_eq!(default_path().file_name().unwrap(), "store.json");
}

#[test]
fn default_store_is_seeded_with_the_defaults() {
    let store = Store::default();
    assert_eq!(store.aliases, alias::defaults());
    assert!(store.history.is_empty());
    assert_eq!(store.version, SCHEMA_VERSION);
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
    assert_eq!(loaded, store);
    assert_eq!(loaded.version, SCHEMA_VERSION);
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

/// Legacy keys still load, but saving rewrites them in the UI
/// vocabulary: `"triggers"` holds the trigger words, `"shortcuts"` holds
/// the key → value map, and neither `"args"` nor a trigger array under
/// `"shortcuts"` survives to disk.

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
