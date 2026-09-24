//! TUI state: the [`App`] snapshot plus candidate-list helpers.
//!
//! The App is a plain data structure; all behaviour lives in free functions
//! (`crate::action` maps keys, `crate::app` mutates, `crate::render` draws).

use crate::alias::AliasDef;
use crate::keyspec::{self, KeySpec};
use crate::matcher::{self, Candidate};
use crate::storage::{self, Store};

/// Max candidates shown (and ranked) at once.
pub const CANDIDATE_LIMIT: usize = 10;

/// Whether the launcher bar is on screen. New apps start [`Visibility::Shown`]
/// — a fresh launch must be visible, not look like it exited instantly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Hidden,
    Shown,
}

/// Which page owns the event loop: the launcher bar or the `/settings`
/// screen (whose state lives in [`crate::settings`]).
#[derive(Debug, Clone)]
pub enum Mode {
    Normal,
    Settings(Box<crate::settings::Settings>),
}

/// Whole TUI state. `store` is the persisted part; `aliases` is the derived
/// `storage::merge_aliases` view used for resolution and rendering.
#[derive(Debug)]
pub struct App {
    pub store: Store,
    pub aliases: Vec<AliasDef>,
    pub input: String,
    /// Selected index into `state::candidates`.
    pub cursor: usize,
    pub visibility: Visibility,
    /// Transient message: `(ok?, text)`. Reset by most input changes.
    pub status: Option<(bool, String)>,
    pub quit: bool,
    /// The wake/sleep key, parsed from `store.config.wake_key`.
    pub wake: KeySpec,
    /// The command-palette key, parsed from `store.config.command_key`.
    pub command: KeySpec,
    /// Summon mode (started from a shell keybind): the wake key / Esc quit
    /// back to the prompt instead of hiding the bar.
    pub summon: bool,
    /// Which page owns keys/rendering: launcher bar or the settings screen.
    pub mode: Mode,
    /// Command palette: `Some(selected row)` while it is open. Only ever set
    /// while the bar is shown; `None` means closed.
    pub palette: Option<usize>,
}

/// Build an app from a loaded store: **shown**, empty input, merged aliases.
/// An unparsable stored wake key falls back to [`keyspec::DEFAULT`]; an
/// unparsable command-palette key to [`keyspec::DEFAULT_COMMAND`].
pub fn new(store: Store, summon: bool) -> App {
    let aliases = storage::merge_aliases(&store.aliases);
    let wake = keyspec::parse(&store.config.wake_key).unwrap_or(keyspec::DEFAULT);
    let command = keyspec::parse(&store.config.command_key).unwrap_or(keyspec::DEFAULT_COMMAND);
    App {
        store,
        aliases,
        input: String::new(),
        cursor: 0,
        visibility: Visibility::Shown,
        status: None,
        quit: false,
        wake,
        command,
        summon,
        mode: Mode::Normal,
        palette: None,
    }
}

/// Ranked candidates for the current input (trimmed), capped at
/// [`CANDIDATE_LIMIT`].
pub fn candidates(app: &App) -> Vec<Candidate> {
    // `<alias> <partial>` switches to that alias's named args; anything else
    // (including a bare alias) keeps the normal history/alias ranking.
    let args = matcher::arg_candidates(&app.aliases, &app.input, CANDIDATE_LIMIT);
    if args.is_empty() {
        matcher::candidates(&app.store, &app.aliases, app.input.trim(), CANDIDATE_LIMIT)
    } else {
        args
    }
}

