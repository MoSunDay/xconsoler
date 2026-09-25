//! Pure key mapping: `KeyEvent` -> [`Action`]. No state is mutated here.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::keyspec;
use crate::state::{App, Visibility};
use crate::textedit::Motion;

/// One user intent, applied by `crate::app::apply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Nop,
    Quit,
    ToggleBar,
    OpenPalette,
    ClosePalette,
    PaletteAccept,
    Execute,
    SubmitColon,
    InsertChar(char),
    Backspace,
    DeleteForward,
    Motion(Motion),
    KillToStart,
    KillToEnd,
    KillWord,
    Transpose,
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

    // Esc (unmodified): an open palette closes first; summon mode quits,
    // otherwise it toggles the bar.
    if key.code == KeyCode::Esc && key.modifiers.is_empty() {
        if app.palette.is_some() {
            return Action::ClosePalette;
        }
        return if app.summon {
            Action::Quit
        } else {
            Action::ToggleBar
        };
    }

    // Command-palette hotkey (`app.command`): toggles the palette while the
    // bar is shown. Falling through keeps the wake key in charge while hidden
    // and in summon mode when both keys are the same.
    if keyspec::matches(&app.command, &key) {
        if app.palette.is_some() {
            return Action::ClosePalette;
        }
        if app.visibility == Visibility::Shown && !(app.summon && app.command == app.wake) {
            return Action::OpenPalette;
        }
    }

    // Global wake/sleep hotkey (`app.wake`, configurable via the store): in
    // summon mode it quits back to the shell prompt, otherwise it toggles.
    if keyspec::matches(&app.wake, &key) {
        return if app.summon {
            Action::Quit
        } else {
            Action::ToggleBar
        };
    }

    match app.visibility {
        Visibility::Hidden => hidden_key(key),
        Visibility::Shown => shown_key(app, key),
    }
}

fn hidden_key(key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Char('c') if ctrl => Action::Quit,
        // Alt+Ctrl+D is a mistyped Alt+D: keep it a no-op, do not quit.
        KeyCode::Char('d') if ctrl && !alt => Action::Quit,
        _ => Action::Nop,
    }
}

