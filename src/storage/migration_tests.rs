//! Legacy-path fallback, corrupt-file handling and seed merging.

use super::tests::user_def;
use super::*;

#[test]
fn corrupt_file_moves_aside_and_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    fs::write(&path, "{ this is not valid json !!!").unwrap();

    let store = load(&path);
    assert_eq!(store.aliases, alias::defaults());
    assert!(store.history.is_empty());
    assert_eq!(store.version, SCHEMA_VERSION);
    assert!(
        path.is_file(),
        "a fresh seeded store replaces the corrupt file"
    );
    let notice = store
        .load_notice
        .as_deref()
        .expect("the corrupt replacement is reported");
    assert!(
        notice.contains("store.json.corrupt"),
        "the notice names the backup: {notice}"
    );
    assert!(dir.path().join("store.json.corrupt").is_file());
}

#[test]
fn missing_file_yields_a_seeded_store_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nope.json");
    let store = load_with_legacy(&path, None);
    assert_eq!(store.aliases, alias::defaults());
    assert!(store.history.is_empty());
    assert!(
        store.load_notice.is_none(),
        "a clean load carries no notice"
    );
    assert!(path.is_file(), "the seed is written back best-effort");
    assert_eq!(
        load_with_legacy(&path, None),
        store,
        "the seed survives a reload"
    );
}

#[test]
fn legacy_path_points_at_the_old_config_dir_store() {
    let legacy = legacy_path().expect("a config dir on this platform");
    assert_eq!(legacy.file_name().unwrap(), "store.json");
    assert_eq!(legacy.parent().unwrap().file_name().unwrap(), "xconsoler");
}

#[test]
fn missing_new_path_migrates_the_legacy_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let legacy = dir.path().join("old-store.json");
    let old = Store {
        version: 1, // pre-versioning: migration must run on the legacy copy
        aliases: vec![user_def("mine", Some("echo legacy"))],
        ..Store::default()
    };
    save(&legacy, &old).unwrap();

    let store = load_with_legacy(&path, Some(&legacy));
    let mine = store
        .aliases
        .iter()
        .find(|d| d.name == "mine")
        .expect("legacy alias is kept");
    assert_eq!(mine.linux.as_deref(), Some("echo legacy"));
    assert_eq!(store.version, SCHEMA_VERSION);
    assert!(
        path.is_file(),
        "the migrated store is persisted to the new path"
    );
    assert_eq!(
        load_with_legacy(&path, None),
        store,
        "the new path now wins over the legacy file"
    );
}

#[test]
fn missing_new_path_without_legacy_still_seeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let store = load_with_legacy(&path, None);
    assert_eq!(store, Store::default());
    assert!(path.is_file(), "the seed is written back best-effort");
}

#[test]
fn present_new_path_wins_over_the_legacy_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let legacy = dir.path().join("old-store.json");
    let current = Store {
        aliases: vec![user_def("mine", Some("echo new"))],
        ..Store::default()
    };
    save(&path, &current).unwrap();
    let old = Store {
        aliases: vec![user_def("mine", Some("echo old"))],
        ..Store::default()
    };
    save(&legacy, &old).unwrap();

    let store = load_with_legacy(&path, Some(&legacy));
    let mine = store.aliases.iter().find(|d| d.name == "mine").unwrap();
    assert_eq!(mine.linux.as_deref(), Some("echo new"));
    assert_eq!(store, current);
}

#[test]
fn corrupt_new_path_moves_aside_without_reading_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let legacy = dir.path().join("old-store.json");
    fs::write(&path, "{ this is not valid json !!!").unwrap();
    let old = Store {
        aliases: vec![user_def("mine", Some("echo legacy"))],
        ..Store::default()
    };
    save(&legacy, &old).unwrap();

    let store = load_with_legacy(&path, Some(&legacy));
    assert_eq!(store.aliases, alias::defaults());
    assert!(store.history.is_empty());
    assert_eq!(store.version, SCHEMA_VERSION);
    assert!(dir.path().join("store.json.corrupt").is_file());
    assert!(
        legacy.is_file(),
        "the legacy store is not consulted, nor moved"
    );
    assert!(path.is_file(), "a fresh seeded store is written back");
    assert_eq!(
        load_with_legacy(&path, Some(&legacy)),
        Store::default(),
        "the legacy alias must not leak into the fresh store"
    );
}

#[test]
fn corrupt_legacy_store_falls_back_to_seeded_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let legacy = dir.path().join("old-store.json");
    fs::write(&legacy, "{ not json either").unwrap();

    let store = load_with_legacy(&path, Some(&legacy));
    assert_eq!(store, Store::default());
    assert!(path.is_file(), "the seed is written back best-effort");
    assert!(
        legacy.is_file(),
        "the corrupt legacy file is left untouched"
    );
}

#[test]
fn migration_merges_defaults_into_an_old_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    // A pre-versioning store: overrides + additions only, no `br`/`cd`.
    let old = Store {
        version: 1,
        aliases: vec![
            user_def("br", Some("echo replaced")),
            user_def("mine", Some("echo hi")),
        ],
        ..Store::default()
    };
    save(&path, &old).unwrap();

    let store = load(&path);
    assert_eq!(store.version, SCHEMA_VERSION);
    assert_eq!(
        store
            .aliases
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        vec!["br", "cd", "app", "mine"],
        "defaults come first, overrides replace in place, extras append"
    );
    let br = &store.aliases[0];
    assert_eq!(br.linux.as_deref(), Some("echo replaced"));
    assert!(br.shortcuts.is_empty(), "the override is used verbatim");
}

#[test]
fn migration_keeps_a_seeded_store_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let mut store = Store::default();
    store.aliases.clear(); // the user deleted br, cd and app: stay deleted
    save(&path, &store).unwrap();

    let loaded = load(&path);
    assert!(loaded.aliases.is_empty());
    assert_eq!(loaded.version, SCHEMA_VERSION);
}

#[test]
fn store_without_version_is_migrated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let old = serde_json::json!({
        "aliases": [user_def("mine", Some("echo {input}"))],
        "history": []
    });
    fs::write(&path, old.to_string()).unwrap();

    let store = load(&path);
    assert_eq!(store.version, SCHEMA_VERSION);
    assert_eq!(
        store
            .aliases
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        vec!["br", "cd", "app", "mine"]
    );
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
    let mine = store
        .aliases
        .iter()
        .find(|d| d.name == "mine")
        .expect("the stored alias survives the migration");
    assert_eq!(mine.linux.as_deref(), Some("echo {input}"));
}

/// The legacy fallback is a default-path-only convenience: a custom
/// `--store` must never pick up the user's real aliases.
#[test]
fn legacy_fallback_applies_only_to_the_default_store_path() {
    let old = PathBuf::from("/tmp/xconsoler-legacy/store.json");
    assert_eq!(
        legacy_fallback(Path::new("/tmp/isolated/store.json"), Some(old.clone())),
        None,
        "a custom store path never falls back"
    );
    assert_eq!(
        legacy_fallback(&default_path(), Some(old.clone())),
        Some(old.clone()),
        "the default path still falls back once"
    );
    assert_eq!(
        legacy_fallback(&default_path(), Some(default_path())),
        None,
        "the default path is never its own fallback"
    );
}
