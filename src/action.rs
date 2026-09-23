//! Pure key mapping: `KeyEvent` -> [`Action`]. No state is mutated here.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::state::{App, Visibility};

/// One user intent, applied by `crate::app::apply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Nop,
    Quit,
    ToggleBar,
    Execute,
    SubmitColon,
    InsertChar(char),
    Backspace,
    ClearInput,
    MoveUp,
    MoveDown,
}

/// Map a key event to an action. Only `KeyEventKind::Press` is honoured —
/// without the filter, terminals that also report Release/Repeat would fire
/// every mapping twice (the opencoder `input.rs` lesson).
pub fn on_key(app: &App, key: KeyEvent) -> Action {
    if key.kind != KeyEventKind::Press {
        return Action::Nop;
    }

    // Global wake/sleep hotkey: Alt+D (exactly ALT — no Ctrl/Shift riding along).
    if let KeyCode::Char('d' | 'D') = key.code {
        if key.modifiers == KeyModifiers::ALT {
            return Action::ToggleBar;
        }
    }

    match app.visibility {
        Visibility::Hidden => hidden_key(key),
        Visibility::Shown => shown_key(app, key),
    }
}

fn hidden_key(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        _ => Action::Nop,
    }
}

fn shown_key(app: &App, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Enter => {
            if app.input.starts_with(':') {
                Action::SubmitColon
            } else {
                Action::Execute
            }
        }
        KeyCode::Esc => Action::ToggleBar,
        KeyCode::Up => Action::MoveUp,
        KeyCode::Down | KeyCode::Tab => Action::MoveDown,
        KeyCode::Backspace if key.modifiers.is_empty() => Action::Backspace,
        KeyCode::Char('u') if ctrl => Action::ClearInput,
        KeyCode::Char('c') if ctrl => Action::Quit,
        KeyCode::Char(c) if !ctrl && !alt => Action::InsertChar(c),
        _ => Action::Nop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn release(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new_with_kind(code, modifiers, KeyEventKind::Release)
    }

    fn hidden() -> App {
        state::new(crate::storage::Store::default())
    }

    fn shown() -> App {
        let mut app = hidden();
        app.visibility = Visibility::Shown;
        app
    }

    #[test]
    fn alt_d_toggles_in_hidden_and_shown() {
        assert_eq!(
            on_key(&hidden(), key(KeyCode::Char('d'), KeyModifiers::ALT)),
            Action::ToggleBar
        );
        assert_eq!(
            on_key(&shown(), key(KeyCode::Char('D'), KeyModifiers::ALT)),
            Action::ToggleBar
        );
    }

    #[test]
    fn alt_d_with_extra_modifiers_is_not_the_hotkey() {
        let mut m = KeyModifiers::ALT;
        m |= KeyModifiers::CONTROL;
        assert_eq!(on_key(&hidden(), key(KeyCode::Char('d'), m)), Action::Nop);
        let mut m = KeyModifiers::ALT;
        m |= KeyModifiers::SHIFT;
        assert_eq!(on_key(&shown(), key(KeyCode::Char('d'), m)), Action::Nop);
    }

    #[test]
    fn ctrl_c_quits_in_both_states() {
        assert_eq!(
            on_key(&hidden(), key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        assert_eq!(
            on_key(&shown(), key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
    }

    #[test]
    fn hidden_ignores_everything_but_hotkey_and_quit() {
        assert_eq!(
            on_key(&hidden(), key(KeyCode::Enter, KeyModifiers::NONE)),
            Action::Nop
        );
        assert_eq!(
            on_key(&hidden(), key(KeyCode::Char('x'), KeyModifiers::NONE)),
            Action::Nop
        );
        assert_eq!(
            on_key(&hidden(), key(KeyCode::Esc, KeyModifiers::NONE)),
            Action::Nop
        );
    }

    #[test]
    fn esc_shown_toggles() {
        assert_eq!(
            on_key(&shown(), key(KeyCode::Esc, KeyModifiers::NONE)),
            Action::ToggleBar
        );
    }

    #[test]
    fn enter_dispatches_on_colon_prefix() {
        let mut app = shown();
        app.input = "br docs".to_string();
        assert_eq!(
            on_key(&app, key(KeyCode::Enter, KeyModifiers::NONE)),
            Action::Execute
        );
        app.input = ":add t echo".to_string();
        assert_eq!(
            on_key(&app, key(KeyCode::Enter, KeyModifiers::NONE)),
            Action::SubmitColon
        );
    }

    #[test]
    fn navigation_and_tab() {
        let app = shown();
        assert_eq!(
            on_key(&app, key(KeyCode::Up, KeyModifiers::NONE)),
            Action::MoveUp
        );
        assert_eq!(
            on_key(&app, key(KeyCode::Down, KeyModifiers::NONE)),
            Action::MoveDown
        );
        assert_eq!(
            on_key(&app, key(KeyCode::Tab, KeyModifiers::NONE)),
            Action::MoveDown
        );
    }

    #[test]
    fn editing_keys() {
        let app = shown();
        assert_eq!(
            on_key(&app, key(KeyCode::Char('z'), KeyModifiers::NONE)),
            Action::InsertChar('z')
        );
        // SHIFT is allowed for plain characters (uppercase letters).
        assert_eq!(
            on_key(&app, key(KeyCode::Char('Z'), KeyModifiers::SHIFT)),
            Action::InsertChar('Z')
        );
        assert_eq!(
            on_key(&app, key(KeyCode::Backspace, KeyModifiers::NONE)),
            Action::Backspace
        );
        assert_eq!(
            on_key(&app, key(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            Action::ClearInput
        );
    }

    #[test]
    fn non_press_events_are_ignored() {
        assert_eq!(
            on_key(&hidden(), release(KeyCode::Char('d'), KeyModifiers::ALT)),
            Action::Nop
        );
        assert_eq!(
            on_key(&shown(), release(KeyCode::Enter, KeyModifiers::NONE)),
            Action::Nop
        );
    }

    #[test]
    fn ctrl_or_alt_characters_are_not_inserted() {
        let app = shown();
        assert_eq!(
            on_key(&app, key(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            Action::Nop
        );
        assert_eq!(
            on_key(&app, key(KeyCode::Char('x'), KeyModifiers::ALT)),
            Action::Nop
        );
    }
}
