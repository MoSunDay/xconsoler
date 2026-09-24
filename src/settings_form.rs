//! Wizard form for the `/settings` page: collects a new alias (name →
//! shortcuts → linux command → macos command) or a new named argument
//! (key → value) from a single bottom input line, validating each field on
//! Enter before advancing. Pure data + free functions; the store is only
//! read (duplicate-name checks) — submissions are applied by
//! [`crate::settings`].

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::alias::{self, AliasDef};
use crate::storage::Store;

/// What the wizard is collecting.
#[derive(Debug, Clone, PartialEq)]
pub enum Purpose {
    NewAlias,
    NewArg { alias: String },
}

/// A finished wizard run, ready to be applied to the store.
#[derive(Debug, Clone, PartialEq)]
pub enum Submission {
    Alias(AliasDef),
    Arg { alias: String, key: String, value: String },
}

/// Form state: answers before `step` are final, `input` is the field being
/// edited, `error` is the inline validation message.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub purpose: Purpose,
    pub step: usize,
    pub name: String,
    pub shortcuts: String,
    pub linux: String,
    pub macos: String,
    pub arg_key: String,
    pub arg_value: String,
    pub input: String,
    pub error: Option<String>,
}

/// Outcome of one key transition on the form.
#[derive(Debug, Clone, PartialEq)]
pub enum FormOutcome {
    /// Stay on the form (input / step / error may have changed).
    Active,
    /// Esc: discard the draft, back to the list.
    Cancel,
    /// The wizard finished; apply this submission.
    Submit(Submission),
    /// Ctrl+C.
    Quit,
}

/// Start the new-alias wizard (step 0 = name).
pub fn new_alias() -> Form {
    Form {
        purpose: Purpose::NewAlias,
        step: 0,
        name: String::new(),
        shortcuts: String::new(),
        linux: String::new(),
        macos: String::new(),
        arg_key: String::new(),
        arg_value: String::new(),
        input: String::new(),
        error: None,
    }
}

/// Start the add-argument wizard for `alias` (step 0 = key).
pub fn new_arg(alias: &str) -> Form {
    Form {
        purpose: Purpose::NewArg {
            alias: alias.to_string(),
        },
        step: 0,
        ..new_alias()
    }
}

/// Number of wizard steps for the form's purpose.
pub fn step_count(f: &Form) -> usize {
    match f.purpose {
        Purpose::NewAlias => 4,
        Purpose::NewArg { .. } => 2,
    }
}

