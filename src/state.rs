//! TUI state: the [`App`] snapshot plus candidate-list helpers.
//!
//! The App is a plain data structure; all behaviour lives in free functions
//! (`crate::action` maps keys, `crate::app` mutates, `crate::render` draws).

use crate::alias::AliasDef;
use crate::keyspec::{self, KeySpec};
use crate::matcher::{self, Candidate};
use crate::storage::Store;

/// Max history rows on the empty bar (most recent successful runs first).
pub const RECENT_LIMIT: usize = 10;
/// Max candidates ranked once the input is non-empty.
pub const CANDIDATE_LIMIT: usize = 5;
/// Tallest the desktop bar window is ever summoned to (rows): the 6 fixed
/// rows above the list (3-row input box + 2-row title/borders + 1-row status)
/// plus every row [`RECENT_LIMIT`] lets the list show. Derived, not
/// hard-coded, so the cap can never silently drift from the list geometry.
pub const MAX_BAR_ROWS: u16 = (6 + RECENT_LIMIT) as u16;

/// Rows the desktop bar wants to fit a list showing `rows` entries: the 6
/// fixed rows (3-row input box + 2 rows of candidate-list frame/title +
/// 1-row status line) plus one row per entry (up to [`RECENT_LIMIT`]) -
/// `rows` is the recent-history count on the empty bar, the live candidate
/// count once the bar follows its list. An empty list gets the bare box;
/// the result never exceeds [`MAX_BAR_ROWS`]. Exposed headlessly as
/// `--print-rows`, so `scripts/xc-bar` can size the terminal without a
/// scripting runtime.
pub fn bar_rows(rows: usize) -> u16 {
    if rows == 0 {
        4
    } else {
        (6 + rows.min(RECENT_LIMIT) as u16).min(MAX_BAR_ROWS)
    }
}

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

