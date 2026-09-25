//! Alias-model tests (`alias.rs`), split out so the source file stays within
//! the size budget. Platform-aware command tests live in
//! `alias/platform_tests.rs`.

use super::*;

fn user_def(name: &str, linux: Option<&str>) -> AliasDef {
    AliasDef {
        name: name.to_string(),
        triggers: vec![],
        linux: linux.map(|s| s.to_string()),
        macos: None,
        shortcuts: BTreeMap::new(),
    }
}

#[test]
fn defaults_are_exactly_br_cd_and_app_and_resolve_by_name_any_case() {
    let defs = defaults();
    assert_eq!(
        defs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
        vec!["br", "cd", "app"],
        "the seeded defaults are exactly br, cd and app"
    );
    assert!(defs.iter().all(|d| d.triggers.is_empty()));
    let br = resolve(&defs, "br").expect("br resolves");
    assert_eq!(br.name, "br");
    assert!(resolve(&defs, "BR").is_some());
    assert!(resolve(&defs, "Cd").is_some());
    assert!(resolve(&defs, "App").is_some());
    assert!(resolve(&defs, "nope").is_none());
    assert!(resolve(&defs, "browser").is_none());
    assert!(resolve(&defs, "clipboard").is_none());
}

#[test]
fn app_default_uses_native_backend() {
    let defs = defaults();
    let app = resolve(&defs, "app").expect("app resolves");
    assert_eq!(app.name, "app");
    assert!(app.triggers.is_empty(), "the input is the app name");
    assert!(app.shortcuts.is_empty());
    assert_eq!(app.linux.as_deref(), Some(crate::launch::TEMPLATE));
    assert_eq!(app.macos.as_deref(), Some(crate::launch::TEMPLATE));
    assert_eq!(app, &crate::launch::default_def());
}

#[test]
fn br_default_opens_quietly_in_the_background() {
    let defs = defaults();
    let br = resolve(&defs, "br").expect("br resolves");
    assert_eq!(
        br.linux.as_deref(),
        Some("xdg-open {input} >/dev/null 2>&1 &"),
        "linux opens through xdg-open, quiet and backgrounded"
    );
    assert_eq!(
        br.macos.as_deref(),
        Some("open {input} >/dev/null 2>&1 &"),
        "macos opens through open, quiet and backgrounded"
    );
}

#[test]
fn cd_default_uses_native_backend() {
    let defs = defaults();
    let cd = resolve(&defs, "cd").expect("cd resolves");
    assert_eq!(cd.name, "cd");
    assert_eq!(cd.linux.as_deref(), Some(crate::clipboard::TEMPLATE));
    assert_eq!(cd.macos.as_deref(), Some(crate::clipboard::TEMPLATE));
}

#[test]
fn defaults_carry_concrete_content_not_empty_shells() {
    let defs = defaults();
    let br = resolve(&defs, "br").expect("br resolves");
    assert_eq!(
        br.shortcuts.get("baidu").map(String::as_str),
        Some("https://www.baidu.com")
    );
    assert_eq!(
        br.shortcuts.get("gm").map(String::as_str),
        Some("https://mail.google.com")
    );
    // The registered shortcuts are wired into the run path.
    assert_eq!(
        crate::exec::resolve_shortcuts(br, "baidu"),
        "https://www.baidu.com"
    );
    assert_eq!(
        crate::exec::resolve_shortcuts(br, "baidu ?q=1"),
        "https://www.baidu.com ?q=1"
    );
    assert_eq!(
        crate::exec::resolve_shortcuts(br, "https://x.dev"),
        "https://x.dev"
    );
    let cd = resolve(&defs, "cd").expect("cd resolves");
    assert!(cd.linux.as_deref().is_some_and(|c| !c.trim().is_empty()));
    assert!(cd.macos.as_deref().is_some_and(|c| !c.trim().is_empty()));
    assert!(defs.iter().all(|d| !d.name.is_empty()));
}

#[test]
fn labels_use_first_trigger_when_present() {
    let defs = defaults();
    let br = resolve(&defs, "br").unwrap();
    assert_eq!(label(br), "br");
    assert_eq!(entry_label(br), "br");

    let mut d = br.clone();
    d.triggers = vec!["b".to_string()];
    assert_eq!(label(&d), "br (b)");
    assert_eq!(entry_label(&d), "b");
}

#[test]
fn set_shortcut_on_user_alias_and_replacement() {
    let mut user = vec![user_def("t", Some("echo {input}"))];
    assert_eq!(
        set_shortcut(&mut user, "t", "here", "cd /tmp").unwrap(),
        None
    );
    assert_eq!(
        set_shortcut(&mut user, "t", "here", "cd /var").unwrap(),
        Some("cd /tmp".into())
    );
    assert_eq!(
        user[0].shortcuts.get("here").map(String::as_str),
        Some("cd /var")
    );
    assert_eq!(
        set_shortcut(&mut user, "nope", "k", "v").unwrap_err(),
        "alias not found: nope"
    );
}

