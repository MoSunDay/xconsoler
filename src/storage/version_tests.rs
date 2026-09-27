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
    assert_eq!(json["version"], serde_json::json!(SCHEMA_VERSION));
    let aliases = json["aliases"].as_array().unwrap();
    let mine = aliases.iter().find(|d| d["name"] == "mine").unwrap();
    assert_eq!(mine["triggers"], serde_json::json!(["tt", "tw"]));
    assert_eq!(
        mine["shortcuts"],
        serde_json::json!({ "here": encode_b64("cd /tmp") }),
        "the value is rewritten base64-encoded (schema v5)"
    );
    assert!(mine["shortcuts"].is_object(), "the legacy array is gone");
    assert!(mine.get("args").is_none(), "the legacy map key is gone");
}

/// A v4 file stores shortcut values as plaintext: they load exactly as
/// written, and the next save rewrites the file at the current version (6)
/// with every value base64-encoded. Re-loading that file restores the
/// plaintext.
#[test]
fn v4_plaintext_shortcuts_resave_as_base64_at_the_current_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    fs::write(
        &path,
        r#"{"version":4,"aliases":[{"name":"br","triggers":["b"],
            "linux":"xdg-open {input}","macos":null,
            "shortcuts":{"zhipu":"https://bigmodel.cn/console/overview"}}],
            "history":[]}"#,
    )
    .unwrap();

    let store = load(&path);
    assert_eq!(store.version, SCHEMA_VERSION);
    assert_eq!(
        store.aliases[0].shortcuts["zhipu"], "https://bigmodel.cn/console/overview",
        "a v4 value loads as the plaintext it was written as"
    );

    save(&path, &store).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(json["version"], serde_json::json!(6));
    assert_eq!(
        json["aliases"][0]["shortcuts"]["zhipu"],
        serde_json::json!(encode_b64("https://bigmodel.cn/console/overview")),
        "the value is stored base64-encoded"
    );
    assert!(!json.to_string().contains("bigmodel.cn"));

    let reloaded = load(&path);
    assert_eq!(
        reloaded.aliases[0].shortcuts["zhipu"], "https://bigmodel.cn/console/overview",
        "the round trip restores the original plaintext"
    );
}

/// Regression (raw disk bytes): the v5 encoding must not stay lazy. A v3-era
/// file keeps its plaintext shortcut values on disk until something saves,
/// so loading one rewrites the file at the current version with every value
/// base64-encoded. The plain round-trip tests above cannot catch a lingering
/// plaintext file, because `decode_b64` falls back to the raw string.
#[test]
fn loading_an_old_store_rewrites_the_raw_file_with_encoded_shortcuts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let v3 = serde_json::json!({
        "version": 3,
        "aliases": [{
            "name": "cd",
            "triggers": ["cd"],
            "linux": "cd {input}",
            "macos": null,
            "shortcuts": { "proj": "cd ~/projects/{input}" }
        }],
        "history": []
    });
    fs::write(&path, v3.to_string()).unwrap();

    let store = load(&path);
    assert_eq!(
        store.aliases[0].shortcuts["proj"], "cd ~/projects/{input}",
        "the plaintext value still loads into memory"
    );

    let raw = fs::read_to_string(&path).unwrap();
    assert!(
        raw.contains(&format!("\"version\": {SCHEMA_VERSION}")),
        "the raw file is rewritten at the current version: {raw}"
    );
    assert!(
        raw.contains(&encode_b64("cd ~/projects/{input}")),
        "the shortcut value is base64 on disk: {raw}"
    );
    assert!(
        !raw.contains("cd ~/projects/{input}"),
        "the plaintext value is absent from the raw file: {raw}"
    );

    // The rewrite happens once: a file already at the current version is
    // left byte-identical by later loads.
    let settled = fs::read_to_string(&path).unwrap();
    load(&path);
    assert_eq!(fs::read_to_string(&path).unwrap(), settled);
}

/// The eager migration write-back must never touch a newer-build file:
/// `load` leaves its raw bytes alone (and `save` keeps refusing it).
#[test]
fn loading_a_newer_version_file_leaves_the_raw_bytes_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let future = serde_json::json!({
        "version": SCHEMA_VERSION + 93, // 99: far beyond this build
        "aliases": [user_def("mine", Some("echo future"))],
        "history": []
    });
    let original = serde_json::to_string_pretty(&future).unwrap();
    fs::write(&path, &original).unwrap();

    let store = load(&path);
    assert!(store.from_newer_version);
    assert_eq!(store.version, SCHEMA_VERSION + 93, "the version is kept");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        original,
        "load must not rewrite a store from a newer build"
    );
}

