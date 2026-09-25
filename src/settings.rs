//! `/settings` page state and key handling: the alias table (list screen)
//! plus dispatch into the wizard form ([`crate::settings_form`]).
//!
//! Everything here is a pure transition:
//! `handle_key(state, store, key) -> (new state, new store, effect)` — no
//! I/O, no clocks. Persistence stays with the caller
//! ([`crate::settings_apply::apply`] runs `storage::save` when the effect says
//! [`Effect::Save`] and pushes data-carrying effects through the
//! `alias::` helpers).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::alias::{self, AliasDef};
use crate::platform::{self, Platform};
use crate::settings_form::{self, Form, FormOutcome, Submission};
use crate::storage::Store;

/// What the caller should do after a key transition.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Nothing changed.
    None,
    /// Leave settings mode, back to the launcher bar.
    Back,
    /// The store changed; persist it.
    Save,
    /// Quit the app.
    Quit,
    /// Insert or replace a user alias definition.
    AddAlias(AliasDef),
    /// Set (or replace) a concrete shortcut (`alias::set_shortcut`).
    SetShortcut {
        alias: String,
        key: String,
        value: String,
    },
    /// Add a trigger word (`alias::add_trigger`).
    AddTrigger { alias: String, trigger: String },
    /// Drop a trigger word (`alias::remove_trigger`).
    RemoveTrigger { alias: String, trigger: String },
    /// Rewrite the selected platform's command (`alias::set_command`); the
    /// other platform's stored command is never touched.
    SetCommand {
        alias: String,
        platform: Platform,
        command: String,
    },
    /// Rewrite one concrete shortcut, key included (`alias::edit_shortcut`).
    EditShortcut {
        alias: String,
        old_key: String,
        key: String,
        value: String,
    },
    /// Rename one trigger word in place (`alias::rename_trigger`).
    RenameTrigger {
        alias: String,
        old: String,
        new: String,
    },
}

/// One flattened, selectable row of the list screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Alias {
        idx: usize,
    },
    /// Trigger word of an expanded alias; `idx` indexes its triggers.
    Trigger {
        alias: usize,
        idx: usize,
    },
    /// Concrete shortcut of an expanded alias (key → value).
    Shortcut {
        alias: usize,
        key: String,
    },
}

/// Settings page state. `cursor` indexes [`rows`]; `expanded` is the alias
/// whose triggers and concrete shortcuts are shown indented below it.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Active wizard form; `None` means the list screen.
    pub form: Option<Form>,
    pub cursor: usize,
    pub expanded: Option<usize>,
    /// Transient message `(ok?, text)` shown above the key hints.
    pub status: Option<(bool, String)>,
    /// Platform the page edits: only this platform's commands are shown by
    /// the table and offered by the wizards.
    pub platform: Platform,
}

/// Fresh list state for the detected platform: cursor on the first alias,
/// nothing expanded.
pub fn new() -> Settings {
    new_for(platform::current())
}

/// Fresh list state for an explicit platform (tests force the "other" one).
pub fn new_for(platform: Platform) -> Settings {
    Settings {
        form: None,
        cursor: 0,
        expanded: None,
        status: None,
        platform,
    }
}

/// Alias list for the page: what the store carries, seeded defaults included.
pub fn view(store: &Store) -> Vec<AliasDef> {
    store.aliases.clone()
}

/// Flattened rows: one per alias, plus indented rows for `expanded` (its
/// triggers first, then its concrete shortcuts).
pub fn rows(aliases: &[AliasDef], expanded: Option<usize>) -> Vec<Row> {
    let mut out = Vec::new();
    for (idx, def) in aliases.iter().enumerate() {
        out.push(Row::Alias { idx });
        if expanded == Some(idx) {
            for (i, _) in def.triggers.iter().enumerate() {
                out.push(Row::Trigger { alias: idx, idx: i });
            }
            for key in def.shortcuts.keys() {
                out.push(Row::Shortcut {
                    alias: idx,
                    key: key.clone(),
                });
            }
        }
    }
    out
}

/// One key transition over the whole page (list or form).
pub fn handle_key(st: &Settings, store: &Store, key: KeyEvent) -> (Settings, Store, Effect) {
    if key.kind != KeyEventKind::Press {
        return (st.clone(), store.clone(), Effect::None);
    }
    // ALT belongs to the launcher: the wake hotkey keeps working on this
    // page, and without this guard its plain-char arms (Alt+D => delete)
    // would fire instead. The wizard's input line is the exception: Alt+B /
    // Alt+F must reach the form as word motions. Every other ALT key stays a
    // no-op there too, because the form's plain-char arm excludes alt.
    if key.modifiers.contains(KeyModifiers::ALT) && st.form.is_none() {
        return (st.clone(), store.clone(), Effect::None);
    }
    match st.form.as_ref() {
        Some(form) => form_key(st, form, store, key),
        None => list_key(st, store, key),
    }
}

