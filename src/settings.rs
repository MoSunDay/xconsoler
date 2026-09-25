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
use crate::settings_form::{self, Form, FormOutcome, Submission};
use crate::storage::{self, Store};

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
    /// Rewrite both commands of an alias (`alias::set_commands`).
    SetCommands {
        alias: String,
        linux: String,
        macos: String,
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
}

/// Fresh list state: cursor on the first alias, nothing expanded.
pub fn new() -> Settings {
    Settings {
        form: None,
        cursor: 0,
        expanded: None,
        status: None,
    }
}

/// Effective alias list for the page (built-ins merged with user defs).
pub fn view(store: &Store) -> Vec<AliasDef> {
    storage::merge_aliases(&store.aliases)
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
        KeyCode::Char('n') if !ctrl => next.form = Some(settings_form::new_alias()),
        KeyCode::Char('s') if !ctrl => {
            next.form = selected_alias(&cur_rows, st, &aliases)
                .map(|name| settings_form::new_shortcut(&name))
        }
        KeyCode::Char('t') if !ctrl => {
            next.form = selected_alias(&cur_rows, st, &aliases)
                .map(|name| settings_form::new_trigger(&name))
        }
        KeyCode::Char('e') if !ctrl => {
            next.form = selected_def(&cur_rows, st, &aliases).map(|d| {
                settings_form::new_edit_command(&d.name, d.linux.as_deref(), d.macos.as_deref())
            })
        }
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
/// alias row loses the whole alias (built-ins refuse, like `:del`).
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
            // The key is checked against the effective list, the removal runs
            // on the user list (`alias::remove_shortcut`): a built-in keeps
            // its fixed keys until an override materialises it.
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
            Some(def) if def.builtin => {
                next.status = Some((
                    false,
                    "builtin alias cannot be deleted (override with :add)".to_string(),
                ));
                Effect::None
            }
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
        Submission::Commands {
            alias,
            linux,
            macos,
        } => Effect::SetCommands {
            alias,
            linux,
            macos,
        },
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
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn store_with_t() -> Store {
        let mut store = Store::default();
        store.aliases.push(crate::alias::AliasDef {
            name: "t".to_string(),
            triggers: vec!["tt".to_string()],
            linux: Some("printf %s {input}".to_string()),
            macos: None,
            shortcuts: [("baidu".to_string(), "https://www.baidu.com".to_string())]
                .into_iter()
                .collect(),
            builtin: false,
        });
        store
    }

    fn type_str(st: &Settings, store: &Store, s: &str) -> Settings {
        let mut st = st.clone();
        for c in s.chars() {
            let (next, _, _) = handle_key(&st, store, key(KeyCode::Char(c)));
            st = next;
        }
        st
    }

    /// Select alias `t` (index 2 of the merged list) with `t`'s 1 trigger +
    /// 1 concrete shortcut shown.
    fn on_t() -> (Settings, Store) {
        let store = store_with_t();
        let st = Settings { cursor: 2, ..new() };
        (st, store)
    }

    #[test]
    fn rows_list_aliases_then_expanded_entries() {
        let store = store_with_t();
        let aliases = view(&store);
        let flat = rows(&aliases, None);
        assert_eq!(flat.len(), 3); // br, cd, t
        let expanded = rows(&aliases, Some(2));
        assert_eq!(
            expanded,
            vec![
                Row::Alias { idx: 0 },
                Row::Alias { idx: 1 },
                Row::Alias { idx: 2 },
                // expanded rows: triggers first, then concrete shortcuts
                Row::Trigger { alias: 2, idx: 0 },
                Row::Shortcut {
                    alias: 2,
                    key: "baidu".to_string()
                },
            ]
        );
    }

    #[test]
    fn expanded_rows_list_every_trigger_and_shortcut() {
        let mut store = store_with_t();
        store.aliases[0].triggers = vec!["tt".to_string(), "tw".to_string()];
        store.aliases[0]
            .shortcuts
            .insert("cc".to_string(), "x".to_string());
        let aliases = view(&store);
        let flat = rows(&aliases, Some(2));
        assert_eq!(
            flat[3..7],
            [
                Row::Trigger { alias: 2, idx: 0 },
                Row::Trigger { alias: 2, idx: 1 },
                Row::Shortcut {
                    alias: 2,
                    key: "baidu".to_string()
                },
                Row::Shortcut {
                    alias: 2,
                    key: "cc".to_string()
                },
            ]
        );
    }

    #[test]
    fn s_opens_the_shortcut_wizard_for_the_selected_alias() {
        let (st, store) = on_t();
        let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Char('s')));
        assert_eq!(eff, Effect::None);
        let form = st.form.as_ref().expect("shortcut wizard open");
        assert_eq!(
            form.purpose,
            settings_form::Purpose::NewShortcut {
                alias: "t".to_string()
            }
        );
        assert_eq!(settings_form::step_count(form), 2);
        // Esc cancels, nothing was touched
        let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Esc));
        assert_eq!((st.form, eff), (None, Effect::None));
    }

    #[test]
    fn submitting_the_shortcut_wizard_yields_the_effect() {
        let (st, store) = on_t();
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('s')));
        let st = type_str(&st, &store, "gc");
        let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter)); // key -> value step
        assert_eq!(eff, Effect::None);
        assert_eq!(st.form.as_ref().map(|f| f.step), Some(1));
        let st = type_str(&st, &store, "git clone {input}");
        let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter));
        assert_eq!(st.form, None, "the wizard closes on submit");
        assert_eq!(
            eff,
            Effect::SetShortcut {
                alias: "t".to_string(),
                key: "gc".to_string(),
                value: "git clone {input}".to_string()
            }
        );
    }

    #[test]
    fn t_opens_the_trigger_wizard_and_submits_a_trigger() {
        let (st, store) = on_t();
        let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Char('t')));
        assert_eq!(eff, Effect::None);
        let form = st.form.as_ref().expect("trigger wizard open");
        assert_eq!(
            form.purpose,
            settings_form::Purpose::NewTrigger {
                alias: "t".to_string()
            }
        );
        assert_eq!(settings_form::step_count(form), 1);
        let st = type_str(&st, &store, "gc");
        let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter));
        assert_eq!(st.form, None);
        assert_eq!(
            eff,
            Effect::AddTrigger {
                alias: "t".to_string(),
                trigger: "gc".to_string()
            }
        );
    }

    #[test]
    fn a_is_no_longer_bound_on_the_list() {
        let (st, store) = on_t();
        let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Char('a')));
        assert_eq!((st.form, eff), (None, Effect::None));
    }

    #[test]
    fn e_opens_the_edit_wizard_prefilled_with_both_commands() {
        let (st, store) = on_t();
        let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Char('e')));
        assert_eq!(eff, Effect::None);
        let form = st.form.expect("edit wizard open");
        assert_eq!(
            form.purpose,
            settings_form::Purpose::EditCommand {
                alias: "t".to_string()
            }
        );
        assert_eq!(form.input, "printf %s {input}", "linux step prefilled");
        assert_eq!(form.macos, "", "t has no macos command");
        assert_eq!(settings_form::step_count(&form), 2);
    }

    #[test]
    fn edit_wizard_accepts_prefilled_text_and_mirrors_blank_macos() {
        let (st, store) = on_t();
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('e')));
        // Enter accepts the prefilled linux step, Enter the (empty) macos one
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Enter));
        let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter));
        assert_eq!(st.form, None);
        assert_eq!(
            eff,
            Effect::SetCommands {
                alias: "t".to_string(),
                linux: "printf %s {input}".to_string(),
                macos: String::new(),
            },
            "a blank macos answer mirrors linux in alias::set_commands"
        );
    }

    #[test]
    fn entry_rows_route_e_s_and_t_to_their_parent_alias() {
        let (st, store) = on_t();
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Enter)); // expand
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down)); // trigger row
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('t')));
        assert_eq!(
            st.form.as_ref().map(|f| f.purpose.clone()),
            Some(settings_form::Purpose::NewTrigger {
                alias: "t".to_string()
            })
        );
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Esc));
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down)); // shortcut row
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('s')));
        assert_eq!(
            st.form.as_ref().map(|f| f.purpose.clone()),
            Some(settings_form::Purpose::NewShortcut {
                alias: "t".to_string()
            })
        );
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Esc));
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('e')));
        assert_eq!(
            st.form.as_ref().map(|f| f.purpose.clone()),
            Some(settings_form::Purpose::EditCommand {
                alias: "t".to_string()
            })
        );
    }

    #[test]
    fn d_on_a_trigger_row_asks_for_its_removal() {
        let (st, store) = on_t();
        let (st, _, eff) = handle_key(&st, &store, key(KeyCode::Enter)); // expand
        assert_eq!(eff, Effect::None);
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down)); // trigger row
        let (_, s2, eff) = handle_key(&st, &store, key(KeyCode::Char('d')));
        assert_eq!(
            eff,
            Effect::RemoveTrigger {
                alias: "t".to_string(),
                trigger: "tt".to_string()
            },
            "`d` on a trigger row drops just that word"
        );
        assert_eq!(s2, store, "the pure transition leaves the store alone");
        assert_eq!(s2.aliases[0].triggers, vec!["tt".to_string()]);
    }

    #[test]
    fn d_on_a_shortcut_row_removes_it_inline() {
        let (st, store) = on_t();
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Enter)); // expand
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down)); // trigger row
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down)); // shortcut row
        let (st, s2, eff) = handle_key(&st, &store, key(KeyCode::Char('d')));
        assert_eq!(eff, Effect::Save, "an inline store edit still persists");
        assert!(s2.aliases[0].shortcuts.is_empty());
        assert_eq!(
            st.status,
            Some((true, "shortcut removed: t.baidu".to_string()))
        );
        assert_eq!(st.expanded, None, "the list collapses after a removal");
    }

    #[test]
    fn reclamp_keeps_the_cursor_on_a_row() {
        let store = store_with_t();
        let mut st = Settings {
            cursor: 4,
            expanded: Some(2),
            ..new()
        };
        reclamp(&mut st, &store);
        assert_eq!(st.cursor, 4, "rows: 3 aliases + 1 trigger + 1 shortcut");
        let mut empty = new();
        empty.cursor = 7;
        reclamp(&mut empty, &Store::default());
        assert_eq!(empty.cursor, 1, "clamped to the last of 2 built-ins");
    }

    #[test]
    fn navigation_moves_and_clamps() {
        let store = store_with_t();
        let aliases = view(&store);
        let flat = rows(&aliases, None);
        assert_eq!(move_sel(&flat, 0, -1), 0);
        assert_eq!(move_sel(&flat, 0, 1), 1);
        assert_eq!(move_sel(&flat, 2, 5), 2);
        assert_eq!(move_sel(&flat, 99, -1), 1);
        assert_eq!(move_sel(&[], 3, 1), 0);
    }

    #[test]
    fn enter_toggles_expansion_then_collapses() {
        let store = store_with_t();
        let (st, _, _) = handle_key(&new(), &store, key(KeyCode::Enter));
        assert_eq!(st.expanded, Some(0));
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Right));
        assert_eq!(st.expanded, None, "Enter again collapses");
        // expanding, landing on an entry row, then Left collapses too
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Enter));
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down));
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down));
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Left));
        assert_eq!(st.expanded, None);
    }

    #[test]
    fn q_and_esc_back_ctrl_c_quits() {
        let store = store_with_t();
        assert_eq!(
            handle_key(&new(), &store, key(KeyCode::Esc)).2,
            Effect::Back
        );
        assert_eq!(
            handle_key(&new(), &store, key(KeyCode::Char('q'))).2,
            Effect::Back
        );
        assert_eq!(
            handle_key(
                &new(),
                &store,
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
            )
            .2,
            Effect::Quit
        );
    }

    /// Alt+D (the default wake key) and the other modified chars must not
    /// reach the plain-char arms: only Ctrl+C / Ctrl+D quit from here.
    #[test]
    fn modified_char_keys_do_not_fire_list_actions() {
        let (st, store) = on_t();
        let alt_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT);
        assert_eq!(
            handle_key(&st, &store, alt_d),
            (st.clone(), store.clone(), Effect::None),
            "Alt+D must not delete the selection"
        );
        let alt_k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::ALT);
        assert_eq!(
            handle_key(&st, &store, alt_k),
            (st.clone(), store.clone(), Effect::None),
            "Alt+K must not move the selection"
        );

        for c in ['k', 'j', 'q', 'n', 's', 'a', 't', 'e'] {
            let ctrl = KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
            assert_eq!(
                handle_key(&st, &store, ctrl),
                (st.clone(), store.clone(), Effect::None),
                "Ctrl+{c} must not fire the list actions"
            );
        }
        assert_eq!(store.aliases.len(), 1, "the user alias survives");
    }

    #[test]
    fn alt_word_motions_reach_the_form_but_stay_dead_on_the_list() {
        let (st, store) = on_t();
        let alt_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT);
        assert_eq!(
            handle_key(&st, &store, alt_d),
            (st.clone(), store.clone(), Effect::None),
            "Alt+D must not delete the selection from the list"
        );

        // ...but inside the wizard Alt+B / Alt+F move the form caret.
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Char('e')));
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::End));
        let form = st.form.as_ref().expect("edit wizard open");
        assert_eq!(form.caret, "printf %s {input}".chars().count());

        let alt_b = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT);
        let (st, _, eff) = handle_key(&st, &store, alt_b);
        assert_eq!(eff, Effect::None);
        let form = st.form.as_ref().expect("form still open");
        assert_eq!(
            form.caret, 10,
            "Alt+B jumps to the start of the placeholder"
        );

        // an unmapped ALT key in the form neither edits nor quits
        let before = form.input.clone();
        let (st, _, eff) = handle_key(&st, &store, alt_d);
        assert_eq!(eff, Effect::None);
        let form = st.form.as_ref().expect("form still open");
        assert_eq!((form.input.clone(), form.caret), (before, 10));
    }

    #[test]
    fn ctrl_c_still_quits_from_the_list() {
        let (st, store) = on_t();
        let (_, _, eff) = handle_key(
            &st,
            &store,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert_eq!(eff, Effect::Quit);
        let (_, _, eff) = handle_key(
            &st,
            &store,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        );
        assert_eq!(eff, Effect::Quit);
    }

    #[test]
    fn release_events_are_ignored() {
        let store = store_with_t();
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        let (st, s2, eff) = handle_key(&new(), &store, release);
        assert_eq!((st, eff), (new(), Effect::None));
        assert_eq!(s2, store);
    }
}
