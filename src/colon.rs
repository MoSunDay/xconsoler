//! Colon (`:`) command parsing: `:add`, `:del`, `:arg`/`:shortcut`,
//! `:unarg`/`:unshortcut`, and `:import-chrome`.
//!
//! The caller strips the leading `:` before handing the line over; [`parse`]
//! trims it itself and returns `Ok(None)` for `help`, empty input, and
//! unknown sub-commands so the caller can show its usage block.

use std::collections::BTreeMap;

use crate::alias::{valid_ident, AliasDef};

/// A mutation requested through a `:` command.
#[derive(Debug, Clone, PartialEq)]
pub enum AliasOp {
    Add(AliasDef),
    Remove(String),
    /// `:arg` / `:shortcut name key value...` — set (or replace) a shortcut.
    SetShortcut {
        name: String,
        key: String,
        value: String,
    },
    /// `:unarg` / `:unshortcut name key` — remove a shortcut.
    DelShortcut {
        name: String,
        key: String,
    },
    /// `:import-chrome [alias]` — import this machine's Chrome bookmarks as
    /// shortcut rows on the target alias (`br` when omitted).
    ImportChrome {
        target: Option<String>,
    },
}

/// Parse a colon command. The leading `:` has already been stripped by the
/// caller and the line is trimmed here. Returns `Ok(None)` for `help`,
/// empty input, or unknown sub-commands (the caller shows help text).
///
/// Supported forms:
///
/// ```text
/// add <name>[,<trigger>...] <linux-cmd> [// <macos-cmd>]
/// del <name>
/// arg <name> <key> <value...>  (shortcut ... is a synonym)
/// unarg <name> <key>           (unshortcut ... is a synonym)
/// import-chrome [alias]
/// ```
///
/// A command of `-` means "not configured on this platform". The command
/// strings keep their original spacing (only re-joined by whitespace tokens);
/// shortcut values keep theirs too — the value is the raw remainder.
pub fn parse(line: &str) -> Result<Option<AliasOp>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let tokens: Vec<&str> = line.split_whitespace().collect();
    match tokens[0].to_lowercase().as_str() {
        "add" => parse_add(&tokens[1..]),
        "del" => parse_del(&tokens[1..]),
        // `arg` with no arguments must not slice past the line end.
        "arg" | "shortcut" => parse_set_shortcut(line.get(1 + tokens[0].len()..).unwrap_or("")),
        "unarg" | "unshortcut" => parse_del_shortcut(&tokens[1..]),
        "import-chrome" => parse_import_chrome(&tokens[1..]),
        _ => Ok(None), // "help" and anything unknown: caller shows help
    }
}

fn parse_add(rest: &[&str]) -> Result<Option<AliasOp>, String> {
    if rest.is_empty() {
        return Err("usage: add <name>[,<trigger>...] <linux-cmd> // <macos-cmd>".to_string());
    }
    let (name, triggers) = parse_name_list(rest[0])?;

    let cmd_tokens = &rest[1..];
    let seps = cmd_tokens.iter().filter(|t| **t == "//").count();
    if seps > 1 {
        return Err("too many // separators".to_string());
    }
    let (linux, macos) = match cmd_tokens.iter().position(|t| *t == "//") {
        Some(i) => (
            cmd_or_none(&cmd_tokens[..i].join(" ")),
            cmd_or_none(&cmd_tokens[i + 1..].join(" ")),
        ),
        None => (cmd_or_none(&cmd_tokens.join(" ")), None),
    };
    if linux.is_none() && macos.is_none() {
        return Err("no command given ('-' skips one platform, not both)".to_string());
    }

    Ok(Some(AliasOp::Add(AliasDef {
        name,
        triggers,
        linux,
        macos,
        shortcuts: BTreeMap::new(),
    })))
}

