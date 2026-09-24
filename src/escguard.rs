//! ESC-tail guard: reassemble an Alt+\<char\> that the pty split across two
//! writes (ESC first, the letter a beat later).
//!
//! Over SSH some terminals deliver Alt+D as two separate key events: crossterm
//! 0.28 commits a lone `\x1b` as `Esc` immediately, then the trailing `d`
//! arrives as a plain character — the wake key never matches. The guard holds
//! a bare Esc for [`HOLD_MS`] and merges it with a following unmodified char
//! into a synthetic `ALT+char` event. This is a pure state machine (no
//! threads, no clock): the caller drives it with `event::read()` and a
//! bounded `event::poll(HOLD_MS)` (see `main::resolve_held`).
//!
//! Semantics of [`EscGuard::feed`]:
//! 1. idle + bare Esc (Press, no modifiers) → held, nothing emitted;
//! 2. held + unmodified Char(c) Press → emit `ALT+c` (held cleared);
//! 3. held + any other key → emit the held Esc first, then re-absorb the
//!    current key through the same rules (a following bare Esc is held
//!    again, everything else passes straight through);
//! 4. no follow-up within [`HOLD_MS`] → [`EscGuard::flush`] emits the Esc.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Window a lone Esc is held while waiting for a possible Alt-tail.
pub const HOLD_MS: u64 = 40;

/// Guard state: `held` is `Some` exactly while a bare Esc is being held.
#[derive(Debug, Default)]
pub struct EscGuard {
    held: Option<KeyEvent>,
}

impl EscGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while a lone Esc is being held (caller should poll for the next
    /// key within [`HOLD_MS`], then [`flush`]).
    pub fn is_holding(&self) -> bool {
        self.held.is_some()
    }

    /// Feed one key event; returns the events to deliver to the app now, in
    /// order. An empty result means the key was held (bare Esc, case 1).
    pub fn feed(&mut self, key: KeyEvent) -> Vec<KeyEvent> {
        match self.held.take() {
            None => self.absorb(key),
            Some(esc) => match synthesize_alt(&key) {
                Some(alt_key) => vec![alt_key],
                None => {
                    let mut out = vec![esc];
                    out.extend(self.absorb(key));
                    out
                }
            },
        }
    }

    /// Timeout path: release the held Esc (if any) as a real Esc press.
    pub fn flush(&mut self) -> Option<KeyEvent> {
        self.held.take()
    }

    /// Idle-state transition: hold a bare Esc, pass everything else through.
    fn absorb(&mut self, key: KeyEvent) -> Vec<KeyEvent> {
        if is_bare_esc(&key) {
            self.held = Some(key);
            Vec::new()
        } else {
            vec![key]
        }
    }
}

/// A bare Esc key press: Esc code, Press kind, no modifiers at all.
fn is_bare_esc(key: &KeyEvent) -> bool {
    key.code == KeyCode::Esc
        && key.kind == KeyEventKind::Press
        && key.modifiers == KeyModifiers::NONE
}

/// Merge candidate for a key arriving while an Esc is held: an unmodified
/// Char Press becomes a synthetic `ALT+char` event.
fn synthesize_alt(key: &KeyEvent) -> Option<KeyEvent> {
    match key {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            kind: KeyEventKind::Press,
            state,
        } if modifiers.is_empty() => Some(KeyEvent {
            code: KeyCode::Char(*c),
            modifiers: KeyModifiers::ALT,
            kind: KeyEventKind::Press,
            state: *state,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn esc() -> KeyEvent {
        key(KeyCode::Esc, KeyModifiers::NONE)
    }

    #[test]
    fn esc_then_plain_char_synthesizes_alt_char() {
        let mut g = EscGuard::new();
        assert!(g.feed(esc()).is_empty());
        assert!(g.is_holding());
        let out = g.feed(key(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(out, vec![key(KeyCode::Char('d'), KeyModifiers::ALT)]);
        assert!(!g.is_holding());
        // state cleared: a later char passes through untouched
        assert_eq!(
            g.feed(key(KeyCode::Char('d'), KeyModifiers::NONE)),
            vec![key(KeyCode::Char('d'), KeyModifiers::NONE)]
        );
    }

    #[test]
    fn esc_then_enter_does_not_synthesize() {
        let mut g = EscGuard::new();
        g.feed(esc());
        let out = g.feed(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(out, vec![esc(), key(KeyCode::Enter, KeyModifiers::NONE)]);
        assert!(!g.is_holding());
    }

    #[test]
    fn esc_then_modified_char_does_not_synthesize() {
        let mut g = EscGuard::new();
        g.feed(esc());
        let ctrl_d = key(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(g.feed(ctrl_d), vec![esc(), ctrl_d]);
    }

    #[test]
    fn held_esc_is_released_by_flush_on_timeout() {
        let mut g = EscGuard::new();
        assert!(g.feed(esc()).is_empty());
        assert_eq!(g.flush(), Some(esc()));
        assert_eq!(g.flush(), None, "flush is idempotent");
    }

    #[test]
    fn consecutive_plain_keys_are_not_merged() {
        let mut g = EscGuard::new();
        assert_eq!(
            g.feed(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            vec![key(KeyCode::Char('a'), KeyModifiers::NONE)]
        );
        assert_eq!(
            g.feed(key(KeyCode::Char('b'), KeyModifiers::NONE)),
            vec![key(KeyCode::Char('b'), KeyModifiers::NONE)]
        );
        assert!(!g.is_holding());
    }

    #[test]
    fn double_esc_releases_first_and_holds_second() {
        let mut g = EscGuard::new();
        assert!(g.feed(esc()).is_empty());
        assert_eq!(g.feed(esc()), vec![esc()]);
        assert!(g.is_holding(), "second Esc is held like any bare Esc");
        assert_eq!(g.flush(), Some(esc()));
    }

    #[test]
    fn non_bare_esc_is_never_held() {
        let mut g = EscGuard::new();
        let esc_shift = key(KeyCode::Esc, KeyModifiers::SHIFT);
        assert_eq!(g.feed(esc_shift), vec![esc_shift]);
        assert!(!g.is_holding());
        let esc_release =
            KeyEvent::new_with_kind(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Release);
        assert_eq!(g.feed(esc_release), vec![esc_release]);
        assert!(!g.is_holding());
    }

    #[test]
    fn uppercase_tail_keeps_its_case() {
        let mut g = EscGuard::new();
        g.feed(esc());
        assert_eq!(
            g.feed(key(KeyCode::Char('D'), KeyModifiers::NONE)),
            vec![key(KeyCode::Char('D'), KeyModifiers::ALT)]
        );
    }
}
