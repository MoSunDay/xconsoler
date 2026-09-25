//! Alias definitions, the seeded defaults, and shortcut/trigger mutation.
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
use std::fmt;

use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::platform::Platform;

/// A single alias definition; plain data, editable and deletable like any
/// other stored alias.
///
/// On disk the keys match the UI vocabulary: `"triggers"` is the list of
/// alternate trigger words and `"shortcuts"` maps a concrete shortcut key to
/// the text it expands to. Legacy stores used `"shortcuts"` for the trigger
/// list and `"args"` for the map; the manual [`Deserialize`] normalizes both
/// forms: a `"triggers"` array wins over a legacy `"shortcuts"` array, a
/// `"shortcuts"` map wins over `"args"`, and a missing list or map is empty.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AliasDef {
    pub name: String,
    /// Alternate trigger words resolving to this alias.
    pub triggers: Vec<String>,
    pub linux: Option<String>,
    pub macos: Option<String>,
    /// Concrete registered shortcuts: typing `<alias> <key> <more…>` maps
    /// `<key>` to this value before the command runs.
    pub shortcuts: BTreeMap<String, String>,
}

/// Wire form for [`AliasDef`] deserialization: every field is optional so any
/// mix of current and legacy keys loads, and the legacy `"builtin"` flag is
/// simply ignored.
#[derive(Deserialize)]
struct RawAliasDef {
    name: String,
    #[serde(default)]
    triggers: Option<Vec<String>>,
    #[serde(default)]
    linux: Option<String>,
    #[serde(default)]
    macos: Option<String>,
    /// Current key: a shortcut-key → execution-argument map. Legacy stores
    /// used this same key for the trigger-word list (an array).
    #[serde(default)]
    shortcuts: Option<RawShortcuts>,
    /// Legacy key: the shortcut-key → execution-argument map.
    #[serde(default)]
    args: Option<BTreeMap<String, String>>,
}

/// `"shortcuts"` is either the legacy trigger-word array or the current
/// key → value map; the JSON shape tells the two apart. Hand-written rather
/// than `#[serde(untagged)]` so a wrong type reports the accepted shapes
/// instead of serde's generic "did not match any variant" message.
#[derive(Debug)]
enum RawShortcuts {
    /// Legacy form: `"shortcuts": ["tt", "tw"]` holds the trigger words.
    Triggers(Vec<String>),
    /// Current form: `"shortcuts": {"key": "value"}` holds the shortcut map.
    Map(BTreeMap<String, String>),
}

impl<'de> Deserialize<'de> for RawShortcuts {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ShortcutsVisitor;

        impl<'de> Visitor<'de> for ShortcutsVisitor {
            type Value = RawShortcuts;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "an array of trigger words or an object mapping shortcut keys to arguments",
                )
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut words = Vec::new();
                while let Some(word) = seq.next_element::<String>()? {
                    words.push(word);
                }
                Ok(RawShortcuts::Triggers(words))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut pairs = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    pairs.insert(key, value);
                }
                Ok(RawShortcuts::Map(pairs))
            }
        }

        deserializer.deserialize_any(ShortcutsVisitor)
    }
}

impl<'de> Deserialize<'de> for AliasDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawAliasDef::deserialize(deserializer)?;
        let (triggers, shortcuts) = match raw.shortcuts {
            Some(RawShortcuts::Map(map)) => (raw.triggers.unwrap_or_default(), map),
            Some(RawShortcuts::Triggers(legacy)) => {
                (raw.triggers.unwrap_or(legacy), raw.args.unwrap_or_default())
            }
            None => (
                raw.triggers.unwrap_or_default(),
                raw.args.unwrap_or_default(),
            ),
        };
        Ok(AliasDef {
            name: raw.name,
            triggers,
            linux: raw.linux,
            macos: raw.macos,
            shortcuts,
        })
    }
}

