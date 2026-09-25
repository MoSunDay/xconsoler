//! Wizard form for the `/settings` page: collects a new alias (name →
//! triggers → linux command → macos command), a new concrete shortcut
//! (key → value), an edit of an alias's linux/macos commands, or a new
//! trigger word from a single bottom input line, validating each field on
//! Enter before advancing. Pure data + free functions; the store is only read
//! (duplicate-name checks) — submissions are applied by [`crate::settings`] /
//! [`crate::settings_apply`].

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::alias::{self, AliasDef};
use crate::storage::Store;
use crate::textedit::{self, Motion};

/// What the wizard is collecting.
#[derive(Debug, Clone, PartialEq)]
pub enum Purpose {
    NewAlias,
    /// One shortcut (key → value) added to an existing alias.
    NewShortcut {
        alias: String,
    },
    /// One trigger word added to an existing alias.
    NewTrigger {
        alias: String,
    },
    /// Rewrite an existing alias's linux/macos commands (both prefilled).
    EditCommand {
        alias: String,
    },
}

/// A finished wizard run, ready to be applied to the store.
#[derive(Debug, Clone, PartialEq)]
pub enum Submission {
    Alias(AliasDef),
    /// A concrete key → value shortcut on an alias (refused if the key
    /// already exists).
    Shortcut {
        alias: String,
        key: String,
        value: String,
    },
    /// A trigger word on an alias (refused if already registered there).
    Trigger {
        alias: String,
        trigger: String,
    },
    /// Applied by `alias::set_commands` (blank macos mirrors linux).
    Commands {
        alias: String,
        linux: String,
        macos: String,
    },
}

/// Form state: answers before `step` are final, `input` is the field being
/// edited, `error` is the inline validation message.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub purpose: Purpose,
    pub step: usize,
    pub name: String,
    pub triggers: String,
    pub trigger: String,
    pub linux: String,
    pub macos: String,
    pub shortcut_key: String,
    pub shortcut_value: String,
    pub input: String,
    /// Char index of the text caret inside `input` (`0..=input.chars().count()`).
    pub caret: usize,
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
    /// Ctrl+C / Ctrl+D.
    Quit,
}

/// Start the new-alias wizard (step 0 = name).
pub fn new_alias() -> Form {
    Form {
        purpose: Purpose::NewAlias,
        step: 0,
        name: String::new(),
        triggers: String::new(),
        trigger: String::new(),
        linux: String::new(),
        macos: String::new(),
        shortcut_key: String::new(),
        shortcut_value: String::new(),
        input: String::new(),
        caret: 0,
        error: None,
    }
}

/// Start the add-shortcut wizard for `alias` (step 0 = key, step 1 = value).
pub fn new_shortcut(alias: &str) -> Form {
    Form {
        purpose: Purpose::NewShortcut {
            alias: alias.to_string(),
        },
        step: 0,
        ..new_alias()
    }
}

/// Start the add-trigger wizard for `alias` (single step).
pub fn new_trigger(alias: &str) -> Form {
    Form {
        purpose: Purpose::NewTrigger {
            alias: alias.to_string(),
        },
        step: 0,
        ..new_alias()
    }
}

/// Start the edit-commands wizard for `alias`: both steps are prefilled with
/// the alias's current commands so Enter can be pressed straight through.
/// `None` commands prefill empty (an empty macos answer mirrors linux).
pub fn new_edit_command(alias: &str, linux: Option<&str>, macos: Option<&str>) -> Form {
    let linux = linux.unwrap_or_default().to_string();
    Form {
        purpose: Purpose::EditCommand {
            alias: alias.to_string(),
        },
        step: 0,
        input: linux.clone(),
        caret: linux.chars().count(),
        linux,
        macos: macos.unwrap_or_default().to_string(),
        ..new_alias()
    }
}

/// Number of wizard steps for the form's purpose.
pub fn step_count(f: &Form) -> usize {
    match f.purpose {
        Purpose::NewAlias => 4,
        Purpose::NewShortcut { .. } => 2,
        Purpose::NewTrigger { .. } => 1,
        Purpose::EditCommand { .. } => 2,
    }
}

