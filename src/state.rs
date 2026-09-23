//! TUI state: the [`App`] snapshot plus candidate-list helpers.
//!
//! The App is a plain data structure; all behaviour lives in free functions
//! (`crate::action` maps keys, `crate::app` mutates, `crate::render` draws).

use crate::alias::AliasDef;
use crate::matcher::{self, Candidate};
use crate::storage::{self, Store};

/// Max candidates shown (and ranked) at once.
pub const CANDIDATE_LIMIT: usize = 8;

/// Whether the launcher bar is on screen. Starts [`Visibility::Hidden`] so a
/// globally-bound hotkey (tmux `M-d`) can wake it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Hidden,
    Shown,
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
}

/// Build an app from a loaded store: hidden, empty input, merged aliases.
pub fn new(store: Store) -> App {
    let aliases = storage::merge_aliases(&store.aliases);
    App {
        store,
        aliases,
        input: String::new(),
        cursor: 0,
        visibility: Visibility::Hidden,
        status: None,
        quit: false,
    }
}

/// Ranked candidates for the current input (trimmed), capped at
/// [`CANDIDATE_LIMIT`].
pub fn candidates(app: &App) -> Vec<Candidate> {
    matcher::candidates(&app.store, &app.aliases, app.input.trim(), CANDIDATE_LIMIT)
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
        record(&mut store, "browser", "a", 1);
        record(&mut store, "browser", "b", 2);
        new(store)
    }

    #[test]
    fn new_starts_hidden_with_merged_aliases() {
        let app = new(Store::default());
        assert_eq!(app.visibility, Visibility::Hidden);
        assert!(app.input.is_empty());
        assert_eq!(app.cursor, 0);
        assert_eq!(app.status, None);
        assert!(!app.quit);
        assert_eq!(app.aliases, storage::merge_aliases(&[]));
    }

    #[test]
    fn empty_input_lists_history_then_aliases() {
        let app = app_with_history();
        assert_eq!(
            candidates(&app),
            vec![
                Candidate::History { idx: 0 },
                Candidate::History { idx: 1 },
                Candidate::Alias {
                    name: "browser".to_string()
                },
                Candidate::Alias {
                    name: "clipboard".to_string()
                },
            ]
        );
    }

    #[test]
    fn candidates_respect_limit() {
        let app = app_with_history();
        assert!(candidates(&app).len() <= CANDIDATE_LIMIT);
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
        let mut app = new(Store::default());
        app.aliases.clear();
        assert_eq!(selected(&app), None);
    }
}
