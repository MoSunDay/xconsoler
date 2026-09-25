//! Caret and readline-editing tests for the wizard's bottom input line:
//! char/word motions, kill keys and the readline control keys. Split out of
//! `settings_form.rs` so both files stay within the size budget.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
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
fn insert_and_delete_work_around_a_middle_caret() {
    let store = empty_store();
    let f = type_str(&new_alias(), "ac", &store);
    let (f, _) = handle_key(&f, &store, key(KeyCode::Left));
    assert_eq!((f.input.as_str(), f.caret), ("ac", 1));
    let f = type_str(&f, "b", &store);
    assert_eq!((f.input.as_str(), f.caret), ("abc", 2), "inserted mid-line");

    let (f, _) = handle_key(&f, &store, key(KeyCode::Left));
    let (f, _) = handle_key(&f, &store, key(KeyCode::Delete));
    assert_eq!(
        (f.input.as_str(), f.caret),
        ("ac", 1),
        "Delete eats the char under the caret"
    );
    let (f, _) = handle_key(&f, &store, key(KeyCode::Backspace));
    assert_eq!((f.input.as_str(), f.caret), ("c", 0));

    // no-ops at the edges
    let (f, _) = handle_key(&f, &store, key(KeyCode::Delete));
    assert_eq!((f.input.as_str(), f.caret), ("", 0));
    let (f, _) = handle_key(&f, &store, key(KeyCode::Backspace));
    assert_eq!((f.input.as_str(), f.caret), ("", 0));
}

#[test]
fn caret_motions_move_without_changing_the_text() {
    let store = empty_store();
    let f = type_str(&new_alias(), "git commit", &store);
    assert_eq!(f.caret, 10, "typing leaves the caret at the end");

    let (f, _) = handle_key(&f, &store, ctrl('a'));
    assert_eq!((f.input.as_str(), f.caret), ("git commit", 0));
    let (f, _) = handle_key(&f, &store, ctrl('e'));
    assert_eq!(f.caret, 10);
    let (f, _) = handle_key(&f, &store, key(KeyCode::Left));
    assert_eq!(f.caret, 9);
    let (f, _) = handle_key(&f, &store, key(KeyCode::Home));
    assert_eq!(f.caret, 0);
    let (f, _) = handle_key(&f, &store, key(KeyCode::End));
    assert_eq!(f.caret, 10);
    let (f, _) = handle_key(&f, &store, key(KeyCode::Right));
    assert_eq!(f.caret, 10, "Right is clamped at the end");
    let (f, _) = handle_key(&f, &store, ctrl('b'));
    assert_eq!(f.caret, 9);
    let (f, _) = handle_key(&f, &store, ctrl('f'));
    assert_eq!(f.caret, 10);
    assert_eq!(f.input, "git commit", "motions never change the text");

    let (f, _) = handle_key(&f, &store, ctrl('h'));
    assert_eq!(
        (f.input.as_str(), f.caret),
        ("git commi", 9),
        "Ctrl+H is Backspace"
    );
}

#[test]
fn word_motions_use_ctrl_arrows_and_alt_b_f() {
    let store = empty_store();
    let f = type_str(&new_alias(), "git commit -m", &store);
    let (f, _) = handle_key(&f, &store, ctrl('a'));

    let (f, _) = handle_key(&f, &store, alt('f'));
    assert_eq!(f.caret, 3, "Alt+F lands after 'git'");
    let (f, _) = handle_key(&f, &store, alt('f'));
    assert_eq!(f.caret, 10, "Alt+F lands after 'commit'");
    let (f, _) = handle_key(&f, &store, alt('b'));
    assert_eq!(f.caret, 4, "Alt+B lands at the start of 'commit'");
    let (f, _) = handle_key(&f, &store, alt('a'));
    assert_eq!(f.caret, 4, "unmapped Alt keys stay no-ops");

    let ctrl_right = KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL);
    let ctrl_left = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
    let alt_right = KeyEvent::new(KeyCode::Right, KeyModifiers::ALT);
    let alt_left = KeyEvent::new(KeyCode::Left, KeyModifiers::ALT);
    let (f, _) = handle_key(&f, &store, ctrl_right);
    assert_eq!(f.caret, 10, "Ctrl+Right = word right");
    let (f, _) = handle_key(&f, &store, ctrl_left);
    assert_eq!(f.caret, 4, "Ctrl+Left = word left");
    let (f, _) = handle_key(&f, &store, alt_right);
    assert_eq!(f.caret, 10, "Alt+Right = word right");
    let (f, _) = handle_key(&f, &store, alt_left);
    assert_eq!(f.caret, 4, "Alt+Left = word left");
    assert_eq!(f.input, "git commit -m");
}