/// One key transition: `(new form, outcome)`. Only `Press` events count.
///
/// The bottom line is readline-style: a real char-index caret, char/word
/// motions, and the shell's edit keys. Ctrl+D / Ctrl+C quit in any state.
pub fn handle_key(f: &Form, store: &Store, key: KeyEvent) -> (Form, FormOutcome) {
    if key.kind != KeyEventKind::Press {
        return (f.clone(), FormOutcome::Active);
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    // Alt+Ctrl is an undefined combination: the bar treats it as a no-op.
    if ctrl && alt {
        return (f.clone(), FormOutcome::Active);
    }
    let mut next = f.clone();
    match key.code {
        KeyCode::Char('c') if ctrl => (next, FormOutcome::Quit),
        KeyCode::Char('d') if ctrl && !alt => (next, FormOutcome::Quit),
        KeyCode::Esc => (next, FormOutcome::Cancel),
        KeyCode::Enter => advance(f, store),
        KeyCode::Backspace if key.modifiers.is_empty() => {
            edit_with(&mut next, textedit::backspace);
            (next, FormOutcome::Active)
        }
        KeyCode::Delete if !ctrl && !alt => {
            edit_with(&mut next, textedit::delete);
            (next, FormOutcome::Active)
        }
        // Ctrl+H is Backspace (some terminals also report it as Backspace).
        KeyCode::Char('h') if ctrl => {
            edit_with(&mut next, textedit::backspace);
            (next, FormOutcome::Active)
        }
        KeyCode::Char('k') if ctrl => {
            edit_with(&mut next, textedit::kill_to_end);
            (next, FormOutcome::Active)
        }
        KeyCode::Char('u') if ctrl => {
            edit_with(&mut next, textedit::kill_to_start);
            (next, FormOutcome::Active)
        }
        KeyCode::Char('w') if ctrl => {
            edit_with(&mut next, textedit::kill_word);
            (next, FormOutcome::Active)
        }
        KeyCode::Char('t') if ctrl => {
            edit_with(&mut next, textedit::transpose);
            (next, FormOutcome::Active)
        }
        KeyCode::Char('a') if ctrl => {
            move_caret(&mut next, Motion::Home);
            (next, FormOutcome::Active)
        }
        KeyCode::Char('e') if ctrl => {
            move_caret(&mut next, Motion::End);
            (next, FormOutcome::Active)
        }
        KeyCode::Char('b') if ctrl => {
            move_caret(&mut next, Motion::Left);
            (next, FormOutcome::Active)
        }
        KeyCode::Char('f') if ctrl => {
            move_caret(&mut next, Motion::Right);
            (next, FormOutcome::Active)
        }
        KeyCode::Char('b') if alt => {
            move_caret(&mut next, Motion::WordLeft);
            (next, FormOutcome::Active)
        }
        KeyCode::Char('f') if alt => {
            move_caret(&mut next, Motion::WordRight);
            (next, FormOutcome::Active)
        }
        // Modified arrows are word motions; plain arrows are char motions.
        KeyCode::Left if ctrl || alt => {
            move_caret(&mut next, Motion::WordLeft);
            (next, FormOutcome::Active)
        }
        KeyCode::Right if ctrl || alt => {
            move_caret(&mut next, Motion::WordRight);
            (next, FormOutcome::Active)
        }
        KeyCode::Left => {
            move_caret(&mut next, Motion::Left);
            (next, FormOutcome::Active)
        }
        KeyCode::Right => {
            move_caret(&mut next, Motion::Right);
            (next, FormOutcome::Active)
        }
        KeyCode::Home => {
            move_caret(&mut next, Motion::Home);
            (next, FormOutcome::Active)
        }
        KeyCode::End => {
            move_caret(&mut next, Motion::End);
            (next, FormOutcome::Active)
        }
        // Plain chars (ALT-modified ones are the launcher's wake keys).
        KeyCode::Char(c) if !ctrl && !alt => {
            let (input, caret) = textedit::insert(&next.input, next.caret, c);
            next.input = input;
            next.caret = caret;
            (next, FormOutcome::Active)
        }
        _ => (next, FormOutcome::Active),
    }
}

/// Apply one `textedit` operation, keeping `input` and `caret` in sync.
fn edit_with(form: &mut Form, edit: fn(&str, usize) -> (String, usize)) {
    let (input, caret) = edit(&form.input, form.caret);
    form.input = input;
    form.caret = caret;
}

/// Move the caret without touching the text.
fn move_caret(form: &mut Form, motion: Motion) {
    form.caret = textedit::motion(&form.input, form.caret, motion);
}

/// Enter: validate the current field; on success store it and advance (or
/// submit when it was the last step). Errors stay inline.
fn advance(f: &Form, store: &Store) -> (Form, FormOutcome) {
    let mut next = f.clone();
    next.input = f.input.trim().to_string();
    next.caret = next.input.chars().count();
    match validate_step(f, store) {
        Err(e) => {
            next.error = Some(e);
            (next, FormOutcome::Active)
        }
        Ok(()) => {
            next.error = None;
            let value = next.input.clone();
            store_field(&mut next, &value);
            next.step += 1;
            // Some steps start prefilled: Enter accepts the current text,
            // Ctrl+U clears it (edit-command's macos step).
            next.input = prefill(&next);
            next.caret = next.input.chars().count();
            if next.step >= step_count(f) {
                (next.clone(), FormOutcome::Submit(build_submission(&next)))
            } else {
                (next, FormOutcome::Active)
            }
        }
    }
}

/// Initial text of the step just started.
fn prefill(f: &Form) -> String {
    match (&f.purpose, f.step) {
        (Purpose::EditCommand { .. }, 1) => f.macos.clone(),
        _ => String::new(),
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
                    return Err(format!("invalid trigger: {sc}"));
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
        // One word per step: key (step 0) then value (step 1). Duplicate keys
        // and unknown aliases are reported by `alias::set_shortcut` (surfaced
        // on the list's status line).
        (Purpose::NewShortcut { .. }, 0) => {
            if value.is_empty() {
                Err("shortcut key cannot be empty".to_string())
            } else if value.contains(char::is_whitespace) {
                Err("shortcut key must be one word".to_string())
            } else {
                Ok(())
            }
        }
        (Purpose::NewShortcut { .. }, 1) => {
            if value.is_empty() {
                Err("shortcut value cannot be empty".to_string())
            } else {
                Ok(())
            }
        }
        (Purpose::EditCommand { .. }, 0) => {
            if value.is_empty() {
                Err("linux command cannot be empty".to_string())
            } else {
                Ok(())
            }
        }
        (Purpose::EditCommand { .. }, 1) => Ok(()), // empty = same as linux
        // One trigger word, must be a valid identifier; collisions are
        // reported by `alias::add_trigger` (surfaced on the status line).
        (Purpose::NewTrigger { .. }, 0) => {
            if value.is_empty() {
                Err("trigger cannot be empty".to_string())
            } else if !alias::valid_ident(value) {
                Err(format!("invalid trigger: {value} (a-z 0-9 - _)"))
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
        (Purpose::NewAlias, 1) => f.triggers = value.to_string(),
        (Purpose::NewAlias, 2) => f.linux = value.to_string(),
        (Purpose::NewAlias, 3) => f.macos = value.to_string(),
        (Purpose::NewShortcut { .. }, 0) => f.shortcut_key = value.to_string(),
        (Purpose::NewShortcut { .. }, 1) => f.shortcut_value = value.to_string(),
        (Purpose::EditCommand { .. }, 0) => f.linux = value.to_string(),
        (Purpose::EditCommand { .. }, 1) => f.macos = value.to_string(),
        (Purpose::NewTrigger { .. }, 0) => f.trigger = value.to_string(),
        _ => {}
    }
}

fn build_submission(f: &Form) -> Submission {
    match &f.purpose {
        Purpose::NewAlias => {
            let triggers = f
                .triggers
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
                triggers,
                linux: Some(f.linux.clone()),
                macos: Some(macos),
                shortcuts: BTreeMap::new(),
                builtin: false,
            })
        }
        Purpose::NewShortcut { alias } => Submission::Shortcut {
            alias: alias.clone(),
            key: f.shortcut_key.clone(),
            value: f.shortcut_value.clone(),
        },
        Purpose::NewTrigger { alias } => Submission::Trigger {
            alias: alias.clone(),
            trigger: f.trigger.clone(),
        },
        // A blank macos answer is left blank on purpose: `alias::set_commands`
        // mirrors linux for it, the single-platform rule.
        Purpose::EditCommand { alias } => Submission::Commands {
            alias: alias.clone(),
            linux: f.linux.clone(),
            macos: f.macos.clone(),
        },
    }
}

#[cfg(test)]
#[path = "settings_form_edit_tests.rs"]
mod edit_tests;

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
        assert_eq!(
            (f.input.as_str(), f.caret),
            ("ab", 2),
            "typing moves the caret"
        );
        let (f, _) = handle_key(&f, &store, key(KeyCode::Backspace));
        assert_eq!((f.input.as_str(), f.caret), ("a", 1));
        let (f, _) = handle_key(&f, &store, ctrl('u'));
        assert_eq!((f.input.as_str(), f.caret), ("", 0));
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
                assert_eq!(def.triggers, vec!["mt".to_string(), "my".to_string()]);
                assert_eq!(def.linux.as_deref(), Some("printf %s {input}"));
                assert_eq!(def.macos.as_deref(), Some("printf %s {input}"));
                assert!(def.shortcuts.is_empty());
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
        let f = type_str(&new_alias(), "BR", &store);
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
    fn new_shortcut_walks_key_then_value() {
        let mut store = empty_store();
        let mut def = crate::alias::defaults().remove(0);
        def.name = "t".to_string();
        store.aliases.push(def);

        let f = new_shortcut("t");
        assert_eq!(step_count(&f), 2);
        let f = type_str(&f, "baidu", &store);
        let (f, out) = enter(&f, &store);
        assert_eq!(out, FormOutcome::Active);
        assert_eq!(f.step, 1);

        // multi-word keys are rejected inline
        let bad = type_str(&new_shortcut("t"), "two words", &store);
        let (bad, out) = enter(&bad, &store);
        assert_eq!(out, FormOutcome::Active);
        assert_eq!(bad.error.as_deref(), Some("shortcut key must be one word"));

        let f = type_str(&f, "https://www.baidu.com", &store);
        let (_, out) = enter(&f, &store);
        match out {
            FormOutcome::Submit(Submission::Shortcut { alias, key, value }) => {
                assert_eq!(
                    (alias.as_str(), key.as_str(), value.as_str()),
                    ("t", "baidu", "https://www.baidu.com")
                );
            }
            other => panic!("expected Submit, got {other:?}"),
        }
    }

    #[test]
    fn trigger_wizard_is_one_step_and_validates() {
        let store = empty_store();
        let f = new_trigger("t");
        assert_eq!(step_count(&f), 1);
        assert_eq!(f.input, "");

        // empty and malformed answers stay on the (only) step
        let (f, out) = enter(&new_trigger("t"), &store);
        assert_eq!(out, FormOutcome::Active);
        assert_eq!(f.error.as_deref(), Some("trigger cannot be empty"));
        let bad = type_str(&new_trigger("t"), "bad!", &store);
        let (bad, out) = enter(&bad, &store);
        assert_eq!(out, FormOutcome::Active);
        assert!(bad.error.unwrap().contains("invalid trigger"));

        let f = type_str(&new_trigger("t"), "gc", &store);
        let (f, out) = enter(&f, &store);
        match out {
            FormOutcome::Submit(Submission::Trigger { alias, trigger }) => {
                assert_eq!((alias.as_str(), trigger.as_str()), ("t", "gc"));
            }
            other => panic!("expected Submit, got {other:?}"),
        }
        assert_eq!(f.error, None);
    }

    #[test]
    fn edit_command_wizard_prefills_both_steps() {
        let store = empty_store();
        let f = new_edit_command("br", Some("xdg-open {input}"), Some("open {input}"));
        assert_eq!(step_count(&f), 2);
        assert_eq!(f.input, "xdg-open {input}", "linux step starts prefilled");

        // Enter accepts the prefilled linux command as-is
        let (f, out) = enter(&f, &store);
        assert_eq!(out, FormOutcome::Active);
        assert_eq!(f.step, 1);
        assert_eq!(f.input, "open {input}", "macos step starts prefilled too");

        let (f, out) = enter(&f, &store);
        match out {
            FormOutcome::Submit(Submission::Commands {
                alias,
                linux,
                macos,
            }) => {
                assert_eq!(alias, "br");
                assert_eq!(linux, "xdg-open {input}");
                assert_eq!(macos, "open {input}");
            }
            other => panic!("expected Submit, got {other:?}"),
        }
        assert_eq!(f.error, None);
    }

    #[test]
    fn edit_command_ctrl_u_clears_and_blank_macos_stays_blank() {
        let store = empty_store();
        let f = new_edit_command("t", Some("printf %s {input}"), None);
        assert_eq!(f.input, "printf %s {input}");
        assert_eq!(f.macos, "", "no macos command to prefill");

        // Ctrl+U clears the prefilled text; Enter on the empty field refuses
        let (f, _) = handle_key(&f, &store, ctrl('u'));
        assert_eq!(f.input, "");
        let (f, out) = enter(&f, &store);
        assert_eq!(out, FormOutcome::Active);
        assert_eq!(f.error.as_deref(), Some("linux command cannot be empty"));

        let f = type_str(&f, "echo {input}", &store);
        let (f, _) = enter(&f, &store);
        assert_eq!(f.step, 1);
        assert_eq!(f.input, "", "nothing to prefill this time");

        let (_, out) = enter(&f, &store);
        match out {
            FormOutcome::Submit(Submission::Commands { linux, macos, .. }) => {
                assert_eq!(linux, "echo {input}");
                assert_eq!(macos, "", "left blank: set_commands mirrors linux");
            }
            other => panic!("expected Submit, got {other:?}"),
        }
    }

    #[test]
    fn edit_command_step_two_can_be_rewritten() {
        let store = empty_store();
        let f = new_edit_command("t", Some("xdg-open {input}"), Some("open {input}"));
        let (f, _) = enter(&f, &store); // accept linux
        let (f, _) = handle_key(&f, &store, ctrl('u')); // clear macos
        let f = type_str(&f, "open -a Safari {input}", &store);
        let (_, out) = enter(&f, &store);
        match out {
            FormOutcome::Submit(Submission::Commands { linux, macos, .. }) => {
                assert_eq!(linux, "xdg-open {input}");
                assert_eq!(macos, "open -a Safari {input}");
            }
            other => panic!("expected Submit, got {other:?}"),
        }
    }
}
