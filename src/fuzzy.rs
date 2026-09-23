//! Case-insensitive subsequence fuzzy scoring.
//!
//! Scoring rules (applied per matched needle character):
//!
//! * +3 for every matched character
//! * +1 extra when the hit directly follows the previous hit (consecutive)
//! * +5 extra when the hit starts a word (previous haystack char is a space,
//!   `-`, `_`, `/`, `.`, or the haystack start)
//! * +8 extra when the haystack starts with the whole needle
//!   (case-insensitive)
//!
//! A non-matching needle yields `None`; an empty needle yields `Some(0)`.

/// Score `needle` against `haystack` (both compared case-insensitively).
pub fn score(needle: &str, haystack: &str) -> Option<i32> {
    let n: Vec<char> = needle.chars().map(|c| c.to_ascii_lowercase()).collect();
    if n.is_empty() {
        return Some(0);
    }
    let h: Vec<char> = haystack.chars().map(|c| c.to_ascii_lowercase()).collect();

    let mut total = 0i32;
    if h.len() >= n.len() && h[..n.len()] == n[..] {
        total += 8; // haystack starts with the needle
    }

    let mut search_from = 0usize;
    let mut prev_hit: Option<usize> = None;
    for &nc in &n {
        let hit = h[search_from..]
            .iter()
            .position(|&c| c == nc)
            .map(|i| search_from + i);
        let pos = hit?;
        let mut s = 3i32;
        if let Some(prev) = prev_hit {
            if pos == prev + 1 {
                s += 1; // consecutive hit
            }
        }
        if pos == 0 || matches!(h[pos - 1], ' ' | '-' | '_' | '/' | '.') {
            s += 5; // word start
        }
        total += s;
        prev_hit = Some(pos);
        search_from = pos + 1;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_match_returns_none() {
        assert_eq!(score("zzz", "browser br"), None);
        assert_eq!(score("rx", "browser"), None); // order must be respected
    }

    #[test]
    fn empty_needle_returns_zero() {
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(score("br", "browser br"), score("BR", "Browser BR"));
        assert!(score("Br", "BROWSER").is_some());
    }

    #[test]
    fn consecutive_hits_beat_jumps() {
        // 'ab' in "xab": a at 1 (3), b at 2 (3+1) = 7
        assert_eq!(score("ab", "xab"), Some(7));
        // 'ab' in "xaxb": a at 1 (3), b at 3 (3) = 6
        assert_eq!(score("ab", "xaxb"), Some(6));
        assert!(score("ab", "xab") > score("ab", "xaxb"));
    }

    #[test]
    fn word_start_bonus() {
        // 'f' follows '-' in "my-file": 3 + 5
        assert_eq!(score("f", "my-file"), Some(8));
        // 'l' is mid-word: 3
        assert_eq!(score("l", "my-file"), Some(3));
        // 'a' after a space: 3 + 5
        assert_eq!(score("g", "say go"), Some(8));
        // greedy matching: first 'a' in "say alpha" is mid-word, so no bonus
        assert_eq!(score("a", "say alpha"), Some(3));
        // leading char counts as word start AND triggers prefix bonus:
        // m(3+5) + prefix(8) = 16
        assert_eq!(score("m", "my-file"), Some(16));
    }

    #[test]
    fn prefix_bonus_stacks() {
        // "my" is a prefix of "my-file": m(3+5) + y(3+1) + prefix(8) = 20
        assert_eq!(score("my", "my-file"), Some(20));
        assert!(score("my", "my-file") > score("my", "say my-file"));
    }
}
