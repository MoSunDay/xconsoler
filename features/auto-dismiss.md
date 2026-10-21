# Auto-dismiss after a run (closed 2026-10-21)

The bar used to close only in `--summon` mode, and only after a proven
success. Two user-visible rules now hold in every mode:

- **History records proven success only** — `ExecOutcome::Success` appends
  the entry and saves the store. `Started` (a backgrounded job still running
  after the grace) and `Failure` leave history untouched.
- **A handed-off run dismisses the bar** — both `Success` and `Started` set
  `app.quit`. `Started` cannot prove success, but the command has left the
  launcher; keeping the bar open would only hide the window the run opened.
  `Failure` keeps the bar open so the input can be fixed and retried.

`summon` still controls Esc/wake-key behavior; it no longer gates the
post-run quit. The pty smoke check exercised all three outcomes: success
quit + one history entry, `false` stayed open with empty history, and
`sleep 3 &` quit with empty history.

Verified with `cargo test` (471), `cargo clippy --all-targets -D warnings`,
and the pty smoke check against the deployed `/usr/local/bin/xconsoler`.