fn parse_del(rest: &[&str]) -> Result<Option<AliasOp>, String> {
    if rest.len() != 1 {
        return Err("usage: del <name>".to_string());
    }
    Ok(Some(AliasOp::Remove(rest[0].to_string())))
}

/// `arg <name> <key> <value...>` (or `shortcut ...`): the value is the raw
/// remainder of the line after the key token, so spaces survive.
fn parse_set_shortcut(rest: &str) -> Result<Option<AliasOp>, String> {
    let toks = indexed_tokens(rest);
    if toks.len() < 3 {
        return Err("usage: arg <name> <key> <value...>".to_string());
    }
    let (_, name) = toks[0];
    let (_, key) = toks[1];
    let (value_at, _) = toks[2];
    let value = rest[value_at..].trim();
    Ok(Some(AliasOp::SetShortcut {
        name: name.to_string(),
        key: key.to_string(),
        value: value.to_string(),
    }))
}

fn parse_del_shortcut(rest: &[&str]) -> Result<Option<AliasOp>, String> {
    if rest.len() != 2 {
        return Err("usage: unarg <name> <key>".to_string());
    }
    Ok(Some(AliasOp::DelShortcut {
        name: rest[0].to_string(),
        key: rest[1].to_string(),
    }))
}

/// `import-chrome` or `import-chrome <alias>`: the target must be a single
/// well-formed identifier (same rule as `:add` names).
fn parse_import_chrome(rest: &[&str]) -> Result<Option<AliasOp>, String> {
    match rest {
        [] => Ok(Some(AliasOp::ImportChrome { target: None })),
        [target] if valid_ident(target) => Ok(Some(AliasOp::ImportChrome {
            target: Some(target.to_string()),
        })),
        [target] => Err(format!("invalid alias name: {target}")),
        _ => Err("usage: import-chrome [alias]".to_string()),
    }
}

/// Whitespace-separated tokens of `s` with their byte offsets, e.g.
/// `indexed_tokens(" a  bc ") == [(1, "a"), (4, "bc")]`. Used to slice the
/// raw remainder after a token without re-joining (spacing survives).
fn indexed_tokens(s: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            if let Some(st) = start.take() {
                out.push((st, &s[st..i]));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push((st, &s[st..]));
    }
    out
}

fn parse_name_list(raw: &str) -> Result<(String, Vec<String>), String> {
    let mut parts = raw.split(',');
    let name = parts.next().unwrap_or("");
    if name.is_empty() {
        return Err("alias name cannot be empty".to_string());
    }
    if !valid_ident(name) {
        return Err(format!("invalid alias name: {name}"));
    }
    let mut triggers = Vec::new();
    for sc in parts {
        if sc.is_empty() || !valid_ident(sc) {
            return Err(format!("invalid trigger: {sc}"));
        }
        triggers.push(sc.to_string());
    }
    Ok((name.to_string(), triggers))
}