/// The candidate the cursor points at. An out-of-range cursor falls back to
/// index 0 so Enter keeps doing something sensible after alias removals.
pub fn selected(app: &App) -> Option<Candidate> {
    let cands = candidates(app);
    let idx = if app.cursor < cands.len() {
        app.cursor
    } else {
        0
    };
    cands.get(idx).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::record;

    fn app_with_history() -> App {
        let mut store = Store::default();
        record(&mut store, "br", "a", 1);
        record(&mut store, "br", "b", 2);
        new(store, false)
    }

    fn app_with_named_arg() -> App {
        let mut store = Store::default();
        let mut def = crate::alias::defaults().remove(0); // br builtin
        def.args.clear(); // fixture: exactly the args below
        def.builtin = false;
        def.args
            .insert("baidu".to_string(), "https://www.baidu.com".to_string());
        store.aliases.push(def);
        new(store, false)
    }

    #[test]
    fn named_arg_context_lists_args_once_a_space_is_typed() {
        let mut app = app_with_named_arg();
        assert!(
            candidates(&app)
                .iter()
                .all(|c| !matches!(c, Candidate::Arg { .. })),
            "empty input: normal ranking"
        );
        app.input = "br".to_string();
        assert!(
            candidates(&app)
                .iter()
                .all(|c| !matches!(c, Candidate::Arg { .. })),
            "bare alias still ranks aliases"
        );
        app.input = "br ".to_string();
        assert_eq!(
            candidates(&app),
            vec![Candidate::Arg {
                alias: "br".to_string(),
                key: "baidu".to_string()
            }]
        );
        app.input = "br bai".to_string();
        assert_eq!(
            candidates(&app),
            vec![Candidate::Arg {
                alias: "br".to_string(),
                key: "baidu".to_string()
            }]
        );
        app.input = "br zzz".to_string();
        assert!(
            candidates(&app)
                .iter()
                .all(|c| !matches!(c, Candidate::Arg { .. })),
            "unmatched partial falls back to the normal ranking"
        );
    }

    #[test]
    fn new_starts_shown_with_merged_aliases() {
        let app = new(Store::default(), false);
        assert_eq!(app.visibility, Visibility::Shown);
        assert!(app.input.is_empty());
        assert_eq!(app.cursor, 0);
        assert_eq!(app.status, None);
        assert!(!app.quit);
        assert!(!app.summon);
        assert_eq!(app.wake, keyspec::parse(keyspec::DEFAULT_SPEC).unwrap());
        assert_eq!(
            app.command,
            keyspec::parse(keyspec::DEFAULT_COMMAND_SPEC).unwrap()
        );
        assert_eq!(app.palette, None);
        assert_eq!(app.aliases, storage::merge_aliases(&[]));
    }

    #[test]
    fn new_records_summon_and_parses_stored_wake_key() {
        let mut store = Store::default();
        store.config.wake_key = "ctrl+g".to_string();
        let app = new(store, true);
        assert!(app.summon);
        assert_eq!(app.visibility, Visibility::Shown);
        assert_eq!(app.wake, keyspec::parse("ctrl+g").unwrap());
    }

    #[test]
    fn invalid_stored_wake_key_falls_back_to_default() {
        let mut store = Store::default();
        store.config.wake_key = "garbage".to_string();
        let app = new(store, false);
        assert_eq!(app.wake, keyspec::DEFAULT);
    }

    #[test]
    fn stored_command_key_is_parsed_and_invalid_falls_back() {
        let mut store = Store::default();
        store.config.command_key = "ctrl+o".to_string();
        assert_eq!(new(store, false).command, keyspec::parse("ctrl+o").unwrap());

        let mut store = Store::default();
        store.config.command_key = "garbage".to_string();
        assert_eq!(new(store, false).command, keyspec::DEFAULT_COMMAND);
    }

    /// Empty input lists the recent history only: aliases come back as soon
    /// as a query character is typed.
    #[test]
    fn empty_input_lists_recent_history_only() {
        let app = app_with_history();
        assert_eq!(
            candidates(&app),
            vec![Candidate::History { idx: 0 }, Candidate::History { idx: 1 },]
        );
    }

    #[test]
    fn candidates_respect_limit() {
        let app = app_with_history();
        assert!(candidates(&app).len() <= CANDIDATE_LIMIT);
    }

    /// The twist only applies to the empty bar: a typed query ranks history
    /// and aliases together again.
    #[test]
    fn typed_query_brings_aliases_back() {
        let mut app = app_with_history();
        app.input = "br".to_string();
        let out = candidates(&app);
        assert!(out.contains(&Candidate::History { idx: 0 }), "{out:?}");
        assert!(
            out.contains(&Candidate::Alias {
                name: "br".to_string()
            }),
            "{out:?}"
        );
    }

    #[test]
    fn empty_input_without_history_is_empty_despite_aliases() {
        let app = new(Store::default(), false);
        assert!(candidates(&app).is_empty());
        assert_eq!(selected(&app), None);
    }

    #[test]
    fn selected_follows_cursor() {
        let mut app = app_with_history();
        app.cursor = 1;
        assert_eq!(selected(&app), Some(Candidate::History { idx: 1 }));
    }

    #[test]
    fn selected_out_of_range_cursor_falls_back_to_first() {
        let mut app = app_with_history();
        app.cursor = 99;
        assert_eq!(selected(&app), Some(Candidate::History { idx: 0 }));
    }

    #[test]
    fn selected_none_when_no_candidates_and_no_match() {
        let mut app = new(Store::default(), false);
        app.aliases.clear();
        assert_eq!(selected(&app), None);
    }
}