/// v5 files store base64 values, which decode on load; a value that is not
/// valid base64 (a hand-edited plaintext entry) falls back to the raw
/// string, and a save/load round trip keeps every value stable.
#[test]
fn v5_shortcut_values_decode_with_raw_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let v5 = serde_json::json!({
        "version": 5,
        "aliases": [{
            "name": "br",
            "triggers": [],
            "linux": "xdg-open {input}",
            "macos": null,
            "shortcuts": {
                "baidu": encode_b64("https://www.baidu.com"),
                "hand": "https://plain.example/not-base64"
            }
        }],
        "history": []
    });
    fs::write(&path, v5.to_string()).unwrap();

    let store = load(&path);
    assert_eq!(store.version, SCHEMA_VERSION);
    let br = &store.aliases[0];
    assert_eq!(
        br.shortcuts["baidu"], "https://www.baidu.com",
        "a base64 value decodes on load"
    );
    assert_eq!(
        br.shortcuts["hand"], "https://plain.example/not-base64",
        "a non-base64 value loads verbatim"
    );

    save(&path, &store).unwrap();
    assert_eq!(
        load(&path).aliases[0].shortcuts,
        br.shortcuts,
        "the decoded values survive a save/load round trip"
    );
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
        2,
        "a v4 snapshot keeps its aliases (plus the version-6 pw seed)"
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
/// merge the seeded defaults back in. The `app` seed added in version 4 and
/// the `pw` seed added in version 6 are the exceptions - every snapshot
/// older than those versions gains them appended once.
#[test]
fn version_2_store_keeps_its_aliases_and_gains_the_newer_seeds() {
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
        vec!["mine", "app", "pw"],
        "a v2 snapshot keeps its aliases and gains only the newer seeds"
    );
    assert_eq!(store.aliases[0].linux.as_deref(), Some("echo hi"));
}

/// Version 4 seeded `app`: v3 snapshots gain it at the end, while v4
/// snapshots do not (they still gain the version-6 `pw` seed, covered by
/// `version_4_and_5_stores_gain_the_pw_alias_once` below).
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
                vec!["br", "app", "pw"],
                "a v{version} store gains the seed"
            );
            let app = &store.aliases[1];
            assert_eq!(app.linux.as_deref(), Some(crate::launch::TEMPLATE));
            assert_eq!(app.macos.as_deref(), Some(crate::launch::TEMPLATE));
            assert!(app.triggers.is_empty());
        } else {
            assert_eq!(
                names,
                vec!["br", "pw"],
                "a v4 snapshot gains no app, only the pw seed"
            );
        }
        assert_eq!(store.version, SCHEMA_VERSION);
    }
}

/// Version 6 seeded `pw`: v4 and v5 snapshots gain it at the end, while v6
/// snapshots stay exactly as written.
#[test]
fn version_4_and_5_stores_gain_the_pw_alias_once() {
    let dir = tempfile::tempdir().unwrap();
    for version in [4u32, 5u32, 6u32] {
        let path = dir.path().join(format!("store-{version}.json"));
        let saved = Store {
            version,
            aliases: vec![user_def("br", Some("echo replaced"))],
            ..Store::default()
        };
        save(&path, &saved).unwrap();

        let store = load(&path);
        let names: Vec<&str> = store.aliases.iter().map(|d| d.name.as_str()).collect();
        if version < 6 {
            assert_eq!(names, vec!["br", "pw"], "a v{version} store gains the seed");
            let pw = &store.aliases[1];
            assert_eq!(pw.linux.as_deref(), Some(crate::plugins::pw::TEMPLATE));
            assert_eq!(pw.macos.as_deref(), Some(crate::plugins::pw::TEMPLATE));
            assert!(pw.triggers.is_empty());
            assert_eq!(pw.shortcuts.len(), 3, "s/m/c profile shortcuts");
        } else {
            assert_eq!(names, vec!["br"], "a v6 snapshot gains nothing");
        }
        assert_eq!(store.version, SCHEMA_VERSION);
    }
}

/// A user alias that already owns the name `app` is never duplicated by the
/// version-4 backfill, case-insensitively. (A v3 snapshot still gains the
/// version-6 `pw` seed - that gate is covered by its own tests below.)
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
    assert_eq!(
        store
            .aliases
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        vec!["App", "pw"],
        "the user's alias is not duplicated, only the pw seed is appended"
    );
    assert_eq!(store.aliases[0].linux.as_deref(), Some("echo mine"));
}

/// A user alias that already owns the name `pw` is never duplicated by the
/// version-6 backfill, case-insensitively.
#[test]
fn a_stored_alias_named_pw_wins_over_the_seed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.json");
    let saved = Store {
        version: 5,
        aliases: vec![user_def("PW", Some("echo mine"))],
        ..Store::default()
    };
    save(&path, &saved).unwrap();

    let store = load(&path);
    assert_eq!(store.aliases.len(), 1, "the user's alias is not duplicated");
    assert_eq!(store.aliases[0].name, "PW");
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
