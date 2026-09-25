//! Candidate ranking: history always outranks shortcuts (newest first), then
//! the shortcut rows - alias-scoped for `<alias> <partial>`, fuzzy scored
//! otherwise - fill only the slots history leaves.

use crate::alias::{self, AliasDef};
use crate::fuzzy;
use crate::storage::Store;

/// A selectable completion candidate.
#[derive(Debug, Clone, PartialEq)]
pub enum Candidate {
    History {
        idx: usize,
    },
    /// Concrete registered shortcut: `key` is one of the alias's shortcuts
    /// (`<alias> <partial>` narrows the list, a full-query hit offers them
    /// directly).
    Shortcut {
        alias: String,
        key: String,
    },
}

/// The single ranking rule, for a raw (untrimmed) `input`:
///
/// * Empty/whitespace input: the most recent history entries only (newest
///   first, no shortcut rows), up to `limit`.
/// * Otherwise: every history entry whose `"<entry label> <input>"`
///   fuzzy-matches the trimmed input comes first, in store order (newest
///   first, no score sort); the shortcut rows follow, and the whole list is
///   capped at `limit`. Those rows are the alias's concrete shortcut keys when
///   the input is `<alias> <partial>` (see [`shortcut_candidates`]), otherwise
///   every shortcut whose `"<name> <triggers> <key> <value>"` fuzzy-matches,
///   by score descending, ties by alias order then key order.
///
/// History keeps the top slots even in the `<alias> <partial>` case: what was
/// actually run predicts the next run better than a registry lookup that
/// merely shares the same letters, so retyping `br b` re-offers the recorded
/// `br baidu` first instead of burying it under the alias's key list.
pub fn ranked_candidates(
    store: &Store,
    aliases: &[AliasDef],
    input: &str,
    limit: usize,
) -> Vec<Candidate> {
    let query = input.trim();
    if query.is_empty() {
        return recent_history(store, limit);
    }

    // `<alias> <partial>` narrows the shortcut rows to that alias's concrete
    // keys; anything else (including a bare alias) fuzzy-ranks every shortcut.
    // The raw `input` keeps the trailing space that marks the narrow case.
    let scoped = shortcut_candidates(aliases, input, limit);
    let shortcuts = if scoped.is_empty() {
        fuzzy_shortcuts(aliases, query)
    } else {
        scoped
    };

    history_hits(store, aliases, query)
        .into_iter()
        .chain(shortcuts)
        .take(limit)
        .collect()
}

/// Rank candidates for `query` (whitespace-trimmed): the same rule as
/// [`ranked_candidates`], for callers that already hold a trimmed query.
pub fn candidates(
    store: &Store,
    aliases: &[AliasDef],
    query: &str,
    limit: usize,
) -> Vec<Candidate> {
    ranked_candidates(store, aliases, query, limit)
}

/// The most recent history entries, newest first. An empty bar shows what was
/// run last, never shortcuts (they come back as soon as a character is typed).
fn recent_history(store: &Store, limit: usize) -> Vec<Candidate> {
    (0..store.history.len().min(limit))
        .map(|idx| Candidate::History { idx })
        .collect()
}

/// History entries fuzzy-matching `query` on `"<entry label> <input>"`, in
/// store order (newest first). No score sort: recency is the whole point.
fn history_hits(store: &Store, aliases: &[AliasDef], query: &str) -> Vec<Candidate> {
    store
        .history
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            let label = match alias::resolve(aliases, &entry.alias) {
                Some(def) => alias::entry_label(def),
                None => entry.alias.as_str(),
            };
            fuzzy::score(query, &format!("{} {}", label, entry.input())).is_some()
        })
        .map(|(idx, _)| Candidate::History { idx })
        .collect()
}