#[test]
fn set_shortcut_edits_a_seeded_alias_in_place() {
    let mut user = defaults();
    // The seeded `br` already carries concrete registered content
    // (baidu/gm); registering another key edits that same definition.
    assert_eq!(
        set_shortcut(&mut user, "br", "gh", "https://github.com").unwrap(),
        None
    );
    assert_eq!(user.len(), 3, "no duplicate definition appears");
    let br = resolve(&user, "br").expect("br still resolves");
    assert!(br.linux.is_some(), "the seeded command survives");
    assert_eq!(
        br.shortcuts.get("gh").map(String::as_str),
        Some("https://github.com")
    );
    assert_eq!(
        br.shortcuts.get("baidu").map(String::as_str),
        Some("https://www.baidu.com"),
        "the seeded shortcuts stay put"
    );
    // Re-registering an existing key reports the previous value.
    assert_eq!(
        set_shortcut(&mut user, "br", "gh", "https://gitlab.com").unwrap(),
        Some("https://github.com".to_string())
    );
}

#[test]
fn remove_shortcut_errors_and_success() {
    let mut user = vec![user_def("t", Some("echo {input}"))];
    set_shortcut(&mut user, "t", "here", "cd /tmp").unwrap();
    assert!(remove_shortcut(&mut user, "t", "here").is_ok());
    assert!(user[0].shortcuts.is_empty());
    assert_eq!(
        remove_shortcut(&mut user, "t", "here").unwrap_err(),
        "no shortcut \"here\" on t"
    );
    assert_eq!(
        remove_shortcut(&mut user, "ghost", "here").unwrap_err(),
        "alias not found: ghost"
    );
}

#[test]
fn add_trigger_on_user_alias_is_idempotent() {
    let mut user = vec![user_def("t", Some("echo {input}"))];
    assert!(add_trigger(&mut user, "t", "tt").unwrap());
    assert_eq!(user[0].triggers, vec!["tt".to_string()]);
    // Case-insensitive: the alias already answers to "tt" (and to "t").
    assert!(!add_trigger(&mut user, "t", "TT").unwrap());
    assert!(!add_trigger(&mut user, "t", "T").unwrap());
    assert_eq!(user[0].triggers, vec!["tt".to_string()]);

    let err = add_trigger(&mut user, "ghost", "g").unwrap_err();
    assert_eq!(err, "alias not found: ghost");
    assert_eq!(
        add_trigger(&mut user, "t", "bad!").unwrap_err(),
        "invalid trigger: bad!"
    );
    assert_eq!(user.len(), 1, "errors never touch the list");
    assert_eq!(user[0].triggers, vec!["tt".to_string()]);
}

#[test]
fn add_trigger_edits_a_seeded_alias_in_place() {
    let mut user = defaults();
    assert!(add_trigger(&mut user, "br", "b").unwrap());
    assert_eq!(user.len(), 3, "no duplicate definition appears");
    let br = resolve(&user, "br").expect("br still resolves");
    assert_eq!(br.name, "br");
    assert_eq!(br.triggers, vec!["b".to_string()]);
    assert!(br.linux.is_some() && br.macos.is_some());
    assert_eq!(
        br.shortcuts.get("baidu").map(String::as_str),
        Some("https://www.baidu.com")
    );
    // Already there: no duplicate, no second entry.
    assert!(!add_trigger(&mut user, "br", "B").unwrap());
    assert_eq!(user.len(), 3);
    assert_eq!(
        resolve(&user, "br").unwrap().triggers,
        vec!["b".to_string()]
    );
}

#[test]
fn add_trigger_rejects_a_word_taken_by_another_alias() {
    let mut user = defaults();
    user.push(user_def("t", Some("echo {input}")));
    add_trigger(&mut user, "t", "tt").unwrap();
    user.push(user_def("u", Some("printf {input}")));

    // "cd" is a seeded name, "tt" a trigger of another alias.
    for taken in ["cd", "CD", "tt", "Tt"] {
        let err = add_trigger(&mut user, "u", taken).unwrap_err();
        assert!(err.contains("already used"), "{err}");
    }
    assert!(user[4].triggers.is_empty(), "nothing was appended");
}

#[test]
fn remove_trigger_reports_when_nothing_was_removed() {
    let mut user = defaults();
    user.push(user_def("t", Some("echo {input}")));
    add_trigger(&mut user, "t", "tt").unwrap();
    add_trigger(&mut user, "t", "t2").unwrap();
    assert!(
        remove_trigger(&mut user, "t", "TT").unwrap(),
        "case-insensitive"
    );
    assert_eq!(user[3].triggers, vec!["t2".to_string()], "one match goes");
    assert!(remove_trigger(&mut user, "t", "t2").unwrap());
    assert!(user[3].triggers.is_empty());
    // Unknown trigger, unknown alias, and a seeded alias without that
    // trigger all report "nothing removed" instead of an error.
    assert!(!remove_trigger(&mut user, "t", "tt").unwrap());
    assert!(!remove_trigger(&mut user, "ghost", "tt").unwrap());
    assert!(!remove_trigger(&mut user, "br", "b").unwrap());
}