/// `-` (or an empty side) means the platform has no command configured.
fn cmd_or_none(joined: &str) -> Option<String> {
    let t = joined.trim();
    if t.is_empty() || t == "-" {
        None
    } else {
        Some(t.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_add_both_platforms() {
        let op = parse("add myalias,ma echo {input} // say {input}")
            .unwrap()
            .unwrap();
        match op {
            AliasOp::Add(def) => {
                assert_eq!(def.name, "myalias");
                assert_eq!(def.triggers, vec!["ma".to_string()]);
                assert_eq!(def.linux.as_deref(), Some("echo {input}"));
                assert_eq!(def.macos.as_deref(), Some("say {input}"));
            }
            other => panic!("expected Add, got {other:?}"),
        }
    }

    #[test]
    fn parse_add_linux_only_and_dash_skip() {
        match parse("add only-one xdg-open {input}").unwrap().unwrap() {
            AliasOp::Add(d) => {
                assert_eq!(d.linux.as_deref(), Some("xdg-open {input}"));
                assert_eq!(d.macos, None);
            }
            other => panic!("expected Add, got {other:?}"),
        }
        match parse("add skip - // open {input}").unwrap().unwrap() {
            AliasOp::Add(d) => {
                assert_eq!(d.linux, None);
                assert_eq!(d.macos.as_deref(), Some("open {input}"));
            }
            other => panic!("expected Add, got {other:?}"),
        }
    }

    #[test]
    fn parse_del() {
        match parse("del t").unwrap().unwrap() {
            AliasOp::Remove(name) => assert_eq!(name, "t"),
            other => panic!("expected Remove, got {other:?}"),
        }
        assert!(parse("del").is_err());
        assert!(parse("del a b").is_err());
    }

    #[test]
    fn parse_help_and_unknown_return_none() {
        assert_eq!(parse("help").unwrap(), None);
        assert_eq!(parse("").unwrap(), None);
        assert_eq!(parse("   ").unwrap(), None);
        assert_eq!(parse("frobnicate x y").unwrap(), None);
    }

    #[test]
    fn parse_shortcut_keeps_value_spacing() {
        match parse("arg br baidu https://www.baidu.com")
            .unwrap()
            .unwrap()
        {
            AliasOp::SetShortcut { name, key, value } => {
                assert_eq!(name, "br");
                assert_eq!(key, "baidu");
                assert_eq!(value, "https://www.baidu.com");
            }
            other => panic!("expected SetShortcut, got {other:?}"),
        }
        // the value is the raw remainder: internal spacing survives
        match parse("arg  t   here   cd /tmp  &&   ls ").unwrap().unwrap() {
            AliasOp::SetShortcut { value, .. } => assert_eq!(value, "cd /tmp  &&   ls"),
            other => panic!("expected SetShortcut, got {other:?}"),
        }
    }

    #[test]
    fn parse_shortcut_requires_three_parts() {
        assert!(parse("arg").is_err());
        assert!(parse("arg br").is_err());
        assert!(parse("arg br baidu").is_err());
    }

    #[test]
    fn parse_unshortcut() {
        match parse("unarg br baidu").unwrap().unwrap() {
            AliasOp::DelShortcut { name, key } => {
                assert_eq!(name, "br");
                assert_eq!(key, "baidu");
            }
            other => panic!("expected DelShortcut, got {other:?}"),
        }
        assert!(parse("unarg br").is_err());
        assert!(parse("unarg a b c").is_err());
    }

    #[test]
    fn parse_shortcut_synonyms_map_to_the_same_ops() {
        let cmd = |line: &str| parse(line).unwrap();
        assert_eq!(cmd("shortcut t tt value"), cmd("arg t tt value"));
        assert_eq!(cmd("unshortcut t tt"), cmd("unarg t tt"));
    }

    #[test]
    fn parse_add_errors() {
        assert!(parse("add").is_err()); // no arguments at all
        assert!(parse("add bad!name echo").is_err()); // invalid name char
        assert!(parse("add x,b@d! echo").is_err()); // invalid shortcut char
        assert!(parse("add ,x echo").is_err()); // empty name
        assert!(parse("add x - // -").is_err()); // both sides skipped
        assert!(parse("add x").is_err()); // no command at all
        assert!(parse("add x a // b // c").is_err()); // too many separators
    }

    #[test]
    fn parse_import_chrome_default_and_target() {
        assert_eq!(
            parse("import-chrome").unwrap(),
            Some(AliasOp::ImportChrome { target: None })
        );
        assert_eq!(
            parse("import-chrome browser").unwrap(),
            Some(AliasOp::ImportChrome {
                target: Some("browser".to_string())
            })
        );
    }

    #[test]
    fn parse_import_chrome_rejects_bad_targets() {
        assert_eq!(
            parse("import-chrome bad!name").unwrap_err(),
            "invalid alias name: bad!name"
        );
        assert_eq!(
            parse("import-chrome a b").unwrap_err(),
            "usage: import-chrome [alias]"
        );
    }
}
