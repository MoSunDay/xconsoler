//! Command execution for aliases (template expansion + `sh -c`).

use std::io::Write;
use std::process::{Command, Stdio};

use crate::alias::AliasDef;
use crate::clipboard;
use crate::platform::{self, Platform};

/// Result of running an alias command.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecOutcome {
    Success(String),
    Failure(String),
}

const STDIN_MARKER: &str = "@stdin";
const INPUT_PLACEHOLDER: &str = "{input}";

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

/// Resolve **named arguments** in `rest`, the input after the alias trigger
/// word.
///
/// If the first whitespace-separated token of `rest` is a key of `def.args`,
/// that token is replaced by the mapped value and the remaining tokens are
/// appended after it (`br baidu -incognito` → `https://baidu.com -incognito`
/// when `baidu` maps to the URL). Otherwise — no key match, no args at all,
/// empty rest, or a whitespace-only value — the trimmed `rest` is returned
/// unchanged, so aliases without named args behave exactly as before.
pub fn resolve_args(def: &AliasDef, rest: &str) -> String {
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    let tail = parts.next().unwrap_or("").trim();
    match def.args.get(head) {
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

/// Run `def` for `input` on `platform`.
///
/// Template handling:
///
/// 1. missing command for the platform -> `Failure`
/// 2. `@stdin` stripped from the command; input is written to child stdin
/// 3. `{input}` replaced with `shell_quote(input)`; empty (whitespace-only)
///    input -> `Failure("input required")`
///
/// stdout is piped and discarded on success so the TUI stays clean; stderr
/// (tail) and the exit code are reported on failure.
pub fn run_alias(def: &AliasDef, input: &str, platform: Platform) -> ExecOutcome {
    let template = match platform {
        Platform::Linux => def.linux.as_deref(),
        Platform::Macos => def.macos.as_deref(),
    };
    let template = match template {
        Some(t) if !t.trim().is_empty() => t,
        _ => {
            return ExecOutcome::Failure(format!(
                "no command configured for {}",
                platform::name(platform)
            ))
        }
    };

    // Built-in native backends (`@native clipboard`) bypass the shell and
    // the {input}/@stdin conventions: the raw input is the payload.
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

    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        // stdin piped only when the template asks for it, otherwise null so
        // the child can never steal keystrokes from the TUI
        .stdin(if needs_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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
    } else {
        let stderr_tail = tail_chars(&String::from_utf8_lossy(&output.stderr), 200);
        match output.status.code() {
            Some(code) => ExecOutcome::Failure(format!("exit {code}: {stderr_tail}")),
            None => ExecOutcome::Failure(format!("terminated by signal: {stderr_tail}")),
        }
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
            shortcuts: vec![],
            linux: Some(linux.to_string()),
            macos: None,
            args: BTreeMap::new(),
            builtin: false,
        }
    }

    #[test]
    fn quote_wraps_in_single_quotes() {
        assert_eq!(shell_quote("abc"), "'abc'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("a b'c"), "'a b'\\''c'");
    }

    fn def_with_args(
        name: &str,
        linux: &str,
        args: &[(&str, &str)],
    ) -> AliasDef {
        let mut d = def(name, linux);
        for (k, v) in args {
            d.args.insert(k.to_string(), v.to_string());
        }
        d
    }

    #[test]
    fn resolve_args_without_args_returns_trimmed_rest() {
        let d = def("t", "printf %s {input}");
        assert_eq!(resolve_args(&d, "hello world"), "hello world");
        assert_eq!(resolve_args(&d, "  spaced  "), "spaced");
    }

    #[test]
    fn resolve_args_replaces_matching_key() {
        let d = def_with_args("br", "xdg-open {input}", &[("baidu", "https://www.baidu.com")]);
        assert_eq!(resolve_args(&d, "baidu"), "https://www.baidu.com");
        // surrounding whitespace is trimmed before the lookup
        assert_eq!(resolve_args(&d, "   baidu   "), "https://www.baidu.com");
    }

    #[test]
    fn resolve_args_appends_remaining_tokens_after_value() {
        let d = def_with_args("br", "xdg-open {input}", &[("baidu", "https://www.baidu.com")]);
        assert_eq!(
            resolve_args(&d, "baidu extra tokens"),
            "https://www.baidu.com extra tokens"
        );
    }

    #[test]
    fn resolve_args_keeps_unmatched_first_token() {
        let d = def_with_args("br", "xdg-open {input}", &[("baidu", "https://www.baidu.com")]);
        assert_eq!(resolve_args(&d, "google.com search"), "google.com search");
    }

    #[test]
    fn resolve_args_empty_rest_stays_empty() {
        let d = def_with_args("br", "xdg-open {input}", &[("baidu", "https://www.baidu.com")]);
        assert_eq!(resolve_args(&d, ""), "");
        assert_eq!(resolve_args(&d, "   "), "");
    }

    #[test]
    fn resolve_args_value_may_contain_spaces() {
        let d = def_with_args("run", "sh -c {input}", &[("here", "cd /tmp && ls")]);
        assert_eq!(resolve_args(&d, "here -la"), "cd /tmp && ls -la");
        assert_eq!(resolve_args(&d, "here"), "cd /tmp && ls");
    }

    #[test]
    fn resolve_args_ignores_whitespace_only_value() {
        // a blank mapping would silently eat the input; treat it as no match
        let d = def_with_args("t", "printf %s {input}", &[("blank", "   ")]);
        assert_eq!(resolve_args(&d, "blank tail"), "blank tail");
    }

    #[test]
    fn missing_platform_command_fails() {
        match run_alias(&def("browser", "xdg-open {input}"), "x", Platform::Macos) {
            ExecOutcome::Failure(msg) => {
                assert!(msg.contains("no command configured for macos"), "{msg}")
            }
            ExecOutcome::Success(_) => panic!("expected failure"),
        }
        let empty = AliasDef {
            name: "e".to_string(),
            shortcuts: vec![],
            linux: Some("   ".to_string()),
            macos: None,
            args: BTreeMap::new(),
            builtin: false,
        };
        assert!(matches!(
            run_alias(&empty, "x", Platform::Linux),
            ExecOutcome::Failure(_)
        ));
    }

    #[test]
    fn empty_input_with_placeholder_fails() {
        match run_alias(&def("browser", "printf %s {input}"), "   ", Platform::Linux) {
            ExecOutcome::Failure(msg) => assert_eq!(msg, "input required"),
            ExecOutcome::Success(_) => panic!("expected failure"),
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
            ExecOutcome::Success(_) => panic!("expected failure"),
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
            ExecOutcome::Success(_) => panic!("expected failure"),
        }
    }
}
