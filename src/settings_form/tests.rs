//! Wizard tests for `settings_form.rs`: the 3-step new-alias wizard and the
//! single-step command editor, split out so both files stay within the size
//! budget. Readline editing tests live in `settings_form_edit_tests.rs`.

use super::*;
use crate::platform::Platform;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn type_str(f: &Form, s: &str, store: &Store) -> Form {
    let mut f = f.clone();
    for c in s.chars() {
        let (nf, out) = handle_key(&f, store, key(KeyCode::Char(c)));
        assert_eq!(out, FormOutcome::Active);
        f = nf;
    }
    f
}

fn enter(f: &Form, store: &Store) -> (Form, FormOutcome) {
    handle_key(f, store, key(KeyCode::Enter))
}

fn empty_store() -> Store {
    Store::default()
}

#[test]
fn cancel_and_quit_leave_the_draft_behind() {
    let f = new_alias(Platform::Linux);
    let (_, out) = handle_key(&f, &empty_store(), key(KeyCode::Esc));
    assert_eq!(out, FormOutcome::Cancel);
    let (_, out) = handle_key(&f, &empty_store(), ctrl('c'));
    assert_eq!(out, FormOutcome::Quit);
}

#[test]
fn typing_edits_and_backspaces_the_input() {
    let store = empty_store();
    let f = type_str(&new_alias(Platform::Linux), "ab", &store);
    assert_eq!(
        (f.input.as_str(), f.caret),
        ("ab", 2),
        "typing moves the caret"
    );
    let (f, _) = handle_key(&f, &store, key(KeyCode::Backspace));
    assert_eq!((f.input.as_str(), f.caret), ("a", 1));
    let (f, _) = handle_key(&f, &store, ctrl('u'));
    assert_eq!((f.input.as_str(), f.caret), ("", 0));
}

#[test]
fn new_alias_walks_all_steps_and_submits_linux_only() {
    let store = empty_store();
    let f = type_str(&new_alias(Platform::Linux), " mytool ", &store);
    let (f, out) = enter(&f, &store);
    assert_eq!(out, FormOutcome::Active);
    assert_eq!(f.step, 1);
    assert_eq!(f.name, "mytool");

    let f = type_str(&f, "mt, my", &store);
    let (f, _) = enter(&f, &store);
    assert_eq!(f.step, 2);

    let f = type_str(&f, "printf %s {input}", &store);
    let (f, out) = enter(&f, &store);
    assert_eq!(f.step, 3, "the third Enter submits");

    match out {
        FormOutcome::Submit(Submission::Alias(def)) => {
            assert_eq!(def.name, "mytool");
            assert_eq!(def.triggers, vec!["mt".to_string(), "my".to_string()]);
            assert_eq!(def.linux.as_deref(), Some("printf %s {input}"));
            assert_eq!(def.macos, None, "only the wizard's platform is stored");
            assert!(def.shortcuts.is_empty());
        }
        other => panic!("expected Submit, got {other:?}"),
    }
    assert_eq!(step_count(&f), 3);
}

#[test]
fn new_alias_on_macos_stores_only_the_macos_command() {
    let store = empty_store();
    let f = type_str(&new_alias(Platform::Macos), "mytool", &store);
    let (f, _) = enter(&f, &store); // name
    let (f, _) = enter(&f, &store); // empty triggers
    let f = type_str(&f, "open {input}", &store);
    let (f, out) = enter(&f, &store);
    assert_eq!(f.error, None);
    match out {
        FormOutcome::Submit(Submission::Alias(def)) => {
            assert_eq!(def.name, "mytool");
            assert_eq!(def.macos.as_deref(), Some("open {input}"));
            assert_eq!(def.linux, None, "linux is not mirrored");
        }
        other => panic!("expected Submit, got {other:?}"),
    }
}

#[test]
fn validation_errors_stay_on_the_step() {
    let store = empty_store();
    // empty name
    let (f, out) = enter(&new_alias(Platform::Linux), &store);
    assert_eq!(out, FormOutcome::Active);
    assert_eq!(f.step, 0);
    assert_eq!(f.error.as_deref(), Some("name cannot be empty"));
    // invalid name chars
    let f = type_str(&new_alias(Platform::Linux), "bad!", &store);
    let (f, out) = enter(&f, &store);
    assert_eq!(out, FormOutcome::Active);
    assert!(f.error.unwrap().contains("invalid name"));
    // duplicate name (against the seeded aliases, case-insensitive)
    let f = type_str(&new_alias(Platform::Linux), "BR", &store);
    let (f, out) = enter(&f, &store);
    assert_eq!(out, FormOutcome::Active);
    assert!(f.error.unwrap().contains("already in use"));
    // empty command on the command step; the message names the platform
    let mut f = new_alias(Platform::Linux);
    f.step = 2;
    let (f, out) = enter(&f, &store);
    assert_eq!(out, FormOutcome::Active);
    assert_eq!(f.error.as_deref(), Some("linux command cannot be empty"));
    let mut f = new_alias(Platform::Macos);
    f.step = 2;
    let (f, out) = enter(&f, &store);
    assert_eq!(out, FormOutcome::Active);
    assert_eq!(f.error.as_deref(), Some("macos command cannot be empty"));
}

