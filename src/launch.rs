//! Launching an application by name (`@native app {input}`).
//!
//! The `app` alias is a native backend like `cd`: no shell. What it adds is
//! name resolution -- the input is matched against the applications installed
//! on this machine, so `app xx` reaches the same program the desktop menu
//! does. Resolution lives in [`desktop`]; this module turns a match into a
//! detached process and reports what happened on the status line.
//!
//! [`pinyin`] supplies initials acronyms, so a Chinese name is reachable
//! through its pinyin shorthand (`app wjglq`).

use std::collections::BTreeMap;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::alias::AliasDef;
use crate::platform::Platform;

pub(crate) mod pinyin;
// Generated data (`scripts/gen-pinyin.py`) spells `concat!(...)` with a
// single argument for the empty and one-chunk groups; clippy 1.98 flags that
// as `useless_concat` and the table must not be edited by hand, so the lint
// is allowed at the module.
pub mod desktop;
#[allow(clippy::useless_concat)]
mod pinyin_table;

#[cfg(test)]
mod tests;

/// Marker template handled by this module instead of the shell.
pub const TEMPLATE: &str = "@native app";

/// True when an alias template selects the native application launcher.
pub fn is_native(template: &str) -> bool {
    template.trim() == TEMPLATE
}

/// The seeded `app` alias: the native backend on both platforms, with no
/// triggers or registered shortcuts (the input is the application name).
pub fn default_def() -> AliasDef {
    AliasDef {
        name: "app".to_string(),
        triggers: vec![],
        linux: Some(TEMPLATE.to_string()),
        macos: Some(TEMPLATE.to_string()),
        shortcuts: BTreeMap::new(),
    }
}

/// An application to launch, plus what the status line should say about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// Display label of the entry, or the URL itself on macOS.
    pub label: String,
    /// Where it came from: desktop file path, `.app` path, or the URL.
    pub source: String,
    /// Raw `Exec=` value, when the entry carries one.
    pub exec: Option<String>,
}

/// One-line description of a resolved target (`label (source)`).
pub fn describe(target: &Resolved) -> String {
    format!("{} ({})", target.label, target.source)
}

/// Result of matching an input against the installed applications.
#[derive(Debug, Clone, PartialEq)]
pub enum Match {
    /// Exactly one label in the top-scoring group.
    One(Resolved),
    /// Several distinct labels match equally well; the caller shows them.
    Ambiguous(Vec<String>),
    /// Nothing matched.
    None,
}

/// Pure half of [`resolve`]: rank `entries` for `input` on its own.
///
/// The top-scoring group decides: one distinct label means "this is it",
/// several mean the user has to type more. Guessing between two differently
/// named apps would send `app xx` to the wrong window.
pub fn resolve_from(entries: &[desktop::Entry], input: &str) -> Match {
    let top = desktop::best(entries, input);
    let Some(first) = top.first() else {
        return Match::None;
    };
    let mut labels: Vec<String> = Vec::new();
    for entry in &top {
        if !labels.contains(&entry.name) {
            labels.push(entry.name.clone());
        }
    }
    if labels.len() > 1 {
        return Match::Ambiguous(labels);
    }
    Match::One(Resolved {
        label: first.name.clone(),
        source: first.path.display().to_string(),
        exec: first.exec.clone(),
    })
}

/// [`resolve_from`] over the applications installed on `platform`.
pub fn resolve(input: &str, platform: Platform) -> Match {
    url_fallback(
        input,
        platform,
        resolve_from(&desktop::scan(platform), input),
    )
}

/// On macOS a URL passes through when no application matches: `open` hands it
/// to the default browser, the same thing the desktop would do.
fn url_fallback(input: &str, platform: Platform, matched: Match) -> Match {
    if platform != Platform::Macos || !matches!(matched, Match::None) {
        return matched;
    }
    let url = input.trim();
    if !url.contains("://") {
        return matched;
    }
    Match::One(Resolved {
        label: url.to_string(),
        source: url.to_string(),
        exec: None,
    })
}

/// Status-line text for [`Match::Ambiguous`]: the first three labels, with a
/// trailing ellipsis when the list continues.
pub fn ambiguous_message(input: &str, labels: &[String]) -> String {
    let shown = labels
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let more = if labels.len() > 3 { ", ..." } else { "" };
    format!("{input} matches {} apps: {shown}{more}", labels.len())
}

/// What became of a launch attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The launcher ran and exited zero: the app is on its way. History may
    /// record it.
    Launched(String),
    /// Still running when the grace period expired: likely the app itself,
    /// but nothing proves it -- history must not record it.
    Started(String),
    /// Nothing was launched; the payload is the message for the status line.
    Failed(String),
}

