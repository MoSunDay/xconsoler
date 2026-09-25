//! tmux half of the window fit: getting a resize request *through* tmux.
//!
//! Inside a pane tmux eats the resize escape (`CSI 8 ; rows ; cols t`): it
//! resizes nothing and the sequence never reaches the terminal that owns the
//! window. The pane's DCS passthrough is the channel that gets out - tmux
//! forwards `ESC P tmux ; <payload> ESC \` verbatim to the outer terminal,
//! which is then the one honoured ([`passthrough`]; every `ESC` in the payload
//! is doubled). tmux 3.3 and newer only forward it while the window option
//! `allow-passthrough` is on, so [`allow_passthrough`] switches it on for this
//! window and returns the previous value for [`restore_passthrough`] to put
//! back at exit; older tmux has no such option and always passes through.
//!
//! The outer terminal also carries tmux's own chrome - the status line - so
//! asking it for `rows` leaves the pane at `rows - chrome`. [`client_sizes`]
//! reads the sizes tmux reports for this pane and [`outer_rows`] adds the
//! chrome back. Only a pane that fills its window is fitted at all
//! ([`owns_window`]): in a split the other panes' share must not be traded for
//! a shorter bar.

use std::process::Command;

/// What tmux reports for the pane: `(client_height, pane_height, window_height)`
/// - outer terminal rows, our rows, the window's content rows (all panes, no
///   status line).
const SIZE_FORMAT: &str = "#{client_height} #{pane_height} #{window_height}";

/// The pane the bar runs in, when it runs inside tmux: `TMUX` is what tmux
/// exports to every process in a pane, `TMUX_PANE` names ours (missing only in
/// odd setups, where tmux's current pane is a fine default).
pub fn session(tmux: Option<&str>, pane: Option<&str>) -> Option<String> {
    tmux.filter(|v| !v.trim().is_empty())?;
    Some(pane.unwrap_or_default().to_string())
}

/// The tmux DCS passthrough envelope around `payload`, every `ESC` doubled as
/// tmux requires. The caller writes the result to the pane.
pub fn passthrough(payload: &str) -> String {
    let mut out = String::with_capacity(payload.len() + 12);
    out.push_str("\x1bPtmux;");
    for ch in payload.chars() {
        if ch == '\x1b' {
            out.push('\x1b');
        }
        out.push(ch);
    }
    out.push_str("\x1b\\");
    out
}

/// Parse `#{client_height} #{pane_height} #{window_height}`. `None` when tmux
/// printed anything else (a missing value, a detached client, ...).
pub fn parse_sizes(out: &str) -> Option<(u16, u16, u16)> {
    let mut it = out.split_whitespace();
    let client = it.next()?.parse().ok()?;
    let pane = it.next()?.parse().ok()?;
    let window = it.next()?.parse().ok()?;
    Some((client, pane, window))
}

/// Whether our pane fills the window, so resizing the window resizes the bar.
/// In a split it does not: the fit leaves the window alone rather than
/// shrinking everybody's share.
pub fn owns_window(pane: u16, window: u16) -> bool {
    pane >= window
}

/// Rows to ask the outer terminal for so that our pane ends up `want` rows
/// tall: the window is the pane plus tmux's chrome (the status line).
pub fn outer_rows(want: u16, client: u16, pane: u16) -> u16 {
    want.saturating_add(client.saturating_sub(pane))
}

/// `(client_height, pane_height, window_height)` for `pane`, as tmux reports
/// them for a request from inside it. `None` when tmux cannot be asked.
pub fn client_sizes(pane: &str) -> Option<(u16, u16, u16)> {
    let mut args = vec!["display-message", "-p"];
    args.extend(target_args(pane));
    args.push(SIZE_FORMAT);
    parse_sizes(&run(&args)?)
}

/// The outer window height that leaves our pane at `want` rows, or `None` when
/// this pane must not be fitted: tmux cannot report its sizes, or a split
/// shares the window height with other panes (which [`owns_window`] rules out).
pub fn outer_request(pane: &str, want: u16) -> Option<u16> {
    let (client, pane_rows, window) = client_sizes(pane)?;
    (pane_rows > 0 && owns_window(pane_rows, window)).then(|| outer_rows(want, client, pane_rows))
}