/// One key transition: `(new form, outcome)`. Only `Press` events count.
pub fn handle_key(f: &Form, store: &Store, key: KeyEvent) -> (Form, FormOutcome) {
    if key.kind != KeyEventKind::Press {
        return (f.clone(), FormOutcome::Active);
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let mut next = f.clone();
    match key.code {
        KeyCode::Char('c') if ctrl => (next, FormOutcome::Quit),
        KeyCode::Esc => (next, FormOutcome::Cancel),
        KeyCode::Enter => advance(f, store),
        KeyCode::Backspace if key.modifiers.is_empty() => {
            next.input.pop();
            (next, FormOutcome::Active)
        }
        KeyCode::Char('u') if ctrl => {
            next.input.clear();
            (next, FormOutcome::Active)
        }
        KeyCode::Char(c) if !ctrl && !alt => {
            next.input.push(c);
            (next, FormOutcome::Active)
        }
        _ => (next, FormOutcome::Active),
    }
}

/// Enter: validate the current field; on success store it and advance (or
/// submit when it was the last step). Errors stay inline.
fn advance(f: &Form, store: &Store) -> (Form, FormOutcome) {
    let mut next = f.clone();
    next.input = f.input.trim().to_string();
    match validate_step(f, store) {
        Err(e) => {
            next.error = Some(e);
            (next, FormOutcome::Active)
        }
        Ok(()) => {
            next.error = None;
            let value = next.input.clone();
            store_field(&mut next, &value);
            next.input.clear();
            next.step += 1;
            if next.step >= step_count(f) {
                (next.clone(), FormOutcome::Submit(build_submission(&next)))
            } else {
                (next, FormOutcome::Active)
            }
        }
    }
}

fn validate_step(f: &Form, store: &Store) -> Result<(), String> {
    let value = f.input.trim();
    match (&f.purpose, f.step) {
        (Purpose::NewAlias, 0) => {
            if value.is_empty() {
                Err("name cannot be empty".to_string())
            } else if !alias::valid_ident(value) {
                Err(format!("invalid name: {value} (a-z 0-9 - _)"))
            } else if alias::resolve(&crate::storage::merge_aliases(&store.aliases), value)
                .is_some()
            {
                Err(format!("name already in use: {value}"))
            } else {
                Ok(())
            }
        }
        (Purpose::NewAlias, 1) => {
            for sc in value.split(',') {
                let sc = sc.trim();
                if sc.is_empty() {
                    continue; // "a,,b" and a trailing comma are tolerated
                }
                if !alias::valid_ident(sc) {
                    return Err(format!("invalid shortcut: {sc}"));
                }
            }
            Ok(())
        }
        (Purpose::NewAlias, 2) => {
            if value.is_empty() {
                Err("linux command cannot be empty".to_string())
            } else {
                Ok(())
            }
        }
        (Purpose::NewAlias, 3) => Ok(()), // empty = same as linux
        (Purpose::NewArg { .. }, 0) => {
            if value.is_empty() {
                Err("arg key cannot be empty".to_string())
            } else if value.contains(char::is_whitespace) {
                Err("arg key must be one word".to_string())
            } else {
                Ok(())
            }
        }
        (Purpose::NewArg { .. }, 1) => {
            if value.is_empty() {
                Err("arg value cannot be empty".to_string())
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

fn store_field(f: &mut Form, value: &str) {
    match (&f.purpose, f.step) {
        (Purpose::NewAlias, 0) => f.name = value.to_string(),
        (Purpose::NewAlias, 1) => f.shortcuts = value.to_string(),
        (Purpose::NewAlias, 2) => f.linux = value.to_string(),
        (Purpose::NewAlias, 3) => f.macos = value.to_string(),
        (Purpose::NewArg { .. }, 0) => f.arg_key = value.to_string(),
        (Purpose::NewArg { .. }, 1) => f.arg_value = value.to_string(),
        _ => {}
    }
}

fn build_submission(f: &Form) -> Submission {
    match &f.purpose {
        Purpose::NewAlias => {
            let shortcuts = f
                .shortcuts
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            // Empty macos answer = mirror linux (single-platform aliases).
            let macos = if f.macos.is_empty() {
                f.linux.clone()
            } else {
                f.macos.clone()
            };
            Submission::Alias(AliasDef {
                name: f.name.clone(),
                shortcuts,
                linux: Some(f.linux.clone()),
                macos: Some(macos),
                args: BTreeMap::new(),
                builtin: false,
            })
        }
        Purpose::NewArg { alias } => Submission::Arg {
            alias: alias.clone(),
            key: f.arg_key.clone(),
            value: f.arg_value.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_str(f: &Form, s: &str, store: &Store) -> Form {
        let mut f = f.clone();
        for c in s.chars() {
            let (nf, out) = handle_key(&f, store, key(KeyCode::Char(c)));
            assert_eq!(out, FormOutcome::Active);
            f = nf;
        }
        f
    }

    fn enter(f: &Form, store: &Store) -> (Form, FormOutcome) {
        handle_key(f, store, key(KeyCode::Enter))
    }

    fn empty_store() -> Store {
        Store::default()
    }

    #[test]
    fn cancel_and_quit_leave_the_draft_behind() {
        let f = new_alias();
        let (_, out) = handle_key(&f, &empty_store(), key(KeyCode::Esc));
        assert_eq!(out, FormOutcome::Cancel);
        let (_, out) = handle_key(&f, &empty_store(), ctrl('c'));
        assert_eq!(out, FormOutcome::Quit);
    }

    #[test]
    fn typing_edits_and_backspaces_the_input() {
        let store = empty_store();
        let f = type_str(&new_alias(), "ab", &store);
        assert_eq!(f.input, "ab");
        let (f, _) = handle_key(&f, &store, key(KeyCode::Backspace));
        assert_eq!(f.input, "a");
        let (f, _) = handle_key(&f, &store, ctrl('u'));
        assert_eq!(f.input, "");
    }

    #[test]
    fn new_alias_walks_all_steps_and_submits() {
        let store = empty_store();
        let f = type_str(&new_alias(), " mytool ", &store);
        let (f, out) = enter(&f, &store);
        assert_eq!(out, FormOutcome::Active);
        assert_eq!(f.step, 1);
        assert_eq!(f.name, "mytool");

        let f = type_str(&f, "mt, my", &store);
        let (f, _) = enter(&f, &store);
        assert_eq!(f.step, 2);

        let f = type_str(&f, "printf %s {input}", &store);
        let (f, _) = enter(&f, &store);
        assert_eq!(f.step, 3);

        // empty macos falls back to the linux command
        let (f, out) = enter(&f, &store);
        match out {
            FormOutcome::Submit(Submission::Alias(def)) => {
                assert_eq!(def.name, "mytool");
                assert_eq!(def.shortcuts, vec!["mt".to_string(), "my".to_string()]);
                assert_eq!(def.linux.as_deref(), Some("printf %s {input}"));
                assert_eq!(def.macos.as_deref(), Some("printf %s {input}"));
                assert!(def.args.is_empty());
                assert!(!def.builtin);
            }
            other => panic!("expected Submit, got {other:?}"),
        }
        assert_eq!(step_count(&f), 4);
    }

    #[test]
    fn validation_errors_stay_on_the_step() {
        let store = empty_store();
        // empty name
        let (f, out) = enter(&new_alias(), &store);
        assert_eq!(out, FormOutcome::Active);
        assert_eq!(f.step, 0);
        assert_eq!(f.error.as_deref(), Some("name cannot be empty"));
        // invalid name chars
        let f = type_str(&new_alias(), "bad!", &store);
        let (f, out) = enter(&f, &store);
        assert_eq!(out, FormOutcome::Active);
        assert!(f.error.unwrap().contains("invalid name"));
        // duplicate name (against builtins, case-insensitive)
        let f = type_str(&new_alias(), "BROWSER", &store);
        let (f, out) = enter(&f, &store);
        assert_eq!(out, FormOutcome::Active);
        assert!(f.error.unwrap().contains("already in use"));
        // empty linux command
        let mut f = new_alias();
        f.step = 2;
        let (f, out) = enter(&f, &store);
        assert_eq!(out, FormOutcome::Active);
        assert_eq!(f.error.as_deref(), Some("linux command cannot be empty"));
    }

    #[test]
    fn new_arg_walks_key_then_value() {
        let mut store = empty_store();
        let mut def = crate::alias::defaults().remove(0);
        def.name = "t".to_string();
        store.aliases.push(def);

        let f = new_arg("t");
        assert_eq!(step_count(&f), 2);
        let f = type_str(&f, "baidu", &store);
        let (f, out) = enter(&f, &store);
        assert_eq!(out, FormOutcome::Active);
        assert_eq!(f.step, 1);

        // multi-word keys are rejected inline
        let bad = type_str(&new_arg("t"), "two words", &store);
        let (bad, out) = enter(&bad, &store);
        assert_eq!(out, FormOutcome::Active);
        assert_eq!(bad.error.as_deref(), Some("arg key must be one word"));

        let f = type_str(&f, "https://www.baidu.com", &store);
        let (_, out) = enter(&f, &store);
        match out {
            FormOutcome::Submit(Submission::Arg { alias, key, value }) => {
                assert_eq!(
                    (alias.as_str(), key.as_str(), value.as_str()),
                    ("t", "baidu", "https://www.baidu.com")
                );
            }
            other => panic!("expected Submit, got {other:?}"),
        }
    }
}
