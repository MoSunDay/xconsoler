//! List-screen transitions and dispatch tests for `/settings`, split out of
//! `settings.rs` so both files stay within the size budget. The row-level
//! `e` edit-wizard coverage lives in `settings/edit_tests.rs`.

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn store_with_t() -> Store {
    let mut store = Store::default();
    store.aliases.push(crate::alias::AliasDef {
        name: "t".to_string(),
        triggers: vec!["tt".to_string()],
        linux: Some("printf %s {input}".to_string()),
        macos: None,
        shortcuts: [("baidu".to_string(), "https://www.baidu.com".to_string())]
            .into_iter()
            .collect(),
    });
    store
}

fn type_str(st: &Settings, store: &Store, s: &str) -> Settings {
    let mut st = st.clone();
    for c in s.chars() {
        let (next, _, _) = handle_key(&st, store, key(KeyCode::Char(c)));
        st = next;
    }
    st
}

/// Select alias `t` (index 2 of the stored list) with `t`'s 1 trigger +
/// 1 concrete shortcut shown.
fn on_t() -> (Settings, Store) {
    let store = store_with_t();
    let st = Settings { cursor: 2, ..new() };
    (st, store)
}

#[test]
fn rows_list_aliases_then_expanded_entries() {
    let store = store_with_t();
    let aliases = view(&store);
    let flat = rows(&aliases, None);
    assert_eq!(flat.len(), 3); // br, cd, t
    let expanded = rows(&aliases, Some(2));
    assert_eq!(
        expanded,
        vec![
            Row::Alias { idx: 0 },
            Row::Alias { idx: 1 },
            Row::Alias { idx: 2 },
            // expanded rows: triggers first, then concrete shortcuts
            Row::Trigger { alias: 2, idx: 0 },
            Row::Shortcut {
                alias: 2,
                key: "baidu".to_string()
            },
        ]
    );
}

#[test]
fn expanded_rows_list_every_trigger_and_shortcut() {
    let mut store = store_with_t();
    let t = store
        .aliases
        .iter_mut()
        .find(|d| d.name == "t")
        .expect("t exists");
    t.triggers = vec!["tt".to_string(), "tw".to_string()];
    t.shortcuts.insert("cc".to_string(), "x".to_string());
    let aliases = view(&store);
    let flat = rows(&aliases, Some(2));
    assert_eq!(
        flat[3..7],
        [
            Row::Trigger { alias: 2, idx: 0 },
            Row::Trigger { alias: 2, idx: 1 },
            Row::Shortcut {
                alias: 2,
                key: "baidu".to_string()
            },
            Row::Shortcut {
                alias: 2,
                key: "cc".to_string()
            },
        ]
    );
}

#[test]
fn s_opens_the_shortcut_wizard_for_the_selected_alias() {
    let (st, store) = on_t();
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Char('s')));
    assert_eq!(eff, Effect::None);
    let form = st.form.as_ref().expect("shortcut wizard open");
    assert_eq!(
        form.purpose,
        settings_form::Purpose::NewShortcut {
            alias: "t".to_string()
        }
    );
    assert_eq!(settings_form::step_count(form), 2);
    // Esc cancels, nothing was touched
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Esc));
    assert_eq!((st.form, eff), (None, Effect::None));
}

#[test]
fn submitting_the_shortcut_wizard_yields_the_effect() {
    let (st, store) = on_t();
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('s')));
    let st = type_str(&st, &store, "gc");
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter)); // key -> value step
    assert_eq!(eff, Effect::None);
    assert_eq!(st.form.as_ref().map(|f| f.step), Some(1));
    let st = type_str(&st, &store, "git clone {input}");
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter));
    assert_eq!(st.form, None, "the wizard closes on submit");
    assert_eq!(
        eff,
        Effect::SetShortcut {
            alias: "t".to_string(),
            key: "gc".to_string(),
            value: "git clone {input}".to_string()
        }
    );
}