fn list_key(st: &Settings, store: &Store, key: KeyEvent) -> (Settings, Store, Effect) {
    let aliases = view(store);
    let cur_rows = rows(&aliases, st.expanded);
    let mut next = Settings {
        form: None,
        cursor: st.cursor,
        expanded: st.expanded,
        status: None,
        platform: st.platform,
    };
    let mut store = store.clone();
    let mut effect = Effect::None;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        KeyCode::Char('c') if ctrl => effect = Effect::Quit,
        KeyCode::Char('d') if ctrl => effect = Effect::Quit,
        KeyCode::Esc => effect = Effect::Back,
        KeyCode::Char('q') if !ctrl => effect = Effect::Back,
        KeyCode::Up => next.cursor = move_sel(&cur_rows, st.cursor, -1),
        KeyCode::Down => next.cursor = move_sel(&cur_rows, st.cursor, 1),
        KeyCode::Char('k') if !ctrl => next.cursor = move_sel(&cur_rows, st.cursor, -1),
        KeyCode::Char('j') if !ctrl => next.cursor = move_sel(&cur_rows, st.cursor, 1),
        KeyCode::Enter | KeyCode::Right => next.expanded = toggle(&cur_rows, st),
        // ← is a pure collapse: it never opens a different alias by accident.
        KeyCode::Left => next.expanded = None,
        KeyCode::Char('n') if !ctrl => next.form = Some(settings_form::new_alias(st.platform)),
        KeyCode::Char('s') if !ctrl => {
            next.form = selected_alias(&cur_rows, st, &aliases)
                .map(|name| settings_form::new_shortcut(&name))
        }
        KeyCode::Char('t') if !ctrl => {
            next.form = selected_alias(&cur_rows, st, &aliases)
                .map(|name| settings_form::new_trigger(&name))
        }
        KeyCode::Char('e') if !ctrl => next.form = edit_form(&cur_rows, st, &aliases),
        KeyCode::Char('d') if !ctrl => {
            effect = delete_row(&cur_rows, st, &mut store, &mut next);
        }
        _ => {}
    }
    clamp_cursor(&mut next, &view(&store));
    (next, store, effect)
}

/// Enter/→ expands the selected alias (or collapses when already open,
/// and on an entry row collapses too); ← collapses whatever is open.
fn toggle(rows: &[Row], st: &Settings) -> Option<usize> {
    match rows.get(st.cursor) {
        Some(Row::Alias { idx }) => {
            if st.expanded == Some(*idx) {
                None
            } else {
                Some(*idx)
            }
        }
        _ => None,
    }
}

/// Name of the alias the selection belongs to (an entry row's parent).
fn selected_alias(rows: &[Row], st: &Settings, aliases: &[AliasDef]) -> Option<String> {
    selected_def(rows, st, aliases).map(|d| d.name.clone())
}

/// Definition the selection belongs to (an entry row's parent alias).
fn selected_def<'a>(rows: &[Row], st: &Settings, aliases: &'a [AliasDef]) -> Option<&'a AliasDef> {
    let idx = match rows.get(st.cursor) {
        Some(Row::Alias { idx }) => *idx,
        Some(Row::Trigger { alias, .. }) => *alias,
        Some(Row::Shortcut { alias, .. }) => *alias,
        None => return None,
    };
    aliases.get(idx)
}

/// `e`: the wizard matching the selected row — an alias row edits that
/// platform's command, a shortcut row that key/value pair (key and value
/// prefilled), a trigger row that word. `None` when nothing valid is
/// selected.
fn edit_form(rows: &[Row], st: &Settings, aliases: &[AliasDef]) -> Option<Form> {
    let def = selected_def(rows, st, aliases)?;
    match rows.get(st.cursor)? {
        Row::Shortcut { key, .. } => def
            .shortcuts
            .get(key)
            .map(|value| settings_form::new_edit_shortcut(&def.name, key, value)),
        Row::Trigger { idx, .. } => def
            .triggers
            .get(*idx)
            .map(|trigger| settings_form::new_edit_trigger(&def.name, trigger)),
        Row::Alias { .. } => Some(settings_form::new_edit_command(
            &def.name,
            st.platform,
            alias::platform_command(def, st.platform),
        )),
    }
}

/// Move by `delta`, clamped into `rows`.
fn move_sel(rows: &[Row], cur: usize, delta: i32) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let cur = cur.min(rows.len() - 1) as i32;
    (cur + delta).clamp(0, rows.len() as i32 - 1) as usize
}

