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

/// Built-in aliases shipped with xconsoler: exactly `br` and `cd`, each an
/// alias, each carrying its concrete content - the command templates *and*
/// the registered named args - so a fresh machine gets a working `br baidu`
/// without any store copy.
pub fn defaults() -> Vec<AliasDef> {
    vec![
        AliasDef {
            name: "br".to_string(),
            shortcuts: vec![],
            // Native launchers, quiet and backgrounded: stray stdout/stderr
            // would scribble over the TUI, and `&` returns the bar at once
            // instead of waiting on the launcher (minutes on some boxes).
            linux: Some("xdg-open {input} >/dev/null 2>&1 &".to_string()),
            macos: Some("open {input} >/dev/null 2>&1 &".to_string()),
            // Concrete registered content, not an empty shell: `br baidu` /
            // `br gm` resolve to these urls before the command runs
            // (see `crate::exec::resolve_args`).
            args: BTreeMap::from([
                ("baidu".to_string(), "https://www.baidu.com".to_string()),
                ("gm".to_string(), "https://mail.google.com".to_string()),
            ]),
            builtin: true,
        },
        AliasDef {
            name: "cd".to_string(),
            shortcuts: vec![],
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

/// Human label such as `t (tt)`; just the name when no shortcut exists.
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
/// user definition first — that is how `:arg br k v` persists. Returns
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

/// Append shortcut `shortcut` to user alias `name`.
///
/// Follows [`set_arg`]: a built-in name is materialised into the user list
/// first (clone of the default, `builtin = false`, registered args kept), so
/// built-ins can gain quick-launch entries too. The shortcut must be a
/// [`valid_ident`] and must not already resolve to a *different* alias -
/// names and shortcuts collide case-insensitively, the way [`resolve`]
/// matches them. `Ok(true)` when appended, `Ok(false)` when this alias
/// already answers to that word, `Err` when the alias is unknown or its new
/// word is malformed or taken.
pub fn add_shortcut(user: &mut Vec<AliasDef>, name: &str, shortcut: &str) -> Result<bool, String> {
    if !valid_ident(shortcut) {
        return Err(format!("invalid shortcut: {shortcut}"));
    }
    // The alias must exist: a user def, or a built-in to materialise below.
    if !user.iter().any(|d| d.name == name) && !defaults().iter().any(|d| d.name == name) {
        return Err(format!("alias not found: {name}"));
    }
    // Collision check against the effective list (user defs shadow same-named
    // built-ins). An error here must leave `user` untouched.
    let effective = crate::storage::merge_aliases(user);
    if let Some(other) = resolve(&effective, shortcut) {
        if other.name.eq_ignore_ascii_case(name) {
            return Ok(false); // this alias already answers to it
        }
        return Err(format!(
            "shortcut \"{shortcut}\" already used by {}",
            other.name
        ));
    }

    // Materialise a built-in override, exactly like `set_arg` does.
    let idx = match user.iter().position(|d| d.name == name) {
        Some(i) => i,
        None => match defaults().into_iter().find(|d| d.name == name) {
            Some(mut def) => {
                def.builtin = false;
                user.push(def);
                user.len() - 1
            }
            None => return Err(format!("alias not found: {name}")),
        },
    };
    let def = &mut user[idx];
    // `resolve` ignores case, so `Tt` on a `tt` shortcut is the same word.
    if def
        .shortcuts
        .iter()
        .any(|s| s.eq_ignore_ascii_case(shortcut))
    {
        return Ok(false);
    }
    def.shortcuts.push(shortcut.to_string());
    Ok(true)
}

/// Remove shortcut `shortcut` from user alias `name`, case-insensitively.
///
/// Only the user list is scanned, like [`remove_arg`]: a built-in keeps its
/// own fixed names until an override materialises it. `Ok(true)` when a
/// shortcut was removed, `Ok(false)` when the alias or the shortcut is
/// unknown to the user list.
// `&mut Vec` (not `&mut [_]`) keeps the three editing helpers on one
// signature; only `add_shortcut`/`set_commands` actually push.
#[allow(clippy::ptr_arg)]
pub fn remove_shortcut(
    user: &mut Vec<AliasDef>,
    name: &str,
    shortcut: &str,
) -> Result<bool, String> {
    match user.iter_mut().find(|d| d.name == name) {
        Some(def) => match def
            .shortcuts
            .iter()
            .position(|s| s.eq_ignore_ascii_case(shortcut))
        {
            Some(i) => {
                def.shortcuts.remove(i);
                Ok(true)
            }
            None => Ok(false),
        },
        None => Ok(false),
    }
}

/// Set the linux/macos command of user alias `name`.
///
/// A built-in name is materialised into the user list first (clone of the
/// default, `builtin = false`, registered args kept). A blank `macos` mirrors
/// `linux`, the same rule the settings wizard applies to single-platform
/// aliases; both fields end up `Some(...)` so the run path always finds a
/// command. `Err` only when the alias is unknown.
pub fn set_commands(
    user: &mut Vec<AliasDef>,
    name: &str,
    linux: &str,
    macos: &str,
) -> Result<(), String> {
    let idx = match user.iter().position(|d| d.name == name) {
        Some(i) => i,
        None => match defaults().into_iter().find(|d| d.name == name) {
            Some(mut def) => {
                def.builtin = false;
                user.push(def);
                user.len() - 1
            }
            None => return Err(format!("alias not found: {name}")),
        },
    };
    let macos = if macos.trim().is_empty() {
        linux
    } else {
        macos
    };
    let def = &mut user[idx];
    def.linux = Some(linux.to_string());
    def.macos = Some(macos.to_string());
    Ok(())
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
    fn defaults_have_no_shortcuts_and_resolve_by_name_any_case() {
        let defs = defaults();
        assert_eq!(
            defs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec!["br", "cd"],
            "builtins are exactly br and cd"
        );
        assert!(defs.iter().all(|d| d.shortcuts.is_empty() && d.builtin));
        let br = resolve(&defs, "br").expect("br resolves");
        assert_eq!(br.name, "br");
        assert!(resolve(&defs, "BR").is_some());
        assert!(resolve(&defs, "Cd").is_some());
        assert!(resolve(&defs, "nope").is_none());
        assert!(resolve(&defs, "browser").is_none());
        assert!(resolve(&defs, "clipboard").is_none());
    }

    #[test]
    fn br_default_opens_quietly_in_the_background() {
        let defs = defaults();
        let br = resolve(&defs, "br").expect("br resolves");
        assert_eq!(
            br.linux.as_deref(),
            Some("xdg-open {input} >/dev/null 2>&1 &"),
            "linux opens through xdg-open, quiet and backgrounded"
        );
        assert_eq!(
            br.macos.as_deref(),
            Some("open {input} >/dev/null 2>&1 &"),
            "macos opens through open, quiet and backgrounded"
        );
    }

    #[test]
    fn cd_default_uses_native_backend() {
        let defs = defaults();
        let cd = resolve(&defs, "cd").expect("cd resolves");
        assert_eq!(cd.name, "cd");
        assert_eq!(cd.linux.as_deref(), Some(crate::clipboard::TEMPLATE));
        assert_eq!(cd.macos.as_deref(), Some(crate::clipboard::TEMPLATE));
        assert!(cd.builtin);
    }

    #[test]
    fn builtins_carry_concrete_content_not_empty_shells() {
        let defs = defaults();
        let br = resolve(&defs, "br").expect("br resolves");
        assert_eq!(
            br.args.get("baidu").map(String::as_str),
            Some("https://www.baidu.com")
        );
        assert_eq!(
            br.args.get("gm").map(String::as_str),
            Some("https://mail.google.com")
        );
        // The registered args are wired into the run path.
        assert_eq!(
            crate::exec::resolve_args(br, "baidu"),
            "https://www.baidu.com"
        );
        assert_eq!(
            crate::exec::resolve_args(br, "baidu ?q=1"),
            "https://www.baidu.com ?q=1"
        );
        assert_eq!(
            crate::exec::resolve_args(br, "https://x.dev"),
            "https://x.dev"
        );
        let cd = resolve(&defs, "cd").expect("cd resolves");
        assert!(cd.linux.as_deref().is_some_and(|c| !c.trim().is_empty()));
        assert!(cd.macos.as_deref().is_some_and(|c| !c.trim().is_empty()));
        assert!(defs.iter().all(|d| d.builtin && !d.name.is_empty()));
    }

    #[test]
    fn labels_use_first_shortcut_when_present() {
        let defs = defaults();
        let br = resolve(&defs, "br").unwrap();
        assert_eq!(label(br), "br");
        assert_eq!(entry_label(br), "br");

        let mut d = br.clone();
        d.shortcuts = vec!["b".to_string()];
        assert_eq!(label(&d), "br (b)");
        assert_eq!(entry_label(&d), "b");
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
        match parse_colon_cmd("del t").unwrap().unwrap() {
            AliasOp::Remove(name) => assert_eq!(name, "t"),
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
        match parse_colon_cmd("arg br baidu https://www.baidu.com")
            .unwrap()
            .unwrap()
        {
            AliasOp::SetArg { name, key, value } => {
                assert_eq!(name, "br");
                assert_eq!(key, "baidu");
                assert_eq!(value, "https://www.baidu.com");
            }
            other => panic!("expected SetArg, got {other:?}"),
        }
        // the value is the raw remainder: internal spacing survives
        match parse_colon_cmd("arg  t   here   cd /tmp  &&   ls ")
            .unwrap()
            .unwrap()
        {
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
        assert_eq!(
            set_arg(&mut user, "t", "here", "cd /var").unwrap(),
            Some("cd /tmp".into())
        );
        assert_eq!(
            user[0].args.get("here").map(String::as_str),
            Some("cd /var")
        );
        assert_eq!(
            set_arg(&mut user, "nope", "k", "v").unwrap_err(),
            "alias not found: nope"
        );
    }

    #[test]
    fn set_arg_on_builtin_materializes_user_override() {
        let mut user = vec![];
        // A new key on a builtin materializes a user override that keeps the
        // builtin's own registered content (br ships with baidu/gm already).
        assert_eq!(
            set_arg(&mut user, "br", "gh", "https://github.com").unwrap(),
            None
        );
        assert_eq!(user.len(), 1);
        assert!(!user[0].builtin, "override must be a plain user def");
        assert!(user[0].linux.is_some(), "override keeps the command");
        assert_eq!(
            user[0].args.get("gh").map(String::as_str),
            Some("https://github.com")
        );
        assert_eq!(
            user[0].args.get("baidu").map(String::as_str),
            Some("https://www.baidu.com"),
            "builtin args carry over into the override"
        );
        // Re-registering an existing key reports the previous value.
        assert_eq!(
            set_arg(&mut user, "br", "gh", "https://gitlab.com").unwrap(),
            Some("https://github.com".to_string())
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
    fn add_shortcut_on_user_alias_is_idempotent() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        assert!(add_shortcut(&mut user, "t", "tt").unwrap());
        assert_eq!(user[0].shortcuts, vec!["tt".to_string()]);
        // Case-insensitive: the alias already answers to "tt" (and to "t").
        assert!(!add_shortcut(&mut user, "t", "TT").unwrap());
        assert!(!add_shortcut(&mut user, "t", "T").unwrap());
        assert_eq!(user[0].shortcuts, vec!["tt".to_string()]);

        let err = add_shortcut(&mut user, "ghost", "g").unwrap_err();
        assert_eq!(err, "alias not found: ghost");
        assert_eq!(
            add_shortcut(&mut user, "t", "bad!").unwrap_err(),
            "invalid shortcut: bad!"
        );
        assert_eq!(user.len(), 1, "errors never touch the list");
        assert_eq!(user[0].shortcuts, vec!["tt".to_string()]);
    }

    #[test]
    fn add_shortcut_on_builtin_materializes_user_override() {
        let mut user = vec![];
        assert!(add_shortcut(&mut user, "br", "b").unwrap());
        assert_eq!(user.len(), 1);
        assert!(!user[0].builtin, "override must be a plain user def");
        assert_eq!(user[0].shortcuts, vec!["b".to_string()]);
        assert!(user[0].linux.is_some() && user[0].macos.is_some());
        assert_eq!(
            user[0].args.get("baidu").map(String::as_str),
            Some("https://www.baidu.com")
        );
        // Already there: no duplicate, no second override.
        assert!(!add_shortcut(&mut user, "br", "B").unwrap());
        assert_eq!(user.len(), 1);
        assert_eq!(user[0].shortcuts, vec!["b".to_string()]);
    }

    #[test]
    fn add_shortcut_rejects_a_word_taken_by_another_alias() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        add_shortcut(&mut user, "t", "tt").unwrap();
        user.push(user_def("u", Some("printf {input}")));

        // "cd" is a built-in name, "tt" a shortcut of another user alias.
        for taken in ["cd", "CD", "tt", "Tt"] {
            let err = add_shortcut(&mut user, "u", taken).unwrap_err();
            assert!(err.contains("already used"), "{err}");
        }
        assert!(user[1].shortcuts.is_empty(), "nothing was appended");
    }

    #[test]
    fn remove_shortcut_only_scans_the_user_list() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        add_shortcut(&mut user, "t", "tt").unwrap();
        add_shortcut(&mut user, "t", "t2").unwrap();
        assert!(
            remove_shortcut(&mut user, "t", "TT").unwrap(),
            "case-insensitive"
        );
        assert_eq!(user[0].shortcuts, vec!["t2".to_string()], "one match goes");
        assert!(remove_shortcut(&mut user, "t", "t2").unwrap());
        assert!(user[0].shortcuts.is_empty());
        // Unknown shortcut, unknown alias, and an un-materialised builtin all
        // report "nothing removed" instead of an error.
        assert!(!remove_shortcut(&mut user, "t", "tt").unwrap());
        assert!(!remove_shortcut(&mut user, "ghost", "tt").unwrap());
        assert!(!remove_shortcut(&mut user, "br", "b").unwrap());
    }

    #[test]
    fn set_commands_mirrors_blank_macos() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        set_commands(&mut user, "t", "printf %s {input}", "").unwrap();
        assert_eq!(user[0].linux.as_deref(), Some("printf %s {input}"));
        assert_eq!(user[0].macos.as_deref(), Some("printf %s {input}"));
        set_commands(&mut user, "t", "printf %s {input}", "   ").unwrap();
        assert_eq!(user[0].macos.as_deref(), Some("printf %s {input}"));
        set_commands(&mut user, "t", "xdg-open {input}", "open {input}").unwrap();
        assert_eq!(user[0].linux.as_deref(), Some("xdg-open {input}"));
        assert_eq!(user[0].macos.as_deref(), Some("open {input}"));
        assert_eq!(user.len(), 1);
    }

    #[test]
    fn set_commands_on_builtin_materializes_user_override() {
        let mut user = vec![];
        set_commands(&mut user, "br", "xdg-open {input}", "").unwrap();
        assert_eq!(user.len(), 1);
        assert!(!user[0].builtin, "override must be a plain user def");
        assert_eq!(user[0].linux.as_deref(), Some("xdg-open {input}"));
        assert_eq!(user[0].macos.as_deref(), Some("xdg-open {input}"));
        assert_eq!(user[0].args.len(), 2, "registered args carry over");
        let err = set_commands(&mut user, "ghost", "a", "b").unwrap_err();
        assert_eq!(err, "alias not found: ghost");
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
