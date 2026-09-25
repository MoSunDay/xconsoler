//! Command execution for aliases (template expansion + `sh -c`).

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::alias::AliasDef;
use crate::clipboard;
use crate::platform::{self, Platform};

/// Result of running an alias command.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecOutcome {
    /// The command ran and exited zero: the only outcome worth recording.
    Success(String),
    /// The command ran and failed visibly.
    Failure(String),
    /// A backgrounded command was still running when the grace period ran
    /// out, so it never proved success (nor failed visibly). Callers must not
    /// record it: a launch that only *may* have worked must not enter the
    /// history. The job itself stays detached and keeps running.
    Started(String),
}

const STDIN_MARKER: &str = "@stdin";
const INPUT_PLACEHOLDER: &str = "{input}";
/// How long a backgrounded job gets to prove itself (exit zero) or die
/// visibly. After this the run is only `Started`.
const BG_GRACE_SECS: &str = "1";
/// Exit status the probe reports when that grace period expires while the job
/// is still running. 125 is `timeout(1)`'s "the command did not finish" code
/// and is not produced by a shell itself.
const BG_STILL_RUNNING: i32 = 125;

/// True when a template backgrounds its own work, i.e. ends in a single `&`
/// (`&&` is a shell operator, not a backgrounded job).
fn backgrounds(template: &str) -> bool {
    let t = template.trim_end();
    t.ends_with('&') && !t.ends_with("&&")
}

/// Probe appended to a backgrounding template. Without it `sh` exits 0 the
/// instant the job is forked, so a launch that dies at once (missing binary,
/// bad argument) still looked like a success and entered the history. `$!` is
/// the job's pid:
///
/// - empty: the template backgrounded nothing (`echo x \&`), a plain success;
/// - exits inside the grace: reaped, its real status decides;
/// - still alive when the grace expires: the watchdog subshell TERMs the
///   probe, whose trap exits `BG_STILL_RUNNING` -- the job neither proved
///   nor disproved success, and is left running detached.
///
/// The watchdog also bounds how long the bar can block, which matters
/// because a backgrounded job keeps running after the probe is gone.
fn bg_probe(grace: &str) -> String {
    // The `\`-continuations below eat the newline and the next line's leading
    // whitespace, so the produced command keeps exactly one space per gap.
    // `$$` inside the subshell is still the probe shell's pid, and the trap
    // turns the watchdog's TERM into a plain exit status. The fast path kills
    // the watchdog with SIGKILL: dash can swallow a TERM that lands in the
    // first instants of the subshell's life, and a watchdog that survives
    // turns a fast success into a false "still running" a second later.
    format!(
        "p=$!; if [ -z \"$p\" ]; then exit 0; fi; \
         trap 'exit {BG_STILL_RUNNING}' TERM; \
         ( sleep {grace}; kill -TERM $$ ) & w=$!; \
         wait \"$p\"; rc=$?; \
         kill -9 \"$w\" 2>/dev/null; wait \"$w\" 2>/dev/null; exit \"$rc\""
    )
}

/// Quote a string into a single safe shell word using POSIX single quotes:
/// inner single quotes are rewritten as `'\''`.
pub fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Resolve **registered shortcuts** in `rest`, the input after the alias
/// trigger word.
///
/// If the first whitespace-separated token of `rest` is a key of
/// `def.shortcuts`, that token is replaced by the mapped value and the
/// remaining tokens are appended after it (`br baidu -incognito` →
/// `https://baidu.com -incognito` when `baidu` maps to the URL). Otherwise —
/// no key match, no shortcuts at all, empty rest, or a whitespace-only value —
/// the trimmed `rest` is returned unchanged, so aliases without registered
/// shortcuts behave exactly as before.
pub fn resolve_shortcuts(def: &AliasDef, rest: &str) -> String {
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    let tail = parts.next().unwrap_or("").trim();
    match def.shortcuts.get(head) {
        Some(value) if !value.trim().is_empty() => {
            let mut out = value.trim().to_string();
            if !tail.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(tail);
            }
            out
        }
        _ => trimmed.to_string(),
    }
}

