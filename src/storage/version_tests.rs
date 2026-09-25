//! Alias JSON vocabulary and cross-version loading rules.

use super::tests::user_def;
use super::*;

#[test]
fn legacy_store_json_loads_and_saves_with_the_new_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let old = serde_json::json!({
        "aliases": [{
            "name": "mine", "shortcuts": ["tt", "tw"], "linux": "echo {input}",
            "macos": null, "args": { "here": "cd /tmp" }, "builtin": false
        }],
        "history": []
    });
    fs::write(&path, old.to_string()).unwrap();

    let store = load(&path);
    assert_eq!(store.version, SCHEMA_VERSION);
    let def = store.aliases.iter().find(|d| d.name == "mine").unwrap();
    assert_eq!(def.triggers, vec!["tt".to_string(), "tw".to_string()]);
    assert_eq!(
        def.shortcuts.get("here").map(String::as_str),
        Some("cd /tmp")
    );

    save(&path, &store).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(json["version"], serde_json::json!(4));
    let aliases = json["aliases"].as_array().unwrap();
    let mine = aliases.iter().find(|d| d["name"] == "mine").unwrap();
    assert_eq!(mine["triggers"], serde_json::json!(["tt", "tw"]));
    assert_eq!(mine["shortcuts"], serde_json::json!({ "here": "cd /tmp" }));
    assert!(mine["shortcuts"].is_object(), "the legacy array is gone");
    assert!(mine.get("args").is_none(), "the legacy map key is gone");
}

/// Every hand-written shape loads: the current `"triggers"` array plus
/// `"shortcuts"` map, a map with no `"triggers"` key at all, and mixes
/// where `"triggers"`/`"shortcuts"`-map win over the legacy keys.
#[test]
fn shortcut_json_shapes_load_into_the_new_model() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    fs::write(
        &path,
        r#"{"version":4,"aliases":[{"name":"br","triggers":["b"],
            "linux":"xdg-open {input}","macos":"open {input}",
            "shortcuts":{"zhipu":"https://bigmodel.cn/console/overview"}}],
            "history":[]}"#,
    )
    .unwrap();
    let store = load(&path);
    let br = &store.aliases[0];
    assert_eq!(br.triggers, vec!["b".to_string()]);
    assert_eq!(br.linux.as_deref(), Some("xdg-open {input}"));
    assert_eq!(br.macos.as_deref(), Some("open {input}"));
    assert_eq!(
        br.shortcuts["zhipu"],
        "https://bigmodel.cn/console/overview"
    );
    assert_eq!(
        store.aliases.len(),
        1,
        "a v4 snapshot is not backfilled with the newer seeds"
    );

    let parse = |json: &str| serde_json::from_str::<AliasDef>(json).unwrap();
    let map = parse(r#"{"name":"br","shortcuts":{"baidu":"https://www.baidu.com"}}"#);
    assert!(map.triggers.is_empty(), "a missing list defaults to empty");
    assert_eq!(map.shortcuts["baidu"], "https://www.baidu.com");
    let mixed = parse(r#"{"name":"a","triggers":["new"],"shortcuts":["legacy"],"args":{"k":"v"}}"#);
    assert_eq!(mixed.triggers, vec!["new".to_string()], "triggers wins");
    assert_eq!(mixed.shortcuts["k"], "v");
    let old = parse(r#"{"name":"b","shortcuts":{"new":"map"},"args":{"k":"legacy"}}"#);
    assert!(old.triggers.is_empty());
    assert_eq!(old.shortcuts["new"], "map", "the map wins over args");
    assert!(!old.shortcuts.contains_key("k"));
}

/// Version 2 stores are full snapshots: the older version bumps must not
/// merge the seeded defaults back in. The `app` seed added in version 4 is
/// the exception - every snapshot older than 4 gets it appended once.
#[test]
fn version_2_store_keeps_its_aliases_and_gains_the_app_seed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let old = Store {
        version: 2,
        aliases: vec![user_def("mine", Some("echo hi"))],
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
        vec!["mine", "app"],
        "a v2 snapshot keeps its aliases and gains only the new seed"
    );
    assert_eq!(store.aliases[0].linux.as_deref(), Some("echo hi"));
}

/// Version 4 seeded `app`: v3 snapshots gain it at the end, while v4
/// snapshots stay exactly as written.
#[test]
fn version_3_store_gains_the_app_alias_once() {
    let dir = tempfile::tempdir().unwrap();
    for version in [3u32, 4u32] {
        let path = dir.path().join(format!("store-{version}.json"));
        let saved = Store {
            version,
            aliases: vec![user_def("br", Some("echo replaced"))],
            ..Store::default()
        };
        save(&path, &saved).unwrap();

        let store = load(&path);
        let names: Vec<&str> = store.aliases.iter().map(|d| d.name.as_str()).collect();
        if version < 4 {
            assert_eq!(
                names,
                vec!["br", "app"],
                "a v{version} store gains the seed"
            );
            let app = &store.aliases[1];
            assert_eq!(app.linux.as_deref(), Some(crate::launch::TEMPLATE));
            assert_eq!(app.macos.as_deref(), Some(crate::launch::TEMPLATE));
            assert!(app.triggers.is_empty());
        } else {
            assert_eq!(names, vec!["br"], "a v4 snapshot gains nothing");
        }
        assert_eq!(store.version, SCHEMA_VERSION);
    }
}

/// A user alias that already owns the name `app` is never duplicated by the
/// version-4 backfill, case-insensitively.
#[test]
fn a_stored_alias_named_app_wins_over_the_seed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let saved = Store {
        version: 3,
        aliases: vec![user_def("App", Some("echo mine"))],
        ..Store::default()
    };
    save(&path, &saved).unwrap();

    let store = load(&path);
    assert_eq!(store.aliases.len(), 1, "the user's alias is not duplicated");
    assert_eq!(store.aliases[0].name, "App");
    assert_eq!(store.aliases[0].linux.as_deref(), Some("echo mine"));
}

