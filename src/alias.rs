//! Alias definitions, built-in defaults, and shortcut/trigger mutation.
//!
//! Colon-command parsing lives in [`crate::colon`].
//!
//! Command template conventions:
//!
//! * `{input}` - placeholder for the user's input; it is shell-quoted
//!   (see [`crate::exec::shell_quote`]) before being substituted. Registered
//!   shortcuts are resolved into the input first
//!   (see [`crate::exec::resolve_shortcuts`]).
//! * `@stdin` - marker meaning the input is delivered through the child
//!   process stdin; the marker is removed from the command string before
//!   execution (see [`crate::exec::run_alias`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A single alias definition (built-in or user defined).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AliasDef {
    pub name: String,
    /// Alternate trigger words resolving to this alias (JSON key `"shortcuts"`, legacy name).
    #[serde(rename = "shortcuts")]
    pub triggers: Vec<String>,
    pub linux: Option<String>,
    pub macos: Option<String>,
    /// Concrete registered shortcuts: typing `<alias> <key> <more…>` maps
    /// `<key>` to this value before the command runs (JSON key `"args"`, kept
    /// for store compatibility; `#[serde(default)]` keeps older stores loadable).
    #[serde(rename = "args", default)]
    pub shortcuts: BTreeMap<String, String>,
    pub builtin: bool,
}

/// Built-in aliases shipped with xconsoler: exactly `br` and `cd`, each an alias carrying its
/// concrete content - the command templates *and* the registered shortcuts - so a fresh machine
/// gets a working `br baidu` without any store copy.
pub fn defaults() -> Vec<AliasDef> {
    vec![
        AliasDef {
            name: "br".to_string(),
            triggers: vec![],
            // Native launchers, quiet and backgrounded: stray stdout/stderr
            // would scribble over the TUI, and `&` returns the bar at once
            // instead of waiting on the launcher (minutes on some boxes).
            linux: Some("xdg-open {input} >/dev/null 2>&1 &".to_string()),
            macos: Some("open {input} >/dev/null 2>&1 &".to_string()),
            // Concrete registered shortcuts, not an empty shell: `br baidu`
            // resolves to these urls (see `crate::exec::resolve_shortcuts`).
            shortcuts: BTreeMap::from([
                ("baidu".to_string(), "https://www.baidu.com".to_string()),
                ("gm".to_string(), "https://mail.google.com".to_string()),
            ]),
            builtin: true,
        },
        AliasDef {
            name: "cd".to_string(),
            triggers: vec![],
            // Native backend: no xclip/wl-copy/xsel/pbcopy dependency.
            linux: Some(crate::clipboard::TEMPLATE.to_string()),
            macos: Some(crate::clipboard::TEMPLATE.to_string()),
            shortcuts: BTreeMap::new(),
            builtin: true,
        },
    ]
}

/// Resolve a token (full name or trigger) to a definition, case-insensitively.
pub fn resolve<'a>(defs: &'a [AliasDef], token: &str) -> Option<&'a AliasDef> {
    let token = token.to_lowercase();
    defs.iter().find(|d| {
        d.name.to_lowercase() == token || d.triggers.iter().any(|s| s.to_lowercase() == token)
    })
}

/// Human label such as `t (tt)`; just the name when no trigger exists.
pub fn label(def: &AliasDef) -> String {
    if def.triggers.is_empty() {
        def.name.clone()
    } else {
        format!("{} ({})", def.name, def.triggers.join(", "))
    }
}

/// First trigger, or the name; used when rendering history entries.
pub fn entry_label(def: &AliasDef) -> &str {
    match def.triggers.first() {
        Some(s) => s.as_str(),
        None => def.name.as_str(),
    }
}

/// A valid alias name, trigger, or shortcut key: non-empty ASCII letters,
/// digits, `-` and `_` (no spaces - the run path splits on whitespace).
pub fn valid_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Set (or replace) shortcut `key` on user alias `name` in the persisted
/// alias list. A built-in name is materialized as an overriding user
/// definition first — that is how `:arg br k v` persists. Returns the
/// previous value when the key already existed. `Err` for unknown names.
pub fn set_shortcut(
    user: &mut Vec<AliasDef>,
    name: &str,
    key: &str,
    value: &str,
) -> Result<Option<String>, String> {
    if let Some(def) = user.iter_mut().find(|d| d.name == name) {
        return Ok(def.shortcuts.insert(key.to_string(), value.to_string()));
    }
    if let Some(mut def) = defaults().into_iter().find(|d| d.name == name) {
        def.builtin = false;
        let prev = def.shortcuts.insert(key.to_string(), value.to_string());
        user.push(def);
        return Ok(prev);
    }
    Err(format!("alias not found: {name}"))
}

/// Remove shortcut `key` from user alias `name`. `Err` when the alias is
/// unknown or does not carry that key.
pub fn remove_shortcut(user: &mut [AliasDef], name: &str, key: &str) -> Result<(), String> {
    match user.iter_mut().find(|d| d.name == name) {
        Some(def) => match def.shortcuts.remove(key) {
            Some(_) => Ok(()),
            None => Err(format!("no shortcut \"{key}\" on {name}")),
        },
        None => Err(format!("alias not found: {name}")),
    }
}

