//! Effect application for the `/settings` page: the impure-ish half of the
//! page. The state machine ([`crate::settings`]) stays a pure transition and
//! hands every store mutation over as an [`Effect`]; this module runs the
//! `alias::` helpers, reports the outcome on the list's status line and
//! persists through `storage::save` — one save path for every edit.
//!
//! Still no I/O of its own beyond the store file, and no UI code: it is the
//! former `app::apply_settings` body, kept out of `app.rs` so both modules
//! stay reviewable.

use std::path::Path;

use crossterm::event::KeyEvent;

use crate::alias;
use crate::settings::{self, Effect};
use crate::state::{App, Mode};
use crate::storage::{self, Store};

/// Apply one effect to the store. `Some(Ok(msg))` when the store changed
/// (msg goes to the status line), `Some(Err(msg))` when the helper refused
/// (store untouched), `None` for structural effects that carry no data.
pub fn apply_effect(store: &mut Store, effect: &Effect) -> Option<Result<String, String>> {
    match effect {
        Effect::AddAlias(def) => {
            let label = alias::label(def);
            match store.aliases.iter().position(|d| d.name == def.name) {
                Some(i) => store.aliases[i] = def.clone(),
                None => store.aliases.push(def.clone()),
            }
            Some(Ok(format!("alias added: {label}")))
        }
        Effect::SetShortcut { alias, key, value } => Some(
            match alias::set_shortcut(&mut store.aliases, alias, key, value) {
                Ok(Some(_)) => Ok(format!(
                    "shortcut set: {alias}.{key} (replaced previous value)"
                )),
                Ok(None) => Ok(format!("shortcut set: {alias}.{key} = {value}")),
                Err(e) => Err(e),
            },
        ),
        Effect::AddTrigger { alias, trigger } => Some(
            match alias::add_trigger(&mut store.aliases, alias, trigger) {
                Ok(true) => Ok(format!("trigger added: {alias} ({trigger})")),
                Ok(false) => Ok(format!("trigger already on {alias}: {trigger}")),
                Err(e) => Err(e),
            },
        ),
        Effect::RemoveTrigger { alias, trigger } => Some(
            match alias::remove_trigger(&mut store.aliases, alias, trigger) {
                Ok(true) => Ok(format!("trigger removed: {alias} ({trigger})")),
                Ok(false) => Err(format!("trigger not found on {alias}: {trigger}")),
                Err(e) => Err(e),
            },
        ),
        Effect::SetCommands {
            alias,
            linux,
            macos,
        } => Some(
            match alias::set_commands(&mut store.aliases, alias, linux, macos) {
                Ok(()) => Ok(format!("commands updated: {alias}")),
                Err(e) => Err(e),
            },
        ),
        Effect::None | Effect::Back | Effect::Save | Effect::Quit => None,
    }
}

/// Resolve an effect into the structural effect to run plus the status line
/// update to show. Data-carrying effects become [`Effect::Save`] (ok) or
/// [`Effect::None`] (refused), so a failed wizard never persists anything.
fn resolve(store: &mut Store, effect: Effect) -> (Effect, Option<(bool, String)>) {
    match apply_effect(store, &effect) {
        Some(Ok(msg)) => (Effect::Save, Some((true, msg))),
        Some(Err(e)) => (Effect::None, Some((false, e))),
        None => (effect, None),
    }
}

