//! Readline-style single-line editing: pure functions over `(text, caret)`.
//!
//! The caret is a **char index** in `0..=char_count`, so every operation is
//! unicode-safe and never splits a multi-byte char. Word motions use the
//! shell convention (`unix-word-rubout`, Ctrl+W in bash): a word is a run of
//! non-whitespace, so `sudo rm -rf /tmp/x` kills `/tmp/x` whole even though
//! it contains slashes. Nothing here mutates state - callers own the text.

use unicode_width::UnicodeWidthChar;

/// Caret motions that leave the text unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// One char left.
    Left,
    /// One char right.
    Right,
    /// Start of the previous whitespace-delimited word.
    WordLeft,
    /// End of the next whitespace-delimited word.
    WordRight,
    /// Start of the line.
    Home,
    /// End of the line.
    End,
}

/// Number of chars in `s`.
fn len(s: &str) -> usize {
    s.chars().count()
}

/// Clamp `caret` into `0..=char_count`.
pub fn clamp(s: &str, caret: usize) -> usize {
    caret.min(len(s))
}

/// Insert `c` at `caret`; returns the new text and the caret after it.
pub fn insert(s: &str, caret: usize, c: char) -> (String, usize) {
    let at = clamp(s, caret);
    let mut cs: Vec<char> = s.chars().collect();
    cs.insert(at, c);
    (cs.into_iter().collect(), at + 1)
}

/// Delete the char before `caret` (Backspace); no-op at the start.
pub fn backspace(s: &str, caret: usize) -> (String, usize) {
    let at = clamp(s, caret);
    if at == 0 {
        return (s.to_string(), 0);
    }
    let mut cs: Vec<char> = s.chars().collect();
    cs.remove(at - 1);
    (cs.into_iter().collect(), at - 1)
}

/// Delete the char at `caret` (Delete); no-op at the end.
pub fn delete(s: &str, caret: usize) -> (String, usize) {
    let at = clamp(s, caret);
    if at >= len(s) {
        return (s.to_string(), at);
    }
    let mut cs: Vec<char> = s.chars().collect();
    cs.remove(at);
    (cs.into_iter().collect(), at)
}

/// Resolve one motion against `caret`, clamped at both ends.
pub fn motion(s: &str, caret: usize, motion: Motion) -> usize {
    let cs: Vec<char> = s.chars().collect();
    let n = cs.len();
    let at = caret.min(n);
    match motion {
        Motion::Left => at.saturating_sub(1),
        Motion::Right => (at + 1).min(n),
        Motion::Home => 0,
        Motion::End => n,
        Motion::WordLeft => word_start(&cs, at),
        Motion::WordRight => word_end(&cs, at),
    }
}