/// Append trigger `trigger` to user alias `name`.
///
/// Follows [`set_shortcut`]: a built-in name is materialised into the user
/// list first (registered shortcuts kept), so built-ins can gain alternate
/// words too. The trigger must be a [`valid_ident`] and must not already
/// resolve to a *different* alias. `Ok(true)` when appended, `Ok(false)`
/// when this alias already answers to that word, `Err` on an unknown alias
/// or a malformed/taken word.
pub fn add_trigger(user: &mut Vec<AliasDef>, name: &str, trigger: &str) -> Result<bool, String> {
    if !valid_ident(trigger) {
        return Err(format!("invalid trigger: {trigger}"));
    }
    // The alias must exist: a user def, or a built-in to materialise below.
    if !user.iter().any(|d| d.name == name) && !defaults().iter().any(|d| d.name == name) {
        return Err(format!("alias not found: {name}"));
    }
    // Collision check against the effective list (user defs shadow same-named
    // built-ins). An error here must leave `user` untouched.
    let effective = crate::storage::merge_aliases(user);
    if let Some(other) = resolve(&effective, trigger) {
        if other.name.eq_ignore_ascii_case(name) {
            return Ok(false); // this alias already answers to it
        }
        return Err(format!(
            "trigger \"{trigger}\" already used by {}",
            other.name
        ));
    }

    // Materialise a built-in override, exactly like `set_shortcut` does.
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
    // `resolve` ignores case, so `Tt` on a `tt` trigger is the same word.
    if def.triggers.iter().any(|s| s.eq_ignore_ascii_case(trigger)) {
        return Ok(false);
    }
    def.triggers.push(trigger.to_string());
    Ok(true)
}

