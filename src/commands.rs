//! Catalog behind the command palette: the built-in `:`/`/` commands as
//! selectable rows. Plain data - behaviour lives in `crate::app`.

/// One palette row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    /// Token inserted into the input (`:add`, `/settings`).
    pub token: &'static str,
    /// One-line description shown next to the token.
    pub desc: &'static str,
    /// True when the command still needs typed arguments before it can run.
    pub needs_arg: bool,
}

/// Every built-in `:`/`/` command, in palette order.
pub const ALL: [CommandSpec; 7] = [
    CommandSpec {
        token: ":add",
        desc: "define or override an alias",
        needs_arg: true,
    },
    CommandSpec {
        token: ":del",
        desc: "delete a user alias",
        needs_arg: true,
    },
    CommandSpec {
        token: ":arg",
        desc: "set a shortcut (alias key value)",
        needs_arg: true,
    },
    CommandSpec {
        token: ":unarg",
        desc: "remove a shortcut (alias key)",
        needs_arg: true,
    },
    CommandSpec {
        token: ":help",
        desc: "show the colon-command help",
        needs_arg: false,
    },
    CommandSpec {
        token: ":import-chrome",
        desc: "import Chrome bookmarks (default: br)",
        needs_arg: true,
    },
    CommandSpec {
        token: "/settings",
        desc: "open the settings page",
        needs_arg: false,
    },
];

/// Number of palette rows.
pub fn len() -> usize {
    ALL.len()
}

/// Row `i`, or `None` when out of range.
pub fn get(i: usize) -> Option<&'static CommandSpec> {
    ALL.get(i)
}

/// Text `Enter` completes into the input: a trailing space while the command
/// still needs arguments, the bare token otherwise.
pub fn insert_text(c: &CommandSpec) -> String {
    if c.needs_arg {
        format!("{} ", c.token)
    } else {
        c.token.to_string()
    }
}

/// Catalog rows matching `query`, best score first. The haystack is
/// `"{token} {desc}"`; an empty query keeps the whole catalog. The sort is
/// stable, so equal scores keep catalog order.
pub fn matching(query: &str) -> Vec<&'static CommandSpec> {
    if query.trim().is_empty() {
        return ALL.iter().collect();
    }
    let mut ranked: Vec<(i32, &'static CommandSpec)> = ALL
        .iter()
        .filter_map(|c| {
            crate::fuzzy::score(query, &format!("{} {}", c.token, c.desc)).map(|s| (s, c))
        })
        .collect();
    ranked.sort_by_key(|r| std::cmp::Reverse(r.0)); // stable: ties keep catalog order
    ranked.into_iter().map(|(_, c)| c).collect()
}

/// Rows the palette shows for `input`: a `:`/`/` line is a query (ranked
/// matches), anything else - empty included - means the plain catalog.
pub fn palette_items(input: &str) -> Vec<&'static CommandSpec> {
    if input.starts_with('/') || input.starts_with(':') {
        matching(input)
    } else {
        ALL.iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn tokens_are_unique_and_prefixed() {
        let mut seen = BTreeSet::new();
        for c in &ALL {
            assert!(
                c.token.starts_with(':') || c.token.starts_with('/'),
                "token {} must start with ':' or '/'",
                c.token
            );
            assert!(seen.insert(c.token), "duplicate token {}", c.token);
        }
        assert_eq!(seen.len(), len());
    }

    #[test]
    fn descriptions_are_non_empty() {
        for c in &ALL {
            assert!(!c.desc.is_empty(), "{} has an empty desc", c.token);
        }
    }

    #[test]
    fn insert_text_adds_a_space_only_where_args_are_needed() {
        let add = get(0).expect(":add");
        assert_eq!(insert_text(add), ":add ");
        let help = ALL.iter().find(|c| c.token == ":help").expect(":help");
        assert_eq!(insert_text(help), ":help");
        let settings = ALL
            .iter()
            .find(|c| c.token == "/settings")
            .expect("/settings");
        assert_eq!(insert_text(settings), "/settings");
    }

    #[test]
    fn get_is_bounds_checked() {
        assert_eq!(get(0).map(|c| c.token), Some(":add"));
        assert_eq!(get(len() - 1).map(|c| c.token), Some("/settings"));
        assert!(get(len()).is_none());
        assert!(get(usize::MAX).is_none());
    }

    #[test]
    fn import_chrome_row_sits_before_settings() {
        let i = ALL
            .iter()
            .position(|c| c.token == ":import-chrome")
            .expect(":import-chrome row");
        assert_eq!(ALL[i].desc, "import Chrome bookmarks (default: br)");
        assert_eq!(insert_text(&ALL[i]), ":import-chrome ");
        assert_eq!(ALL[i + 1].token, "/settings", "inserted before /settings");
        assert_eq!(get(i).map(|c| c.token), Some(":import-chrome"));
    }

    fn tokens(items: &[&CommandSpec]) -> Vec<&'static str> {
        items.iter().map(|c| c.token).collect()
    }

    #[test]
    fn an_exact_slash_token_matches_only_itself() {
        let hits = matching("/settings");
        assert_eq!(tokens(&hits), vec!["/settings"]);
        assert!(matching("/zz").is_empty(), "no fuzzy hit for /zz");
        assert!(matching("/abc").is_empty(), "no fuzzy hit for /abc");
    }

    #[test]
    fn a_partial_query_ranks_the_closest_token_first() {
        let hits = matching("/set");
        assert_eq!(tokens(&hits), vec!["/settings"]);
        assert_eq!(hits[0].desc, "open the settings page");
    }

    #[test]
    fn colon_a_ranks_the_arg_commands_above_the_mid_word_hits() {
        let hits = matching(":a");
        let pos = |t: &str| hits.iter().position(|c| c.token == t).unwrap_or(usize::MAX);
        // `:add`/`:arg` hit the 'a' right after ':' (consecutive bonus) while
        // `:unarg`, `:help` and `:import-chrome` only match 'a' mid-word.
        assert!(pos(":add") < pos(":unarg"), "{:?}", tokens(&hits));
        assert!(pos(":arg") < pos(":unarg"), "{:?}", tokens(&hits));
        assert!(pos(":add") < pos(":help"), "{:?}", tokens(&hits));
        assert!(pos(":arg") < pos(":import-chrome"), "{:?}", tokens(&hits));
        assert!(pos(":add") < pos(":arg"), "ties keep catalog order");
        assert!(!hits.iter().any(|c| c.token == "/settings"));
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(tokens(&matching("/SETTING")), tokens(&matching("/setting")));
        assert_eq!(tokens(&matching(":HELP")), vec![":help"]);
    }

    #[test]
    fn an_empty_query_yields_the_whole_catalog() {
        assert_eq!(
            tokens(&matching("")),
            tokens(&ALL.iter().collect::<Vec<_>>())
        );
        assert_eq!(
            tokens(&matching("   ")),
            tokens(&ALL.iter().collect::<Vec<_>>())
        );
    }

    #[test]
    fn palette_items_filter_only_on_a_prefixed_query() {
        let all = tokens(&ALL.iter().collect::<Vec<_>>());
        assert_eq!(tokens(&palette_items("br")), all, "plain text: catalog");
        assert_eq!(tokens(&palette_items("")), all, "empty: catalog");
        assert_eq!(
            tokens(&palette_items("/")),
            vec!["/settings"],
            "a bare slash is a query"
        );
        assert!(palette_items("/zz").is_empty(), "no hits, no rows");
    }
}