#[test]
fn kill_keys_cut_around_the_caret() {
    let store = empty_store();
    let f = type_str(&new_alias(), "git commit foo", &store);
    let (f, _) = handle_key(&f, &store, alt('b'));
    assert_eq!((f.input.as_str(), f.caret), ("git commit foo", 11));

    let (f, _) = handle_key(&f, &store, ctrl('u'));
    assert_eq!(
        (f.input.as_str(), f.caret),
        ("foo", 0),
        "Ctrl+U kills to the start"
    );
    let (f, _) = handle_key(&f, &store, ctrl('e'));
    let (f, _) = handle_key(&f, &store, ctrl('k'));
    assert_eq!(
        (f.input.as_str(), f.caret),
        ("foo", 3),
        "Ctrl+K at the end is a no-op"
    );

    let (f, _) = handle_key(&f, &store, key(KeyCode::Home));
    let (f, _) = handle_key(&f, &store, key(KeyCode::Right));
    let (f, _) = handle_key(&f, &store, ctrl('k'));
    assert_eq!(
        (f.input.as_str(), f.caret),
        ("f", 1),
        "Ctrl+K kills to the end"
    );

    let f = type_str(&f, "oo bar", &store);
    let (f, _) = handle_key(&f, &store, ctrl('w'));
    assert_eq!(
        (f.input.as_str(), f.caret),
        ("foo ", 4),
        "Ctrl+W kills the whitespace-delimited word before the caret"
    );
}

#[test]
fn ctrl_t_transposes_around_the_caret() {
    let store = empty_store();
    let f = type_str(&new_alias(), "ab", &store);
    let (f, _) = handle_key(&f, &store, ctrl('t'));
    assert_eq!(
        (f.input.as_str(), f.caret),
        ("ba", 2),
        "swaps the last two at the end"
    );

    let (f, _) = handle_key(&f, &store, ctrl('a'));
    let (f, _) = handle_key(&f, &store, ctrl('t'));
    assert_eq!(
        (f.input.as_str(), f.caret),
        ("ba", 0),
        "caret at the start is a no-op"
    );

    let (f, _) = handle_key(&f, &store, key(KeyCode::Right));
    let (f, _) = handle_key(&f, &store, ctrl('t'));
    assert_eq!(
        (f.input.as_str(), f.caret),
        ("ab", 2),
        "swaps then steps over the pair"
    );
}

#[test]
fn ctrl_d_quits_and_delete_deletes_forward() {
    let store = empty_store();
    // Ctrl+D quits with or without text; it never edits.
    let (_, out) = handle_key(&new_alias(), &store, ctrl('d'));
    assert_eq!(out, FormOutcome::Quit, "Ctrl+D quits on an empty line");
    let f = type_str(&new_alias(), "ab", &store);
    let (f, out) = handle_key(&f, &store, ctrl('d'));
    assert_eq!(out, FormOutcome::Quit, "Ctrl+D quits with text too");
    assert_eq!(
        (f.input.as_str(), f.caret),
        ("ab", 2),
        "Ctrl+D leaves the text alone"
    );

    // Delete still eats the char under the caret.
    let f = type_str(&new_alias(), "ab", &store);
    let (f, _) = handle_key(&f, &store, key(KeyCode::Home));
    let (f, out) = handle_key(&f, &store, key(KeyCode::Delete));
    assert_eq!(out, FormOutcome::Active);
    assert_eq!((f.input.as_str(), f.caret), ("b", 0));
    let (f, out) = handle_key(&f, &store, key(KeyCode::Delete));
    assert_eq!(out, FormOutcome::Active);
    assert_eq!((f.input.as_str(), f.caret), ("", 0));
}

#[test]
fn prefill_and_advance_leave_the_caret_at_the_end() {
    let store = empty_store();
    let f = new_edit_command("t", Some("printf %s {input}"), Some("open {input}"));
    assert_eq!(f.caret, f.input.chars().count());
    assert_eq!(f.caret, 17, "prefilled linux command: caret at the end");

    // the second edit-command step is prefilled too
    let (f, _) = enter(&f, &store);
    assert_eq!((f.input.as_str(), f.caret), ("open {input}", 12));

    // Enter trims the field and parks the caret at the end of what stays
    let f = type_str(&new_alias(), " bad! ", &store);
    let (f, out) = enter(&f, &store);
    assert_eq!(out, FormOutcome::Active, "invalid name stays on the step");
    assert_eq!((f.input.as_str(), f.caret), ("bad!", 4));
}