/// Concrete shortcuts fuzzy-matching `query`, by score descending, ties by
/// alias order then key order. Uncapped: the caller owns the limit.
fn fuzzy_shortcuts(aliases: &[AliasDef], query: &str) -> Vec<Candidate> {
    // (score, alias order, key, candidate): only these rows get score-sorted.
    let mut ranked: Vec<(i32, usize, String, Candidate)> = aliases
        .iter()
        .enumerate()
        .flat_map(|(order, def)| {
            def.shortcuts.iter().filter_map(move |(key, value)| {
                let haystack = format!("{} {} {} {}", def.name, def.triggers.join(" "), key, value);
                fuzzy::score(query, &haystack).map(|s| {
                    (
                        s,
                        order,
                        key.clone(),
                        Candidate::Shortcut {
                            alias: def.name.clone(),
                            key: key.clone(),
                        },
                    )
                })
            })
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    ranked.into_iter().map(|t| t.3).collect()
}

/// Concrete-shortcut sub-candidates for a raw input of the form
/// `<alias> <partial>`. A bare `br` (no whitespace yet) yields nothing, so
/// [`ranked_candidates`] falls back to the fuzzy-scored shortcut rows; once
/// there is a space the alias's own shortcuts take over those rows, ranked by
/// fuzzy score on the key (every key when the partial is empty), ties by key
/// order. Matching history entries still outrank them.
pub fn shortcut_candidates(aliases: &[AliasDef], input: &str, limit: usize) -> Vec<Candidate> {
    let mut parts = input.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    let Some(rest) = parts.next() else {
        return Vec::new();
    };
    let Some(def) = alias::resolve(aliases, head) else {
        return Vec::new();
    };
    if def.shortcuts.is_empty() {
        return Vec::new();
    }
    let partial = rest.trim();
    let mut scored: Vec<(i32, &String)> = def
        .shortcuts
        .keys()
        .filter_map(|k| {
            if partial.is_empty() {
                Some((0, k))
            } else {
                fuzzy::score(partial, k).map(|s| (s, k))
            }
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, k)| Candidate::Shortcut {
            alias: def.name.clone(),
            key: k.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::HistoryEntry;

    fn history(alias: &str, input: &str, ts: u64) -> HistoryEntry {
        HistoryEntry::new(alias, input, ts)
    }

    fn with_shortcuts() -> Vec<AliasDef> {
        let mut def = alias::defaults().remove(0); // br (builtin)
        def.shortcuts.clear(); // fixture: exactly the shortcuts below
        def.shortcuts
            .insert("baidu".to_string(), "https://www.baidu.com".to_string());
        def.shortcuts
            .insert("gh".to_string(), "https://github.com".to_string());
        vec![def]
    }

    #[test]
    fn shortcut_candidates_need_a_space_after_the_trigger() {
        let aliases = with_shortcuts();
        assert!(
            shortcut_candidates(&aliases, "br", 8).is_empty(),
            "bare alias"
        );
        assert!(
            shortcut_candidates(&aliases, "nope x", 8).is_empty(),
            "unknown head"
        );
        assert!(
            shortcut_candidates(&alias::defaults(), "cd x", 8).is_empty(),
            "no shortcuts"
        );
    }

    #[test]
    fn shortcut_candidates_list_and_filter_keys() {
        let aliases = with_shortcuts();
        assert_eq!(
            shortcut_candidates(&aliases, "br ", 8),
            vec![
                Candidate::Shortcut {
                    alias: "br".to_string(),
                    key: "baidu".to_string()
                },
                Candidate::Shortcut {
                    alias: "br".to_string(),
                    key: "gh".to_string()
                },
            ],
            "empty partial lists every key, ties by key order"
        );
        assert_eq!(
            shortcut_candidates(&aliases, "br bai", 8),
            vec![Candidate::Shortcut {
                alias: "br".to_string(),
                key: "baidu".to_string()
            }]
        );
        assert!(
            shortcut_candidates(&aliases, "br zzz", 8).is_empty(),
            "no key matches"
        );
        assert_eq!(
            shortcut_candidates(&aliases, "br ", 1).len(),
            1,
            "limit applies"
        );
    }

    /// Empty input shows the recent history only - no shortcut rows, even
    /// though the limit would leave room for them. Whitespace counts as
    /// empty.
    #[test]
    fn empty_query_lists_recent_history_only() {
        let mut store = Store::default();
        store.history.push(history("br", "a", 2));
        store.history.push(history("br", "b", 1));
        let aliases = alias::defaults();

        let out = candidates(&store, &aliases, "   ", 10);
        assert_eq!(
            out,
            vec![Candidate::History { idx: 0 }, Candidate::History { idx: 1 },]
        );
        assert!(
            out.iter().all(|c| !matches!(c, Candidate::Shortcut { .. })),
            "shortcuts stay out of the empty-input list: {out:?}"
        );
    }

    #[test]
    fn empty_query_without_history_is_empty_even_with_aliases() {
        let store = Store::default();
        for query in ["", "  "] {
            assert_eq!(
                candidates(&store, &alias::defaults(), query, 10),
                Vec::new(),
                "query {query:?}"
            );
        }
    }

    #[test]
    fn empty_query_limit_one_history_only() {
        let mut store = Store::default();
        store.history.push(history("br", "a", 2));
        store.history.push(history("br", "b", 1));
        let out = candidates(&store, &alias::defaults(), "", 1);
        assert_eq!(out, vec![Candidate::History { idx: 0 }]);
    }

    #[test]
    fn empty_query_caps_the_recent_history_at_the_limit() {
        let mut store = Store::default();
        for i in 0..12 {
            store.history.push(history("br", &format!("x{i}"), 12 - i));
        }
        let out = candidates(&store, &alias::defaults(), "", 10);
        assert_eq!(out.len(), 10, "limit caps the recent list: {out:?}");
        assert_eq!(out[0], Candidate::History { idx: 0 }, "newest first");
        assert_eq!(out[9], Candidate::History { idx: 9 });
    }

    #[test]
    fn newer_history_outranks_older_and_shortcuts() {
        let mut store = Store::default();
        store.history.push(history("br", "newest", 20));
        store.history.push(history("br", "older", 10));
        let out = candidates(&store, &alias::defaults(), "br", 10);
        assert_eq!(
            out,
            vec![
                Candidate::History { idx: 0 },
                Candidate::History { idx: 1 },
                Candidate::Shortcut {
                    alias: "br".to_string(),
                    key: "baidu".to_string()
                },
                Candidate::Shortcut {
                    alias: "br".to_string(),
                    key: "gm".to_string()
                },
            ]
        );
    }

    /// History order wins over fuzzy score: the older `gm` entry scores a
    /// prefix bonus, yet the newer `br gm` entry is emitted first.
    #[test]
    fn newer_history_beats_higher_fuzzy_score() {
        let mut store = Store::default();
        store.history.push(history("br", "gm", 2));
        store.history.push(history("gm", "x", 1));
        let out = candidates(&store, &alias::defaults(), "gm", 10);
        assert_eq!(
            out,
            vec![
                Candidate::History { idx: 0 },
                Candidate::History { idx: 1 },
                Candidate::Shortcut {
                    alias: "br".to_string(),
                    key: "gm".to_string()
                },
            ]
        );
    }

    /// Shortcuts only fill the slots history leaves: with one matching
    /// history entry, `limit = 1` hides them and `limit = 2` shows the first.
    #[test]
    fn shortcuts_only_fill_the_slots_history_leaves() {
        let mut store = Store::default();
        store.history.push(history("br", "baidu", 1));
        let aliases = alias::defaults();

        assert_eq!(
            candidates(&store, &aliases, "br", 1),
            vec![Candidate::History { idx: 0 }]
        );
        assert_eq!(
            candidates(&store, &aliases, "br", 2),
            vec![
                Candidate::History { idx: 0 },
                Candidate::Shortcut {
                    alias: "br".to_string(),
                    key: "baidu".to_string()
                },
            ]
        );
    }

    /// The `<alias> <partial>` case narrows the shortcut rows to that alias's
    /// own keys, but history still owns the top slots: a run the user actually
    /// made predicts the next run better than a registry lookup that merely
    /// shares the same letters.
    #[test]
    fn history_outranks_alias_scoped_shortcuts() {
        let mut store = Store::default();
        store.history.push(history("br", "baidu", 1));
        let aliases = with_shortcuts();

        let out = ranked_candidates(&store, &aliases, "br b", 5);
        assert_eq!(out[0], Candidate::History { idx: 0 }, "history first");
        assert_eq!(
            out,
            vec![
                Candidate::History { idx: 0 },
                Candidate::Shortcut {
                    alias: "br".to_string(),
                    key: "baidu".to_string()
                },
            ],
            "then the alias's narrowed keys"
        );
    }

    /// History alone can fill the whole cap: the alias-scoped shortcut rows are
    /// pushed out entirely, exactly like the fuzzy-scored ones already were.
    #[test]
    fn history_can_fill_the_cap_before_alias_scoped_shortcuts() {
        let mut store = Store::default();
        store.history.push(history("br", "baidu", 3));
        store.history.push(history("br", "bai", 2));
        store.history.push(history("br", "b", 1));
        let aliases = with_shortcuts();

        let out = ranked_candidates(&store, &aliases, "br b", 2);
        assert_eq!(
            out,
            vec![Candidate::History { idx: 0 }, Candidate::History { idx: 1 }],
            "newest first, no slot left for the shortcuts"
        );
        assert!(
            out.iter().all(|c| !matches!(c, Candidate::Shortcut { .. })),
            "no shortcut row survives a full history: {out:?}"
        );
    }

    /// `candidates` is the very same rule as `ranked_candidates`, so the
    /// empty bar and the typed bar cannot drift apart.
    #[test]
    fn candidates_is_the_ranked_rule() {
        let mut store = Store::default();
        store.history.push(history("br", "baidu", 1));
        let aliases = with_shortcuts();
        for input in ["", "   ", "br", "br b", "baidu", "zzz"] {
            assert_eq!(
                candidates(&store, &aliases, input, 5),
                ranked_candidates(&store, &aliases, input, 5),
                "input {input:?}"
            );
        }
    }

    /// Only concrete registered shortcuts become rows: a single letter can
    /// hit several rows (the alias name is part of every haystack), `baidu`
    /// hits only the baidu row, while plain aliases (`cd`) yield nothing.
    #[test]
    fn single_letter_query_matches_concrete_shortcuts_only() {
        let store = Store::default();
        assert_eq!(
            candidates(&store, &alias::defaults(), "b", 5),
            vec![
                Candidate::Shortcut {
                    alias: "br".to_string(),
                    key: "baidu".to_string()
                },
                Candidate::Shortcut {
                    alias: "br".to_string(),
                    key: "gm".to_string()
                },
            ],
            "the alias name makes every br row carry a 'b'"
        );
        assert_eq!(
            candidates(&store, &alias::defaults(), "baidu", 5),
            vec![Candidate::Shortcut {
                alias: "br".to_string(),
                key: "baidu".to_string()
            }],
            "only the baidu key/value matches its own name"
        );
        assert_eq!(
            candidates(&store, &alias::defaults(), "br", 5),
            vec![
                Candidate::Shortcut {
                    alias: "br".to_string(),
                    key: "baidu".to_string()
                },
                Candidate::Shortcut {
                    alias: "br".to_string(),
                    key: "gm".to_string()
                },
            ],
            "both br urls are hit, key order breaks the score tie"
        );
        assert!(
            candidates(&store, &alias::defaults(), "cd", 5).is_empty(),
            "bare aliases without concrete shortcuts never become rows"
        );
    }

    #[test]
    fn unknown_history_alias_uses_own_name() {
        let mut store = Store::default();
        store.history.push(history("mytool", "arg", 1));
        let out = candidates(&store, &alias::defaults(), "mytool", 5);
        assert_eq!(out, vec![Candidate::History { idx: 0 }]);
    }

    #[test]
    fn limit_truncates_ranked_results() {
        let mut store = Store::default();
        for i in 0..5 {
            store.history.push(history("br", &format!("x{i}"), i));
        }
        let out = candidates(&store, &alias::defaults(), "x", 2);
        assert_eq!(
            out,
            vec![Candidate::History { idx: 0 }, Candidate::History { idx: 1 }]
        );
    }
}
