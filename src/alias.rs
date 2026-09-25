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

/// Set the linux/macos command of alias `name` in the persisted list.
///
/// A blank `macos` mirrors `linux`, the same rule the settings wizard applies
/// to single-platform aliases; both fields end up `Some(...)` so the run path
/// always finds a command. `Err` only when the alias is unknown.
pub fn set_commands(
    user: &mut [AliasDef],
    name: &str,
    linux: &str,
    macos: &str,
) -> Result<(), String> {
    let Some(def) = user.iter_mut().find(|d| d.name == name) else {
        return Err(format!("alias not found: {name}"));
    };
    let macos = if macos.trim().is_empty() {
        linux
    } else {
        macos
    };
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
        }
    }

    #[test]
    fn defaults_are_exactly_br_and_cd_and_resolve_by_name_any_case() {
        let defs = defaults();
        assert_eq!(
            defs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec!["br", "cd"],
            "the seeded defaults are exactly br and cd"
        );
        assert!(defs.iter().all(|d| d.triggers.is_empty()));
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
    }

    #[test]
    fn defaults_carry_concrete_content_not_empty_shells() {
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
        assert!(defs.iter().all(|d| !d.name.is_empty()));
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
    fn set_shortcut_edits_a_seeded_alias_in_place() {
        let mut user = defaults();
        // The seeded `br` already carries concrete registered content
        // (baidu/gm); registering another key edits that same definition.
        assert_eq!(
            set_shortcut(&mut user, "br", "gh", "https://github.com").unwrap(),
            None
        );
        assert_eq!(user.len(), 2, "no duplicate definition appears");
        let br = resolve(&user, "br").expect("br still resolves");
        assert!(br.linux.is_some(), "the seeded command survives");
        assert_eq!(
            br.shortcuts.get("gh").map(String::as_str),
            Some("https://github.com")
        );
        assert_eq!(
            br.shortcuts.get("baidu").map(String::as_str),
            Some("https://www.baidu.com"),
            "the seeded shortcuts stay put"
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
    fn add_trigger_edits_a_seeded_alias_in_place() {
        let mut user = defaults();
        assert!(add_trigger(&mut user, "br", "b").unwrap());
        assert_eq!(user.len(), 2, "no duplicate definition appears");
        let br = resolve(&user, "br").expect("br still resolves");
        assert_eq!(br.name, "br");
        assert_eq!(br.triggers, vec!["b".to_string()]);
        assert!(br.linux.is_some() && br.macos.is_some());
        assert_eq!(
            br.shortcuts.get("baidu").map(String::as_str),
            Some("https://www.baidu.com")
        );
        // Already there: no duplicate, no second entry.
        assert!(!add_trigger(&mut user, "br", "B").unwrap());
        assert_eq!(user.len(), 2);
        assert_eq!(
            resolve(&user, "br").unwrap().triggers,
            vec!["b".to_string()]
        );
    }

    #[test]
    fn add_trigger_rejects_a_word_taken_by_another_alias() {
        let mut user = defaults();
        user.push(user_def("t", Some("echo {input}")));
        add_trigger(&mut user, "t", "tt").unwrap();
        user.push(user_def("u", Some("printf {input}")));

        // "cd" is a seeded name, "tt" a trigger of another alias.
        for taken in ["cd", "CD", "tt", "Tt"] {
            let err = add_trigger(&mut user, "u", taken).unwrap_err();
            assert!(err.contains("already used"), "{err}");
        }
        assert!(user[3].triggers.is_empty(), "nothing was appended");
    }

    #[test]
    fn remove_trigger_reports_when_nothing_was_removed() {
        let mut user = defaults();
        user.push(user_def("t", Some("echo {input}")));
        add_trigger(&mut user, "t", "tt").unwrap();
        add_trigger(&mut user, "t", "t2").unwrap();
        assert!(
            remove_trigger(&mut user, "t", "TT").unwrap(),
            "case-insensitive"
        );
        assert_eq!(user[2].triggers, vec!["t2".to_string()], "one match goes");
        assert!(remove_trigger(&mut user, "t", "t2").unwrap());
        assert!(user[2].triggers.is_empty());
        // Unknown trigger, unknown alias, and a seeded alias without that
        // trigger all report "nothing removed" instead of an error.
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
    fn set_commands_edits_a_seeded_alias_in_place() {
        let mut user = defaults();
        set_commands(&mut user, "br", "xdg-open {input}", "").unwrap();
        assert_eq!(user.len(), 2, "no duplicate definition appears");
        let br = resolve(&user, "br").expect("br still resolves");
        assert_eq!(br.linux.as_deref(), Some("xdg-open {input}"));
        assert_eq!(br.macos.as_deref(), Some("xdg-open {input}"));
        assert_eq!(br.shortcuts.len(), 2, "the seeded shortcuts stay put");
        let err = set_commands(&mut user, "ghost", "a", "b").unwrap_err();
        assert_eq!(err, "alias not found: ghost");
    }

    #[test]
    fn edit_shortcut_rewrites_the_value_in_place() {
        let mut user = defaults();
        edit_shortcut(&mut user, "br", "baidu", "baidu", "https://x.dev").unwrap();
        let br = resolve(&user, "br").expect("br still resolves");
        assert_eq!(
            br.shortcuts.get("baidu").map(String::as_str),
            Some("https://x.dev")
        );
        assert_eq!(br.shortcuts.len(), 2, "the other seeded key stays put");
    }

    #[test]
    fn edit_shortcut_renames_the_key_and_replaces_a_collision() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        set_shortcut(&mut user, "t", "baidu", "https://a").unwrap();
        set_shortcut(&mut user, "t", "gm", "https://b").unwrap();
        edit_shortcut(&mut user, "t", "baidu", "gh", "https://github.com").unwrap();
        let t = resolve(&user, "t").expect("t still resolves");
        assert_eq!(t.shortcuts.get("baidu"), None, "the old key is gone");
        assert_eq!(
            t.shortcuts.get("gh").map(String::as_str),
            Some("https://github.com")
        );
        // A key change onto an existing key replaces its value, like set_shortcut.
        edit_shortcut(&mut user, "t", "gh", "gm", "https://gitlab.com").unwrap();
        let t = resolve(&user, "t").expect("t still resolves");
        assert_eq!(t.shortcuts.len(), 1);
        assert_eq!(
            t.shortcuts.get("gm").map(String::as_str),
            Some("https://gitlab.com")
        );
    }

    #[test]
    fn edit_shortcut_errors_leave_the_list_untouched() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        set_shortcut(&mut user, "t", "here", "cd /tmp").unwrap();
        let before = user.clone();
        assert_eq!(
            edit_shortcut(&mut user, "ghost", "here", "here", "x").unwrap_err(),
            "alias not found: ghost"
        );
        assert_eq!(
            edit_shortcut(&mut user, "t", "nope", "nope", "x").unwrap_err(),
            "no shortcut \"nope\" on t"
        );
        assert_eq!(user, before);
        assert_eq!(user[0].shortcuts.len(), 1);
    }

    #[test]
    fn rename_trigger_renames_in_place_and_same_word_is_a_noop() {
        let mut user = vec![user_def("t", Some("echo {input}"))];
        add_trigger(&mut user, "t", "tt").unwrap();
        add_trigger(&mut user, "t", "t2").unwrap();
        rename_trigger(&mut user, "t", "tt", "tw").unwrap();
        assert_eq!(
            user[0].triggers,
            vec!["tw".to_string(), "t2".to_string()],
            "order is preserved"
        );
        // The same word, and the same word in another case, are both no-ops.
        rename_trigger(&mut user, "t", "tw", "tw").unwrap();
        assert_eq!(user[0].triggers, vec!["tw".to_string(), "t2".to_string()]);
        rename_trigger(&mut user, "t", "tw", "TW").unwrap();
        assert_eq!(user[0].triggers, vec!["TW".to_string(), "t2".to_string()]);
    }

    #[test]
    fn rename_trigger_rejects_bad_words_unknowns_and_collisions() {
        let mut user = defaults();
        user.push(user_def("t", Some("echo {input}")));
        add_trigger(&mut user, "t", "tt").unwrap();
        let before = user.clone();
        assert_eq!(
            rename_trigger(&mut user, "t", "tt", "bad word").unwrap_err(),
            "invalid trigger: bad word"
        );
        assert_eq!(
            rename_trigger(&mut user, "ghost", "tt", "tw").unwrap_err(),
            "alias not found: ghost"
        );
        assert_eq!(
            rename_trigger(&mut user, "t", "nope", "tw").unwrap_err(),
            "trigger not found on t: nope"
        );
        // "cd" is another alias's name (and resolves case-insensitively).
        assert_eq!(
            rename_trigger(&mut user, "t", "tt", "CD").unwrap_err(),
            "trigger \"CD\" already used by cd"
        );
        assert_eq!(user, before, "every refusal leaves the list untouched");
        // A word already on the same alias must not be silently dropped.
        add_trigger(&mut user, "t", "t2").unwrap();
        let before = user.clone();
        assert_eq!(
            rename_trigger(&mut user, "t", "tt", "T2").unwrap_err(),
            "trigger already on t: T2"
        );
        assert_eq!(user, before);
    }

    #[test]
    fn wrong_shortcuts_type_reports_the_accepted_shapes() {
        let err = serde_json::from_str::<AliasDef>(r#"{"name":"t","shortcuts":42}"#).unwrap_err();
        assert!(
            err.to_string().contains("array of trigger words"),
            "expectation is named, got: {err}"
        );
        let err =
            serde_json::from_str::<AliasDef>(r#"{"name":"t","shortcuts":{"k":7}}"#).unwrap_err();
        assert!(
            err.to_string().contains("expected a string"),
            "map values must be strings, got: {err}"
        );
    }
}