/// A legacy store missing the map entirely (only the trigger array
/// present) still loads, and a missing map defaults to empty.
#[test]
fn missing_args_key_defaults_to_no_shortcuts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    fs::write(
        &path,
        r#"{"aliases":[{"name":"mine","shortcuts":["tt"],
            "linux":"echo {input}","macos":null}],"history":[]}"#,
    )
    .unwrap();
    let store = load(&path);
    let def = &store.aliases[..].iter().find(|d| d.name == "mine").unwrap();
    assert_eq!(def.triggers, vec!["tt".to_string()]);
    assert!(def.shortcuts.is_empty());
}

/// A store from a newer build must survive a load untouched: no version
/// downgrade, no seed merge, no rewrite - and `save` must refuse to clobber
/// it.
#[test]
fn newer_store_version_loads_verbatim_and_is_never_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let future = serde_json::json!({
        "version": SCHEMA_VERSION + 97,
        "aliases": [user_def("mine", Some("echo future"))],
        "history": []
    });
    let original = serde_json::to_string_pretty(&future).unwrap();
    fs::write(&path, &original).unwrap();

    let store = load(&path);
    assert_eq!(store.version, SCHEMA_VERSION + 97, "the version is kept");
    assert!(store.from_newer_version);
    assert_eq!(store.aliases.len(), 1, "no seed merge for newer stores");
    assert_eq!(store.aliases[0].name, "mine");
    assert!(!dir.path().join("store.json.corrupt").exists());
    assert_eq!(fs::read_to_string(&path).unwrap(), original);

    let err = save(&path, &store).unwrap_err();
    assert!(err.to_string().contains("newer xconsoler"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        original,
        "save must not overwrite a newer store"
    );
}

/// A newer file whose shape this build cannot parse is left in place (not
/// renamed `.corrupt`) and the session runs on read-only defaults.
#[test]
fn unparseable_newer_store_is_left_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let future = serde_json::json!({
        "version": SCHEMA_VERSION + 1,
        "aliases": "a shape this build does not know",
        "history": []
    });
    fs::write(&path, future.to_string()).unwrap();

    let store = load(&path);
    assert!(store.from_newer_version);
    assert_eq!(
        store,
        Store {
            from_newer_version: true,
            ..Store::default()
        }
    );
    assert!(save(&path, &store).is_err());
    assert!(path.is_file(), "the newer file stays at its own path");
    assert!(!dir.path().join("store.json.corrupt").exists());
}