fn shown_key(app: &App, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Enter => {
            if app.palette.is_some() {
                Action::PaletteAccept
            } else if app.input.starts_with(':') {
                Action::SubmitColon
            } else {
                Action::Execute
            }
        }
        KeyCode::Esc => Action::ToggleBar,
        KeyCode::Up => Action::MoveUp,
        KeyCode::Down | KeyCode::Tab => Action::MoveDown,
        KeyCode::Backspace if key.modifiers.is_empty() => Action::Backspace,
        KeyCode::Delete if key.modifiers.is_empty() => Action::DeleteForward,
        // Ctrl/Alt + arrows are word motions, bare arrows char motions.
        KeyCode::Left if ctrl || alt => Action::Motion(Motion::WordLeft),
        KeyCode::Right if ctrl || alt => Action::Motion(Motion::WordRight),
        KeyCode::Left => Action::Motion(Motion::Left),
        KeyCode::Right => Action::Motion(Motion::Right),
        KeyCode::Home => Action::Motion(Motion::Home),
        KeyCode::End => Action::Motion(Motion::End),
        // Readline's Emacs bindings: Ctrl+A/E/B/F and Alt+B/F (the latter
        // arrive through `EscGuard` as synthetic ALT+char events).
        KeyCode::Char('a') if ctrl => Action::Motion(Motion::Home),
        KeyCode::Char('e') if ctrl => Action::Motion(Motion::End),
        KeyCode::Char('b') if ctrl => Action::Motion(Motion::Left),
        KeyCode::Char('f') if ctrl => Action::Motion(Motion::Right),
        KeyCode::Char('b') if alt => Action::Motion(Motion::WordLeft),
        KeyCode::Char('f') if alt => Action::Motion(Motion::WordRight),
        // Ctrl+D is the terminal EOF convention: it quits in any state,
        // with or without text. Deletion under the caret is the Delete key.
        KeyCode::Char('d') if ctrl && !alt => Action::Quit,
        KeyCode::Char('h') if ctrl => Action::Backspace,
        KeyCode::Char('k') if ctrl => Action::KillToEnd,
        KeyCode::Char('u') if ctrl => Action::KillToStart,
        KeyCode::Char('w') if ctrl => Action::KillWord,
        KeyCode::Char('t') if ctrl => Action::Transpose,
        KeyCode::Char('n') if ctrl => Action::MoveDown,
        KeyCode::Char('p') if ctrl => Action::MoveUp,
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

    /// Non-summon app parked in Hidden (apps start Shown; hide explicitly).
    fn hidden() -> App {
        let mut app = state::new(crate::storage::Store::default(), false);
        app.visibility = Visibility::Hidden;
        app
    }

    /// Non-summon app, default state (shown).
    fn shown() -> App {
        state::new(crate::storage::Store::default(), false)
    }

    /// Summon-mode app (shell keybind): shown, wake key / Esc quit.
    fn summon() -> App {
        state::new(crate::storage::Store::default(), true)
    }

    #[test]
    fn alt_d_toggles_when_hidden_and_opens_the_palette_when_shown() {
        assert_eq!(
            on_key(&hidden(), key(KeyCode::Char('d'), KeyModifiers::ALT)),
            Action::ToggleBar
        );
        assert_eq!(
            on_key(&shown(), key(KeyCode::Char('D'), KeyModifiers::ALT)),
            Action::OpenPalette
        );
    }

    #[test]
    fn alt_shift_d_still_wakes_but_ctrl_riding_along_does_not() {
        let alt_shift = KeyModifiers::ALT | KeyModifiers::SHIFT;
        assert_eq!(
            on_key(&shown(), key(KeyCode::Char('d'), alt_shift)),
            Action::OpenPalette,
            "SHIFT is ignored by the key matcher"
        );
        let alt_ctrl = KeyModifiers::ALT | KeyModifiers::CONTROL;
        assert_eq!(
            on_key(&hidden(), key(KeyCode::Char('d'), alt_ctrl)),
            Action::Nop
        );
    }

    #[test]
    fn custom_wake_key_replaces_alt_d() {
        let mut app = shown();
        app.wake = keyspec::parse("ctrl+g").unwrap();
        assert_eq!(
            on_key(&app, key(KeyCode::Char('g'), KeyModifiers::CONTROL)),
            Action::ToggleBar
        );
        // alt+d stays the default command-palette key while shown
        assert_eq!(
            on_key(&app, key(KeyCode::Char('d'), KeyModifiers::ALT)),
            Action::OpenPalette
        );
    }

    #[test]
    fn summon_wake_key_quits() {
        assert_eq!(
            on_key(&summon(), key(KeyCode::Char('d'), KeyModifiers::ALT)),
            Action::Quit
        );
        let mut app = summon();
        app.wake = keyspec::parse("ctrl+g").unwrap();
        assert_eq!(
            on_key(&app, key(KeyCode::Char('g'), KeyModifiers::CONTROL)),
            Action::Quit
        );
    }

    #[test]
    fn summon_esc_quits_non_summon_esc_toggles() {
        assert_eq!(
            on_key(&summon(), key(KeyCode::Esc, KeyModifiers::NONE)),
            Action::Quit
        );
        assert_eq!(
            on_key(&shown(), key(KeyCode::Esc, KeyModifiers::NONE)),
            Action::ToggleBar
        );
        assert_eq!(
            on_key(&hidden(), key(KeyCode::Esc, KeyModifiers::NONE)),
            Action::ToggleBar
        );
    }

    #[test]
    fn summon_alt_d_still_quits() {
        // Default command key == wake key: in summon mode the wake key wins.
        assert_eq!(
            on_key(&summon(), key(KeyCode::Char('d'), KeyModifiers::ALT)),
            Action::Quit
        );
    }

    #[test]
    fn command_key_closes_the_open_palette() {
        let mut app = shown();
        app.palette = Some(0);
        assert_eq!(
            on_key(&app, key(KeyCode::Char('d'), KeyModifiers::ALT)),
            Action::ClosePalette
        );
    }

    #[test]
    fn esc_closes_the_open_palette_in_both_modes() {
        let mut app = shown();
        app.palette = Some(2);
        assert_eq!(
            on_key(&app, key(KeyCode::Esc, KeyModifiers::NONE)),
            Action::ClosePalette
        );
        let mut app = summon();
        app.palette = Some(2);
        assert_eq!(
            on_key(&app, key(KeyCode::Esc, KeyModifiers::NONE)),
            Action::ClosePalette
        );
    }

    #[test]
    fn distinct_command_key_opens_palette_and_leaves_the_wake_key_alone() {
        let mut app = shown();
        app.command = keyspec::parse("ctrl+o").unwrap();
        assert_eq!(
            on_key(&app, key(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            Action::OpenPalette
        );
        assert_eq!(
            on_key(&app, key(KeyCode::Char('d'), KeyModifiers::ALT)),
            Action::ToggleBar
        );

        // Even in summon mode a distinct command key opens the palette.
        let mut app = summon();
        app.command = keyspec::parse("ctrl+o").unwrap();
        assert_eq!(
            on_key(&app, key(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            Action::OpenPalette
        );
    }

    #[test]
    fn enter_on_the_open_palette_accepts() {
        let mut app = shown();
        app.palette = Some(0);
        assert_eq!(
            on_key(&app, key(KeyCode::Enter, KeyModifiers::NONE)),
            Action::PaletteAccept
        );
    }

    #[test]
    fn ctrl_c_and_ctrl_d_quit_in_both_states() {
        assert_eq!(
            on_key(&hidden(), key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        assert_eq!(
            on_key(&hidden(), key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        assert_eq!(
            on_key(&shown(), key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        // Ctrl+D is the EOF convention: it quits with text typed too.
        assert_eq!(
            on_key(&shown(), key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        let mut typed = shown();
        typed.input = "br".to_string();
        assert_eq!(
            on_key(&typed, key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        let mut hidden_typed = hidden();
        hidden_typed.input = "br".to_string();
        assert_eq!(
            on_key(
                &hidden_typed,
                key(KeyCode::Char('d'), KeyModifiers::CONTROL)
            ),
            Action::Quit
        );
        // Under-caret deletion stays on the Delete key (see `editing_keys`).
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

    /// The same mapping while the palette is open: `app::move_selection`
    /// routes the action to the palette rows there, and to the candidate
    /// cursor (the list highlight) while it is closed.
    #[test]
    fn navigation_maps_the_same_while_the_palette_is_open() {
        let mut app = shown();
        app.palette = Some(0);
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
        assert_eq!(
            on_key(&app, key(KeyCode::Enter, KeyModifiers::NONE)),
            Action::PaletteAccept
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
            on_key(&app, key(KeyCode::Delete, KeyModifiers::NONE)),
            Action::DeleteForward
        );
        assert_eq!(
            on_key(&app, key(KeyCode::Char('h'), KeyModifiers::CONTROL)),
            Action::Backspace
        );
    }

    #[test]
    fn motion_keys_map_to_readline_motions() {
        let app = shown();
        assert_eq!(
            on_key(&app, key(KeyCode::Left, KeyModifiers::NONE)),
            Action::Motion(Motion::Left)
        );
        assert_eq!(
            on_key(&app, key(KeyCode::Right, KeyModifiers::NONE)),
            Action::Motion(Motion::Right)
        );
        // Ctrl/Alt + arrows move by word.
        for m in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
            assert_eq!(
                on_key(&app, key(KeyCode::Left, m)),
                Action::Motion(Motion::WordLeft)
            );
            assert_eq!(
                on_key(&app, key(KeyCode::Right, m)),
                Action::Motion(Motion::WordRight)
            );
        }
        assert_eq!(
            on_key(&app, key(KeyCode::Home, KeyModifiers::NONE)),
            Action::Motion(Motion::Home)
        );
        assert_eq!(
            on_key(&app, key(KeyCode::End, KeyModifiers::NONE)),
            Action::Motion(Motion::End)
        );
    }

    #[test]
    fn readline_control_letters_map_to_editing() {
        let app = shown();
        let cases = [
            ('a', Action::Motion(Motion::Home)),
            ('e', Action::Motion(Motion::End)),
            ('b', Action::Motion(Motion::Left)),
            ('f', Action::Motion(Motion::Right)),
            ('h', Action::Backspace),
            ('k', Action::KillToEnd),
            ('u', Action::KillToStart),
            ('w', Action::KillWord),
            ('t', Action::Transpose),
            ('n', Action::MoveDown),
            ('p', Action::MoveUp),
        ];
        for (c, want) in cases {
            assert_eq!(
                on_key(&app, key(KeyCode::Char(c), KeyModifiers::CONTROL)),
                want,
                "ctrl+{c}"
            );
        }
    }

    #[test]
    fn alt_letters_map_to_word_motions() {
        let app = shown();
        assert_eq!(
            on_key(&app, key(KeyCode::Char('b'), KeyModifiers::ALT)),
            Action::Motion(Motion::WordLeft)
        );
        assert_eq!(
            on_key(&app, key(KeyCode::Char('f'), KeyModifiers::ALT)),
            Action::Motion(Motion::WordRight)
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
        let mut app = shown();
        app.palette = Some(0);
        assert_eq!(
            on_key(&app, release(KeyCode::Char('d'), KeyModifiers::ALT)),
            Action::Nop
        );
        assert_eq!(
            on_key(&app, release(KeyCode::Esc, KeyModifiers::NONE)),
            Action::Nop
        );
        assert_eq!(
            on_key(&app, release(KeyCode::Enter, KeyModifiers::NONE)),
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