/// The template `def` runs on `platform`: the platform's own non-blank
/// command, else the other platform's non-blank command, else `None` (no
/// command configured at all).
fn chosen_template(def: &AliasDef, platform: Platform) -> Option<&str> {
    let (primary, fallback) = match platform {
        Platform::Linux => (def.linux.as_deref(), def.macos.as_deref()),
        Platform::Macos => (def.macos.as_deref(), def.linux.as_deref()),
    };
    match (primary, fallback) {
        (Some(t), _) if !t.trim().is_empty() => Some(t),
        // Single-command aliases (":add t <cmd>", the settings wizard's empty
        // macos answer) must run on both platforms, not just the one they
        // were written for.
        (_, Some(t)) if !t.trim().is_empty() => Some(t),
        _ => None,
    }
}

/// True when `def` dispatches to the native clipboard backend on `platform`
/// (its chosen template is `@native clipboard`).
pub fn uses_native_clipboard(def: &AliasDef, platform: Platform) -> bool {
    chosen_template(def, platform).is_some_and(clipboard::is_native)
}

/// Run `def` for `input` on `platform`.
///
/// Template handling:
///
/// 1. missing (or blank) command for the current platform -> the other
///    platform's command is used when it is present; only both missing is a
///    `Failure("no command configured for <platform>")`
/// 2. `@stdin` stripped from the command; input is written to child stdin
/// 3. `{input}` replaced with `shell_quote(input)`; empty (whitespace-only)
///    input -> `Failure("input required")`
///
/// stdout is piped and discarded on success so the TUI stays clean; stderr
/// (tail) and the exit code are reported on failure.
pub fn run_alias(def: &AliasDef, input: &str, platform: Platform) -> ExecOutcome {
    let template = match chosen_template(def, platform) {
        Some(t) => t,
        None => {
            return ExecOutcome::Failure(format!(
                "no command configured for {}",
                platform::name(platform)
            ))
        }
    };

    // Built-in native backends (`@native clipboard`) bypass the shell and
    // the {input}/@stdin conventions: the input here is the decoded payload
    // (the base64 form is storage/input-side, decoded by `crate::run`).
    if clipboard::is_native(template) {
        return match clipboard::copy(input) {
            Ok(()) => ExecOutcome::Success(format!("{} ok", def.name)),
            Err(msg) => ExecOutcome::Failure(msg),
        };
    }

    let needs_stdin = template.contains(STDIN_MARKER);
    let mut cmd = if needs_stdin {
        // sh treats the leftover doubled spaces as a single separator.
        template.replace(STDIN_MARKER, " ").trim().to_string()
    } else {
        template.to_string()
    };

    if cmd.contains(INPUT_PLACEHOLDER) {
        if input.trim().is_empty() {
            return ExecOutcome::Failure("input required".to_string());
        }
        cmd = cmd.replace(INPUT_PLACEHOLDER, &shell_quote(input));
    }

    let bg = backgrounds(&cmd);
    if bg {
        cmd.push(' ');
        cmd.push_str(&bg_probe(BG_GRACE_SECS));
    }

    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        // Own process group: a summon bar closes its terminal as soon as a
        // run succeeds, and the hangup SIGHUPs the terminal's foreground
        // group -- a browser that is still starting up would die with it.
        // The new group is entered before `sh` execs, so there is no race.
        .process_group(0)
        // stdin piped only when the template asks for it, otherwise null so
        // the child can never steal keystrokes from the TUI
        .stdin(if needs_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        // A detached job must not inherit the bar's pipes: it would hold the
        // read end open for its whole lifetime, freezing `wait_with_output`
        // until the job exits, and would die with SIGPIPE once the bar does.
        // Diagnostics for a backgrounded run come from its exit status alone.
        .stdout(if bg { Stdio::null() } else { Stdio::piped() })
        .stderr(if bg { Stdio::null() } else { Stdio::piped() })
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return ExecOutcome::Failure(format!("spawn failed: {e}")),
    };

    if needs_stdin {
        if let Some(stdin) = child.stdin.take() {
            let mut stdin = stdin;
            // Inputs are short (clipboard snippets, URLs), so this write
            // cannot fill the pipe buffer. For very large inputs the write
            // should happen on a helper thread while stdout/stderr drain,
            // otherwise the child could block on a full output pipe.
            let _ = stdin.write_all(input.as_bytes());
            // stdin drops here, closing the pipe (EOF) for the child
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return ExecOutcome::Failure(format!("wait failed: {e}")),
    };

    if output.status.success() {
        ExecOutcome::Success(format!("{} ok", def.name))
    } else if bg && output.status.code() == Some(BG_STILL_RUNNING) {
        // The watchdog fired: the job is still running, so nothing about it
        // is proven and the history must stay untouched.
        ExecOutcome::Started(format!("{} started", def.name))
    } else {
        let stderr_tail = tail_chars(&String::from_utf8_lossy(&output.stderr), 200);
        match output.status.code() {
            Some(code) => {
                ExecOutcome::Failure(failure_message(&format!("exit {code}"), &stderr_tail))
            }
            None => ExecOutcome::Failure(failure_message("terminated by signal", &stderr_tail)),
        }
    }
}

/// `exit 3: <stderr tail>`, with the tail dropped when the command wrote
/// nothing to stderr (always the case for a backgrounded run).
fn failure_message(head: &str, stderr_tail: &str) -> String {
    let tail = stderr_tail.trim();
    if tail.is_empty() {
        head.to_string()
    } else {
        format!("{head}: {tail}")
    }
}

/// Last `n` chars of `s` (char-based, safe for multi-byte stderr).
fn tail_chars(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len <= n {
        s.to_string()
    } else {
        s.chars().skip(len - n).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn def(name: &str, linux: &str) -> AliasDef {
        AliasDef {
            name: name.to_string(),
            triggers: vec![],
            linux: Some(linux.to_string()),
            macos: None,
            shortcuts: BTreeMap::new(),
        }
    }

    fn def_both(name: &str, linux: Option<&str>, macos: Option<&str>) -> AliasDef {
        AliasDef {
            name: name.to_string(),
            triggers: vec![],
            linux: linux.map(str::to_string),
            macos: macos.map(str::to_string),
            shortcuts: BTreeMap::new(),
        }
    }

    #[test]
    fn quote_wraps_in_single_quotes() {
        assert_eq!(shell_quote("abc"), "'abc'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("a b'c"), "'a b'\\''c'");
    }

    fn def_with_shortcuts(name: &str, linux: &str, args: &[(&str, &str)]) -> AliasDef {
        let mut d = def(name, linux);
        for (k, v) in args {
            d.shortcuts.insert(k.to_string(), v.to_string());
        }
        d
    }

    #[test]
    fn resolve_shortcuts_without_args_returns_trimmed_rest() {
        let d = def("t", "printf %s {input}");
        assert_eq!(resolve_shortcuts(&d, "hello world"), "hello world");
        assert_eq!(resolve_shortcuts(&d, "  spaced  "), "spaced");
    }

    #[test]
    fn resolve_shortcuts_replaces_matching_key() {
        let d = def_with_shortcuts(
            "br",
            "xdg-open {input}",
            &[("baidu", "https://www.baidu.com")],
        );
        assert_eq!(resolve_shortcuts(&d, "baidu"), "https://www.baidu.com");
        // surrounding whitespace is trimmed before the lookup
        assert_eq!(
            resolve_shortcuts(&d, "   baidu   "),
            "https://www.baidu.com"
        );
    }

    #[test]
    fn resolve_shortcuts_appends_remaining_tokens_after_value() {
        let d = def_with_shortcuts(
            "br",
            "xdg-open {input}",
            &[("baidu", "https://www.baidu.com")],
        );
        assert_eq!(
            resolve_shortcuts(&d, "baidu extra tokens"),
            "https://www.baidu.com extra tokens"
        );
    }

    #[test]
    fn resolve_shortcuts_keeps_unmatched_first_token() {
        let d = def_with_shortcuts(
            "br",
            "xdg-open {input}",
            &[("baidu", "https://www.baidu.com")],
        );
        assert_eq!(
            resolve_shortcuts(&d, "google.com search"),
            "google.com search"
        );
    }

    #[test]
    fn resolve_shortcuts_empty_rest_stays_empty() {
        let d = def_with_shortcuts(
            "br",
            "xdg-open {input}",
            &[("baidu", "https://www.baidu.com")],
        );
        assert_eq!(resolve_shortcuts(&d, ""), "");
        assert_eq!(resolve_shortcuts(&d, "   "), "");
    }

    #[test]
    fn resolve_shortcuts_value_may_contain_spaces() {
        let d = def_with_shortcuts("run", "sh -c {input}", &[("here", "cd /tmp && ls")]);
        assert_eq!(resolve_shortcuts(&d, "here -la"), "cd /tmp && ls -la");
        assert_eq!(resolve_shortcuts(&d, "here"), "cd /tmp && ls");
    }

    #[test]
    fn resolve_shortcuts_ignores_whitespace_only_value() {
        // a blank mapping would silently eat the input; treat it as no match
        let d = def_with_shortcuts("t", "printf %s {input}", &[("blank", "   ")]);
        assert_eq!(resolve_shortcuts(&d, "blank tail"), "blank tail");
    }

    #[test]
    fn uses_native_clipboard_detects_native_templates_and_fallbacks() {
        // Both platforms native.
        let both = def_both("cd", Some(clipboard::TEMPLATE), Some(clipboard::TEMPLATE));
        assert!(uses_native_clipboard(&both, Platform::Linux));
        assert!(uses_native_clipboard(&both, Platform::Macos));

        // Linux blank: the linux run falls back to the native macOS command.
        let fallback = def_both("cd", Some(""), Some(clipboard::TEMPLATE));
        assert!(uses_native_clipboard(&fallback, Platform::Linux));
        assert!(uses_native_clipboard(&fallback, Platform::Macos));
    }

    #[test]
    fn uses_native_clipboard_is_false_for_shell_and_blank_defs() {
        let shell = def("br", "xdg-open {input}");
        assert!(!uses_native_clipboard(&shell, Platform::Linux));
        // macOS falls back to the shell command, which is not native either.
        assert!(!uses_native_clipboard(&shell, Platform::Macos));

        let both_blank = def_both("x", Some("   "), None);
        assert!(!uses_native_clipboard(&both_blank, Platform::Linux));
        assert!(!uses_native_clipboard(&both_blank, Platform::Macos));
    }

    #[test]
    fn missing_platform_command_falls_back_to_the_other_platform() {
        // macOS without a macos command: the linux one serves both.
        match run_alias(&def("browser", "printf %s {input}"), "x", Platform::Macos) {
            ExecOutcome::Success(msg) => assert_eq!(msg, "browser ok"),
            other => panic!("expected the linux fallback to run: {other:?}"),
        }
        // ... and the same the other way round.
        let macos_only = AliasDef {
            name: "browser".to_string(),
            triggers: vec![],
            linux: None,
            macos: Some("printf %s {input}".to_string()),
            shortcuts: BTreeMap::new(),
        };
        assert_eq!(
            run_alias(&macos_only, "x", Platform::Linux),
            ExecOutcome::Success("browser ok".to_string())
        );
        // A blank field counts as missing, not as a command.
        let blank_linux = AliasDef {
            name: "blank".to_string(),
            triggers: vec![],
            linux: Some("   ".to_string()),
            macos: Some("printf %s {input}".to_string()),
            shortcuts: BTreeMap::new(),
        };
        assert_eq!(
            run_alias(&blank_linux, "x", Platform::Linux),
            ExecOutcome::Success("blank ok".to_string())
        );
    }

    #[test]
    fn missing_commands_on_both_platforms_fail() {
        let none = AliasDef {
            name: "e".to_string(),
            triggers: vec![],
            linux: None,
            macos: None,
            shortcuts: BTreeMap::new(),
        };
        match run_alias(&none, "x", Platform::Linux) {
            ExecOutcome::Failure(msg) => {
                assert!(msg.contains("no command configured for linux"), "{msg}")
            }
            other => panic!("expected failure, got {other:?}"),
        }
        match run_alias(&none, "x", Platform::Macos) {
            ExecOutcome::Failure(msg) => {
                assert!(msg.contains("no command configured for macos"), "{msg}")
            }
            other => panic!("expected failure, got {other:?}"),
        }
        // Blank on both sides is just as missing.
        let blank = AliasDef {
            linux: Some("   ".to_string()),
            macos: Some(String::new()),
            ..none
        };
        assert!(matches!(
            run_alias(&blank, "x", Platform::Linux),
            ExecOutcome::Failure(_)
        ));
        assert!(matches!(
            run_alias(&blank, "x", Platform::Macos),
            ExecOutcome::Failure(_)
        ));
    }

    #[test]
    fn empty_input_with_placeholder_fails() {
        match run_alias(&def("browser", "printf %s {input}"), "   ", Platform::Linux) {
            ExecOutcome::Failure(msg) => assert_eq!(msg, "input required"),
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[test]
    fn runs_real_command_successfully() {
        let d = def("echo-test", "printf %s {input}");
        assert_eq!(
            run_alias(&d, "hi", Platform::Linux),
            ExecOutcome::Success("echo-test ok".to_string())
        );
    }

    #[test]
    fn stdin_marker_delivers_input() {
        let d = def("clip-sim", "cat @stdin");
        assert_eq!(
            run_alias(&d, "payload", Platform::Linux),
            ExecOutcome::Success("clip-sim ok".to_string())
        );
    }

    #[test]
    fn failing_command_reports_exit_code() {
        match run_alias(&def("bad", "exit 7"), "", Platform::Linux) {
            ExecOutcome::Failure(msg) => assert!(msg.contains('7'), "{msg}"),
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[test]
    fn failing_command_includes_stderr_tail() {
        let d = def("loud", "echo boom >&2; exit 3");
        match run_alias(&d, "", Platform::Linux) {
            ExecOutcome::Failure(msg) => {
                assert!(msg.contains('3'), "{msg}");
                assert!(msg.contains("boom"), "{msg}");
            }
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[test]
    fn child_runs_in_its_own_process_group() {
        // A summon bar tears its terminal down right after a successful run,
        // and the hangup SIGHUPs the terminal's foreground group: a browser
        // still starting up would be killed. Children must not share it.
        let out = std::env::temp_dir().join(format!("xc-pgid-{}.txt", std::process::id()));
        let target = shell_quote(&out.display().to_string());
        let d = def("pg", &format!("ps -o pgid= -p $$ > {target}"));
        assert_eq!(
            run_alias(&d, "", Platform::Linux),
            ExecOutcome::Success("pg ok".to_string())
        );
        let child_pgid = std::fs::read_to_string(&out).unwrap().trim().to_string();
        let _ = std::fs::remove_file(&out);
        let own = Command::new("sh")
            .arg("-c")
            .arg("ps -o pgid= -p $$")
            .output()
            .unwrap();
        let own_pgid = String::from_utf8_lossy(&own.stdout).trim().to_string();
        assert!(!child_pgid.is_empty());
        assert_ne!(child_pgid, own_pgid, "child must leave the bar's group");
    }

    #[test]
    fn backgrounds_only_accepts_a_single_trailing_ampersand() {
        assert!(backgrounds("firefox {input} &"));
        assert!(backgrounds("firefox {input} >/dev/null 2>&1 &  "));
        assert!(!backgrounds("firefox {input}"));
        assert!(!backgrounds("test -n {input} && firefox"));
        assert!(!backgrounds("echo \"a &\""));
    }

    #[test]
    fn the_probe_command_has_no_stray_whitespace() {
        let probe = bg_probe("0.5");
        let expected = concat!(
            "p=$!; if [ -z \"$p\" ]; then exit 0; fi; ",
            "trap 'exit 125' TERM; ",
            "( sleep 0.5; kill -TERM $$ ) & w=$!; ",
            "wait \"$p\"; rc=$?; ",
            "kill -9 \"$w\" 2>/dev/null; wait \"$w\" 2>/dev/null; exit \"$rc\""
        );
        assert_eq!(probe, expected);
        assert!(!probe.contains("  "), "stray double space in {probe:?}");
    }

    #[test]
    fn the_probe_reports_the_real_status_of_a_fast_background_failure() {
        // The job dies with a real non-zero code inside the grace period, so
        // the probe must reap and report that code, not a generic one. A
        // backgrounded run has no stderr, so the message is the code alone.
        let d = def("bg", "false >/dev/null 2>&1 &");
        match run_alias(&d, "", Platform::Linux) {
            ExecOutcome::Failure(msg) => assert_eq!(msg, "exit 1"),
            other => panic!("must report exit 1, got {other:?}"),
        }
    }

    #[test]
    fn the_probe_treats_an_empty_job_pid_as_success() {
        // `\&` is a literal ampersand, so the shell backgrounds nothing and
        // leaves `$!` empty -- the `[ -z "$p" ]` branch must call that a
        // success instead of failing on a `wait` without a pid.
        let template = "echo a \\&";
        assert!(backgrounds(template), "the probe must be appended");
        match run_alias(&def("bg", template), "", Platform::Linux) {
            ExecOutcome::Success(msg) => assert_eq!(msg, "bg ok"),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn the_probe_reports_a_backgrounded_launch_that_dies_at_once() {
        let d = def("bg", "nosuchbin_xconsoler_probe {input} >/dev/null 2>&1 &");
        match run_alias(&d, "x", Platform::Linux) {
            ExecOutcome::Failure(msg) => assert!(msg.contains("127"), "{msg}"),
            other => panic!("must fail, got {other:?}"),
        }
    }

    #[test]
    fn the_probe_calls_a_job_that_outlives_the_grace_started() {
        // Living on is not proof of success: `xdg-open` with no browser
        // installed also lives on (it is stuck), and recording it was the
        // bug that put failed launches into the history.
        let d = def("bg", "sleep 2 >/dev/null 2>&1 &");
        match run_alias(&d, "", Platform::Linux) {
            ExecOutcome::Started(msg) => assert_eq!(msg, "bg started"),
            other => panic!("expected started, got {other:?}"),
        }
    }

    #[test]
    fn the_probe_reports_a_backgrounded_failure_inside_the_grace() {
        // Failing late -- after the old fixed 0.2s window -- must still count
        // as a failure, not as a success.
        let d = def("bg", "(sleep 0.3; exit 9) >/dev/null 2>&1 &");
        match run_alias(&d, "", Platform::Linux) {
            ExecOutcome::Failure(msg) => assert!(msg.contains("exit 9"), "{msg}"),
            other => panic!("expected exit 9, got {other:?}"),
        }
    }

    #[test]
    fn the_probe_accepts_a_fast_backgrounded_success() {
        let d = def("bg", "true >/dev/null 2>&1 &");
        assert!(matches!(
            run_alias(&d, "", Platform::Linux),
            ExecOutcome::Success(_)
        ));
    }

    #[test]
    fn the_probe_does_not_wait_for_the_program_to_finish() {
        let d = def("bg", "sleep 5 >/dev/null 2>&1 &");
        let t0 = std::time::Instant::now();
        let outcome = run_alias(&d, "", Platform::Linux);
        let elapsed = t0.elapsed();
        assert!(matches!(outcome, ExecOutcome::Started(_)), "{outcome:?}");
        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "the bar blocked for {elapsed:?}"
        );
    }
}
