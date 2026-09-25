//! Candidate ranking: matching history first (newest first), aliases by fuzzy
//! score filling only the slots history leaves.

use crate::alias::{self, AliasDef};
use crate::fuzzy;
use crate::storage::Store;

/// A selectable completion candidate.
#[derive(Debug, Clone, PartialEq)]
pub enum Candidate {
    Alias {
        name: String,
    },
    History {
        idx: usize,
    },
    /// Named-arg sub-candidate: the input is `<alias> <partial>` and `key`
    /// is one of that alias's named arguments.
    Arg {
        alias: String,
        key: String,
    },
}

/// Rank candidates for `query` (whitespace-trimmed).
///
/// * Empty query: the most recent history entries only (newest first, no
///   alias rows), up to `limit`.
/// * Otherwise: history entries whose `"<label> <input>"` fuzzy-matches
///   `query` come first, in store order (newest first, no score sort); then
///   aliases whose `"<name> <shortcuts>"` matches, by fuzzy score descending,
///   ties by alias order. Aliases only fill the slots history leaves.
pub fn candidates(
    store: &Store,
    aliases: &[AliasDef],
    query: &str,
    limit: usize,
) -> Vec<Candidate> {
    let query = query.trim();
    if query.is_empty() {
        // Recent history only: an empty bar shows what was run last, never
        // aliases (they come back as soon as a query character is typed).
        return (0..store.history.len().min(limit))
            .map(|idx| Candidate::History { idx })
            .collect();
    }

    // History hits stay in store order (newest first); no score sort.
    let history: Vec<Candidate> = store
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
        .collect();

    // (score, alias order, candidate): only these rows get score-sorted.
    let mut ranked: Vec<(i32, usize, Candidate)> = aliases
        .iter()
        .enumerate()
        .filter_map(|(order, def)| {
            let haystack = format!("{} {}", def.name, def.shortcuts.join(" "));
            fuzzy::score(query, &haystack).map(|s| {
                (
                    s,
                    order,
                    Candidate::Alias {
                        name: def.name.clone(),
                    },
                )
            })
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    history
        .into_iter()
        .chain(ranked.into_iter().map(|t| t.2))
        .take(limit)
        .collect()
}

/// Named-arg sub-candidates for a raw input of the form `<trigger> <partial>`.
/// A bare `br` (no whitespace yet) keeps the normal history/alias ranking;
/// once there is a space the alias's args take over, ranked by fuzzy score on
/// the key (every key when the partial is empty), ties by key order.
pub fn arg_candidates(aliases: &[AliasDef], input: &str, limit: usize) -> Vec<Candidate> {
    let mut parts = input.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    let Some(rest) = parts.next() else {
        return Vec::new();
    };
    let Some(def) = alias::resolve(aliases, head) else {
        return Vec::new();
    };
    if def.args.is_empty() {
        return Vec::new();
    }
    let partial = rest.trim();
    let mut scored: Vec<(i32, &String)> = def
        .args
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
        .map(|(_, k)| Candidate::Arg {
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

    fn with_args() -> Vec<AliasDef> {
        let mut def = alias::defaults().remove(0); // br (builtin)
        def.args.clear(); // fixture: exactly the args below
        def.args
            .insert("baidu".to_string(), "https://www.baidu.com".to_string());
        def.args
            .insert("gh".to_string(), "https://github.com".to_string());
        vec![def]
    }

    #[test]
    fn arg_candidates_need_a_space_after_the_trigger() {
        let aliases = with_args();
        assert!(arg_candidates(&aliases, "br", 8).is_empty(), "bare alias");
        assert!(
            arg_candidates(&aliases, "nope x", 8).is_empty(),
            "unknown head"
        );
        assert!(
            arg_candidates(&alias::defaults(), "cd x", 8).is_empty(),
            "no args"
        );
    }

    #[test]
    fn arg_candidates_list_and_filter_keys() {
        let aliases = with_args();
        assert_eq!(
            arg_candidates(&aliases, "br ", 8),
            vec![
                Candidate::Arg {
                    alias: "br".to_string(),
                    key: "baidu".to_string()
                },
                Candidate::Arg {
                    alias: "br".to_string(),
                    key: "gh".to_string()
                },
            ],
            "empty partial lists every key, ties by key order"
        );
        assert_eq!(
            arg_candidates(&aliases, "br bai", 8),
            vec![Candidate::Arg {
                alias: "br".to_string(),
                key: "baidu".to_string()
            }]
        );
        assert!(
            arg_candidates(&aliases, "br zzz", 8).is_empty(),
            "no key matches"
        );
        assert_eq!(arg_candidates(&aliases, "br ", 1).len(), 1, "limit applies");
    }

    /// Empty input shows the recent history only - no alias rows, even
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
            out.iter().all(|c| !matches!(c, Candidate::Alias { .. })),
            "aliases stay out of the empty-input list: {out:?}"
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
    fn newer_history_outranks_older_and_alias() {
        let mut store = Store::default();
        store.history.push(history("br", "newest", 20));
        store.history.push(history("br", "older", 10));
        let out = candidates(&store, &alias::defaults(), "br", 10);
        assert_eq!(
            out,
            vec![
                Candidate::History { idx: 0 },
                Candidate::History { idx: 1 },
                Candidate::Alias {
                    name: "br".to_string()
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
            vec![Candidate::History { idx: 0 }, Candidate::History { idx: 1 }]
        );
    }

    /// Aliases only fill the slots history leaves: with one matching history
    /// entry, `limit = 1` hides the alias and `limit = 2` shows it after.
    #[test]
    fn aliases_only_fill_the_slots_history_leaves() {
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
                Candidate::Alias {
                    name: "br".to_string()
                },
            ]
        );
    }

    /// Fuzzy-typing `b` / `c` resolves to exactly the `br` / `cd` builtins;
    /// a trailing space (`br ` / `cd ` before Enter) changes nothing.
    #[test]
    fn single_letter_query_matches_exactly_one_builtin() {
        let store = Store::default();
        for query in ["b", "br", "br "] {
            assert_eq!(
                candidates(&store, &alias::defaults(), query, 5),
                vec![Candidate::Alias {
                    name: "br".to_string()
                }],
                "query {query:?}"
            );
        }
        for query in ["c", "cd", "cd "] {
            assert_eq!(
                candidates(&store, &alias::defaults(), query, 5),
                vec![Candidate::Alias {
                    name: "cd".to_string()
                }],
                "query {query:?}"
            );
        }
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
