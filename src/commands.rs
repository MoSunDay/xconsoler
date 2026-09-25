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
pub const ALL: [CommandSpec; 6] = [
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
}
