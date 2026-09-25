//! Row-level `e` edit-wizard tests for `/settings`: the command-editing
//! wizard for the selected alias plus the per-entry edit/rename wizards.
//! Split out of `settings/list_tests.rs` to keep both files small.

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
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
fn e_opens_the_edit_wizard_prefilled_with_both_commands() {
    let (st, store) = on_t();
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Char('e')));
    assert_eq!(eff, Effect::None);
    let form = st.form.expect("edit wizard open");
    assert_eq!(
        form.purpose,
        settings_form::Purpose::EditCommand {
            alias: "t".to_string()
        }
    );
    assert_eq!(form.input, "printf %s {input}", "linux step prefilled");
    assert_eq!(form.macos, "", "t has no macos command");
    assert_eq!(settings_form::step_count(&form), 2);
}

#[test]
fn edit_wizard_accepts_prefilled_text_and_mirrors_blank_macos() {
    let (st, store) = on_t();
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('e')));
    // Enter accepts the prefilled linux step, Enter the (empty) macos one
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Enter));
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter));
    assert_eq!(st.form, None);
    assert_eq!(
        eff,
        Effect::SetCommands {
            alias: "t".to_string(),
            linux: "printf %s {input}".to_string(),
            macos: String::new(),
        },
        "a blank macos answer mirrors linux in alias::set_commands"
    );
}

/// Expand `t` and park the cursor on its `idx`-th entry row (0 = the trigger,
/// 1 = the concrete shortcut).
fn on_entry(idx: usize) -> (Settings, Store) {
    let (st, store) = on_t();
    let (mut st, _, _) = handle_key(&st, &store, key(KeyCode::Enter));
    for _ in 0..=idx {
        let (next, _, _) = handle_key(&st, &store, key(KeyCode::Down));
        st = next;
    }
    (st, store)
}

#[test]
fn e_on_a_shortcut_row_opens_the_prefilled_edit_wizard() {
    let (st, store) = on_entry(1); // shortcut row
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Char('e')));
    assert_eq!(eff, Effect::None);
    let form = st.form.as_ref().expect("edit-shortcut wizard open");
    assert_eq!(
        form.purpose,
        settings_form::Purpose::EditShortcut {
            alias: "t".to_string(),
            old_key: "baidu".to_string(),
        }
    );
    assert_eq!(form.input, "baidu", "the current key is prefilled");
    assert_eq!(settings_form::step_count(form), 2);
    // Enter accepts the key; the value step is prefilled with the current one.
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Enter));
    let form = st.form.as_ref().expect("value step open");
    assert_eq!(form.step, 1);
    assert_eq!(form.input, "https://www.baidu.com");
}

#[test]
fn edit_shortcut_wizard_submits_the_old_key_with_the_new_pair() {
    let (st, store) = on_entry(1);
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('e')));
    // Enter alone accepts the prefilled key/value pair, old_key included.
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Enter));
    let (_, _, eff) = handle_key(&st, &store, key(KeyCode::Enter));
    assert_eq!(
        eff,
        Effect::EditShortcut {
            alias: "t".to_string(),
            old_key: "baidu".to_string(),
            key: "baidu".to_string(),
            value: "https://www.baidu.com".to_string(),
        },
        "Enter through both steps keeps the entry as it was"
    );

    // Rewriting both fields carries the original key for the swap.
    let (fresh, _) = on_entry(1);
    let (st, _, _) = handle_key(&fresh, &store, key(KeyCode::Char('e')));
    let (st, _, _) = handle_key(&st, &store, ctrl('u'));
    let st = type_str(&st, &store, "tieba");
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Enter));
    let (st, _, _) = handle_key(&st, &store, ctrl('u'));
    let st = type_str(&st, &store, "https://tieba.baidu.com");
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter));
    assert_eq!(st.form, None, "the wizard closes on submit");
    assert_eq!(
        eff,
        Effect::EditShortcut {
            alias: "t".to_string(),
            old_key: "baidu".to_string(),
            key: "tieba".to_string(),
            value: "https://tieba.baidu.com".to_string(),
        }
    );
}

#[test]
fn e_on_a_trigger_row_opens_the_prefilled_rename_wizard() {
    let (st, store) = on_entry(0); // trigger row
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Char('e')));
    assert_eq!(eff, Effect::None);
    let form = st.form.as_ref().expect("edit-trigger wizard open");
    assert_eq!(
        form.purpose,
        settings_form::Purpose::EditTrigger {
            alias: "t".to_string(),
            old: "tt".to_string(),
        }
    );
    assert_eq!(form.input, "tt", "the current word is prefilled");
    assert_eq!(settings_form::step_count(form), 1);
    // Enter alone renames it to itself; the store helper treats that as a
    // no-op, so the list is never touched.
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter));
    assert_eq!(st.form, None);
    assert_eq!(
        eff,
        Effect::RenameTrigger {
            alias: "t".to_string(),
            old: "tt".to_string(),
            new: "tt".to_string(),
        }
    );
}

#[test]
fn edit_trigger_wizard_submits_the_new_word() {
    let (st, store) = on_entry(0);
    let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('e')));
    let (st, _, _) = handle_key(&st, &store, ctrl('u'));
    let st = type_str(&st, &store, "tw");
    let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter));
    assert_eq!(st.form, None);
    assert_eq!(
        eff,
        Effect::RenameTrigger {
            alias: "t".to_string(),
            old: "tt".to_string(),
            new: "tw".to_string(),
        }
    );
}