/// First index of the whitespace-delimited word before `at`.
fn word_start(cs: &[char], at: usize) -> usize {
    let mut i = at;
    while i > 0 && cs[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !cs[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

/// Index just past the whitespace-delimited word from `at` on.
fn word_end(cs: &[char], at: usize) -> usize {
    let n = cs.len();
    let mut i = at.min(n);
    while i < n && cs[i].is_whitespace() {
        i += 1;
    }
    while i < n && !cs[i].is_whitespace() {
        i += 1;
    }
    i
}

/// Ctrl+U: kill from the start of the line to the caret.
pub fn kill_to_start(s: &str, caret: usize) -> (String, usize) {
    let at = clamp(s, caret);
    (s.chars().skip(at).collect(), 0)
}

/// Ctrl+K: kill from the caret to the end of the line.
pub fn kill_to_end(s: &str, caret: usize) -> (String, usize) {
    let at = clamp(s, caret);
    (s.chars().take(at).collect(), at)
}

/// Ctrl+W: kill the whitespace-delimited word before the caret, plus the
/// whitespace between it and the caret (bare Ctrl+W kills the whitespace).
pub fn kill_word(s: &str, caret: usize) -> (String, usize) {
    let cs: Vec<char> = s.chars().collect();
    let at = caret.min(cs.len());
    let start = word_start(&cs, at);
    let text: String = cs[..start].iter().chain(&cs[at..]).collect();
    (text, start)
}

/// Ctrl+T: swap the two chars around the caret, then step over the pair.
/// At the end of the line the last two chars are swapped. No-op on a line
/// with fewer than two chars or with the caret at its start.
pub fn transpose(s: &str, caret: usize) -> (String, usize) {
    let mut cs: Vec<char> = s.chars().collect();
    let n = cs.len();
    let at = caret.min(n);
    if n < 2 || at == 0 {
        return (s.to_string(), at);
    }
    let (a, b) = if at >= n {
        (n - 2, n - 1)
    } else {
        (at - 1, at)
    };
    cs.swap(a, b);
    let next = if at >= n { n } else { at + 1 };
    (cs.into_iter().collect(), next)
}

/// Display width of `c` in terminal cells; control chars count as one so the
/// caret arithmetic never divides by zero.
fn cell_width(c: char) -> usize {
    c.width().unwrap_or(1)
}

/// Visible window around the caret for a cell `budget`: `(before, at, after)`.
/// `before` holds the chars before the caret, `at` the char the caret sits on
/// (`None` at the end of the line) and `after` the chars that follow it. The
/// caller paints one caret cell **between `before` and `at`**, so the window
/// always reserves exactly that one cell; widths are counted in terminal
/// cells (CJK/emoji take two), never in chars. A char is shown whole or not
/// at all, and the window scrolls right so the caret never leaves it: with a
/// short line the whole text plus the caret cell stays visible.
pub fn window(s: &str, caret: usize, budget: usize) -> (String, Option<char>, String) {
    if budget == 0 {
        return (String::new(), None, String::new());
    }
    let cs: Vec<char> = s.chars().collect();
    let n = cs.len();
    let at = caret.min(n);
    let mut start = at;
    let mut used = 1; // the caret cell itself
    while start > 0 && used + cell_width(cs[start - 1]) <= budget {
        used += cell_width(cs[start - 1]);
        start -= 1;
    }
    // The char under the caret, then as many following chars as fit. A wide
    // char that does not fit is dropped whole, with nothing after it.
    let mut end = at;
    if let Some(c) = cs.get(at).copied() {
        if used + cell_width(c) <= budget {
            used += cell_width(c);
            end = at + 1;
        }
    }
    if end > at {
        while end < n && used + cell_width(cs[end]) <= budget {
            used += cell_width(cs[end]);
            end += 1;
        }
    }
    let before: String = cs[start..at].iter().collect();
    let at_char = cs.get(at).copied().filter(|_| end > at);
    let after_start = if end > at { at + 1 } else { end };
    let after: String = cs[after_start..end].iter().collect();
    (before, at_char, after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_delete_honour_the_caret() {
        assert_eq!(insert("ac", 1, 'b'), ("abc".to_string(), 2));
        assert_eq!(insert("", 9, 'x'), ("x".to_string(), 1));
        assert_eq!(backspace("abc", 2), ("ac".to_string(), 1));
        assert_eq!(backspace("abc", 0), ("abc".to_string(), 0));
        assert_eq!(delete("abc", 1), ("ac".to_string(), 1));
        assert_eq!(delete("abc", 9), ("abc".to_string(), 3));
    }

    #[test]
    fn insert_counts_chars_not_bytes() {
        // Caret 1 is between the two CJK chars, not inside the first one.
        assert_eq!(insert("中文", 1, '-'), ("中-文".to_string(), 2));
    }

    #[test]
    fn motions_clamp_at_both_ends() {
        assert_eq!(motion("abc", 0, Motion::Left), 0);
        assert_eq!(motion("abc", 3, Motion::Right), 3);
        assert_eq!(motion("abc", 2, Motion::Home), 0);
        assert_eq!(motion("abc", 1, Motion::End), 3);
        assert_eq!(motion("abc", 9, Motion::End), 3);
    }

    #[test]
    fn word_motions_use_whitespace_boundaries() {
        let s = "git commit -m foo";
        // From the end: back over `foo`, over the space, over `-m`, then `commit`.
        assert_eq!(motion(s, 17, Motion::WordLeft), 14);
        assert_eq!(motion(s, 14, Motion::WordLeft), 11);
        assert_eq!(motion(s, 11, Motion::WordLeft), 4);
        // Forward from the start: skip `git`, land past the space.
        assert_eq!(motion(s, 0, Motion::WordRight), 3);
        assert_eq!(motion(s, 3, Motion::WordRight), 10);
        assert_eq!(motion(s, 17, Motion::WordRight), 17);
    }

    #[test]
    fn word_motions_keep_paths_and_flags_whole() {
        let s = "sudo rm -rf /tmp/x";
        assert_eq!(motion(s, 18, Motion::WordLeft), 12);
        assert_eq!(motion(s, 12, Motion::WordLeft), 8);
        assert_eq!(motion(s, 0, Motion::WordRight), 4);
    }

    #[test]
    fn kill_to_start_and_end_split_the_line() {
        assert_eq!(kill_to_start("abc def", 4), ("def".to_string(), 0));
        assert_eq!(kill_to_start("abc", 0), ("abc".to_string(), 0));
        assert_eq!(kill_to_start("abc", 9), (String::new(), 0));
        assert_eq!(kill_to_end("abc def", 4), ("abc ".to_string(), 4));
        assert_eq!(kill_to_end("abc", 9), ("abc".to_string(), 3));
    }

    #[test]
    fn kill_word_eats_trailing_whitespace_too() {
        assert_eq!(
            kill_word("git commit foo", 14),
            ("git commit ".to_string(), 11)
        );
        // From the start of `foo` the word before it goes too, space included.
        assert_eq!(kill_word("git commit foo", 11), ("git foo".to_string(), 4));
        assert_eq!(kill_word("git", 0), ("git".to_string(), 0));
    }

    #[test]
    fn transpose_swaps_around_the_caret() {
        assert_eq!(transpose("abc", 3), ("acb".to_string(), 3));
        assert_eq!(transpose("abc", 2), ("acb".to_string(), 3));
        assert_eq!(transpose("abc", 1), ("bac".to_string(), 2));
        assert_eq!(transpose("abc", 0), ("abc".to_string(), 0));
        assert_eq!(transpose("a", 1), ("a".to_string(), 1));
        assert_eq!(transpose("中文", 2), ("文中".to_string(), 2));
    }

    #[test]
    fn window_keeps_the_caret_visible() {
        // Short line: everything plus the caret cell.
        assert_eq!(window("ab", 2, 8), ("ab".to_string(), None, String::new()));
        assert_eq!(
            window("ab", 1, 8),
            ("a".to_string(), Some('b'), String::new())
        );
        // Long line, caret at the end: the last budget-1 chars show.
        assert_eq!(
            window("abcdef", 6, 4),
            ("def".to_string(), None, String::new())
        );
        // Long line, caret at the start: the caret cell, then what fits.
        assert_eq!(
            window("abcdef", 0, 4),
            (String::new(), Some('a'), "bc".to_string())
        );
        // Degenerate budgets never panic.
        assert_eq!(window("abc", 2, 0), (String::new(), None, String::new()));
        assert_eq!(window("abc", 2, 1), (String::new(), None, String::new()));
    }

    #[test]
    fn window_counts_display_cells_not_chars() {
        // 文 is two cells: budget 5 fits "文a" (3) + caret (1) + 'b' (1).
        assert_eq!(
            window("中文abc", 3, 5),
            ("文a".to_string(), Some('b'), String::new())
        );
        // The wide char under the caret is shown whole or not at all.
        assert_eq!(window("中文", 0, 2), (String::new(), None, String::new()));
        assert_eq!(
            window("中文", 0, 3),
            (String::new(), Some('中'), String::new())
        );
        // Emoji take two cells as well.
        assert_eq!(
            window("👍ab", 0, 4),
            (String::new(), Some('👍'), "a".to_string())
        );
    }

    #[test]
    fn window_never_exceeds_its_cell_budget() {
        let width = |s: &str| s.chars().map(cell_width).sum::<usize>();
        for s in ["中文abc", "a中b文c", "👍ab", "abc", "a\u{301}b"] {
            let cs: Vec<char> = s.chars().collect();
            for caret in 0..=cs.len() {
                for budget in 1..=8 {
                    let (before, at, after) = window(s, caret, budget);
                    assert!(
                        width(&before) + 1 + at.map_or(0, cell_width) + width(&after) <= budget,
                        "{s:?} caret={caret} budget={budget}: {before:?} {at:?} {after:?}"
                    );
                    // Never split a char: the pieces line up around the caret.
                    assert!(cs[..caret].ends_with(&before.chars().collect::<Vec<_>>()));
                    if let Some(c) = at {
                        assert_eq!(c, cs[caret], "the char shown sits at the caret");
                    }
                    let tail_start = (caret + 1).min(cs.len());
                    assert!(cs[tail_start..].starts_with(&after.chars().collect::<Vec<_>>()));
                }
            }
        }
    }
}