/// How long a spawned launcher gets to prove itself (exited zero) or die
/// visibly (nonzero). Mirrors `exec`'s background grace: a launcher that is
/// still running may have worked, but history records only proven runs.
const GRACE_MS: u64 = 1000;
/// Poll step while waiting for the launcher to prove itself.
const POLL_MS: u64 = 10;

/// Resolve `input` and start the match, detached from the bar.
pub fn launch(input: &str, platform: Platform) -> Outcome {
    if input.trim().is_empty() {
        return Outcome::Failed("app name required".to_string());
    }
    match resolve(input, platform) {
        Match::None => Outcome::Failed(format!("no app matches: {input}")),
        Match::Ambiguous(labels) => Outcome::Failed(ambiguous_message(input, &labels)),
        Match::One(target) => spawn(&target, platform),
    }
}

/// Spawn the resolved target and give it [`GRACE_MS`] to prove itself.
///
/// The child is detached: null stdio, its own process group, and never
/// `wait`ed for past the grace. The bar exits on a successful run and the
/// app must survive that, and its output must never scribble over the TUI.
fn spawn(target: &Resolved, platform: Platform) -> Outcome {
    let Some(mut command) = command_for(target, platform) else {
        return Outcome::Failed(nothing_to_launch(&target.label));
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Own process group: the hangup that follows the bar dismissing
        // itself must not kill an app that is still starting up.
        .process_group(0);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(err) => return Outcome::Failed(format!("{}: {err}", target.label)),
    };
    wait_grace(child, &target.label)
}

/// Launcher command for `target`, best first.
///
/// Linux prefers the desktop's own launchers: `gtk-launch` by id, then
/// `gio launch` on the file, both of which handle startup notification and
/// DBus activation. With neither installed, the entry's own `Exec=` runs
/// through `setsid sh -c`, a new session so the app leaves the bar behind.
/// macOS has exactly one answer: `open` on the `.app` bundle or URL.
fn command_for(target: &Resolved, platform: Platform) -> Option<Command> {
    if platform == Platform::Macos {
        let mut command = Command::new("open");
        command.arg(&target.source);
        return Some(command);
    }
    if desktop::on_path("gtk-launch") {
        if let Some(stem) = Path::new(&target.source).file_stem() {
            let mut command = Command::new("gtk-launch");
            command.arg(stem);
            return Some(command);
        }
    }
    if desktop::on_path("gio") {
        let mut command = Command::new("gio");
        command.arg("launch").arg(&target.source);
        return Some(command);
    }
    let exec = target
        .exec
        .as_deref()
        .filter(|exec| !exec.trim().is_empty())?;
    let mut command = Command::new("setsid");
    command.arg("sh").arg("-c").arg(strip_field_codes(exec));
    Some(command)
}

/// Text for a Linux entry with no launcher left to try.
fn nothing_to_launch(label: &str) -> String {
    format!("{label}: no gtk-launch, gio or Exec= to start it")
}

/// Drop the desktop spec's `%`-field codes (`%U`, `%f`, `%i`, ...): they are
/// placeholders for files or URLs the entry expects, and passing them through
/// to the app would show up as literal arguments. `%%` survives as `%`.
fn strip_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some(code) if "fFuUdDnNickvm".contains(code) => {}
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out.trim().to_string()
}

/// Give a freshly spawned launcher up to [`GRACE_MS`] to exit, polling in
/// [`POLL_MS`] steps.
///
/// Only an exit proves something: zero means the app was handed over, a
/// nonzero code (bad desktop file, no display) must fail the run instead of
/// entering the history. A process still running at the deadline is `Started`
/// and left detached; dropping its handle does not kill it.
fn wait_grace(mut child: Child, label: &str) -> Outcome {
    let deadline = Instant::now() + Duration::from_millis(GRACE_MS);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Outcome::Launched(label.to_string()),
            Ok(Some(status)) => return Outcome::Failed(exit_message(label, status)),
            Ok(None) if Instant::now() >= deadline => return Outcome::Started(label.to_string()),
            Ok(None) => std::thread::sleep(Duration::from_millis(POLL_MS)),
            Err(err) => return Outcome::Failed(format!("{label}: launcher probe failed: {err}")),
        }
    }
}

/// Failure text for a launcher that exited: its code, or the signal that
/// killed it.
fn exit_message(label: &str, status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("{label}: launcher exited {code}"),
        None => format!(
            "{label}: launcher killed by signal {}",
            status.signal().unwrap_or(0)
        ),
    }
}
