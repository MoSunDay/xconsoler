# AGENTS.md — repository-local memory for xconsoler

Working memory for coding agents. Keep entries short and factual; per-task
records live in `features/` and are appended below when a task closes.

## Project snapshot

Rust TUI keyboard launcher (ratatui + crossterm): a summoned bar where an
alias + input runs a mapped command; working runs enter history. Store is
`store.json` (schema v6, seeds `br` / `cd` / `app` / `pw`).

## Module map

- `src/run.rs` — input → alias dispatch; `src/exec.rs` — template expansion,
  `{input}` / `@stdin`, native-backend dispatch
- `src/launch.rs` + `src/launch/` — `@native app`: desktop-entry scan and
  name matching (names, keywords, exec basename, pinyin initials); a `://`
  input matching no entry opens in the default browser (`open`/`xdg-open`)
- `src/alias.rs`, `src/storage/` — alias defs and the store; `load()` is NOT
  read-only: out-of-date schema files are rewritten eagerly (`materialize`)
- `src/plugins/` — home for built-in `@native` mini-tools (`mod.rs` registry); `pw` = password generator (simple/medium/complex profiles)
- `src/cli.rs` — headless flags (`--print-app`, `--print-pass`, `--print-rows`, `--print-bind`)
- `scripts/xc-{bar,key,icon,wechat,deploy}` — runtime/deploy helpers
  (`xc-wechat` installs the `assets/wechat-web.desktop` template)

## Conventions

- New files < 400 lines, files under iteration < 800; pure functions, no classes
- Commits: lowercase area prefix (`feat(app):`, `fix(run):`, `matcher:` …)
- Tests are colocated (`src/**/tests.rs`, `*_tests.rs`) and environment-independent
- `.probe/` is scratch space (gitignored)

## Task memory index

- `features/app-chrome-wechat.md` — wiring the `app` alias to Chrome WeChat
  (blueprint applied 2026-09-26: `xc-wechat` entry + linux URL passthrough)
- `features/store-eager-migration.md` — `load()` materializes old-store
  migrations (v5 base64 was lazy: plaintext lived on disk until the next
  save; fix shipped 2026-09-27 as `78577fa`)
- `features/pw-plugin.md` — built-in `pw` password generator: `src/plugins/`
  home, store v6 seed, `--print-pass` (shipped 2026-09-26 as `ae7c14c`)