#[test]
fn t_opens_the_trigger_wizard_and_submits_a_trigger() {
    let (st, store) = on_t();
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Char('t')));
    assert_eq!(eff, Effect::None);
    let form = st.form.as_ref().expect("trigger wizard open");
    assert_eq!(
        form.purpose,
        settings_form::Purpose::NewTrigger {
            alias: "t".to_string()
        }
    );
    assert_eq!(settings_form::step_count(form), 1);
    let st = type_str(&st, &store, "gc");
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter));
    assert_eq!(st.form, None);
    assert_eq!(
        eff,
        Effect::AddTrigger {
            alias: "t".to_string(),
            trigger: "gc".to_string()
        }
    );
}

#[test]
fn a_is_no_longer_bound_on_the_list() {
    let (st, store) = on_t();
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Char('a')));
    assert_eq!((st.form, eff), (None, Effect::None));
}

#[test]
fn entry_rows_route_e_s_and_t_to_their_parent_alias() {
    let (st, store) = on_t();
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Enter)); // expand
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down)); // trigger row
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('t')));
    assert_eq!(
        st.form.as_ref().map(|f| f.purpose.clone()),
        Some(settings_form::Purpose::NewTrigger {
            alias: "t".to_string()
        })
    );
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Esc));
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down)); // shortcut row
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('s')));
    assert_eq!(
        st.form.as_ref().map(|f| f.purpose.clone()),
        Some(settings_form::Purpose::NewShortcut {
            alias: "t".to_string()
        })
    );
    // `e` on the shortcut row edits that very key/value pair.
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Esc));
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('e')));
    assert_eq!(
        st.form.as_ref().map(|f| f.purpose.clone()),
        Some(settings_form::Purpose::EditShortcut {
            alias: "t".to_string(),
            old_key: "baidu".to_string()
        })
    );
}

#[test]
fn d_on_a_trigger_row_asks_for_its_removal() {
    let (st, store) = on_t();
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter)); // expand
    assert_eq!(eff, Effect::None);
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down)); // trigger row
    let (_, s2, eff) = handle_key(&st, &store, key(KeyCode::Char('d')));
    assert_eq!(
        eff,
        Effect::RemoveTrigger {
            alias: "t".to_string(),
            trigger: "tt".to_string()
        },
        "`d` on a trigger row drops just that word"
    );
    assert_eq!(s2, store, "the pure transition leaves the store alone");
    let t = s2.aliases.iter().find(|d| d.name == "t").expect("t exists");
    assert_eq!(t.triggers, vec!["tt".to_string()]);
}

#[test]
fn d_on_a_shortcut_row_removes_it_inline() {
    let (st, store) = on_t();
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Enter)); // expand
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down)); // trigger row
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down)); // shortcut row
    let (st, s2, eff) = handle_key(&st, &store, key(KeyCode::Char('d')));
    assert_eq!(eff, Effect::Save, "an inline store edit still persists");
    let t = s2.aliases.iter().find(|d| d.name == "t").expect("t exists");
    assert!(t.shortcuts.is_empty());
    assert_eq!(
        st.status,
        Some((true, "shortcut removed: t.baidu".to_string()))
    );
    assert_eq!(st.expanded, None, "the list collapses after a removal");
}

#[test]
fn reclamp_keeps_the_cursor_on_a_row() {
    let store = store_with_t();
    let mut st = Settings {
        cursor: 4,
        expanded: Some(2),
        ..new()
    };
    reclamp(&mut st, &store);
    assert_eq!(st.cursor, 4, "rows: 3 aliases + 1 trigger + 1 shortcut");
    let mut empty = new();
    empty.cursor = 7;
    reclamp(&mut empty, &Store::default());
    assert_eq!(empty.cursor, 1, "clamped to the last of 2 aliases");
}

#[test]
fn navigation_moves_and_clamps() {
    let store = store_with_t();
    let aliases = view(&store);
    let flat = rows(&aliases, None);
    assert_eq!(move_sel(&flat, 0, -1), 0);
    assert_eq!(move_sel(&flat, 0, 1), 1);
    assert_eq!(move_sel(&flat, 2, 5), 2);
    assert_eq!(move_sel(&flat, 99, -1), 1);
    assert_eq!(move_sel(&[], 3, 1), 0);
}

