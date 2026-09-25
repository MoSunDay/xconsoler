//! Shared fixtures for the `settings_view` tests: a headless terminal and the
//! seeded `t` alias store. Split out so every test file stays small.

use super::*;
use crate::storage::Store;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

pub(super) fn frame_text(terminal: &Terminal<TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let mut s = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

pub(super) fn draw_once(st: &Settings, aliases: &[AliasDef]) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
    terminal.draw(|f| draw(f, st, aliases)).unwrap();
    frame_text(&terminal)
}

/// Same page on the real window width (xterm geometry is 140x14), where
/// the whole footer hint line fits.
pub(super) fn draw_wide(st: &Settings, aliases: &[AliasDef]) -> String {
    draw_at(140, 14, st, aliases)
}

/// The page at an arbitrary terminal size (the deployed bar is 52 wide).
pub(super) fn draw_at(w: u16, h: u16, st: &Settings, aliases: &[AliasDef]) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| draw(f, st, aliases)).unwrap();
    frame_text(&terminal)
}

pub(super) fn t_store() -> (Store, Vec<AliasDef>) {
    let mut store = Store::default();
    store.aliases.push(AliasDef {
        name: "t".to_string(),
        triggers: vec!["tt".to_string()],
        linux: Some("printf %s {input}".to_string()),
        macos: Some("printf %s {input}".to_string()),
        shortcuts: [("baidu".to_string(), "https://www.baidu.com".to_string())]
            .into_iter()
            .collect(),
    });
    let aliases = settings::view(&store);
    (store, aliases)
}