#[test]
fn new_shortcut_walks_key_then_value() {
    let mut store = empty_store();
    let mut def = crate::alias::defaults().remove(0);
    def.name = "t".to_string();
    store.aliases.push(def);

    let f = new_shortcut("t");
    assert_eq!(step_count(&f), 2);
    let f = type_str(&f, "baidu", &store);
    let (f, out) = enter(&f, &store);
    assert_eq!(out, FormOutcome::Active);
    assert_eq!(f.step, 1);

    // multi-word keys are rejected inline
    let bad = type_str(&new_shortcut("t"), "two words", &store);
    let (bad, out) = enter(&bad, &store);
    assert_eq!(out, FormOutcome::Active);
    assert_eq!(bad.error.as_deref(), Some("shortcut key must be one word"));

    let f = type_str(&f, "https://www.baidu.com", &store);
    let (_, out) = enter(&f, &store);
    match out {
        FormOutcome::Submit(Submission::Shortcut { alias, key, value }) => {
            assert_eq!(
                (alias.as_str(), key.as_str(), value.as_str()),
                ("t", "baidu", "https://www.baidu.com")
            );
        }
        other => panic!("expected Submit, got {other:?}"),
    }
}

#[test]
fn trigger_wizard_is_one_step_and_validates() {
    let store = empty_store();
    let f = new_trigger("t");
    assert_eq!(step_count(&f), 1);
    assert_eq!(f.input, "");

    // empty and malformed answers stay on the (only) step
    let (f, out) = enter(&new_trigger("t"), &store);
    assert_eq!(out, FormOutcome::Active);
    assert_eq!(f.error.as_deref(), Some("trigger cannot be empty"));
    let bad = type_str(&new_trigger("t"), "bad!", &store);
    let (bad, out) = enter(&bad, &store);
    assert_eq!(out, FormOutcome::Active);
    assert!(bad.error.unwrap().contains("invalid trigger"));

    let f = type_str(&new_trigger("t"), "gc", &store);
    let (f, out) = enter(&f, &store);
    match out {
        FormOutcome::Submit(Submission::Trigger { alias, trigger }) => {
            assert_eq!((alias.as_str(), trigger.as_str()), ("t", "gc"));
        }
        other => panic!("expected Submit, got {other:?}"),
    }
    assert_eq!(f.error, None);
}

#[test]
fn edit_command_wizard_is_one_prefilled_step() {
    let store = empty_store();
    let f = new_edit_command("br", Platform::Linux, Some("xdg-open {input}"));
    assert_eq!(step_count(&f), 1);
    assert_eq!(f.input, "xdg-open {input}", "the step starts prefilled");
    assert_eq!(f.command, "xdg-open {input}");
    assert_eq!(f.caret, "xdg-open {input}".chars().count());
    assert_eq!(
        f.purpose,
        Purpose::EditCommand {
            alias: "br".to_string(),
            platform: Platform::Linux,
        }
    );

    // Enter accepts the prefilled command as-is and submits immediately.
    let (f, out) = enter(&f, &store);
    match out {
        FormOutcome::Submit(Submission::Command {
            alias,
            platform,
            command,
        }) => {
            assert_eq!(alias, "br");
            assert_eq!(platform, Platform::Linux);
            assert_eq!(command, "xdg-open {input}");
        }
        other => panic!("expected Submit, got {other:?}"),
    }
    assert_eq!(f.error, None);
}

#[test]
fn edit_command_no_stored_command_prefills_empty() {
    let store = empty_store();
    let f = new_edit_command("t", Platform::Macos, None);
    assert_eq!(f.input, "", "no macos command to prefill");
    assert_eq!(f.command, "");

    // Enter on the empty prefilled line refuses and names the platform.
    let (f, out) = enter(&f, &store);
    assert_eq!(out, FormOutcome::Active);
    assert_eq!(f.error.as_deref(), Some("macos command cannot be empty"));

    let f = type_str(&f, "open -a Safari {input}", &store);
    let (_, out) = enter(&f, &store);
    match out {
        FormOutcome::Submit(Submission::Command {
            platform, command, ..
        }) => {
            assert_eq!(platform, Platform::Macos);
            assert_eq!(command, "open -a Safari {input}");
        }
        other => panic!("expected Submit, got {other:?}"),
    }
}

#[test]
fn edit_command_ctrl_u_clears_and_blank_is_refused() {
    let store = empty_store();
    let f = new_edit_command("t", Platform::Linux, Some("printf %s {input}"));

    // Ctrl+U clears the prefilled text; Enter on the empty field refuses
    let (f, _) = handle_key(&f, &store, ctrl('u'));
    assert_eq!(f.input, "");
    let (f, out) = enter(&f, &store);
    assert_eq!(out, FormOutcome::Active);
    assert_eq!(f.error.as_deref(), Some("linux command cannot be empty"));

    let f = type_str(&f, "echo {input}", &store);
    let (_, out) = enter(&f, &store);
    match out {
        FormOutcome::Submit(Submission::Command { command, .. }) => {
            assert_eq!(command, "echo {input}");
        }
        other => panic!("expected Submit, got {other:?}"),
    }
}