/// Route one key event to the settings page, apply its effect, persist when
/// the store changed. The store returned by the pure transition replaces
/// `app.store` wholesale.
pub fn apply(app: &mut App, key: KeyEvent, path: &Path) {
    let Mode::Settings(st) = std::mem::replace(&mut app.mode, Mode::Normal) else {
        return;
    };
    let (mut st, store, effect) = settings::handle_key(&st, &app.store, key);
    app.store = store;
    let (effect, status) = resolve(&mut app.store, effect);
    if status.is_some() {
        st.status = status;
    }
    settings::reclamp(&mut st, &app.store);
    match effect {
        Effect::None => app.mode = Mode::Settings(Box::new(st)),
        Effect::Back => {}
        Effect::Save => {
            app.aliases = storage::merge_aliases(&app.store.aliases);
            // A failed save must not be reported as a successful edit.
            if let Err(e) = storage::save(path, &app.store) {
                st.status = Some((false, format!("save failed: {e}")));
            }
            app.mode = Mode::Settings(Box::new(st));
        }
        Effect::Quit => app.quit = true,
        // `resolve` turned every data-carrying effect into Save / None.
        Effect::AddAlias(_)
        | Effect::SetShortcut { .. }
        | Effect::AddTrigger { .. }
        | Effect::RemoveTrigger { .. }
        | Effect::SetCommands { .. } => app.mode = Mode::Settings(Box::new(st)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use crate::state;
    use crossterm::event::{KeyCode, KeyModifiers};
    use tempfile::TempDir;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    /// App on the settings page, cursor on the first merged alias (`br`).
    fn setup() -> (App, TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("store.json");
        let mut app = state::new(Store::default(), false);
        app.mode = Mode::Settings(Box::new(Settings {
            cursor: 0,
            ..settings::new()
        }));
        (app, dir, path)
    }

    fn type_str(app: &mut App, s: &str, path: &Path) {
        for c in s.chars() {
            apply(app, key(KeyCode::Char(c)), path);
        }
    }

    fn status(app: &App) -> Option<(bool, String)> {
        match &app.mode {
            Mode::Settings(st) => st.status.clone(),
            Mode::Normal => None,
        }
    }

    fn form_open(app: &App) -> bool {
        matches!(&app.mode, Mode::Settings(st) if st.form.is_some())
    }

    #[test]
    fn structural_effects_are_left_alone() {
        let mut store = Store::default();
        assert_eq!(apply_effect(&mut store, &Effect::None), None);
        assert_eq!(apply_effect(&mut store, &Effect::Back), None);
        assert_eq!(apply_effect(&mut store, &Effect::Save), None);
        assert_eq!(apply_effect(&mut store, &Effect::Quit), None);
    }

    #[test]
    fn a_failed_save_reports_the_error_instead_of_success() {
        let (mut app, dir, _good) = setup();
        // A path whose parent is a plain file: `storage::save` cannot write.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let bad = blocker.join("store.json");

        apply(&mut app, key(KeyCode::Char('s')), &bad);
        type_str(&mut app, "gc", &bad);
        apply(&mut app, key(KeyCode::Enter), &bad); // key -> value step
        type_str(&mut app, "git clone {input}", &bad);
        apply(&mut app, key(KeyCode::Enter), &bad);

        // The edit happened in memory, but the status says the save failed.
        assert_eq!(
            app.store.aliases[0].shortcuts.get("gc").map(String::as_str),
            Some("git clone {input}")
        );
        let (ok, msg) = status(&app).expect("a status line");
        assert!(!ok, "a failed save must not look like success");
        assert!(msg.starts_with("save failed:"), "got {msg}");
    }

    #[test]
    fn shortcut_wizard_adds_and_persists_through_the_save_path() {
        let (mut app, _dir, path) = setup();
        apply(&mut app, key(KeyCode::Char('s')), &path);
        assert!(form_open(&app), "`s` opens the shortcut wizard");
        type_str(&mut app, "gc", &path);
        apply(&mut app, key(KeyCode::Enter), &path); // key -> value step
        type_str(&mut app, "git clone {input}", &path);
        apply(&mut app, key(KeyCode::Enter), &path);

        let def = app.store.aliases.first().expect("br materialised");
        assert_eq!(def.name, "br");
        assert!(!def.builtin, "a builtin override, like :arg does");
        assert_eq!(
            def.shortcuts.get("gc").map(String::as_str),
            Some("git clone {input}")
        );
        assert!(status(&app).is_some_and(|(ok, m)| ok && m.contains("shortcut set")));
        assert!(!form_open(&app), "the wizard closed");
        assert!(app.aliases.iter().any(|d| d.shortcuts.contains_key("gc")));

        let reloaded = storage::load(&path);
        assert_eq!(
            reloaded.aliases[0].shortcuts.get("gc").map(String::as_str),
            Some("git clone {input}"),
            "same save path as the other edits"
        );
    }

    #[test]
    fn trigger_collision_lands_on_the_status_line() {
        let (mut app, _dir, path) = setup();
        // `cd` already answers to `cd`, so add_trigger refuses
        apply(&mut app, key(KeyCode::Char('t')), &path);
        type_str(&mut app, "cd", &path);
        apply(&mut app, key(KeyCode::Enter), &path);

        let (ok, msg) = status(&app).expect("the failure is reported");
        assert!(!ok);
        assert!(msg.contains("already used by cd"), "got: {msg}");
        assert!(
            app.store.aliases.is_empty(),
            "a refused edit changes nothing"
        );
        assert!(!path.exists(), "nothing persisted");
        assert!(!form_open(&app), "back on the list, error above the hints");
    }

    #[test]
    fn edit_wizard_can_rewrite_both_commands() {
        let (mut app, _dir, path) = setup();
        apply(&mut app, key(KeyCode::Char('e')), &path);
        assert!(form_open(&app));
        apply(&mut app, ctrl('u'), &path); // clear the prefilled linux
        type_str(&mut app, "echo {input}", &path);
        apply(&mut app, key(KeyCode::Enter), &path); // accept -> step 2
        apply(&mut app, ctrl('u'), &path); // clear the prefilled macos
        type_str(&mut app, "open {input}", &path);
        apply(&mut app, key(KeyCode::Enter), &path);

        let def = app.store.aliases.first().expect("br materialised");
        assert_eq!(def.linux.as_deref(), Some("echo {input}"));
        assert_eq!(def.macos.as_deref(), Some("open {input}"));
        assert!(status(&app).is_some_and(|(ok, m)| ok && m.contains("commands updated")));

        let reloaded = storage::load(&path);
        assert_eq!(reloaded.aliases[0].linux.as_deref(), Some("echo {input}"));
        assert_eq!(reloaded.aliases[0].macos.as_deref(), Some("open {input}"));
    }

    #[test]
    fn edit_wizard_blank_macos_mirrors_linux() {
        let (mut app, _dir, path) = setup();
        apply(&mut app, key(KeyCode::Char('e')), &path);
        apply(&mut app, ctrl('u'), &path);
        type_str(&mut app, "printf %s {input}", &path);
        apply(&mut app, key(KeyCode::Enter), &path);
        apply(&mut app, ctrl('u'), &path); // blank macos step
        apply(&mut app, key(KeyCode::Enter), &path);

        let def = app.store.aliases.first().expect("br materialised");
        assert_eq!(def.linux.as_deref(), Some("printf %s {input}"));
        assert_eq!(
            def.macos.as_deref(),
            Some("printf %s {input}"),
            "blank macos mirrors linux (alias::set_commands)"
        );
    }

    #[test]
    fn d_on_a_shortcut_row_removes_and_saves_it() {
        let (mut app, _dir, path) = setup();
        apply(&mut app, key(KeyCode::Char('s')), &path);
        type_str(&mut app, "gc", &path);
        apply(&mut app, key(KeyCode::Enter), &path); // key -> value step
        type_str(&mut app, "git clone {input}", &path);
        apply(&mut app, key(KeyCode::Enter), &path);

        apply(&mut app, key(KeyCode::Enter), &path); // expand br
        apply(&mut app, key(KeyCode::Down), &path); // baidu shortcut row
        apply(&mut app, key(KeyCode::Down), &path); // gc shortcut row
        apply(&mut app, key(KeyCode::Char('d')), &path);

        assert!(!app.store.aliases[0].shortcuts.contains_key("gc"));
        assert!(app.store.aliases[0].shortcuts.contains_key("baidu"));
        assert!(status(&app).is_some_and(|(ok, m)| ok && m.contains("shortcut removed")));
        assert!(!storage::load(&path).aliases[0].shortcuts.contains_key("gc"));
    }

    #[test]
    fn d_on_a_trigger_row_removes_and_saves_it() {
        let (mut app, _dir, path) = setup();
        apply(&mut app, key(KeyCode::Char('t')), &path);
        type_str(&mut app, "tt", &path);
        apply(&mut app, key(KeyCode::Enter), &path);

        apply(&mut app, key(KeyCode::Enter), &path); // expand br
        apply(&mut app, key(KeyCode::Down), &path); // trigger row
        apply(&mut app, key(KeyCode::Char('d')), &path);

        assert!(app.store.aliases[0].triggers.is_empty());
        assert!(status(&app).is_some_and(|(ok, m)| ok && m.contains("trigger removed")));
        assert!(storage::load(&path).aliases[0].triggers.is_empty());
    }

    #[test]
    fn remove_trigger_of_a_missing_entry_reports_an_error() {
        let mut store = Store::default();
        let out = apply_effect(
            &mut store,
            &Effect::RemoveTrigger {
                alias: "br".to_string(),
                trigger: "nope".to_string(),
            },
        );
        assert!(out.unwrap().is_err(), "unknown trigger is refused");
        assert!(store.aliases.is_empty(), "nothing was materialised");
    }

    #[test]
    fn set_commands_on_an_unknown_alias_reports_an_error() {
        let mut store = Store::default();
        let out = apply_effect(
            &mut store,
            &Effect::SetCommands {
                alias: "nope".to_string(),
                linux: "echo".to_string(),
                macos: String::new(),
            },
        );
        assert!(out.unwrap().is_err());
    }

    #[test]
    fn add_alias_and_set_shortcut_keep_their_messages() {
        let mut store = Store::default();
        let def = crate::alias::AliasDef {
            name: "t".to_string(),
            triggers: vec![],
            linux: Some("printf %s {input}".to_string()),
            macos: Some("printf %s {input}".to_string()),
            shortcuts: Default::default(),
            builtin: false,
        };
        let out = apply_effect(&mut store, &Effect::AddAlias(def)).unwrap();
        assert_eq!(out.unwrap(), "alias added: t");
        let out = apply_effect(
            &mut store,
            &Effect::SetShortcut {
                alias: "t".to_string(),
                key: "here".to_string(),
                value: "cd /tmp".to_string(),
            },
        )
        .unwrap();
        assert_eq!(out.unwrap(), "shortcut set: t.here = cd /tmp");
        let out = apply_effect(
            &mut store,
            &Effect::SetShortcut {
                alias: "t".to_string(),
                key: "here".to_string(),
                value: "cd /var".to_string(),
            },
        )
        .unwrap();
        assert_eq!(
            out.unwrap(),
            "shortcut set: t.here (replaced previous value)"
        );
    }

    #[test]
    fn add_and_remove_trigger_report_their_messages() {
        let mut store = Store::default();
        let out = apply_effect(
            &mut store,
            &Effect::AddTrigger {
                alias: "br".to_string(),
                trigger: "tt".to_string(),
            },
        )
        .unwrap();
        assert_eq!(out.unwrap(), "trigger added: br (tt)");
        // Adding the same word again is a no-op, not an error.
        let out = apply_effect(
            &mut store,
            &Effect::AddTrigger {
                alias: "br".to_string(),
                trigger: "TT".to_string(),
            },
        )
        .unwrap();
        assert_eq!(out.unwrap(), "trigger already on br: TT");
        let out = apply_effect(
            &mut store,
            &Effect::RemoveTrigger {
                alias: "br".to_string(),
                trigger: "tt".to_string(),
            },
        )
        .unwrap();
        assert_eq!(out.unwrap(), "trigger removed: br (tt)");
    }
}
