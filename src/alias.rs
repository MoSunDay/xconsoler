//! Alias definitions, built-in defaults, and `:` command parsing.
//!
//! Command template conventions:
//!
//! * `{input}` - placeholder for the user's input; it is shell-quoted
//!   (see [`crate::exec::shell_quote`]) before being substituted. Named
//!   arguments (see `args`) are resolved into the input first
//!   (see [`crate::exec::resolve_args`]).
//! * `@stdin` - marker meaning the input is delivered through the child
//!   process stdin; the marker is removed from the command string before
//!   execution (see [`crate::exec::run_alias`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A single alias definition (built-in or user defined).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AliasDef {
    pub name: String,
    pub shortcuts: Vec<String>,
    pub linux: Option<String>,
    pub macos: Option<String>,
    /// Named arguments: typing `<alias> <key> <more…>` replaces `<key>` with
    /// the mapped value before the command runs. `#[serde(default)]` keeps
    /// stores written before this field existed loadable.
    #[serde(default)]
    pub args: BTreeMap<String, String>,
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
            args: BTreeMap::new(),
            builtin: true,
        },
        AliasDef {
            name: "clipboard".to_string(),
            shortcuts: vec!["cd".to_string()],
            // Native backend: no xclip/wl-copy/xsel/pbcopy dependency.
            linux: Some(crate::clipboard::TEMPLATE.to_string()),
            macos: Some(crate::clipboard::TEMPLATE.to_string()),
            args: BTreeMap::new(),
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
    /// `:arg name key value...` — set (or replace) a named argument.
    SetArg {
        name: String,
        key: String,
        value: String,
    },
    /// `:unarg name key` — remove a named argument.
    DelArg {
        name: String,
        key: String,
    },
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
/// arg <name> <key> <value...>
/// unarg <name> <key>
/// ```
///
/// A command of `-` means "not configured on this platform". The command
/// strings keep their original spacing (only re-joined by whitespace tokens);
/// `arg` values keep theirs too — the value is the raw remainder of the line.
pub fn parse_colon_cmd(line: &str) -> Result<Option<AliasOp>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let tokens: Vec<&str> = line.split_whitespace().collect();
    match tokens[0].to_lowercase().as_str() {
        "add" => parse_add(&tokens[1..]),
        "del" => parse_del(&tokens[1..]),
        // `arg` with no arguments must not slice past the line end.
        "arg" => parse_arg(line.get(1 + tokens[0].len()..).unwrap_or("")),
        "unarg" => parse_unarg(&tokens[1..]),
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
        args: BTreeMap::new(),
        builtin: false,
    })))
}

fn parse_del(rest: &[&str]) -> Result<Option<AliasOp>, String> {
    if rest.len() != 1 {
        return Err("usage: del <name>".to_string());
    }
    Ok(Some(AliasOp::Remove(rest[0].to_string())))
}

/// `arg <name> <key> <value...>`: the value is the raw remainder of the
/// line after the key token, so it may contain (and keep) spaces.
fn parse_arg(rest: &str) -> Result<Option<AliasOp>, String> {
    let toks = indexed_tokens(rest);
    if toks.len() < 3 {
        return Err("usage: arg <name> <key> <value...>".to_string());
    }
    let (_, name) = toks[0];
    let (_, key) = toks[1];
    let (value_at, _) = toks[2];
    let value = rest[value_at..].trim();
    Ok(Some(AliasOp::SetArg {
        name: name.to_string(),
        key: key.to_string(),
        value: value.to_string(),
    }))
}