/// Ask this tmux to let pane output through to the outer terminal (tmux 3.3+
/// gates that behind the window option `allow-passthrough`). Returns the value
/// [`restore_passthrough`] needs at exit: the previous setting, or an empty
/// string for "was unset". `None` when this tmux has no such option (3.2 and
/// older pass through always) or when switching it on failed.
pub fn allow_passthrough(pane: &str) -> Option<String> {
    let previous = option_value(pane, "allow-passthrough")?;
    if previous == "on" {
        return Some(previous);
    }
    run(&set_args("-w", pane, &["allow-passthrough", "on"]))?;
    Some(previous)
}

/// Put the `allow-passthrough` value from [`allow_passthrough`] back: an
/// explicit value is set again, an empty one unsets the window option, and
/// `None` (no such option) does nothing. Best effort - the bar is on its way
/// out, a failing tmux must not turn that into an error.
pub fn restore_passthrough(pane: &str, previous: Option<&str>) {
    let Some(previous) = previous else { return };
    let args = if previous.is_empty() {
        set_args("-uw", pane, &["allow-passthrough"])
    } else {
        set_args("-w", pane, &["allow-passthrough", previous])
    };
    let _ = run(&args);
}

/// The window option value as tmux resolves it for `pane`, inherited global
/// value included. `None` when this tmux does not have the option at all.
fn option_value(pane: &str, name: &str) -> Option<String> {
    let mut args = vec!["show-options", "-wv", "-A"];
    args.extend(target_args(pane));
    args.push(name);
    // An option that exists but is unset anywhere prints nothing: not an
    // error, and the caller turns the empty value into "unset it again".
    run(&args)
}

/// `set-option` argv for the flags + pane target + option/value tail.
fn set_args(flags: &str, pane: &str, tail: &[&str]) -> Vec<String> {
    let mut args = vec!["set-option".to_string(), flags.to_string()];
    args.extend(target_args(pane).into_iter().map(str::to_string));
    args.extend(tail.iter().map(|s| s.to_string()));
    args
}

/// `-t <pane>`, or nothing when the pane id is unknown (tmux then means "the
/// current pane", which is exactly right inside our own pane).
fn target_args(pane: &str) -> Vec<&str> {
    if pane.is_empty() {
        Vec::new()
    } else {
        vec!["-t", pane]
    }
}

/// Run tmux, capturing trimmed stdout; `None` on a missing tmux or a non-zero
/// exit (an unknown command in an old tmux, a dead server, ...).
fn run<S: AsRef<str>>(args: &[S]) -> Option<String> {
    let out = Command::new("tmux")
        .args(args.iter().map(AsRef::as_ref))
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_tmux_pane_is_a_session() {
        assert_eq!(session(None, Some("%3")), None, "no TMUX: not in tmux");
        assert_eq!(session(Some(""), Some("%3")), None, "empty TMUX");
        assert_eq!(session(Some("  "), Some("%3")), None);
        assert_eq!(
            session(Some("/tmp/tmux-0/default,1,0"), None),
            Some(String::new())
        );
        assert_eq!(
            session(Some("/tmp/tmux-0/default,1,0"), Some("%7")),
            Some("%7".to_string())
        );
    }

    #[test]
    fn passthrough_doubles_escapes() {
        assert_eq!(
            passthrough("\x1b[8;4;80t"),
            "\x1bPtmux;\x1b\x1b[8;4;80t\x1b\\"
        );
        // Plain text needs no doubling, and both payload escapes must be.
        assert_eq!(passthrough("ab"), "\x1bPtmux;ab\x1b\\");
        assert_eq!(
            passthrough("\x1ba\x1b"),
            "\x1bPtmux;\x1b\x1ba\x1b\x1b\x1b\\"
        );
    }

    #[test]
    fn sizes_parse_or_not() {
        assert_eq!(parse_sizes("24 23 23"), Some((24, 23, 23)));
        assert_eq!(parse_sizes(" 4 3 3 \n"), Some((4, 3, 3)));
        assert_eq!(parse_sizes(""), None, "detached client");
        assert_eq!(parse_sizes("24 23"), None, "missing window height");
        assert_eq!(parse_sizes("? 23 23"), None);
    }

    #[test]
    fn only_a_full_height_pane_owns_the_window() {
        assert!(owns_window(23, 23));
        assert!(!owns_window(11, 23), "a split shares the height");
    }

    #[test]
    fn outer_rows_adds_tmux_chrome() {
        assert_eq!(outer_rows(4, 24, 23), 5, "one status row");
        assert_eq!(outer_rows(16, 24, 23), 17);
        assert_eq!(outer_rows(4, 24, 24), 4, "status off");
        assert_eq!(outer_rows(4, 23, 24), 4, "garbage never underflows");
    }
}
