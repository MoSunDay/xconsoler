//! Caret/editing tests for the apply layer: a real text caret moves and
//! edits the input without touching the candidate highlight. Split out of
//! `crate::app` to keep every file within its line budget.

use super::*;
use crate::action::on_key;
use crate::storage::Store;
use crate::textedit::Motion;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;

fn setup() -> (App, TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.json");
    (state::new(Store::default(), false), dir, path)
}

#[test]
fn typing_lands_at_the_caret_not_the_end() {
    let (mut app, _dir, path) = setup();
    app.input = "ac".to_string();
    app.caret = 1;
    app.cursor = 2;
    app.status = Some((true, "stale".to_string()));
    apply(&mut app, Action::InsertChar('b'), Platform::Linux, &path);
    assert_eq!(app.input, "abc");
    assert_eq!(app.caret, 2);
    assert_eq!(app.cursor, 0, "candidate cursor reset by the edit");
    assert_eq!(app.status, None, "status reset by the edit");
}

#[test]
fn backspace_and_delete_move_the_caret_at_the_boundaries() {
    let (mut app, _dir, path) = setup();
    app.input = "abc".to_string();
    app.caret = 1;
    apply(&mut app, Action::Backspace, Platform::Linux, &path);
    assert_eq!((app.input.as_str(), app.caret), ("bc", 0));
    apply(&mut app, Action::Backspace, Platform::Linux, &path);
    assert_eq!((app.input.as_str(), app.caret), ("bc", 0), "no-op at start");

    app.input = "abc".to_string();
    app.caret = 2;
    apply(&mut app, Action::DeleteForward, Platform::Linux, &path);
    assert_eq!((app.input.as_str(), app.caret), ("ab", 2));
    apply(&mut app, Action::DeleteForward, Platform::Linux, &path);
    assert_eq!((app.input.as_str(), app.caret), ("ab", 2), "no-op at end");
}

#[test]
fn motions_leave_text_status_and_palette_alone() {
    let (mut app, _dir, path) = setup();
    app.input = "git commit -m foo".to_string();
    app.caret = 4;
    app.status = Some((true, "kept".to_string()));
    apply(
        &mut app,
        Action::Motion(Motion::End),
        Platform::Linux,
        &path,
    );
    assert_eq!(app.caret, 17);
    assert_eq!(app.input, "git commit -m foo");
    assert_eq!(app.status, Some((true, "kept".to_string())));

    apply(
        &mut app,
        Action::Motion(Motion::Home),
        Platform::Linux,
        &path,
    );
    assert_eq!(app.caret, 0);
    apply(
        &mut app,
        Action::Motion(Motion::WordRight),
        Platform::Linux,
        &path,
    );
    assert_eq!(app.caret, 3);
    apply(
        &mut app,
        Action::Motion(Motion::WordLeft),
        Platform::Linux,
        &path,
    );
    assert_eq!(app.caret, 0);
    apply(
        &mut app,
        Action::Motion(Motion::Left),
        Platform::Linux,
        &path,
    );
    assert_eq!(app.caret, 0, "clamped at the start");
}

#[test]
fn kill_and_transpose_operate_at_the_caret() {
    let (mut app, _dir, path) = setup();
    app.input = "git commit foo".to_string();
    app.caret = 11;
    apply(&mut app, Action::KillWord, Platform::Linux, &path);
    assert_eq!((app.input.as_str(), app.caret), ("git foo", 4));

    app.input = "git commit foo".to_string();
    app.caret = 14;
    apply(&mut app, Action::KillToStart, Platform::Linux, &path);
    assert_eq!((app.input.as_str(), app.caret), ("", 0));

    app.input = "git commit foo".to_string();
    app.caret = 4;
    apply(&mut app, Action::KillToEnd, Platform::Linux, &path);
    assert_eq!((app.input.as_str(), app.caret), ("git ", 4));

    app.input = "abc".to_string();
    app.caret = 3;
    apply(&mut app, Action::Transpose, Platform::Linux, &path);
    assert_eq!((app.input.as_str(), app.caret), ("acb", 3));
}

#[test]
fn set_input_puts_the_caret_at_the_end() {
    let (mut app, _dir, path) = setup();
    app.caret = 0;
    set_input(&mut app, "hello".to_string());
    assert_eq!((app.input.as_str(), app.caret), ("hello", 5));

    // The colon-command reset clears both text and caret.
    app.input = ":add t echo {input}".to_string();
    app.caret = 20;
    apply(&mut app, Action::SubmitColon, Platform::Linux, &path);
    assert!(app.input.is_empty());
    assert_eq!(app.caret, 0);
}

/// End-to-end through the key map: Ctrl+A/E/W/K/T reach the caret ops.
#[test]
fn readline_keys_reach_the_input_through_on_key() {
    let (mut app, _dir, path) = setup();
    let press = |app: &mut App, c: char| {
        let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        let action = on_key(app, key);
        apply(app, action, Platform::Linux, &path);
    };

    app.input = "git commit foo".to_string();
    app.caret = 14;
    press(&mut app, 'a');
    assert_eq!(app.caret, 0, "Ctrl+A goes home");
    press(&mut app, 'e');
    assert_eq!(app.caret, 14, "Ctrl+E goes to the end");

    press(&mut app, 'w');
    assert_eq!((app.input.as_str(), app.caret), ("git commit ", 11));

    app.input = "abc".to_string();
    app.caret = 3;
    press(&mut app, 't');
    assert_eq!((app.input.as_str(), app.caret), ("acb", 3), "Ctrl+T");

    press(&mut app, 'u');
    assert_eq!(
        (app.input.as_str(), app.caret),
        ("", 0),
        "Ctrl+U kills to start"
    );

    app.input = "a b".to_string();
    app.caret = 1;
    press(&mut app, 'k');
    assert_eq!(
        (app.input.as_str(), app.caret),
        ("a", 1),
        "Ctrl+K kills to end"
    );
}