fn parse_unarg(rest: &[&str]) -> Result<Option<AliasOp>, String> {
    if rest.len() != 2 {
        return Err("usage: unarg <name> <key>".to_string());
    }
    Ok(Some(AliasOp::DelArg {
        name: rest[0].to_string(),
        key: rest[1].to_string(),
    }))
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
/// Public so the settings form validates its wizard input the same way.
pub fn valid_ident(s: &str) -> bool {
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

/// Set (or replace) named argument `key` on user alias `name` inside the
/// persisted alias list. A built-in name is materialized as an overriding
/// user definition first — that is how `:arg browser k v` persists. Returns
/// the previous value when the key already existed. `Err` for unknown names.
pub fn set_arg(
    user: &mut Vec<AliasDef>,
    name: &str,
    key: &str,
    value: &str,
) -> Result<Option<String>, String> {
    if let Some(def) = user.iter_mut().find(|d| d.name == name) {
        return Ok(def.args.insert(key.to_string(), value.to_string()));
    }
    if let Some(mut def) = defaults().into_iter().find(|d| d.name == name) {
        def.builtin = false;
        let prev = def.args.insert(key.to_string(), value.to_string());
        user.push(def);
        return Ok(prev);
    }
    Err(format!("alias not found: {name}"))
}

/// Remove named argument `key` from user alias `name`. `Err` when the alias
/// is unknown or does not carry that key.
pub fn remove_arg(user: &mut [AliasDef], name: &str, key: &str) -> Result<(), String> {
    match user.iter_mut().find(|d| d.name == name) {
        Some(def) => match def.args.remove(key) {
            Some(_) => Ok(()),
            None => Err(format!("no named arg \"{key}\" on {name}")),
        },
        None => Err(format!("alias not found: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_def(name: &str, linux: Option<&str>) -> AliasDef {
        AliasDef {
            name: name.to_string(),
            shortcuts: vec![],
            linux: linux.map(|s| s.to_string()),
            macos: None,
            args: BTreeMap::new(),
            builtin: false,
        }
    }

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
    fn clipboard_default_uses_native_backend() {
        let defs = defaults();
        let cd = resolve(&defs, "cd").expect("cd resolves");
        assert_eq!(cd.linux.as_deref(), Some(crate::clipboard::TEMPLATE));
        assert_eq!(cd.macos.as_deref(), Some(crate::clipboard::TEMPLATE));
        assert!(cd.builtin);
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
    fn parse_arg_keeps_value_spacing() {
        match parse_colon_cmd("arg br baidu https://www.baidu.com").unwrap().unwrap() {
            AliasOp::SetArg { name, key, value } => {
                assert_eq!(name, "br");
                assert_eq!(key, "baidu");
                assert_eq!(value, "https://www.baidu.com");
            }
            other => panic!("expected SetArg, got {other:?}"),
        }
        // the value is the raw remainder: internal spacing survives
        match parse_colon_cmd("arg  t   here   cd /tmp  &&   ls ").unwrap().unwrap() {
            AliasOp::SetArg { value, .. } => assert_eq!(value, "cd /tmp  &&   ls"),
            other => panic!("expected SetArg, got {other:?}"),
        }
    }

    #[test]
    fn parse_arg_requires_three_parts() {
        assert!(parse_colon_cmd("arg").is_err());
        assert!(parse_colon_cmd("arg br").is_err());
        assert!(parse_colon_cmd("arg br baidu").is_err());
    }

    #[test]
    fn parse_unarg() {
        match parse_colon_cmd("unarg br baidu").unwrap().unwrap() {
            AliasOp::DelArg { name, key } => {
                assert_eq!(name, "br");
                assert_eq!(key, "baidu");
            }
            other => panic!("expected DelArg, got {other:?}"),
        }
        assert!(parse_colon_cmd("unarg br").is_err());
        assert!(parse_colon_cmd("unarg a b c").is_err());
    }

    #[test]
    fn set_arg_on_user_alias_and_replacement() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        assert_eq!(set_arg(&mut user, "t", "here", "cd /tmp").unwrap(), None);
        assert_eq!(set_arg(&mut user, "t", "here", "cd /var").unwrap(), Some("cd /tmp".into()));
        assert_eq!(user[0].args.get("here").map(String::as_str), Some("cd /var"));
        assert_eq!(
            set_arg(&mut user, "nope", "k", "v").unwrap_err(),
            "alias not found: nope"
        );
    }

    #[test]
    fn set_arg_on_builtin_materializes_user_override() {
        let mut user = vec![];
        assert_eq!(set_arg(&mut user, "browser", "baidu", "https://www.baidu.com").unwrap(), None);
        assert_eq!(user.len(), 1);
        assert!(!user[0].builtin, "override must be a plain user def");
        assert!(user[0].linux.is_some(), "override keeps the command");
        assert_eq!(
            user[0].args.get("baidu").map(String::as_str),
            Some("https://www.baidu.com")
        );
    }

    #[test]
    fn remove_arg_errors_and_success() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        set_arg(&mut user, "t", "here", "cd /tmp").unwrap();
        assert!(remove_arg(&mut user, "t", "here").is_ok());
        assert!(user[0].args.is_empty());
        assert_eq!(
            remove_arg(&mut user, "t", "here").unwrap_err(),
            "no named arg \"here\" on t"
        );
        assert_eq!(
            remove_arg(&mut user, "ghost", "here").unwrap_err(),
            "alias not found: ghost"
        );
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
