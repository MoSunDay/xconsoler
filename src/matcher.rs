//! Candidate ranking: blend history entries and aliases by fuzzy score.

use crate::alias::{self, AliasDef};
use crate::fuzzy;
use crate::storage::Store;

/// A selectable completion candidate.
#[derive(Debug, Clone, PartialEq)]
pub enum Candidate {
    Alias { name: String },
    History { idx: usize },
    /// Named-arg sub-candidate: the input is `<alias> <partial>` and `key`
    /// is one of that alias's named arguments.
    Arg { alias: String, key: String },
}

const RECENCY_BONUS: usize = 40;
const RECENCY_STEP: usize = 250;
const ALIAS_BONUS: i32 = 15;

/// Rank candidates for `query` (whitespace-trimmed).
///
/// * Empty query: newest-first history entries, then aliases, up to `limit`.
/// * Otherwise: each history entry is scored with
///   `fuzzy::score(query, "<entry_label> <input>")` plus a recency bonus of
///   `40 - idx/250`; each alias is scored with
///   `fuzzy::score(query, "<name> <shortcuts>")` plus 15. Results are sorted
///   by score descending; ties put history (newest idx first) before aliases.
pub fn candidates(
    store: &Store,
    aliases: &[AliasDef],
    query: &str,
    limit: usize,
) -> Vec<Candidate> {
    let query = query.trim();
    if query.is_empty() {
        let mut out: Vec<Candidate> = (0..store.history.len().min(limit))
            .map(|idx| Candidate::History { idx })
            .collect();
        for def in aliases {
            if out.len() >= limit {
                break;
            }
            out.push(Candidate::Alias {
                name: def.name.clone(),
            });
        }
        return out;
    }

    // (score, kind_rank, order, candidate): history kind_rank 0, alias 1.
    let mut scored: Vec<(i32, u8, usize, Candidate)> = Vec::new();

    for (idx, entry) in store.history.iter().enumerate() {
        let label = match alias::resolve(aliases, &entry.alias) {
            Some(def) => alias::entry_label(def),
            None => entry.alias.as_str(),
        };
        let haystack = format!("{} {}", label, entry.input());
        if let Some(s) = fuzzy::score(query, &haystack) {
            let recency = RECENCY_BONUS.saturating_sub(idx / RECENCY_STEP) as i32;
            scored.push((s + recency, 0, idx, Candidate::History { idx }));
        }
    }
    for (order, def) in aliases.iter().enumerate() {
        let haystack = format!("{} {}", def.name, def.shortcuts.join(" "));
        if let Some(s) = fuzzy::score(query, &haystack) {
            scored.push((
                s + ALIAS_BONUS,
                1,
                order,
                Candidate::Alias {
                    name: def.name.clone(),
                },
            ));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    scored.into_iter().take(limit).map(|t| t.3).collect()
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
        let mut def = alias::defaults().remove(0); // browser
        def.args.insert("baidu".to_string(), "https://www.baidu.com".to_string());
        def.args.insert("gh".to_string(), "https://github.com".to_string());
        vec![def]
    }

    #[test]
    fn arg_candidates_need_a_space_after_the_trigger() {
        let aliases = with_args();
        assert!(arg_candidates(&aliases, "br", 8).is_empty(), "bare alias");
        assert!(arg_candidates(&aliases, "nope x", 8).is_empty(), "unknown head");
        assert!(arg_candidates(&aliases, "clipboard x", 8).is_empty(), "no args");
    }

    #[test]
    fn arg_candidates_list_and_filter_keys() {
        let aliases = with_args();
        assert_eq!(
            arg_candidates(&aliases, "br ", 8),
            vec![
                Candidate::Arg { alias: "browser".to_string(), key: "baidu".to_string() },
                Candidate::Arg { alias: "browser".to_string(), key: "gh".to_string() },
            ],
            "empty partial lists every key, ties by key order"
        );
        assert_eq!(
            arg_candidates(&aliases, "br bai", 8),
            vec![Candidate::Arg { alias: "browser".to_string(), key: "baidu".to_string() }]
        );
        assert!(arg_candidates(&aliases, "br zzz", 8).is_empty(), "no key matches");
        assert_eq!(arg_candidates(&aliases, "br ", 1).len(), 1, "limit applies");
    }

    #[test]
    fn empty_query_lists_history_then_aliases() {
        let mut store = Store::default();
        store.history.push(history("browser", "a", 2));
        store.history.push(history("browser", "b", 1));
        let aliases = alias::defaults();

        let out = candidates(&store, &aliases, "   ", 3);
        assert_eq!(
            out,
            vec![
                Candidate::History { idx: 0 },
                Candidate::History { idx: 1 },
                Candidate::Alias {
                    name: "browser".to_string()
                },
            ]
        );
    }

    #[test]
    fn empty_query_limit_one_history_only() {
        let mut store = Store::default();
        store.history.push(history("browser", "a", 2));
        store.history.push(history("browser", "b", 1));
        let out = candidates(&store, &alias::defaults(), "", 1);
        assert_eq!(out, vec![Candidate::History { idx: 0 }]);
    }

    #[test]
    fn newer_history_outranks_older_and_alias() {
        let mut store = Store::default();
        store.history.push(history("browser", "newest", 20));
        store.history.push(history("browser", "older", 10));
        let out = candidates(&store, &alias::defaults(), "br", 10);
        assert_eq!(
            out,
            vec![
                Candidate::History { idx: 0 },
                Candidate::History { idx: 1 },
                Candidate::Alias {
                    name: "browser".to_string()
                },
                // "br" is also a subsequence of "clipboard cd", but scores lower
                Candidate::Alias {
                    name: "clipboard".to_string()
                },
            ]
        );
    }

    #[test]
    fn alias_name_match() {
        let store = Store::default();
        let out = candidates(&store, &alias::defaults(), "clip", 5);
        assert_eq!(
            out,
            vec![Candidate::Alias {
                name: "clipboard".to_string()
            }]
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
            store.history.push(history("browser", &format!("x{i}"), i));
        }
        let out = candidates(&store, &alias::defaults(), "x", 2);
        assert_eq!(
            out,
            vec![Candidate::History { idx: 0 }, Candidate::History { idx: 1 }]
        );
    }
}
