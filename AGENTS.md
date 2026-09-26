# AGENTS.md — repository-local memory for xconsoler

Working memory for coding agents. Keep entries short and factual; per-task
records live in `features/` and are appended below when a task closes.

## Project snapshot

Rust TUI keyboard launcher (ratatui + crossterm): a summoned bar where an
alias + input runs a mapped command; working runs enter history. Store is
`store.json` (schema v3, seeds `br` / `cd` / `app`).

## Module map

- `src/run.rs` — input → alias dispatch; `src/exec.rs` — template expansion,
  `{input}` / `@stdin`, native-backend dispatch
- `src/launch.rs` + `src/launch/` — `@native app`: desktop-entry scan and
  name matching (names, keywords, exec basename, pinyin initials)
- `src/alias.rs`, `src/storage/` — alias defs and the store
- `src/cli.rs` — headless flags (`--print-app`, `--print-rows`, `--print-bind`)
- `scripts/xc-{bar,key,icon,deploy}` — runtime/deploy helpers

## Conventions

- New files < 400 lines, files under iteration < 800; pure functions, no classes
- Commits: lowercase area prefix (`feat(app):`, `fix(run):`, `matcher:` …)
- Tests are colocated (`src/**/tests.rs`, `*_tests.rs`) and environment-independent
- `.probe/` is scratch space (gitignored)

## Task memory index

- `features/app-chrome-wechat.md` — wiring the `app` alias to Chrome WeChat
  (plan + verified state, closed 2026-09-26)