#[test]
fn edit_shortcut_rewrites_the_value_in_place() {
    let mut user = defaults();
    edit_shortcut(&mut user, "br", "baidu", "baidu", "https://x.dev").unwrap();
    let br = resolve(&user, "br").expect("br still resolves");
    assert_eq!(
        br.shortcuts.get("baidu").map(String::as_str),
        Some("https://x.dev")
    );
    assert_eq!(br.shortcuts.len(), 2, "the other seeded key stays put");
}

#[test]
fn edit_shortcut_renames_the_key_and_replaces_a_collision() {
    let mut user = vec![user_def("t", Some("echo {input}"))];
    set_shortcut(&mut user, "t", "baidu", "https://a").unwrap();
    set_shortcut(&mut user, "t", "gm", "https://b").unwrap();
    edit_shortcut(&mut user, "t", "baidu", "gh", "https://github.com").unwrap();
    let t = resolve(&user, "t").expect("t still resolves");
    assert_eq!(t.shortcuts.get("baidu"), None, "the old key is gone");
    assert_eq!(
        t.shortcuts.get("gh").map(String::as_str),
        Some("https://github.com")
    );
    // A key change onto an existing key replaces its value, like set_shortcut.
    edit_shortcut(&mut user, "t", "gh", "gm", "https://gitlab.com").unwrap();
    let t = resolve(&user, "t").expect("t still resolves");
    assert_eq!(t.shortcuts.len(), 1);
    assert_eq!(
        t.shortcuts.get("gm").map(String::as_str),
        Some("https://gitlab.com")
    );
}

#[test]
fn edit_shortcut_errors_leave_the_list_untouched() {
    let mut user = vec![user_def("t", Some("echo {input}"))];
    set_shortcut(&mut user, "t", "here", "cd /tmp").unwrap();
    let before = user.clone();
    assert_eq!(
        edit_shortcut(&mut user, "ghost", "here", "here", "x").unwrap_err(),
        "alias not found: ghost"
    );
    assert_eq!(
        edit_shortcut(&mut user, "t", "nope", "nope", "x").unwrap_err(),
        "no shortcut \"nope\" on t"
    );
    assert_eq!(user, before);
    assert_eq!(user[0].shortcuts.len(), 1);
}

#[test]
fn rename_trigger_renames_in_place_and_same_word_is_a_noop() {
    let mut user = vec![user_def("t", Some("echo {input}"))];
    add_trigger(&mut user, "t", "tt").unwrap();
    add_trigger(&mut user, "t", "t2").unwrap();
    rename_trigger(&mut user, "t", "tt", "tw").unwrap();
    assert_eq!(
        user[0].triggers,
        vec!["tw".to_string(), "t2".to_string()],
        "order is preserved"
    );
    // The same word, and the same word in another case, are both no-ops.
    rename_trigger(&mut user, "t", "tw", "tw").unwrap();
    assert_eq!(user[0].triggers, vec!["tw".to_string(), "t2".to_string()]);
    rename_trigger(&mut user, "t", "tw", "TW").unwrap();
    assert_eq!(user[0].triggers, vec!["TW".to_string(), "t2".to_string()]);
}

#[test]
fn rename_trigger_rejects_bad_words_unknowns_and_collisions() {
    let mut user = defaults();
    user.push(user_def("t", Some("echo {input}")));
    add_trigger(&mut user, "t", "tt").unwrap();
    let before = user.clone();
    assert_eq!(
        rename_trigger(&mut user, "t", "tt", "bad word").unwrap_err(),
        "invalid trigger: bad word"
    );
    assert_eq!(
        rename_trigger(&mut user, "ghost", "tt", "tw").unwrap_err(),
        "alias not found: ghost"
    );
    assert_eq!(
        rename_trigger(&mut user, "t", "nope", "tw").unwrap_err(),
        "trigger not found on t: nope"
    );
    // "cd" is another alias's name (and resolves case-insensitively).
    assert_eq!(
        rename_trigger(&mut user, "t", "tt", "CD").unwrap_err(),
        "trigger \"CD\" already used by cd"
    );
    assert_eq!(user, before, "every refusal leaves the list untouched");
    // A word already on the same alias must not be silently dropped.
    add_trigger(&mut user, "t", "t2").unwrap();
    let before = user.clone();
    assert_eq!(
        rename_trigger(&mut user, "t", "tt", "T2").unwrap_err(),
        "trigger already on t: T2"
    );
    assert_eq!(user, before);
}

#[test]
fn wrong_shortcuts_type_reports_the_accepted_shapes() {
    let err = serde_json::from_str::<AliasDef>(r#"{"name":"t","shortcuts":42}"#).unwrap_err();
    assert!(
        err.to_string().contains("array of trigger words"),
        "expectation is named, got: {err}"
    );
    let err = serde_json::from_str::<AliasDef>(r#"{"name":"t","shortcuts":{"k":7}}"#).unwrap_err();
    assert!(
        err.to_string().contains("expected a string"),
        "map values must be strings, got: {err}"
    );
}
