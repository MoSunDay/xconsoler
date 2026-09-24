//! Wake-key specification: parse (`"alt+d"`), describe, match against
//! `KeyEvent`s, and render as a readline escape sequence for `bind`/`bindkey`.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// A key spec (wake key / command-palette key): one alphanumeric char plus
/// exactly one modifier (`alt` xor `ctrl`). Produced by [`parse`]; plain
/// data, freely copyable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySpec {
    pub alt: bool,
    pub ctrl: bool,
    pub ch: char,
}

/// Default wake key, as a spec string.
pub const DEFAULT_SPEC: &str = "alt+d";

/// [`DEFAULT_SPEC`] already parsed, for fallbacks.
pub const DEFAULT: KeySpec = KeySpec {
    alt: true,
    ctrl: false,
    ch: 'd',
};

/// Default command-palette key spec (same key as the default wake key).
pub const DEFAULT_COMMAND_SPEC: &str = "alt+d";

/// [`DEFAULT_COMMAND_SPEC`] already parsed, for fallbacks.
pub const DEFAULT_COMMAND: KeySpec = DEFAULT;

/// Parse `"alt+<c>"` / `"ctrl+<c>"`. The modifier must be lowercase; the
/// character may be any case and is stored lowercase. Trimmed input is fine.
/// Multi-modifier combos (`alt+ctrl+x`, `ctrl+alt+x`) are rejected explicitly.
pub fn parse(spec: &str) -> Result<KeySpec, String> {
    let s = spec.trim();
    let lowered = s.to_ascii_lowercase();
    if lowered.starts_with("alt+ctrl+") || lowered.starts_with("ctrl+alt+") {
        return Err(invalid(spec, "alt+ctrl combo unsupported"));
    }
    let (modifier, ch) = s
        .split_once('+')
        .ok_or_else(|| invalid(spec, "expected \"alt+<c>\" or \"ctrl+<c>\""))?;
    let alt = match modifier {
        "alt" => true,
        "ctrl" => false,
        other => {
            return Err(invalid(
                spec,
                &format!("unknown modifier {other:?} (lowercase \"alt\" or \"ctrl\" only)"),
            ))
        }
    };
    let mut chars = ch.chars();
    let c = chars
        .next()
        .ok_or_else(|| invalid(spec, "expected a single character after '+'"))?;
    if chars.next().is_some() {
        return Err(invalid(spec, "expected a single character after '+'"));
    }
    if !c.is_ascii_alphanumeric() {
        return Err(invalid(spec, "character must be a letter or digit"));
    }
    Ok(KeySpec {
        alt,
        ctrl: !alt,
        ch: c.to_ascii_lowercase(),
    })
}

/// Canonical text form: `"alt+d"` / `"ctrl+g"` (lowercase throughout).
pub fn describe(k: &KeySpec) -> String {
    let modifier = if k.alt { "alt" } else { "ctrl" };
    format!("{modifier}+{}", k.ch)
}

/// True when `key` is exactly the wake key: a `Press` of the spec's char
/// (case-insensitive) with the exact alt/ctrl modifiers — SHIFT is ignored.
pub fn matches(k: &KeySpec, key: &KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&k.ch))
        && key.modifiers.contains(KeyModifiers::ALT) == k.alt
        && key.modifiers.contains(KeyModifiers::CONTROL) == k.ctrl
}

/// Readline escape text as written inside `bind`/`bindkey`: `"\\ed"` for
/// alt keys, `"\\C-g"` for ctrl keys. This is the literal two-character
/// `\e` / `\C-` prefix, not a real ESC control byte.
pub fn readline_seq(k: &KeySpec) -> String {
    if k.alt {
        format!("\\e{}", k.ch)
    } else {
        format!("\\C-{}", k.ch)
    }
}