/// Remove trigger `trigger` from user alias `name`, case-insensitively.
///
/// Only the user list is scanned, like [`remove_shortcut`]: a built-in keeps
/// its own fixed names until an override materialises it.
// `&mut Vec` (not `&mut [_]`): only `add_trigger`/`set_commands` push.
#[allow(clippy::ptr_arg)]
pub fn remove_trigger(user: &mut Vec<AliasDef>, name: &str, trigger: &str) -> Result<bool, String> {
    match user.iter_mut().find(|d| d.name == name) {
        Some(def) => match def
            .triggers
            .iter()
            .position(|s| s.eq_ignore_ascii_case(trigger))
        {
            Some(i) => {
                def.triggers.remove(i);
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
/// default, `builtin = false`, registered shortcuts kept). A blank `macos` mirrors
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
            triggers: vec![],
            linux: linux.map(|s| s.to_string()),
            macos: None,
            shortcuts: BTreeMap::new(),
            builtin: false,
        }
    }

    #[test]
    fn defaults_have_no_triggers_and_resolve_by_name_any_case() {
        let defs = defaults();
        assert_eq!(
            defs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec!["br", "cd"],
            "builtins are exactly br and cd"
        );
        assert!(defs.iter().all(|d| d.triggers.is_empty() && d.builtin));
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
            br.shortcuts.get("baidu").map(String::as_str),
            Some("https://www.baidu.com")
        );
        assert_eq!(
            br.shortcuts.get("gm").map(String::as_str),
            Some("https://mail.google.com")
        );
        // The registered shortcuts are wired into the run path.
        assert_eq!(
            crate::exec::resolve_shortcuts(br, "baidu"),
            "https://www.baidu.com"
        );
        assert_eq!(
            crate::exec::resolve_shortcuts(br, "baidu ?q=1"),
            "https://www.baidu.com ?q=1"
        );
        assert_eq!(
            crate::exec::resolve_shortcuts(br, "https://x.dev"),
            "https://x.dev"
        );
        let cd = resolve(&defs, "cd").expect("cd resolves");
        assert!(cd.linux.as_deref().is_some_and(|c| !c.trim().is_empty()));
        assert!(cd.macos.as_deref().is_some_and(|c| !c.trim().is_empty()));
        assert!(defs.iter().all(|d| d.builtin && !d.name.is_empty()));
    }

    #[test]
    fn labels_use_first_trigger_when_present() {
        let defs = defaults();
        let br = resolve(&defs, "br").unwrap();
        assert_eq!(label(br), "br");
        assert_eq!(entry_label(br), "br");

        let mut d = br.clone();
        d.triggers = vec!["b".to_string()];
        assert_eq!(label(&d), "br (b)");
        assert_eq!(entry_label(&d), "b");
    }

    #[test]
    fn set_shortcut_on_user_alias_and_replacement() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        assert_eq!(
            set_shortcut(&mut user, "t", "here", "cd /tmp").unwrap(),
            None
        );
        assert_eq!(
            set_shortcut(&mut user, "t", "here", "cd /var").unwrap(),
            Some("cd /tmp".into())
        );
        assert_eq!(
            user[0].shortcuts.get("here").map(String::as_str),
            Some("cd /var")
        );
        assert_eq!(
            set_shortcut(&mut user, "nope", "k", "v").unwrap_err(),
            "alias not found: nope"
        );
    }

    #[test]
    fn set_shortcut_on_builtin_materializes_user_override() {
        let mut user = vec![];
        // A new key on a builtin materializes a user override that keeps the
        // builtin's own registered content (br ships with baidu/gm already).
        assert_eq!(
            set_shortcut(&mut user, "br", "gh", "https://github.com").unwrap(),
            None
        );
        assert_eq!(user.len(), 1);
        assert!(!user[0].builtin, "override must be a plain user def");
        assert!(user[0].linux.is_some(), "override keeps the command");
        assert_eq!(
            user[0].shortcuts.get("gh").map(String::as_str),
            Some("https://github.com")
        );
        assert_eq!(
            user[0].shortcuts.get("baidu").map(String::as_str),
            Some("https://www.baidu.com"),
            "builtin args carry over into the override"
        );
        // Re-registering an existing key reports the previous value.
        assert_eq!(
            set_shortcut(&mut user, "br", "gh", "https://gitlab.com").unwrap(),
            Some("https://github.com".to_string())
        );
    }

    #[test]
    fn remove_shortcut_errors_and_success() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        set_shortcut(&mut user, "t", "here", "cd /tmp").unwrap();
        assert!(remove_shortcut(&mut user, "t", "here").is_ok());
        assert!(user[0].shortcuts.is_empty());
        assert_eq!(
            remove_shortcut(&mut user, "t", "here").unwrap_err(),
            "no shortcut \"here\" on t"
        );
        assert_eq!(
            remove_shortcut(&mut user, "ghost", "here").unwrap_err(),
            "alias not found: ghost"
        );
    }

    #[test]
    fn add_trigger_on_user_alias_is_idempotent() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        assert!(add_trigger(&mut user, "t", "tt").unwrap());
        assert_eq!(user[0].triggers, vec!["tt".to_string()]);
        // Case-insensitive: the alias already answers to "tt" (and to "t").
        assert!(!add_trigger(&mut user, "t", "TT").unwrap());
        assert!(!add_trigger(&mut user, "t", "T").unwrap());
        assert_eq!(user[0].triggers, vec!["tt".to_string()]);

        let err = add_trigger(&mut user, "ghost", "g").unwrap_err();
        assert_eq!(err, "alias not found: ghost");
        assert_eq!(
            add_trigger(&mut user, "t", "bad!").unwrap_err(),
            "invalid trigger: bad!"
        );
        assert_eq!(user.len(), 1, "errors never touch the list");
        assert_eq!(user[0].triggers, vec!["tt".to_string()]);
    }

    #[test]
    fn add_trigger_on_builtin_materializes_user_override() {
        let mut user = vec![];
        assert!(add_trigger(&mut user, "br", "b").unwrap());
        assert_eq!(user.len(), 1);
        assert!(!user[0].builtin, "override must be a plain user def");
        assert_eq!(user[0].triggers, vec!["b".to_string()]);
        assert!(user[0].linux.is_some() && user[0].macos.is_some());
        assert_eq!(
            user[0].shortcuts.get("baidu").map(String::as_str),
            Some("https://www.baidu.com")
        );
        // Already there: no duplicate, no second override.
        assert!(!add_trigger(&mut user, "br", "B").unwrap());
        assert_eq!(user.len(), 1);
        assert_eq!(user[0].triggers, vec!["b".to_string()]);
    }

    #[test]
    fn add_trigger_rejects_a_word_taken_by_another_alias() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        add_trigger(&mut user, "t", "tt").unwrap();
        user.push(user_def("u", Some("printf {input}")));

        // "cd" is a built-in name, "tt" a shortcut of another user alias.
        for taken in ["cd", "CD", "tt", "Tt"] {
            let err = add_trigger(&mut user, "u", taken).unwrap_err();
            assert!(err.contains("already used"), "{err}");
        }
        assert!(user[1].triggers.is_empty(), "nothing was appended");
    }

    #[test]
    fn remove_trigger_only_scans_the_user_list() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        add_trigger(&mut user, "t", "tt").unwrap();
        add_trigger(&mut user, "t", "t2").unwrap();
        assert!(
            remove_trigger(&mut user, "t", "TT").unwrap(),
            "case-insensitive"
        );
        assert_eq!(user[0].triggers, vec!["t2".to_string()], "one match goes");
        assert!(remove_trigger(&mut user, "t", "t2").unwrap());
        assert!(user[0].triggers.is_empty());
        // Unknown shortcut, unknown alias, and an un-materialised builtin all
        // report "nothing removed" instead of an error.
        assert!(!remove_trigger(&mut user, "t", "tt").unwrap());
        assert!(!remove_trigger(&mut user, "ghost", "tt").unwrap());
        assert!(!remove_trigger(&mut user, "br", "b").unwrap());
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
        assert_eq!(
            user[0].shortcuts.len(),
            2,
            "registered shortcuts carry over"
        );
        let err = set_commands(&mut user, "ghost", "a", "b").unwrap_err();
        assert_eq!(err, "alias not found: ghost");
    }
}