/// `d`: delete the selection — a trigger row drops just that trigger (through
/// the effect path), a shortcut row just that key (inline, reported here), an
/// alias row loses the whole alias (seeded or not, like `:del`).
fn delete_row(rows: &[Row], st: &Settings, store: &mut Store, next: &mut Settings) -> Effect {
    let aliases = view(store);
    match rows.get(st.cursor) {
        Some(Row::Trigger { alias, idx }) => match aliases.get(*alias) {
            Some(def) => match def.triggers.get(*idx) {
                // The removal itself runs in the effect path (which reports
                // the outcome); the alias stays expanded so more entries can
                // be dropped in a row.
                Some(trigger) => Effect::RemoveTrigger {
                    alias: def.name.clone(),
                    trigger: trigger.clone(),
                },
                None => {
                    next.status = Some((false, "nothing selected".to_string()));
                    Effect::None
                }
            },
            None => {
                next.status = Some((false, "nothing selected".to_string()));
                Effect::None
            }
        },
        Some(Row::Shortcut { alias, key }) => {
            let name = aliases
                .get(*alias)
                .map(|d| d.name.clone())
                .unwrap_or_default();
            // The key is checked against the stored list, and the removal runs
            // on that same list (`alias::remove_shortcut`): every alias,
            // seeded or not, carries its keys.
            if !aliases
                .get(*alias)
                .is_some_and(|d| d.shortcuts.contains_key(key))
            {
                next.status = Some((false, "nothing selected".to_string()));
                return Effect::None;
            }
            match alias::remove_shortcut(&mut store.aliases, &name, key) {
                Ok(()) => {
                    next.status = Some((true, format!("shortcut removed: {name}.{key}")));
                    next.expanded = None;
                    Effect::Save
                }
                Err(e) => {
                    next.status = Some((false, e));
                    Effect::None
                }
            }
        }
        Some(Row::Alias { idx }) => match aliases.get(*idx) {
            Some(def) => {
                store.aliases.retain(|d| d.name != def.name);
                next.status = Some((true, format!("alias removed: {}", def.name)));
                next.expanded = None;
                Effect::Save
            }
            None => {
                next.status = Some((false, "nothing selected".to_string()));
                Effect::None
            }
        },
        None => {
            next.status = Some((false, "nothing selected".to_string()));
            Effect::None
        }
    }
}

fn form_key(st: &Settings, form: &Form, store: &Store, key: KeyEvent) -> (Settings, Store, Effect) {
    let (next_form, outcome) = settings_form::handle_key(form, store, key);
    let list = |form: Option<Form>, status: Option<(bool, String)>| Settings {
        form,
        cursor: st.cursor,
        expanded: st.expanded,
        status,
        platform: st.platform,
    };
    match outcome {
        FormOutcome::Active => (
            list(Some(next_form), st.status.clone()),
            store.clone(),
            Effect::None,
        ),
        FormOutcome::Quit => (
            list(Some(next_form), st.status.clone()),
            store.clone(),
            Effect::Quit,
        ),
        FormOutcome::Cancel => (list(None, None), store.clone(), Effect::None),
        FormOutcome::Submit(sub) => (
            // Every wizard result travels as a data-carrying effect: the
            // caller applies it through the shared `alias::` helpers, saves,
            // and reports the outcome on the list's status line.
            list(None, None),
            store.clone(),
            effect_of(sub),
        ),
    }
}

/// Effect carrying a finished wizard submission.
fn effect_of(sub: Submission) -> Effect {
    match sub {
        Submission::Alias(def) => Effect::AddAlias(def),
        Submission::Shortcut { alias, key, value } => Effect::SetShortcut { alias, key, value },
        Submission::Trigger { alias, trigger } => Effect::AddTrigger { alias, trigger },
        Submission::Command {
            alias,
            platform,
            command,
        } => Effect::SetCommand {
            alias,
            platform,
            command,
        },
        Submission::EditShortcut {
            alias,
            old_key,
            key,
            value,
        } => Effect::EditShortcut {
            alias,
            old_key,
            key,
            value,
        },
        Submission::RenameTrigger { alias, old, new } => Effect::RenameTrigger { alias, old, new },
    }
}

/// Keep the cursor inside the (possibly shrunken) row list.
fn clamp_cursor(st: &mut Settings, aliases: &[AliasDef]) {
    let len = rows(aliases, st.expanded).len();
    st.cursor = st.cursor.min(len.saturating_sub(1));
}

/// Re-clamp the cursor after the store changed outside this module: the
/// effect path applies shortcut/command edits between two key transitions.
pub fn reclamp(st: &mut Settings, store: &Store) {
    clamp_cursor(st, &view(store));
}

#[cfg(test)]
#[path = "settings/list_tests.rs"]
mod list_tests;

#[cfg(test)]
#[path = "settings/edit_tests.rs"]
mod edit_tests;
