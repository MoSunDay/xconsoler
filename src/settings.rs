//! `/settings` page state and key handling: the alias table (list screen)
//! plus dispatch into the wizard form ([`crate::settings_form`]).
//!
//! Everything here is a pure transition:
//! `handle_key(state, store, key) -> (new state, new store, effect)` — no
//! I/O, no clocks. Persistence stays with the caller (`app::apply_settings`
//! runs `storage::save` when the effect says [`Effect::Save`]).

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::alias::{self, AliasDef};
use crate::settings_form::{self, Form, FormOutcome, Submission};
use crate::storage::{self, Store};

/// What the caller should do after a key transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Nothing changed.
    None,
    /// Leave settings mode, back to the launcher bar.
    Back,
    /// The store changed; persist it.
    Save,
    /// Quit the app.
    Quit,
}

/// One flattened, selectable row of the list screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Alias { idx: usize },
    Arg { alias: usize, key: String },
}

/// Settings page state. `cursor` indexes [`rows`]; `expanded` is the alias
/// whose named args are shown indented below it.
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

/// Flattened rows: one per alias, plus indented arg rows for `expanded`.
pub fn rows(aliases: &[AliasDef], expanded: Option<usize>) -> Vec<Row> {
    let mut out = Vec::new();
    for (idx, def) in aliases.iter().enumerate() {
        out.push(Row::Alias { idx });
        if expanded == Some(idx) {
            for key in def.args.keys() {
                out.push(Row::Arg {
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
        KeyCode::Esc | KeyCode::Char('q') => effect = Effect::Back,
        KeyCode::Up | KeyCode::Char('k') => next.cursor = move_sel(&cur_rows, st.cursor, -1),
        KeyCode::Down | KeyCode::Char('j') => next.cursor = move_sel(&cur_rows, st.cursor, 1),
        KeyCode::Enter | KeyCode::Right => next.expanded = toggle(&cur_rows, st),
        // ← is a pure collapse: it never opens a different alias by accident.
        KeyCode::Left => next.expanded = None,
        KeyCode::Char('n') => next.form = Some(settings_form::new_alias()),
        KeyCode::Char('a') => {
            next.form = selected_alias(&cur_rows, st, &aliases)
                .map(|name| settings_form::new_arg(&name))
        }
        KeyCode::Char('d') => {
            effect = delete_row(&cur_rows, st, &mut store, &mut next);
        }
        _ => {}
    }
    clamp_cursor(&mut next, &view(&store));
    (next, store, effect)
}

/// Enter/→ expands the selected alias (or collapses when already open,
/// and on an arg row collapses too); ← collapses whatever is open.
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

/// Name of the alias the selection belongs to (an arg row's parent).
fn selected_alias(rows: &[Row], st: &Settings, aliases: &[AliasDef]) -> Option<String> {
    let idx = match rows.get(st.cursor) {
        Some(Row::Alias { idx }) => *idx,
        Some(Row::Arg { alias, .. }) => *alias,
        None => return None,
    };
    aliases.get(idx).map(|d| d.name.clone())
}

/// Move by `delta`, clamped into `rows`.
fn move_sel(rows: &[Row], cur: usize, delta: i32) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let cur = cur.min(rows.len() - 1) as i32;
    (cur + delta).clamp(0, rows.len() as i32 - 1) as usize
}

/// `d`: delete the selection — an arg row loses just that argument, an
/// alias row loses the whole alias (built-ins refuse, like `:del`).
fn delete_row(
    rows: &[Row],
    st: &Settings,
    store: &mut Store,
    next: &mut Settings,
) -> Effect {
    let aliases = view(store);
    match rows.get(st.cursor) {
        Some(Row::Arg { alias, key }) => {
            let name = aliases.get(*alias).map(|d| d.name.clone()).unwrap_or_default();
            match alias::remove_arg(&mut store.aliases, &name, key) {
                Ok(()) => {
                    next.status = Some((true, format!("arg removed: {key}")));
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

fn form_key(
    st: &Settings,
    form: &Form,
    store: &Store,
    key: KeyEvent,
) -> (Settings, Store, Effect) {
    let (next_form, outcome) = settings_form::handle_key(form, store, key);
    let list = |form: Option<Form>, status: Option<(bool, String)>| Settings {
        form,
        cursor: st.cursor,
        expanded: st.expanded,
        status,
    };
    match outcome {
        FormOutcome::Active => (list(Some(next_form), st.status.clone()), store.clone(), Effect::None),
        FormOutcome::Quit => (list(Some(next_form), st.status.clone()), store.clone(), Effect::Quit),
        FormOutcome::Cancel => (list(None, None), store.clone(), Effect::None),
        FormOutcome::Submit(sub) => {
            let mut store = store.clone();
            match apply_submission(&mut store, sub) {
                Ok(msg) => (list(None, Some((true, msg))), store, Effect::Save),
                Err(e) => {
                    let mut f = next_form;
                    f.error = Some(e);
                    (list(Some(f), st.status.clone()), store, Effect::None)
                }
            }
        }
    }
}

/// Apply a finished wizard submission to the store. Ok carries the status
/// message for the list screen.
fn apply_submission(store: &mut Store, sub: Submission) -> Result<String, String> {
    match sub {
        Submission::Alias(def) => {
            let label = alias::label(&def);
            match store.aliases.iter().position(|d| d.name == def.name) {
                Some(i) => store.aliases[i] = def,
                None => store.aliases.push(def),
            }
            Ok(format!("alias added: {label}"))
        }
        Submission::Arg { alias, key, value } => {
            match alias::set_arg(&mut store.aliases, &alias, &key, &value) {
                Ok(Some(_)) => Ok(format!("arg set: {alias}.{key} (replaced previous value)")),
                Ok(None) => Ok(format!("arg set: {alias}.{key} = {value}")),
                Err(e) => Err(e),
            }
        }
    }
}

/// Keep the cursor inside the (possibly shrunken) row list.
fn clamp_cursor(st: &mut Settings, aliases: &[AliasDef]) {
    let len = rows(aliases, st.expanded).len();
    st.cursor = st.cursor.min(len.saturating_sub(1));
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
            shortcuts: vec!["tt".to_string()],
            linux: Some("printf %s {input}".to_string()),
            macos: None,
            args: [("baidu".to_string(), "https://www.baidu.com".to_string())].into_iter().collect(),
            builtin: false,
        });
        store
    }

    #[test]
    fn rows_list_aliases_then_expanded_args() {
        let store = store_with_t();
        let aliases = view(&store);
        let flat = rows(&aliases, None);
        assert_eq!(flat.len(), 3); // browser, clipboard, t
        let expanded = rows(&aliases, Some(2));
        assert_eq!(
            expanded,
            vec![
                Row::Alias { idx: 0 },
                Row::Alias { idx: 1 },
                Row::Alias { idx: 2 },
                Row::Arg { alias: 2, key: "baidu".to_string() },
            ]
        );
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
        // expanding, landing on the arg row, then Left collapses too
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Enter));
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down));
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Down));
        let (st, _, _) = handle_key(&st, &store, key(KeyCode::Left));
        assert_eq!(st.expanded, None);
    }

    #[test]
    fn q_and_esc_back_ctrl_c_quits() {
        let store = store_with_t();
        assert_eq!(handle_key(&new(), &store, key(KeyCode::Esc)).2, Effect::Back);
        assert_eq!(handle_key(&new(), &store, key(KeyCode::Char('q'))).2, Effect::Back);
        assert_eq!(
            handle_key(&new(), &store, KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)).2,
            Effect::Quit
        );
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