fn invalid(spec: &str, reason: &str) -> String {
    format!("invalid key {spec:?}: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_alt_and_ctrl() {
        assert_eq!(
            parse("alt+d").unwrap(),
            KeySpec {
                alt: true,
                ctrl: false,
                ch: 'd'
            }
        );
        assert_eq!(
            parse("ctrl+g").unwrap(),
            KeySpec {
                alt: false,
                ctrl: true,
                ch: 'g'
            }
        );
    }

    #[test]
    fn parse_trims_and_lowercases_the_char() {
        assert_eq!(parse("  alt+D ").unwrap(), parse("alt+d").unwrap());
        assert_eq!(parse("ctrl+G").unwrap().ch, 'g');
        assert_eq!(parse("alt+7").unwrap().ch, '7');
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());
        assert!(parse("alt").is_err());
        assert!(parse("alt+").is_err());
        assert!(parse("alt+de").is_err());
        assert!(parse("alt+d+e").is_err());
        assert!(parse("alt+/").is_err());
        assert!(parse("meta+d").is_err());
        assert!(parse("ALT+d").is_err(), "modifier must be lowercase");
    }

    #[test]
    fn parse_rejects_alt_ctrl_combo_explicitly() {
        for spec in ["alt+ctrl+x", "ctrl+alt+x", " Alt+Ctrl+X "] {
            let err = parse(spec).unwrap_err();
            assert!(
                err.contains("alt+ctrl combo unsupported"),
                "unexpected error for {spec:?}: {err}"
            );
        }
    }

    #[test]
    fn describe_roundtrips_through_parse() {
        for spec in ["alt+d", "ctrl+g", "alt+J"] {
            assert_eq!(describe(&parse(spec).unwrap()), spec.to_ascii_lowercase());
        }
    }

    #[test]
    fn readline_seq_uses_literal_escape_text() {
        assert_eq!(readline_seq(&parse("alt+d").unwrap()), r"\ed");
        assert_eq!(readline_seq(&parse("ctrl+g").unwrap()), r"\C-g");
        assert_eq!(readline_seq(&parse("alt+d").unwrap()), "\\ed");
        assert!(!readline_seq(&parse("alt+d").unwrap()).contains('\u{1b}'));
    }

    #[test]
    fn matches_exact_modifier_char_and_kind() {
        let k = parse("alt+d").unwrap();
        assert!(matches(
            &k,
            &KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT)
        ));
        assert!(matches(
            &k,
            &KeyEvent::new(KeyCode::Char('D'), KeyModifiers::ALT)
        ));
        // SHIFT riding along is ignored.
        let alt_shift = KeyModifiers::ALT | KeyModifiers::SHIFT;
        assert!(matches(&k, &KeyEvent::new(KeyCode::Char('d'), alt_shift)));
    }

    #[test]
    fn matches_rejects_wrong_char_modifiers_or_kind() {
        let k = parse("alt+d").unwrap();
        // wrong char
        assert!(!matches(
            &k,
            &KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)
        ));
        // ctrl riding along
        let alt_ctrl = KeyModifiers::ALT | KeyModifiers::CONTROL;
        assert!(!matches(&k, &KeyEvent::new(KeyCode::Char('d'), alt_ctrl)));
        // no modifier
        assert!(!matches(
            &k,
            &KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)
        ));
        // ctrl-only spec does not match alt-only press
        let g = parse("ctrl+g").unwrap();
        assert!(!matches(
            &g,
            &KeyEvent::new(KeyCode::Char('g'), KeyModifiers::ALT)
        ));
        assert!(matches(
            &g,
            &KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)
        ));
        // non-char codes and non-press kinds
        assert!(!matches(
            &k,
            &KeyEvent::new(KeyCode::Esc, KeyModifiers::ALT)
        ));
        assert!(!matches(
            &k,
            &KeyEvent::new_with_kind(
                KeyCode::Char('d'),
                KeyModifiers::ALT,
                crossterm::event::KeyEventKind::Release
            )
        ));
    }
}