#[test]
fn enter_toggles_expansion_then_collapses() {
    let store = store_with_t();
    let (st, _, _) = handle_key(&new(), &store, key(KeyCode::Enter));
    assert_eq!(st.expanded, Some(0));
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Right));
    assert_eq!(st.expanded, None, "Enter again collapses");
    // expanding, landing on an entry row, then Left collapses too
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Enter));
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down));
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down));
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Left));
    assert_eq!(st.expanded, None);
}

#[test]
fn q_and_esc_back_ctrl_c_quits() {
    let store = store_with_t();
    assert_eq!(
        handle_key(&new(), &store, key(KeyCode::Esc)).2,
        Effect::Back
    );
    assert_eq!(
        handle_key(&new(), &store, key(KeyCode::Char('q'))).2,
        Effect::Back
    );
    assert_eq!(
        handle_key(
            &new(),
            &store,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
        )
        .2,
        Effect::Quit
    );
}

/// Alt+D (the default wake key) and the other modified chars must not
/// reach the plain-char arms: only Ctrl+C / Ctrl+D quit from here.
#[test]
fn modified_char_keys_do_not_fire_list_actions() {
    let (st, store) = on_t();
    let alt_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT);
    assert_eq!(
        handle_key(&st, &store, alt_d),
        (st.clone(), store.clone(), Effect::None),
        "Alt+D must not delete the selection"
    );
    let alt_k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::ALT);
    assert_eq!(
        handle_key(&st, &store, alt_k),
        (st.clone(), store.clone(), Effect::None),
        "Alt+K must not move the selection"
    );

    for c in ['k', 'j', 'q', 'n', 's', 'a', 't', 'e'] {
        let ctrl = KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        assert_eq!(
            handle_key(&st, &store, ctrl),
            (st.clone(), store.clone(), Effect::None),
            "Ctrl+{c} must not fire the list actions"
        );
    }
    assert_eq!(store.aliases.len(), 3, "no alias was deleted");
}

#[test]
fn alt_word_motions_reach_the_form_but_stay_dead_on_the_list() {
    let (st, store) = on_t();
    let alt_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT);
    assert_eq!(
        handle_key(&st, &store, alt_d),
        (st.clone(), store.clone(), Effect::None),
        "Alt+D must not delete the selection from the list"
    );

    // ...but inside the wizard Alt+B / Alt+F move the form caret.
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('e')));
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::End));
    let form = st.form.as_ref().expect("edit wizard open");
    assert_eq!(form.caret, "printf %s {input}".chars().count());

    let alt_b = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT);
    let (st, _, eff) = handle_key(&st, &store, alt_b);
    assert_eq!(eff, Effect::None);
    let form = st.form.as_ref().expect("form still open");
    assert_eq!(
        form.caret, 10,
        "Alt+B jumps to the start of the placeholder"
    );

    // an unmapped ALT key in the form neither edits nor quits
    let before = form.input.clone();
    let (st, _, eff) = handle_key(&st, &store, alt_d);
    assert_eq!(eff, Effect::None);
    let form = st.form.as_ref().expect("form still open");
    assert_eq!((form.input.clone(), form.caret), (before, 10));
}

#[test]
fn ctrl_c_still_quits_from_the_list() {
    let (st, store) = on_t();
    let (_, _, eff) = handle_key(
        &st,
        &store,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert_eq!(eff, Effect::Quit);
    let (_, _, eff) = handle_key(
        &st,
        &store,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
    );
    assert_eq!(eff, Effect::Quit);
}

#[test]
fn release_events_are_ignored() {
    let store = store_with_t();
    let release = KeyEvent::new_with_kind(
        KeyCode::Char('d'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    );
    let (st, s2, eff) = handle_key(&new(), &store, release);
    assert_eq!((st, eff), (new(), Effect::None));
    assert_eq!(s2, store);
}
