//! Alias definitions, built-in defaults, and `:` command parsing.
//!
//! Command template conventions:
//!
//! * `{input}` - placeholder for the user's input; it is shell-quoted
//!   (see [`crate::exec::shell_quote`]) before being substituted.
//! * `@stdin` - marker meaning the input is delivered through the child
//!   process stdin; the marker is removed from the command string before
//!   execution (see [`crate::exec::run_alias`]).

use serde::{Deserialize, Serialize};

/// A single alias definition (built-in or user defined).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AliasDef {
    pub name: String,
    pub shortcuts: Vec<String>,
    pub linux: Option<String>,
    pub macos: Option<String>,
    pub builtin: bool,
}

/// Built-in aliases shipped with xconsoler.
pub fn defaults() -> Vec<AliasDef> {
    vec![
        AliasDef {
            name: "browser".to_string(),
            shortcuts: vec!["br".to_string()],
            linux: Some("xdg-open {input} >/dev/null 2>&1".to_string()),
            macos: Some("open {input}".to_string()),
            builtin: true,
        },
        AliasDef {
            name: "clipboard".to_string(),
            shortcuts: vec!["cd".to_string()],
            linux: Some(
                "wl-copy @stdin || xclip -selection clipboard @stdin || xsel --clipboard --input"
                    .to_string(),
            ),
            macos: Some("pbcopy @stdin".to_string()),
            builtin: true,
        },
    ]
}

/// Resolve a token (full name or shortcut) to a definition, case-insensitively.
pub fn resolve<'a>(defs: &'a [AliasDef], token: &str) -> Option<&'a AliasDef> {
    let token = token.to_lowercase();
    defs.iter().find(|d| {
        d.name.to_lowercase() == token || d.shortcuts.iter().any(|s| s.to_lowercase() == token)
    })
}

/// Human label such as `browser (br)`; just the name when no shortcut exists.
pub fn label(def: &AliasDef) -> String {
    if def.shortcuts.is_empty() {
        def.name.clone()
    } else {
        format!("{} ({})", def.name, def.shortcuts.join(", "))
    }
}

/// First shortcut, or the name; used when rendering history entries.
pub fn entry_label(def: &AliasDef) -> &str {
    match def.shortcuts.first() {
        Some(s) => s.as_str(),
        None => def.name.as_str(),
    }
}

/// A mutation requested through a `:` command.
#[derive(Debug, Clone, PartialEq)]
pub enum AliasOp {
    Add(AliasDef),
    Remove(String),
}

/// Parse a colon command. The leading `:` has already been stripped by the
/// caller and the line is trimmed here. Returns `Ok(None)` for `help`,
/// empty input, or unknown sub-commands (the caller shows help text).
///
/// Supported forms:
///
/// ```text
/// add <name>[,<shortcut>...] <linux-cmd> [// <macos-cmd>]
/// del <name>
/// ```
///
/// A command of `-` means "not configured on this platform". The command
/// strings keep their original spacing (only re-joined by whitespace tokens).
pub fn parse_colon_cmd(line: &str) -> Result<Option<AliasOp>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let tokens: Vec<&str> = line.split_whitespace().collect();
    match tokens[0].to_lowercase().as_str() {
        "add" => parse_add(&tokens[1..]),
        "del" => parse_del(&tokens[1..]),
        _ => Ok(None), // "help" and anything unknown: caller shows help
    }
}

fn parse_add(rest: &[&str]) -> Result<Option<AliasOp>, String> {
    if rest.is_empty() {
        return Err("usage: add <name>[,<shortcut>...] <linux-cmd> // <macos-cmd>".to_string());
    }
    let (name, shortcuts) = parse_name_list(rest[0])?;

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
        shortcuts,
        linux,
        macos,
        builtin: false,
    })))
}