/// Whole TUI state. `store` is the persisted part; `aliases` mirrors its
/// stored list (the seeded defaults are ordinary entries there) and is used
/// for resolution and rendering.
#[derive(Debug)]
pub struct App {
    pub store: Store,
    pub aliases: Vec<AliasDef>,
    pub input: String,
    /// Text caret: char index into `input` (`0..=char_count`), owned by
    /// `crate::textedit`. Distinct from `cursor`, the candidate highlight.
    pub caret: usize,
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

/// Build an app from a loaded store: **shown**, empty input, the stored
/// aliases verbatim. An unparsable stored wake key falls back to
/// [`keyspec::DEFAULT`]; an unparsable command-palette key to
/// [`keyspec::DEFAULT_COMMAND`]; a pending `load_notice` becomes the startup
/// status message.
pub fn new(mut store: Store, summon: bool) -> App {
    let aliases = store.aliases.clone();
    let wake = keyspec::parse(&store.config.wake_key).unwrap_or(keyspec::DEFAULT);
    let command = keyspec::parse(&store.config.command_key).unwrap_or(keyspec::DEFAULT_COMMAND);
    let status = store.load_notice.take().map(|notice| (false, notice));
    App {
        store,
        aliases,
        input: String::new(),
        caret: 0,
        cursor: 0,
        visibility: Visibility::Shown,
        status,
        quit: false,
        wake,
        command,
        summon,
        mode: Mode::Normal,
        palette: None,
    }
}

/// Ranked candidates for the current input. An empty input lists up to
/// [`RECENT_LIMIT`] recent history rows; a typed input lists up to
/// [`CANDIDATE_LIMIT`]. Either way history outranks every shortcut row:
/// [`matcher::ranked_candidates`] owns that rule, so the empty bar and the
/// typed bar cannot drift apart.
pub fn candidates(app: &App) -> Vec<Candidate> {
    let query = app.input.trim();
    // `:`/`/` inputs are command lines (palette or validation), never alias
    // queries: the urls inside shortcut values would otherwise match.
    if query.starts_with('/') || query.starts_with(':') {
        return Vec::new();
    }
    let limit = if query.is_empty() {
        RECENT_LIMIT
    } else {
        CANDIDATE_LIMIT
    };
    // The raw input, not `query`: `<alias> <partial>` needs its space to tell
    // the alias-scoped shortcut rows apart from a plain fuzzy query.
    matcher::ranked_candidates(&app.store, &app.aliases, &app.input, limit)
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

    fn app_with_shortcut() -> App {
        let mut store = Store::default();
        // The seeded `br` carries exactly the shortcuts below in this fixture.
        let br = store
            .aliases
            .iter_mut()
            .find(|d| d.name == "br")
            .expect("br is seeded");
        br.shortcuts.clear();
        br.shortcuts
            .insert("baidu".to_string(), "https://www.baidu.com".to_string());
        new(store, false)
    }

    /// `<alias> <partial>` lists that alias's concrete shortcut keys once a
    /// space is typed. The fixture has no history, so no history row can
    /// outrank them here - see `typed_alias_partial_keeps_history_first`.
    #[test]
    fn shortcut_context_lists_shortcuts_once_a_space_is_typed() {
        let mut app = app_with_shortcut();
        assert!(
            candidates(&app).is_empty(),
            "empty input with no history: no rows"
        );
        app.input = "br".to_string();
        assert_eq!(
            candidates(&app),
            vec![Candidate::Shortcut {
                alias: "br".to_string(),
                key: "baidu".to_string()
            }],
            "bare alias ranks its concrete shortcuts"
        );
        app.input = "br ".to_string();
        assert_eq!(
            candidates(&app),
            vec![Candidate::Shortcut {
                alias: "br".to_string(),
                key: "baidu".to_string()
            }]
        );
        app.input = "br bai".to_string();
        assert_eq!(
            candidates(&app),
            vec![Candidate::Shortcut {
                alias: "br".to_string(),
                key: "baidu".to_string()
            }]
        );
        app.input = "br zzz".to_string();
        assert!(
            candidates(&app)
                .iter()
                .all(|c| !matches!(c, Candidate::Shortcut { .. })),
            "unmatched partial falls back to the normal ranking"
        );
    }

    /// History has the highest priority, so the `<alias> <partial>` picker
    /// keeps a matching recorded run above the alias's own shortcut keys.
    #[test]
    fn typed_alias_partial_keeps_history_first() {
        let mut app = app_with_shortcut();
        record(&mut app.store, "br", "baidu", 1);

        app.input = "br b".to_string();
        assert_eq!(
            candidates(&app),
            vec![Candidate::History { idx: 0 }],
            "the recorded run leads; its duplicate key row drops"
        );
        assert_eq!(selected(&app), Some(Candidate::History { idx: 0 }));
    }

    #[test]
    fn new_starts_shown_with_the_stored_aliases() {
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
        assert_eq!(app.aliases, crate::alias::defaults());
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
    fn bar_rows_follow_recent_history() {
        assert_eq!(bar_rows(0), 4);
        assert_eq!(bar_rows(1), 7);
        assert_eq!(bar_rows(RECENT_LIMIT), MAX_BAR_ROWS);
        assert_eq!(bar_rows(10_000), MAX_BAR_ROWS);
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

    /// Empty input lists the recent history only: shortcuts come back as
    /// soon as a query character is typed.
    #[test]
    fn empty_input_lists_recent_history_only() {
        let app = app_with_history();
        assert_eq!(
            candidates(&app),
            vec![Candidate::History { idx: 0 }, Candidate::History { idx: 1 },]
        );
    }

    /// The empty bar respects the recent-history cap: shortcuts come back
    /// as soon as a query character is typed.
    #[test]
    fn empty_bar_lists_at_most_recent_limit() {
        let app = app_with_history();
        assert!(candidates(&app).len() <= RECENT_LIMIT);
    }

    /// A typed query caps at `CANDIDATE_LIMIT` and fills the slots with the
    /// newest matching history entries before any shortcut row.
    #[test]
    fn typed_query_lists_at_most_five_history_first() {
        let mut store = Store::default();
        // Record oldest -> newest so `history[0]` is `hit0` once the loop ends.
        for i in (0..8).rev() {
            record(&mut store, "br", &format!("hit{i}"), (8 - i) as u64);
        }
        let mut app = new(store, false);
        app.input = "hit".to_string();
        let out = candidates(&app);
        assert_eq!(out.len(), CANDIDATE_LIMIT);
        assert_eq!(out[0], Candidate::History { idx: 0 });
        assert_eq!(out[4], Candidate::History { idx: 4 });
    }

    /// The twist only applies to the empty bar: a typed query brings
    /// concrete shortcuts back, after every matching history entry.
    #[test]
    fn typed_query_brings_shortcuts_back() {
        let mut app = app_with_history();
        app.input = "br".to_string();
        let out = candidates(&app);
        assert!(out.contains(&Candidate::History { idx: 0 }), "{out:?}");
        assert!(
            out.contains(&Candidate::Shortcut {
                alias: "br".to_string(),
                key: "baidu".to_string()
            }),
            "{out:?}"
        );
    }

    #[test]
    fn empty_input_without_history_is_empty_despite_shortcuts() {
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

    #[test]
    fn load_notice_becomes_a_one_time_status_message() {
        let store = Store {
            load_notice: Some("corrupt store moved to /tmp/store.json.corrupt".to_string()),
            ..Store::default()
        };
        let app = new(store, false);
        assert_eq!(
            app.status,
            Some((
                false,
                "corrupt store moved to /tmp/store.json.corrupt".to_string()
            ))
        );
        assert!(
            app.store.load_notice.is_none(),
            "the notice is consumed once"
        );
    }
}