/// Initial aliases seeded into every fresh store: exactly `br` and `cd`, each
/// carrying its concrete content - the command templates *and* the registered
/// shortcuts - so a fresh machine gets a working `br baidu` without any store
/// copy.
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
        },
        AliasDef {
            name: "cd".to_string(),
            triggers: vec![],
            // Native backend: no xclip/wl-copy/xsel/pbcopy dependency.
            linux: Some(crate::clipboard::TEMPLATE.to_string()),
            macos: Some(crate::clipboard::TEMPLATE.to_string()),
            shortcuts: BTreeMap::new(),
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

/// Set (or replace) shortcut `key` on alias `name` in the persisted alias
/// list. Only `user` is searched: an unknown name is an error. Returns the
/// previous value when the key already existed.
pub fn set_shortcut(
    user: &mut [AliasDef],
    name: &str,
    key: &str,
    value: &str,
) -> Result<Option<String>, String> {
    match user.iter_mut().find(|d| d.name == name) {
        Some(def) => Ok(def.shortcuts.insert(key.to_string(), value.to_string())),
        None => Err(format!("alias not found: {name}")),
    }
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

/// Edit shortcut `old_key` of alias `name` in the persisted list: drop
/// `old_key` when the key changed, then insert `(key, value)` with the same
/// replace semantics as [`set_shortcut`].
///
/// Only `user` is searched: an unknown alias, or one that does not carry
/// `old_key`, is an error and leaves the list untouched.
pub fn edit_shortcut(
    user: &mut [AliasDef],
    name: &str,
    old_key: &str,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let Some(def) = user.iter_mut().find(|d| d.name == name) else {
        return Err(format!("alias not found: {name}"));
    };
    if !def.shortcuts.contains_key(old_key) {
        return Err(format!("no shortcut \"{old_key}\" on {name}"));
    }
    if old_key != key {
        def.shortcuts.remove(old_key);
    }
    def.shortcuts.insert(key.to_string(), value.to_string());
    Ok(())
}

/// Append trigger `trigger` to alias `name` in the persisted alias list.
///
/// The alias must exist in `user`; the trigger must be a [`valid_ident`] and
/// must not already resolve to a *different* alias in that list
/// (case-insensitively). `Ok(true)` when appended, `Ok(false)` when this
/// alias already answers to that word, `Err` on an unknown alias or a
/// malformed/taken word. An error leaves `user` untouched.
pub fn add_trigger(user: &mut [AliasDef], name: &str, trigger: &str) -> Result<bool, String> {
    if !valid_ident(trigger) {
        return Err(format!("invalid trigger: {trigger}"));
    }
    // The alias must exist in the passed list.
    let Some(idx) = user.iter().position(|d| d.name == name) else {
        return Err(format!("alias not found: {name}"));
    };
    // Collision check against the passed list only. An error here must leave
    // `user` untouched.
    if let Some(other) = resolve(user, trigger) {
        if other.name.eq_ignore_ascii_case(name) {
            return Ok(false); // this alias already answers to it
        }
        return Err(format!(
            "trigger \"{trigger}\" already used by {}",
            other.name
        ));
    }
    // `resolve` ignores case, so `Tt` on a `tt` trigger is the same word.
    user[idx].triggers.push(trigger.to_string());
    Ok(true)
}

/// Remove trigger `trigger` from alias `name` in the persisted list,
/// case-insensitively.
///
/// Only the passed list is scanned, like [`remove_shortcut`]; an unknown alias
/// or an alias without that trigger reports `Ok(false)`.
pub fn remove_trigger(user: &mut [AliasDef], name: &str, trigger: &str) -> Result<bool, String> {
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

/// Rename trigger `old` to `new` on alias `name`, in place and preserving
/// order.
///
/// `new == old` is a no-op. Otherwise `new` must be a [`valid_ident`], must
/// not collide with another alias (name or trigger, case-insensitively) and
/// must not duplicate another trigger on the same alias - the word is
/// replaced, never dropped. Every error leaves `user` untouched.
pub fn rename_trigger(
    user: &mut [AliasDef],
    name: &str,
    old: &str,
    new: &str,
) -> Result<(), String> {
    if new == old {
        return Ok(());
    }
    if !valid_ident(new) {
        return Err(format!("invalid trigger: {new}"));
    }
    let Some(idx) = user.iter().position(|d| d.name == name) else {
        return Err(format!("alias not found: {name}"));
    };
    let Some(pos) = user[idx]
        .triggers
        .iter()
        .position(|s| s.eq_ignore_ascii_case(old))
    else {
        return Err(format!("trigger not found on {name}: {old}"));
    };
    // Collision check before any mutation, against every *other* alias.
    if let Some(other) = user
        .iter()
        .enumerate()
        .find(|(i, d)| {
            *i != idx
                && (d.name.eq_ignore_ascii_case(new)
                    || d.triggers.iter().any(|t| t.eq_ignore_ascii_case(new)))
        })
        .map(|(_, d)| d)
    {
        return Err(format!("trigger \"{new}\" already used by {}", other.name));
    }
    // A word already on this alias would be silently dropped: refuse instead.
    // The entry being renamed does not count as a collision.
    if user[idx]
        .triggers
        .iter()
        .enumerate()
        .any(|(i, t)| i != pos && t.eq_ignore_ascii_case(new))
    {
        return Err(format!("trigger already on {name}: {new}"));
    }
    user[idx].triggers[pos] = new.to_string();
    Ok(())
}

/// The command stored for `platform`, if any.
///
/// `None` and whitespace-only both count as "not configured for this
/// platform": the run path falls back to the other platform's command
/// ([`crate::exec::run_alias`]), and the settings table shows `—` instead of
/// leaking the other platform's text.
pub fn platform_command(def: &AliasDef, platform: Platform) -> Option<&str> {
    let command = match platform {
        Platform::Linux => def.linux.as_deref(),
        Platform::Macos => def.macos.as_deref(),
    };
    command.filter(|c| !c.trim().is_empty())
}

/// Set only `platform`'s command of alias `name` to `Some(command)`; the
/// other platform's field is left untouched (hand-edit the store to change
/// it). `Err` only when the alias is unknown.
pub fn set_command(
    user: &mut [AliasDef],
    name: &str,
    platform: Platform,
    command: &str,
) -> Result<(), String> {
    let Some(def) = user.iter_mut().find(|d| d.name == name) else {
        return Err(format!("alias not found: {name}"));
    };
    match platform {
        Platform::Linux => def.linux = Some(command.to_string()),
        Platform::Macos => def.macos = Some(command.to_string()),
    }
    Ok(())
}

#[cfg(test)]
#[path = "alias/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "alias/platform_tests.rs"]
mod platform_tests;