fn parse_del(rest: &[&str]) -> Result<Option<AliasOp>, String> {
    if rest.len() != 1 {
        return Err("usage: del <name>".to_string());
    }
    Ok(Some(AliasOp::Remove(rest[0].to_string())))
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
    let mut shortcuts = Vec::new();
    for sc in parts {
        if sc.is_empty() || !valid_ident(sc) {
            return Err(format!("invalid shortcut: {sc}"));
        }
        shortcuts.push(sc.to_string());
    }
    Ok((name.to_string(), shortcuts))
}

/// Names and shortcuts may contain only alphanumerics, `-` and `_`.
fn valid_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
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
    fn defaults_resolve_by_shortcut_and_name_any_case() {
        let defs = defaults();
        let br = resolve(&defs, "br").expect("shortcut should resolve");
        assert_eq!(br.name, "browser");
        assert!(resolve(&defs, "BROWSER").is_some());
        assert!(resolve(&defs, "Clipboard").is_some());
        assert!(resolve(&defs, "CD").is_some());
        assert!(resolve(&defs, "nope").is_none());
    }

    #[test]
    fn labels_use_first_shortcut() {
        let defs = defaults();
        let br = resolve(&defs, "br").unwrap();
        assert_eq!(label(br), "browser (br)");
        assert_eq!(entry_label(br), "br");

        let mut d = br.clone();
        d.shortcuts.clear();
        assert_eq!(label(&d), "browser");
        assert_eq!(entry_label(&d), "browser");
    }

    #[test]
    fn parse_add_both_platforms() {
        let op = parse_colon_cmd("add myalias,ma echo {input} // say {input}")
            .unwrap()
            .unwrap();
        match op {
            AliasOp::Add(def) => {
                assert_eq!(def.name, "myalias");
                assert_eq!(def.shortcuts, vec!["ma".to_string()]);
                assert_eq!(def.linux.as_deref(), Some("echo {input}"));
                assert_eq!(def.macos.as_deref(), Some("say {input}"));
                assert!(!def.builtin);
            }
            other => panic!("expected Add, got {other:?}"),
        }
    }

    #[test]
    fn parse_add_linux_only_and_dash_skip() {
        match parse_colon_cmd("add only-one xdg-open {input}")
            .unwrap()
            .unwrap()
        {
            AliasOp::Add(d) => {
                assert_eq!(d.linux.as_deref(), Some("xdg-open {input}"));
                assert_eq!(d.macos, None);
            }
            other => panic!("expected Add, got {other:?}"),
        }
        match parse_colon_cmd("add skip - // open {input}")
            .unwrap()
            .unwrap()
        {
            AliasOp::Add(d) => {
                assert_eq!(d.linux, None);
                assert_eq!(d.macos.as_deref(), Some("open {input}"));
            }
            other => panic!("expected Add, got {other:?}"),
        }
    }

    #[test]
    fn parse_del() {
        match parse_colon_cmd("del browser").unwrap().unwrap() {
            AliasOp::Remove(name) => assert_eq!(name, "browser"),
            other => panic!("expected Remove, got {other:?}"),
        }
        assert!(parse_colon_cmd("del").is_err());
        assert!(parse_colon_cmd("del a b").is_err());
    }

    #[test]
    fn parse_help_and_unknown_return_none() {
        assert_eq!(parse_colon_cmd("help").unwrap(), None);
        assert_eq!(parse_colon_cmd("").unwrap(), None);
        assert_eq!(parse_colon_cmd("   ").unwrap(), None);
        assert_eq!(parse_colon_cmd("frobnicate x y").unwrap(), None);
    }

    #[test]
    fn parse_add_errors() {
        assert!(parse_colon_cmd("add").is_err()); // no arguments at all
        assert!(parse_colon_cmd("add bad!name echo").is_err()); // invalid name char
        assert!(parse_colon_cmd("add x,b@d! echo").is_err()); // invalid shortcut char
        assert!(parse_colon_cmd("add ,x echo").is_err()); // empty name
        assert!(parse_colon_cmd("add x - // -").is_err()); // both sides skipped
        assert!(parse_colon_cmd("add x").is_err()); // no command at all
        assert!(parse_colon_cmd("add x a // b // c").is_err()); // too many separators
    }
}
