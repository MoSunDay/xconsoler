# pw plugin — built-in password generator (2026-09-26)

First entry in the new `src/plugins/` home for built-in `@native` mini-tools.

## What landed
- `src/plugins/mod.rs` — plugin registry: `is_native(template)` + `run_native(template, input) -> ExecOutcome`; tool #2 is a one-line addition.
- `src/plugins/pw.rs` (+ `pw/tests.rs`) — `@native pw` backend. Profiles are plain `const` data: `simple` (lower+digit, 8), `medium` (+upper, 16), `complex` (default, +specials, 32); trailing number overrides length (min = active set count, max 128). Spec grammar `[simple|medium|complex] [length]`, seeded shortcuts `s`/`m`/`c`.
- Entropy: `/dev/urandom`, std only (no rand dep); `below()` rejection sampling (limit = 256 - 256%k) kills modulo bias; pure `build(req, entropy)` core makes generation deterministically testable; one char per active set + fill + Fisher-Yates.
- Runtime: password copied via `crate::clipboard::copy` and echoed in the status line (`pw ok: <pw> (copied)`); copy failure is still Success. History keeps only the spec, never the password.

## Wiring
- `exec.rs::run_alias` dispatches `plugins::is_native` after the launch branch.
- `alias::defaults()` seeds `pw`; schema v5 → v6 backfills it (`migrate`, `< 6`, `append_default_alias`); v5 base64-shortcuts work untouched.
- `cli.rs`/`main.rs`: `--print-pass <spec>` headless (prints password, exit 0; bad spec exit 2). Bare `--print-pass` = complex/32.
- Tests adapted to 4 defaults across alias/app/settings/bookmarks/storage suites (469 at landing; 470 after the review guard test).

## Gotchas
- `--print-pass complex 24` (unquoted) fails: the parser takes one argv value — quote multi-token specs (review P3-1, open polish).
- Root `store.json` is v3; `pw` appends in memory on load, persists on next save (same as the v4 `app` seed).
- Review P2-1 fixed (2026-09-26): the optional-value arm now refuses argv values starting with `--` — `--print-pass --summon` keeps summon and gives print-pass the empty spec (single-dash specs like `-5` still flow to the pw parser). Regression test: `parse_args_print_pass_does_not_eat_a_following_flag` (cli tests).
- Review P3-2 done (2026-09-26): "shell-safe specials" claim dropped (README + pw module doc); `!`/interactive-bash-history caveat documented.
- Open discussion (review P2-2): the status line echoes the plaintext password by design (README); a masked / clipboard-only mode is the candidate follow-up for shoulder-surfing or shared-terminal setups.
- Deployed (2026-09-26 23:51) to node02 (192.168.31.57) by mistake - the stale
  `xc-deploy target of record` comment named it; node02 now also carries the new
  build (verified: `--print-pass --print-rows` -> 4/exit 0, bare = 32-char complex,
  simple=8, medium 16=16, bad spec exit 2, `--print-bind` intact).
- Deployed (2026-09-26, correct target) `scripts/xc-deploy root@192.168.31.196`
  (`amos`, the box the session ssh's FROM - confirmed via `SSH_CONNECTION`): same
  new binary + helpers, same on-target verification passed. Lesson: identify the
  deploy target from the live ssh topology (`who` / `$SSH_CONNECTION`), never from
  a remembered host in a comment.
