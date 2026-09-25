//! Wizard form for the `/settings` page: collects a new alias (name →
//! triggers → the current platform's command), a new concrete shortcut
//! (key → value), an edit of an alias's command for the current platform, an
//! in-place edit of one concrete shortcut (key → value) or trigger word, or a
//! new trigger word from a single bottom input line, validating each field on
//! Enter before advancing. Pure data + free functions; the store is only read
//! (duplicate-name checks) — submissions are applied by [`crate::settings`] /
//! [`crate::settings_apply`].
//!
//! Wizard forms are platform-aware: the other platform's stored command is
//! never shown, prefilled or submitted by this module.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::alias::{self, AliasDef};
use crate::platform::{self, Platform};
use crate::storage::Store;
use crate::textedit::{self, Motion};

/// What the wizard is collecting.
#[derive(Debug, Clone, PartialEq)]
pub enum Purpose {
    /// New alias for one platform; only that platform's command is collected.
    NewAlias(Platform),
    /// One shortcut (key → value) added to an existing alias.
    NewShortcut { alias: String },
    /// One trigger word added to an existing alias.
    NewTrigger { alias: String },
    /// Rewrite an existing alias's command for one platform (prefilled).
    EditCommand { alias: String, platform: Platform },
    /// Rewrite one concrete shortcut of an alias; the key may change
    /// (`old_key` is the key as shown on the list).
    EditShortcut { alias: String, old_key: String },
    /// Rename one trigger word of an alias in place.
    EditTrigger { alias: String, old: String },
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
    /// Applied by `alias::set_command`: sets only `platform`'s field.
    Command {
        alias: String,
        platform: Platform,
        command: String,
    },
    /// Applied by `alias::edit_shortcut`: replaces `old_key` with `key`,
    /// both under `alias`.
    EditShortcut {
        alias: String,
        old_key: String,
        key: String,
        value: String,
    },
    /// Applied by `alias::rename_trigger`: renames `old` to `new` in place.
    RenameTrigger {
        alias: String,
        old: String,
        new: String,
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
    pub command: String,
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

/// Fresh wizard state for `purpose`: every answer empty, step 0.
fn blank(purpose: Purpose) -> Form {
    Form {
        purpose,
        step: 0,
        name: String::new(),
        triggers: String::new(),
        trigger: String::new(),
        command: String::new(),
        shortcut_key: String::new(),
        shortcut_value: String::new(),
        input: String::new(),
        caret: 0,
        error: None,
    }
}

/// Start the new-alias wizard for `platform` (step 0 = name, step 2 = that
/// platform's command).
pub fn new_alias(platform: Platform) -> Form {
    blank(Purpose::NewAlias(platform))
}

/// Start the add-shortcut wizard for `alias` (step 0 = key, step 1 = value).
pub fn new_shortcut(alias: &str) -> Form {
    blank(Purpose::NewShortcut {
        alias: alias.to_string(),
    })
}

/// Start the add-trigger wizard for `alias` (single step).
pub fn new_trigger(alias: &str) -> Form {
    blank(Purpose::NewTrigger {
        alias: alias.to_string(),
    })
}

/// Start the edit-command wizard for `alias`: the single step is prefilled
/// with `platform`'s stored command (empty when `current` is `None`), so
/// Enter can be pressed straight through. The other platform is never
/// touched.
pub fn new_edit_command(alias: &str, platform: Platform, current: Option<&str>) -> Form {
    let mut form = blank(Purpose::EditCommand {
        alias: alias.to_string(),
        platform,
    });
    let current = current.unwrap_or_default().to_string();
    form.caret = current.chars().count();
    form.input = current.clone();
    form.command = current;
    form
}

/// Start the edit-shortcut wizard for `alias`: step 0 is the current key,
/// step 1 the current value, both prefilled so Enter can be pressed straight
/// through (the key may be rewritten, which renames the entry in place).
pub fn new_edit_shortcut(alias: &str, key: &str, value: &str) -> Form {
    let mut form = blank(Purpose::EditShortcut {
        alias: alias.to_string(),
        old_key: key.to_string(),
    });
    form.input = key.to_string();
    form.caret = key.chars().count();
    form.shortcut_value = value.to_string();
    form
}

/// Start the edit-trigger wizard for `alias` (single step, prefilled with the
/// current word).
pub fn new_edit_trigger(alias: &str, trigger: &str) -> Form {
    let mut form = blank(Purpose::EditTrigger {
        alias: alias.to_string(),
        old: trigger.to_string(),
    });
    form.input = trigger.to_string();
    form.caret = trigger.chars().count();
    form
}

/// Number of wizard steps for the form's purpose.
pub fn step_count(f: &Form) -> usize {
    match f.purpose {
        Purpose::NewAlias(_) => 3,
        Purpose::NewShortcut { .. } => 2,
        Purpose::NewTrigger { .. } => 1,
        Purpose::EditCommand { .. } => 1,
        Purpose::EditShortcut { .. } => 2,
        Purpose::EditTrigger { .. } => 1,
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
            // The shortcut edit's value step starts prefilled: Enter accepts
            // the current text, Ctrl+U clears it.
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
        // The shortcut edit's value step starts from the current value, so
        // Enter alone accepts it.
        (Purpose::EditShortcut { .. }, 1) => f.shortcut_value.clone(),
        _ => String::new(),
    }
}

fn validate_step(f: &Form, store: &Store) -> Result<(), String> {
    let value = f.input.trim();
    match (&f.purpose, f.step) {
        (Purpose::NewAlias(_), 0) => {
            if value.is_empty() {
                Err("name cannot be empty".to_string())
            } else if !alias::valid_ident(value) {
                Err(format!("invalid name: {value} (a-z 0-9 - _)"))
            } else if alias::resolve(&store.aliases, value).is_some() {
                Err(format!("name already in use: {value}"))
            } else {
                Ok(())
            }
        }
        (Purpose::NewAlias(_), 1) => {
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
        // The command step names the platform being configured: the wizard
        // collects exactly one command (the current platform's).
        (Purpose::NewAlias(platform), 2) | (Purpose::EditCommand { platform, .. }, 0) => {
            if value.is_empty() {
                Err(format!(
                    "{} command cannot be empty",
                    platform::name(*platform)
                ))
            } else {
                Ok(())
            }
        }
        // One word per step: key (step 0) then value (step 1). Duplicate keys
        // and unknown aliases are reported by `alias::set_shortcut` /
        // `alias::edit_shortcut` (surfaced on the list's status line).
        (Purpose::NewShortcut { .. } | Purpose::EditShortcut { .. }, 0) => {
            if value.is_empty() {
                Err("shortcut key cannot be empty".to_string())
            } else if value.contains(char::is_whitespace) {
                Err("shortcut key must be one word".to_string())
            } else {
                Ok(())
            }
        }
        (Purpose::NewShortcut { .. } | Purpose::EditShortcut { .. }, 1) => {
            if value.is_empty() {
                Err("shortcut value cannot be empty".to_string())
            } else {
                Ok(())
            }
        }
        // One trigger word, must be a valid identifier; collisions are
        // reported by `alias::add_trigger` / `alias::rename_trigger`
        // (surfaced on the status line).
        (Purpose::NewTrigger { .. } | Purpose::EditTrigger { .. }, 0) => {
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
        (Purpose::NewAlias(_), 0) => f.name = value.to_string(),
        (Purpose::NewAlias(_), 1) => f.triggers = value.to_string(),
        (Purpose::NewAlias(_), 2) | (Purpose::EditCommand { .. }, 0) => {
            f.command = value.to_string()
        }
        (Purpose::NewShortcut { .. } | Purpose::EditShortcut { .. }, 0) => {
            f.shortcut_key = value.to_string()
        }
        (Purpose::NewShortcut { .. } | Purpose::EditShortcut { .. }, 1) => {
            f.shortcut_value = value.to_string()
        }
        (Purpose::NewTrigger { .. } | Purpose::EditTrigger { .. }, 0) => {
            f.trigger = value.to_string()
        }
        _ => {}
    }
}

fn build_submission(f: &Form) -> Submission {
    match &f.purpose {
        Purpose::NewAlias(platform) => {
            let triggers = f
                .triggers
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            // Only the wizard's platform is stored: the other field stays
            // `None` (the run path already falls back to this command when
            // run on the other platform).
            let command = Some(f.command.clone());
            Submission::Alias(AliasDef {
                name: f.name.clone(),
                triggers,
                linux: if *platform == Platform::Linux {
                    command.clone()
                } else {
                    None
                },
                macos: if *platform == Platform::Macos {
                    command
                } else {
                    None
                },
                shortcuts: BTreeMap::new(),
            })
        }
        Purpose::NewShortcut { alias } => Submission::Shortcut {
            alias: alias.clone(),
            key: f.shortcut_key.clone(),
            value: f.shortcut_value.clone(),
        },
        Purpose::EditShortcut { alias, old_key } => Submission::EditShortcut {
            alias: alias.clone(),
            old_key: old_key.clone(),
            key: f.shortcut_key.clone(),
            value: f.shortcut_value.clone(),
        },
        Purpose::NewTrigger { alias } => Submission::Trigger {
            alias: alias.clone(),
            trigger: f.trigger.clone(),
        },
        Purpose::EditTrigger { alias, old } => Submission::RenameTrigger {
            alias: alias.clone(),
            old: old.clone(),
            new: f.trigger.clone(),
        },
        // One platform per wizard run: the other platform's stored command
        // is neither read nor submitted here.
        Purpose::EditCommand { alias, platform } => Submission::Command {
            alias: alias.clone(),
            platform: *platform,
            command: f.command.clone(),
        },
    }
}

#[cfg(test)]
#[path = "settings_form_edit_tests.rs"]
mod edit_tests;

#[cfg(test)]
#[path = "settings_form/tests.rs"]
mod tests;
