//! Slash-command palette tests for the apply layer: `/` opens and filters the
//! list, Enter runs the highlighted row, and Esc leaves the fuzzy fallback in
//! place. Split out of `crate::app` to keep every file within its budget.

use super::*;
use crate::action::on_key;
use crate::storage::Store;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;

fn setup() -> (App, TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.json");
    (state::new(Store::default(), false), dir, path)
}

fn shown(app: &mut App) {
    app.visibility = Visibility::Shown;
}

fn type_str(app: &mut App, s: &str, path: &Path) {
    for c in s.chars() {
        apply(app, Action::InsertChar(c), Platform::Linux, path);
    }
}

#[test]
fn slash_opens_the_palette_and_plain_text_closes_it() {
    let (mut app, _dir, path) = setup();
    shown(&mut app);
    apply(&mut app, Action::InsertChar('/'), Platform::Linux, &path);
    assert_eq!(app.input, "/");
    assert_eq!(app.palette, Some(0), "`/` opens the slash list");

    // Deleting the `/` leaves the slash query: the list closes.
    apply(&mut app, Action::Backspace, Platform::Linux, &path);
    assert_eq!(app.input, "");
    assert_eq!(app.palette, None, "`/` deleted: no list");

    // Plain text never opens it (the hotkey shows the full catalog).
    apply(&mut app, Action::InsertChar('b'), Platform::Linux, &path);
    assert_eq!(app.palette, None);
    assert_eq!(app.caret, 1, "the edit still landed");
}

#[test]
fn slash_query_filters_the_list_and_arrows_clamp_inside_it() {
    let (mut app, _dir, path) = setup();
    shown(&mut app);
    type_str(&mut app, "/s", &path);
    assert_eq!(app.palette, Some(0));
    assert_eq!(
        commands::palette_items(&app.input).len(),
        1,
        "only /settings matches /s"
    );

    apply(&mut app, Action::MoveDown, Platform::Linux, &path);
    assert_eq!(app.palette, Some(0), "clamped: one row");
    apply(&mut app, Action::MoveUp, Platform::Linux, &path);
    assert_eq!(app.palette, Some(0), "clamped: one row");

    // A stale selection from a longer list is clamped on the next edit.
    app.palette = Some(6);
    type_str(&mut app, "e", &path); // "/se"
    assert_eq!(app.palette, Some(0), "selection clamped into the filter");
    type_str(&mut app, "t", &path); // "/set"
    assert_eq!(app.palette, Some(0));
    assert_eq!(commands::palette_items(&app.input)[0].token, "/settings");
}

#[test]
fn palette_accept_on_a_filtered_slash_query_opens_settings() {
    let (mut app, _dir, path) = setup();
    shown(&mut app);
    type_str(&mut app, "/set", &path);
    assert_eq!(app.palette, Some(0));

    apply(&mut app, Action::PaletteAccept, Platform::Linux, &path);
    assert_eq!(app.palette, None);
    assert!(matches!(app.mode, Mode::Settings(_)));
    assert!(app.input.is_empty(), "command line cleared on entry");
}

#[test]
fn unmatched_slash_query_closes_the_palette_and_keeps_the_unknown_status() {
    let (mut app, _dir, path) = setup();
    shown(&mut app);
    type_str(&mut app, "/zz", &path);
    assert_eq!(app.palette, None, "no matches: no list");

    apply(&mut app, Action::Execute, Platform::Linux, &path);
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.input, "/zz", "unknown command keeps the line");
    assert_eq!(
        app.status,
        Some((
            false,
            "unknown command: /zz \u{b7} try /settings".to_string()
        ))
    );
}

#[test]
fn esc_closes_the_list_and_enter_still_runs_the_top_match() {
    let (mut app, _dir, path) = setup();
    shown(&mut app);
    type_str(&mut app, "/set", &path);

    let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    let action = on_key(&app, esc);
    apply(&mut app, action, Platform::Linux, &path);
    assert_eq!(app.palette, None, "Esc closed the list");

    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let action = on_key(&app, enter);
    apply(&mut app, action, Platform::Linux, &path);
    assert!(
        matches!(app.mode, Mode::Settings(_)),
        "/set falls back to its top fuzzy match"
    );
}

#[test]
fn open_palette_needs_a_non_empty_row_set() {
    let (mut app, _dir, path) = setup();
    shown(&mut app);
    apply(&mut app, Action::OpenPalette, Platform::Linux, &path);
    assert_eq!(app.palette, Some(0), "empty input: the plain catalog");
    assert_eq!(commands::palette_items(&app.input).len(), commands::len());

    app.input = "/zz".to_string();
    apply(&mut app, Action::OpenPalette, Platform::Linux, &path);
    assert_eq!(app.palette, None, "a query with no rows opens nothing");
}

#[test]
fn an_open_palette_follows_a_colon_query_too() {
    let (mut app, _dir, path) = setup();
    shown(&mut app);
    apply(&mut app, Action::OpenPalette, Platform::Linux, &path);
    type_str(&mut app, ":a", &path);
    assert_eq!(app.palette, Some(0), "`:a` keeps the filtered colon list");
    assert!(commands::palette_items(&app.input)
        .iter()
        .all(|c| c.token.starts_with(':')));
}
