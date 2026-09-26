//! Built-in mini-tools (`@native <tool>` plugins).
//!
//! Plugins are self-contained features dispatched through marker templates
//! (`@native pw`) instead of `sh -c`, the same way [`crate::clipboard`] and
//! [`crate::launch`] bypass the shell. They live here so the feature set can
//! grow without widening those modules: every plugin owns one file, exports
//! `TEMPLATE`/`is_native`/`run` (and a `default_def` when it seeds an
//! alias), and registers itself in the two functions below - tool #2 is a
//! one-line addition.
//!
//! Dispatch order in [`crate::exec::run_alias`]: clipboard, launch, then
//! this registry; an unknown marker is a `Failure`, never a shell command.

pub mod pw;

use crate::exec::ExecOutcome;

/// True when the template selects any plugin's native backend.
pub fn is_native(template: &str) -> bool {
    pw::is_native(template)
}

/// Dispatch a plugin template to its backend.
pub fn run_native(template: &str, input: &str) -> ExecOutcome {
    if pw::is_native(template) {
        return pw::run(input);
    }
    ExecOutcome::Failure(format!("unknown native plugin: {template}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_native_plugin_is_a_failure_not_a_shell_run() {
        let outcome = run_native("@native nope", "");
        assert_eq!(
            outcome,
            ExecOutcome::Failure("unknown native plugin: @native nope".to_string())
        );
    }

    #[test]
    fn registry_dispatches_the_known_plugin_only() {
        assert!(is_native(pw::TEMPLATE));
        // Clipboard and launch are their own backends, not plugin templates.
        assert!(!is_native("@native clipboard"));
        assert!(!is_native("@native app"));
        assert!(!is_native("echo {input}"));
        assert!(matches!(
            run_native(pw::TEMPLATE, "bogus!"),
            ExecOutcome::Failure(_)
        ));
    }
}
